//! Finite complete reference analysis and trusted in-memory storage fixtures.
//! These tests establish Engine state/binding semantics, not sandbox security or
//! filesystem persistence. Actual storage and isolation have separate probes.

use celetixo_engine::access::AccessSet;
use celetixo_engine::credentials::CredentialPolicy;
use celetixo_engine::engine::*;
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::repository::{RepositoryAnalysis, RepositoryAnalysisInput};
use celetixo_engine::snapshot::*;
use celetixo_engine::source_evidence::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use celetixo_engine::transaction::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

fn d(value: &str) -> Digest {
    Digest::of("source-evidence-reference/v1", value.as_bytes())
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("org").unwrap(),
        workspace: WorkspaceId::new("ws").unwrap(),
        repository: RepositoryId::new("repo").unwrap(),
    }
}
fn cid() -> CheckId {
    CheckId::new("verify").unwrap()
}
fn element() -> ElementId {
    ElementId::new("file:main").unwrap()
}
fn resources() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 500,
        memory_bytes: 1024 * 1024,
        disk_bytes: 1024 * 1024,
        process_count: 4,
        file_descriptors: 32,
        stdout_bytes: 4096,
        stderr_bytes: 4096,
        artifact_bytes: 1024 * 1024,
    }
}
fn backend() -> BackendIdentity {
    BackendIdentity {
        name: "reference-only".into(),
        version: "v1".into(),
        image: d("image"),
        profile: d("profile"),
    }
}

type ReadHook = Box<dyn FnOnce() + Send>;
struct ReferenceReader {
    objects: Mutex<BTreeMap<Digest, Vec<u8>>>,
    hook: Mutex<Option<ReadHook>>,
    unavailable: AtomicBool,
    reads: AtomicUsize,
}
impl ReferenceReader {
    fn new() -> Self {
        Self {
            objects: Mutex::new(BTreeMap::new()),
            hook: Mutex::new(None),
            unavailable: AtomicBool::new(false),
            reads: AtomicUsize::new(0),
        }
    }
    fn put(&self, name: &str, media: &str, bytes: &[u8]) -> SourceArtifactReference {
        let object = SourceArtifactReference::for_bytes(
            scope(),
            name.into(),
            media.into(),
            bytes,
            d("retention"),
        );
        self.objects
            .lock()
            .unwrap()
            .insert(object.content, bytes.to_vec());
        object
    }
    fn on_read(&self, callback: impl FnOnce() + Send + 'static) {
        *self.hook.lock().unwrap() = Some(Box::new(move || {
            let (done, receiver) = std::sync::mpsc::sync_channel(1);
            let worker = std::thread::spawn(move || {
                callback();
                let _ = done.send(());
            });
            receiver.recv_timeout(Duration::from_secs(3))
                .expect("storage callback could not reacquire Engine authority; artifact I/O may hold the Engine mutex");
            worker.join().unwrap();
        }));
    }
}
impl SourceArtifactReader for ReferenceReader {
    fn read(
        &self,
        requested_scope: &Scope,
        address: Digest,
        max_bytes: u64,
    ) -> Result<Vec<u8>, SourceArtifactError> {
        if requested_scope != &scope() {
            return Err(SourceArtifactError::Scope);
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        let callback = self.hook.lock().unwrap().take();
        if let Some(callback) = callback {
            callback();
        }
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(SourceArtifactError::Persistence(
                "reference storage outage".into(),
            ));
        }
        let bytes = self
            .objects
            .lock()
            .unwrap()
            .get(&address)
            .cloned()
            .ok_or_else(|| SourceArtifactError::Unavailable(address.to_string()))?;
        if bytes.len() as u64 > max_bytes {
            return Err(SourceArtifactError::Limit);
        }
        Ok(bytes)
    }
    fn retention_policy(&self) -> Digest {
        d("retention")
    }
}

/// Models an inconsistent storage read, not an authenticated executor. Returning
/// a valid address on one read and different bytes on the next must never let
/// hashing and canonical comparison validate separate objects.
struct InconsistentReader {
    backing: Arc<ReferenceReader>,
    address: Digest,
    first: Vec<u8>,
    later: Vec<u8>,
    target_reads: AtomicUsize,
}
impl SourceArtifactReader for InconsistentReader {
    fn read(
        &self,
        requested_scope: &Scope,
        address: Digest,
        max_bytes: u64,
    ) -> Result<Vec<u8>, SourceArtifactError> {
        if requested_scope != &scope() {
            return Err(SourceArtifactError::Scope);
        }
        if address != self.address {
            return self.backing.read(requested_scope, address, max_bytes);
        }
        let bytes = if self.target_reads.fetch_add(1, Ordering::SeqCst) == 0 {
            &self.first
        } else {
            &self.later
        };
        if bytes.len() as u64 > max_bytes {
            return Err(SourceArtifactError::Limit);
        }
        Ok(bytes.clone())
    }
    fn retention_policy(&self) -> Digest {
        d("retention")
    }
}

struct RacingReader {
    backing: Arc<ReferenceReader>,
    first_reads: Mutex<usize>,
    both_reading: Condvar,
}
impl SourceArtifactReader for RacingReader {
    fn read(
        &self,
        scope: &Scope,
        address: Digest,
        max_bytes: u64,
    ) -> Result<Vec<u8>, SourceArtifactError> {
        let mut count = self.first_reads.lock().unwrap();
        if *count < 2 {
            *count += 1;
            if *count == 2 {
                self.both_reading.notify_all();
            } else {
                let (count_after_wait, timeout) = self
                    .both_reading
                    .wait_timeout_while(count, Duration::from_secs(5), |count| *count < 2)
                    .unwrap();
                assert!(
                    !timeout.timed_out(),
                    "competing evidence delivery could not reach storage I/O"
                );
                count = count_after_wait;
            }
        }
        drop(count);
        self.backing.read(scope, address, max_bytes)
    }
    fn retention_policy(&self) -> Digest {
        d("retention")
    }
}

struct Attempt {
    transaction: TransactionId,
    patch: SourcePatch,
    admission: SourceAdmission,
    applied: AppliedSourcePatch,
}
struct Bundle {
    evidence: SourceEvidence,
    envelope: SourceArtifactReference,
}
struct Harness {
    admin: Administrator,
    engine: Engine,
    owner: Session,
    observer: Session,
    executor: SourceExecutorCapability,
    reader: Arc<ReferenceReader>,
    storage: SourceArtifactStoreCapability,
    base: Snapshot,
    policy: SourceExecutionPolicy,
}
impl Harness {
    fn new() -> Self {
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
        let owner = admin
            .grant(
                PrincipalId::new("human").unwrap(),
                scope(),
                Permission::ALL.into(),
            )
            .unwrap();
        let observer = admin
            .grant(
                PrincipalId::new("reader").unwrap(),
                scope(),
                [Permission::Read].into(),
            )
            .unwrap();
        let base = Snapshot::new(
            scope(),
            Revision(0),
            SnapshotManifest {
                base_commit: "finite-reference".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "celetixo/reference-source/v1".into(),
                inputs: vec![WorkspaceInput {
                    path: "src/main.rs".into(),
                    layer: InputLayer::Base,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(b"old".to_vec()),
                }],
                environment: BTreeMap::new(),
            },
            State::new(),
        )
        .unwrap();
        let analysis = RepositoryAnalysis::new(
            &base,
            RepositoryAnalysisInput {
                index_generation: d("index"),
                analyzer_version: "complete-finite-reference/v1".into(),
                configuration: d("config"),
                elements: [element()].into(),
                dependencies: BTreeMap::new(),
                checks: [(element(), [cid()].into())].into(),
                gaps: BTreeSet::new(),
            },
        )
        .unwrap();
        engine
            .register_repository(&owner, base.clone(), analysis.clone())
            .unwrap();
        let definition = SourceCheckDefinition::new(SourceCheckInput {
            check: CheckSpec::new(
                cid(),
                vec!["/tools/reference-check".into()],
                "reference/v1".into(),
                d("environment"),
                d("check-config"),
            )
            .unwrap(),
            working_directory: WorkingDirectory::WorkspaceRoot,
            inputs: [(
                "src/main.rs".into(),
                Digest::of("celetixo/source-content/v1", b"new"),
            )]
            .into(),
            outputs: BTreeSet::new(),
            toolchain: ToolchainIdentity {
                executable: "/tools/reference-check".into(),
                executable_digest: d("executable"),
                runtime_closure: d("runtime"),
                version: "reference/v1".into(),
                platform: "linux".into(),
                architecture: "x86_64".into(),
            },
            dependency_inputs: BTreeMap::new(),
            environment: BTreeMap::new(),
            tier: ExecutionTier::T1,
            backend: backend(),
            resources: resources(),
            network: NetworkPolicy::DenyAll,
            credential_references: BTreeSet::new(),
            expected_artifacts: [
                ArtifactRequirement {
                    name: "stdout".into(),
                    media_type: "text/plain".into(),
                    max_bytes: 4096,
                    required: true,
                },
                ArtifactRequirement {
                    name: "stderr".into(),
                    media_type: "text/plain".into(),
                    max_bytes: 4096,
                    required: true,
                },
                ArtifactRequirement {
                    name: "diagnostics".into(),
                    media_type: "application/json".into(),
                    max_bytes: 4096,
                    required: true,
                },
            ]
            .into(),
            depends_on: BTreeSet::new(),
            failure_policy: FailurePolicy::FailAttempt,
        })
        .unwrap();
        let policy = SourceExecutionPolicy::new(SourcePolicyInput {
            version: 1,
            generation: 1,
            scope: scope(),
            snapshot: base.id(),
            analysis_artifact: analysis.digest(),
            index_generation: analysis.index_generation(),
            capture: SourceCaptureBinding {
                snapshot: base.id(),
                manifest: base.manifest().digest(),
                capture_policy: d("capture"),
                uncertainty: SourceCaptureBinding::empty_uncertainty_digest(),
                consistency: SourceCaptureConsistency::ReferenceFixture,
                gaps: BTreeSet::new(),
            },
            patch_policy: PatchPolicy::default(),
            credentials: CredentialPolicy::default(),
            paths: [(
                "src/main.rs".into(),
                SourcePathRule {
                    allowed_operations: [OperationKind::Replace].into(),
                    required_writes: [element()].into(),
                    expected_base: Some(Preimage::of(&base.manifest().inputs[0]).unwrap()),
                },
            )]
            .into(),
            checks: vec![definition],
            invariants: vec![],
            always_required: BTreeSet::new(),
            approved_tiers: [ExecutionTier::T1].into(),
            approved_backends: [backend().digest()].into(),
            resource_ceiling: resources(),
            network_allowed: false,
            approved_credential_references: BTreeSet::new(),
            expires_at_ms: 10_000,
            retention_policy: d("retention"),
        })
        .unwrap();
        engine
            .install_source_policy(&owner, &scope(), None, policy.clone())
            .unwrap();
        let executor = admin
            .grant_source_executor(
                PrincipalId::new("trusted-executor").unwrap(),
                scope(),
                backend(),
                [ExecutionTier::T1].into(),
                resources(),
                10_000,
            )
            .unwrap();
        let reader = Arc::new(ReferenceReader::new());
        let storage = admin
            .register_source_artifact_store(scope(), reader.clone())
            .unwrap();
        Self {
            admin,
            engine,
            owner,
            observer,
            executor,
            reader,
            storage,
            base,
            policy,
        }
    }
    fn start(&self) -> Attempt {
        let transaction = TransactionId::new("transaction").unwrap();
        self.engine
            .propose(
                &self.owner,
                &scope(),
                transaction.clone(),
                self.base.id(),
                TransactionIntent {
                    objective: "finite reference source verification".into(),
                    accesses: AccessSet {
                        writes: [element()].into(),
                        ..Default::default()
                    },
                    invariants: vec![],
                    required_checks: vec![self.policy.checks()[&cid()].input().check.clone()],
                    depends_on: BTreeSet::new(),
                },
            )
            .unwrap();
        self.engine
            .analyze(&self.owner, &scope(), &transaction)
            .unwrap();
        self.engine
            .reserve(&self.owner, &scope(), &transaction, 1000)
            .unwrap();
        self.engine
            .execute(&self.owner, &scope(), &transaction)
            .unwrap();
        self.retry(&transaction, "attempt-1")
    }
    fn retry(&self, transaction: &TransactionId, source_attempt: &str) -> Attempt {
        let context = self
            .engine
            .source_patch_context(&self.owner, &scope(), transaction)
            .unwrap();
        let binding = context
            .bind(
                SourceAttemptId::new(source_attempt).unwrap(),
                "reference fixture".into(),
                500,
            )
            .unwrap();
        let patch = SourcePatch::new(
            binding,
            self.policy.input().patch_policy.clone(),
            vec![PatchOperation::Replace {
                path: "src/main.rs".into(),
                expected: Preimage::of(&self.base.manifest().inputs[0]).unwrap(),
                entry: PatchEntry::file(b"new".to_vec(), FileMode::Regular).unwrap(),
            }],
        )
        .unwrap();
        let admission = self
            .engine
            .admit_source_attempt(
                &self.owner,
                &scope(),
                patch.clone(),
                IdempotencyKey::new(source_attempt).unwrap(),
            )
            .unwrap();
        self.engine
            .claim_source_attempt(&self.executor, &admission)
            .unwrap();
        let applied = self
            .engine
            .source_application(&self.executor, &admission)
            .unwrap();
        self.engine
            .mark_source_patch_applied(&self.executor, &admission)
            .unwrap();
        self.engine
            .begin_source_verification(&self.executor, &admission)
            .unwrap();
        Attempt {
            transaction: transaction.clone(),
            patch,
            admission,
            applied,
        }
    }
    fn input(&self, attempt: &Attempt) -> SourceCheckRecordInput {
        let check = attempt.admission.plan().checks()[0].definition().input();
        let started = self
            .engine
            .source_execution_time(&self.executor, &attempt.admission)
            .unwrap();
        SourceCheckRecordInput {
            admission: attempt.admission.id(),
            plan: attempt.admission.plan().id(),
            candidate: attempt.admission.resulting_snapshot(),
            check: cid(),
            definition: attempt.admission.plan().checks()[0].definition().digest(),
            backend: check.backend.digest(),
            executable: check.toolchain.executable_digest,
            runtime_closure: check.toolchain.runtime_closure,
            environment: check.check.environment(),
            limits: check.resources,
            started_at_ms: started,
            finished_at_ms: started + 10,
            outcome: SourceCheckOutcome::Exited { code: 0 },
            usage: SourceResourceUsage {
                wall_time_ms: 10,
                cpu_time_us: 100,
                peak_memory_bytes: 500,
            },
            post_source_manifest: Some(attempt.admission.resulting_manifest()),
            termination: SourceTermination::Confirmed,
            cleanup: SourceCleanup::Confirmed,
            stdout_truncated: false,
            stderr_truncated: false,
            artifacts: [
                (
                    "stdout".into(),
                    self.reader
                        .put("stdout", "text/plain", b"verified output\n"),
                ),
                (
                    "stderr".into(),
                    self.reader.put("stderr", "text/plain", b""),
                ),
                (
                    "diagnostics".into(),
                    self.reader.put("diagnostics", "application/json", b"[]"),
                ),
            ]
            .into(),
            diagnostics: vec![],
        }
    }
    fn provenance(&self, attempt: &Attempt) -> BTreeMap<String, SourceArtifactReference> {
        [
            ("source-patch", attempt.patch.canonical_bytes()),
            ("base-snapshot", self.base.canonical_bytes()),
            (
                "result-snapshot",
                attempt.applied.snapshot().canonical_bytes(),
            ),
            ("base-manifest", self.base.manifest().canonical_bytes()),
            (
                "result-manifest",
                attempt.applied.snapshot().manifest().canonical_bytes(),
            ),
            (
                "application-report",
                attempt.applied.report().canonical_bytes(),
            ),
            (
                "verification-plan",
                attempt.admission.plan().canonical_bytes(),
            ),
            ("execution-admission", attempt.admission.canonical_bytes()),
            ("execution-policy", self.policy.canonical_bytes()),
            (
                "check-definition/verify",
                attempt.admission.plan().checks()[0]
                    .definition()
                    .canonical_bytes(),
            ),
        ]
        .into_iter()
        .map(|(name, bytes)| {
            (
                name.into(),
                self.reader
                    .put(name, "application/vnd.celetixo.provenance-v1", &bytes),
            )
        })
        .collect()
    }
    fn seal(&self, attempt: &Attempt, input: SourceCheckRecordInput) -> Bundle {
        self.seal_with(attempt, input, self.provenance(attempt), true)
    }
    fn seal_with(
        &self,
        attempt: &Attempt,
        mut input: SourceCheckRecordInput,
        provenance: BTreeMap<String, SourceArtifactReference>,
        include_record: bool,
    ) -> Bundle {
        if include_record {
            let bytes = SourceCheckRecord::new(input.clone())
                .unwrap()
                .execution_bytes();
            input.artifacts.insert(
                "execution-record".into(),
                self.reader.put(
                    "execution-record",
                    "application/vnd.celetixo.source-check-v1",
                    &bytes,
                ),
            );
        }
        let started = input.started_at_ms;
        let finished = input.finished_at_ms;
        let evidence = SourceEvidence::new(
            SourceEvidenceBinding::for_admission(
                &attempt.admission,
                self.executor.principal().clone(),
            ),
            started,
            finished,
            vec![SourceCheckRecord::new(input).unwrap()],
            provenance,
            vec![],
        )
        .unwrap();
        let envelope = self.reader.put(
            "sealed-evidence",
            SOURCE_EVIDENCE_MEDIA_TYPE,
            &evidence.canonical_bytes(),
        );
        self.admin.advance_time(10).unwrap();
        Bundle { evidence, envelope }
    }
    fn deliver(
        &self,
        attempt: &Attempt,
        bundle: &Bundle,
    ) -> Result<SourceEvidenceReceipt, EngineError> {
        self.engine.record_source_evidence(
            &self.executor,
            &attempt.admission,
            &self.storage,
            bundle.evidence.clone(),
            bundle.envelope.clone(),
        )
    }
    fn no_retained_evidence(&self, bundle: &Bundle) {
        assert!(matches!(
            self.engine
                .source_evidence_record(&self.observer, &scope(), bundle.evidence.id()),
            Err(EngineError::NotFound)
        ));
        assert_eq!(
            self.engine.current(&self.observer, &scope()).unwrap(),
            self.base
        );
    }
}

#[test]
fn complete_bound_records_derive_pass_without_advancing_source_revision_or_granting_publication() {
    let h = Harness::new();
    let attempt = h.start();
    let bundle = h.seal(&attempt, h.input(&attempt));
    let receipt = h.deliver(&attempt, &bundle).unwrap();
    assert_eq!(receipt.assessment, SourceEvidenceAssessment::Passed);
    assert_eq!(
        h.engine
            .transaction(&h.observer, &scope(), &attempt.transaction)
            .unwrap()
            .state,
        TransactionState::Validating
    );
    assert_eq!(
        h.engine
            .source_attempt(
                &h.observer,
                &scope(),
                &attempt.admission.binding().source_attempt
            )
            .unwrap()
            .state,
        SourceAttemptState::EvidenceSealed
    );
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    assert_eq!(
        h.engine.commit(
            &h.owner,
            &scope(),
            &attempt.transaction,
            IdempotencyKey::new("publish").unwrap()
        ),
        Err(EngineError::SourceMutationUnsupported)
    );
    assert_eq!(
        h.engine
            .source_evidence_record(&h.observer, &scope(), receipt.evidence)
            .unwrap(),
        (bundle.evidence, receipt)
    );
}

#[test]
fn v2_attestation_is_identity_bound_and_an_unattested_claim_cannot_authorize_evidence() {
    let h = Harness::new();
    let attempt = h.start();
    let original = h.seal(&attempt, h.input(&attempt));
    assert_eq!(
        original.evidence.binding().attestation(),
        SourceAttestationLevel::EngineScopedExecutor
    );
    assert_eq!(original.envelope.media_type, SOURCE_EVIDENCE_MEDIA_TYPE);
    assert_eq!(
        original.evidence.id().0,
        Digest::of(SOURCE_EVIDENCE_FORMAT, &original.evidence.canonical_bytes())
    );
    assert_ne!(
        original.evidence.id().0,
        Digest::of(
            "celetixo/source-evidence/v1",
            &original.evidence.canonical_bytes()
        )
    );
    let evidence = SourceEvidence::new(
        original
            .evidence
            .binding()
            .clone()
            .with_attestation(SourceAttestationLevel::Unattested),
        original.evidence.started_at_ms(),
        original.evidence.finished_at_ms(),
        original.evidence.checks().values().cloned().collect(),
        original.evidence.provenance().clone(),
        vec![],
    )
    .unwrap();
    assert_ne!(evidence.id(), original.evidence.id());
    let envelope = h.reader.put(
        "sealed-evidence",
        SOURCE_EVIDENCE_MEDIA_TYPE,
        &evidence.canonical_bytes(),
    );
    let untrusted = Bundle { evidence, envelope };
    assert_eq!(
        h.deliver(&attempt, &untrusted),
        Err(EngineError::SourceEvidence(
            SourceEvidenceError::BindingMismatch
        ))
    );
    h.no_retained_evidence(&untrusted);
    assert_eq!(
        h.deliver(&attempt, &original).unwrap().assessment,
        SourceEvidenceAssessment::Passed
    );
}

#[test]
fn legacy_envelope_media_is_not_relabelled_as_current_attested_evidence() {
    let h = Harness::new();
    let attempt = h.start();
    let original = h.seal(&attempt, h.input(&attempt));
    let envelope = h.reader.put(
        "sealed-evidence",
        "application/vnd.celetixo.source-evidence-v1",
        &original.evidence.canonical_bytes(),
    );
    let legacy = Bundle {
        evidence: original.evidence.clone(),
        envelope,
    };
    assert!(h.deliver(&attempt, &legacy).is_err());
    h.no_retained_evidence(&legacy);
    assert_eq!(
        h.deliver(&attempt, &original).unwrap().assessment,
        SourceEvidenceAssessment::Passed
    );
}

#[test]
fn empty_check_sets_and_forged_executor_bindings_cannot_enter_the_admission_gate() {
    let h = Harness::new();
    let attempt = h.start();
    let input = h.input(&attempt);
    assert!(
        SourceEvidence::new(
            SourceEvidenceBinding::for_admission(
                &attempt.admission,
                h.executor.principal().clone()
            ),
            input.started_at_ms,
            input.finished_at_ms,
            vec![],
            h.provenance(&attempt),
            vec![]
        )
        .is_err()
    );
    let original = h.seal(&attempt, input);
    let evidence = SourceEvidence::new(
        SourceEvidenceBinding::for_admission(
            &attempt.admission,
            PrincipalId::new("forged-executor").unwrap(),
        ),
        original.evidence.started_at_ms(),
        original.evidence.finished_at_ms(),
        original.evidence.checks().values().cloned().collect(),
        original.evidence.provenance().clone(),
        vec![],
    )
    .unwrap();
    let envelope = h.reader.put(
        "sealed-evidence",
        SOURCE_EVIDENCE_MEDIA_TYPE,
        &evidence.canonical_bytes(),
    );
    let forged = Bundle { evidence, envelope };
    assert!(h.deliver(&attempt, &forged).is_err());
    h.no_retained_evidence(&forged);
}

#[test]
fn the_sealed_envelope_scope_retention_media_label_and_bytes_are_not_producer_assertions() {
    for mutation in 0..5 {
        let h = Harness::new();
        let attempt = h.start();
        let mut bundle = h.seal(&attempt, h.input(&attempt));
        match mutation {
            0 => bundle.envelope.scope.repository = RepositoryId::new("foreign").unwrap(),
            1 => bundle.envelope.retention_policy = d("wrong-retention"),
            2 => bundle.envelope.media_type = "application/wrong".into(),
            3 => bundle.envelope.name = "ordinary-log".into(),
            _ => {
                bundle.envelope = h.reader.put(
                    "sealed-evidence",
                    SOURCE_EVIDENCE_MEDIA_TYPE,
                    b"correctly addressed but false envelope",
                )
            }
        }
        assert!(h.deliver(&attempt, &bundle).is_err());
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn substituted_check_plan_candidate_backend_toolchain_environment_and_limits_are_rejected() {
    for mutation in 0..10 {
        let h = Harness::new();
        let attempt = h.start();
        let mut input = h.input(&attempt);
        match mutation {
            0 => input.check = CheckId::new("substituted").unwrap(),
            1 => input.admission = SourceAdmissionId(d("wrong-admission")),
            2 => input.plan = SourcePlanId(d("wrong-plan")),
            3 => input.candidate = SnapshotId(d("wrong-candidate")),
            4 => input.definition = d("wrong-definition"),
            5 => input.backend = d("wrong-backend"),
            6 => input.executable = d("wrong-executable"),
            7 => input.runtime_closure = d("wrong-runtime"),
            8 => input.environment = d("wrong-environment"),
            _ => input.limits.memory_bytes += 1,
        }
        let bundle = h.seal(&attempt, input);
        assert!(
            h.deliver(&attempt, &bundle).is_err(),
            "accepted substitution {mutation}"
        );
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn nonzero_and_signal_are_retained_as_failed_execution_not_verified_source() {
    for outcome in [
        SourceCheckOutcome::Exited { code: 7 },
        SourceCheckOutcome::Signalled { signal: 9 },
    ] {
        let h = Harness::new();
        let attempt = h.start();
        let mut input = h.input(&attempt);
        input.outcome = outcome;
        let bundle = h.seal(&attempt, input);
        let receipt = h.deliver(&attempt, &bundle).unwrap();
        assert_eq!(
            receipt.assessment,
            SourceEvidenceAssessment::Failed {
                checks: [cid()].into()
            }
        );
        assert_eq!(
            h.engine
                .transaction(&h.observer, &scope(), &attempt.transaction)
                .unwrap()
                .state,
            TransactionState::Executing
        );
        assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    }
}

#[test]
fn missing_results_truncation_resource_overruns_timeout_and_unknown_termination_stay_incomplete() {
    for mutation in 0..21 {
        let h = Harness::new();
        let attempt = h.start();
        let mut input = h.input(&attempt);
        match mutation {
            0 => {
                input.artifacts.remove("stdout");
            }
            1 => {
                input.artifacts.remove("stderr");
            }
            2 => {
                input.artifacts.remove("diagnostics");
            }
            3 => input.stdout_truncated = true,
            4 => input.stderr_truncated = true,
            5 => input.post_source_manifest = None,
            6 => input.post_source_manifest = Some(d("wrong-post-source")),
            7 => {
                input.termination = SourceTermination::Unknown {
                    reason: "kill uncertain".into(),
                }
            }
            8 => {
                input.cleanup = SourceCleanup::Quarantined {
                    reason: "workspace retained".into(),
                }
            }
            9 => input.usage.cpu_time_us = input.limits.cpu_time_ms * 1000 + 1,
            10 => input.usage.peak_memory_bytes = input.limits.memory_bytes + 1,
            11 => input.usage.wall_time_ms = input.limits.wall_time_ms + 1,
            12 => input.outcome = SourceCheckOutcome::TimedOut,
            13 => input.outcome = SourceCheckOutcome::Cancelled,
            14 => {
                input.outcome = SourceCheckOutcome::Infrastructure {
                    reason: "backend failed".into(),
                }
            }
            15 => {
                input.outcome = SourceCheckOutcome::UnknownTermination {
                    reason: "worker lost".into(),
                }
            }
            16 => input.outcome = SourceCheckOutcome::Expired,
            17 => input.outcome = SourceCheckOutcome::Revoked,
            18 => {
                input.outcome = SourceCheckOutcome::Skipped {
                    reason: "required check was skipped".into(),
                }
            }
            19 => {
                input.cleanup = SourceCleanup::Failed {
                    reason: "cleanup failed".into(),
                }
            }
            _ => input.outcome = SourceCheckOutcome::OutputLimit,
        }
        let bundle = h.seal(&attempt, input);
        let receipt = h.deliver(&attempt, &bundle).unwrap();
        assert!(
            matches!(
                receipt.assessment,
                SourceEvidenceAssessment::Incomplete { .. }
            ),
            "false green for mutation {mutation}"
        );
        assert_eq!(
            h.engine
                .source_attempt(
                    &h.observer,
                    &scope(),
                    &attempt.admission.binding().source_attempt
                )
                .unwrap()
                .state,
            SourceAttemptState::EvidenceIncomplete
        );
        assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    }
    let h = Harness::new();
    let attempt = h.start();
    let bundle = h.seal_with(&attempt, h.input(&attempt), h.provenance(&attempt), false);
    assert!(matches!(
        h.deliver(&attempt, &bundle).unwrap().assessment,
        SourceEvidenceAssessment::Incomplete { .. }
    ));
}

#[test]
fn diagnostics_cannot_be_waived_by_registry_configuration() {
    let h = Harness::new();
    let mut check = h.policy.checks()[&cid()].input().clone();
    check
        .expected_artifacts
        .retain(|requirement| requirement.name != "diagnostics");
    assert!(matches!(
        SourceCheckDefinition::new(check),
        Err(SourcePlanError::InvalidCheck("required execution logs"))
    ));
}

#[test]
fn pre_admission_future_and_expired_record_times_are_rejected() {
    for mutation in 0..3 {
        let h = Harness::new();
        h.admin.advance_time(100).unwrap();
        let attempt = h.start();
        let mut input = h.input(&attempt);
        match mutation {
            0 => input.started_at_ms = attempt.admission.admitted_at_ms() - 1,
            1 => input.finished_at_ms += 1,
            _ => input.finished_at_ms = attempt.admission.expires_at_ms() + 1,
        }
        let bundle = h.seal(&attempt, input);
        assert!(
            h.deliver(&attempt, &bundle).is_err(),
            "accepted unbound record time {mutation}"
        );
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn artifact_contract_substitution_and_unplanned_objects_reject_instead_of_becoming_success() {
    for mutation in 0..7 {
        let h = Harness::new();
        let attempt = h.start();
        let mut input = h.input(&attempt);
        match mutation {
            0 => {
                input.artifacts.get_mut("stdout").unwrap().scope.repository =
                    RepositoryId::new("other").unwrap()
            }
            1 => input.artifacts.get_mut("stdout").unwrap().retention_policy = d("wrong-retention"),
            2 => {
                input.artifacts.get_mut("diagnostics").unwrap().media_type =
                    "application/wrong".into()
            }
            3 => {
                input.artifacts.insert(
                    "unplanned".into(),
                    h.reader.put("unplanned", "text/plain", b"unplanned"),
                );
            }
            4 => input.artifacts.get_mut("stdout").unwrap().bytes = u64::MAX,
            5 => {
                let object = input.artifacts.get_mut("stdout").unwrap();
                object.content = d("false-address");
                h.reader
                    .objects
                    .lock()
                    .unwrap()
                    .insert(object.content, b"verified output\n".to_vec());
            }
            _ => input.artifacts.get_mut("stdout").unwrap().bytes -= 1,
        }
        let bundle = h.seal(&attempt, input);
        assert!(
            h.deliver(&attempt, &bundle).is_err(),
            "accepted artifact substitution {mutation}"
        );
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn required_global_provenance_and_execution_record_bytes_are_independently_verified() {
    for mutation in 0..11 {
        let h = Harness::new();
        let attempt = h.start();
        let input = h.input(&attempt);
        let mut provenance = h.provenance(&attempt);
        match mutation {
            0 => {
                provenance.remove("source-patch");
            }
            1 => {
                provenance.insert(
                    "base-manifest".into(),
                    h.reader.put(
                        "base-manifest",
                        "application/octet-stream",
                        b"wrong but correctly addressed base",
                    ),
                );
            }
            2 => {
                provenance.insert(
                    "verification-plan".into(),
                    h.reader.put(
                        "verification-plan",
                        "application/octet-stream",
                        b"substituted plan bytes",
                    ),
                );
            }
            3 => {
                provenance.insert(
                    "application-report".into(),
                    h.reader.put(
                        "application-report",
                        "application/octet-stream",
                        b"false report",
                    ),
                );
            }
            7 => {
                provenance.insert(
                    "base-snapshot".into(),
                    h.reader.put(
                        "base-snapshot",
                        "application/octet-stream",
                        b"wrong base snapshot",
                    ),
                );
            }
            8 => {
                provenance.insert(
                    "execution-admission".into(),
                    h.reader.put(
                        "execution-admission",
                        "application/octet-stream",
                        b"wrong admission",
                    ),
                );
            }
            9 => {
                provenance.insert(
                    "execution-policy".into(),
                    h.reader.put(
                        "execution-policy",
                        "application/octet-stream",
                        b"wrong policy",
                    ),
                );
            }
            10 => {
                provenance.insert(
                    "check-definition/verify".into(),
                    h.reader.put(
                        "check-definition/verify",
                        "application/octet-stream",
                        b"wrong check definition",
                    ),
                );
            }
            _ => {}
        }
        let bundle = h.seal_with(&attempt, input, provenance, true);
        if mutation == 4 {
            h.reader.objects.lock().unwrap().remove(
                &bundle.evidence.checks()[&cid()].input().artifacts["execution-record"].content,
            );
        }
        if mutation == 5 {
            h.reader.objects.lock().unwrap().insert(
                bundle.evidence.checks()[&cid()].input().artifacts["execution-record"].content,
                b"wrong record bytes".to_vec(),
            );
        }
        if mutation == 6 {
            h.reader
                .objects
                .lock()
                .unwrap()
                .remove(&bundle.envelope.content);
        }
        assert!(
            h.deliver(&attempt, &bundle).is_err(),
            "accepted false provenance {mutation}"
        );
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn canonical_comparison_uses_the_same_bytes_that_were_digest_verified() {
    for target in 0..3 {
        let h = Harness::new();
        let attempt = h.start();
        let mut input = h.input(&attempt);
        let mut provenance = h.provenance(&attempt);
        let canonical = match target {
            0 => Vec::new(), // The envelope is available after sealing below.
            1 => attempt.patch.canonical_bytes(),
            _ => SourceCheckRecord::new(input.clone())
                .unwrap()
                .execution_bytes(),
        };
        let mut altered = canonical.clone();
        let mut address = d("unused");
        if target != 0 {
            altered[0] ^= 1;
            let object = SourceArtifactReference::for_bytes(
                scope(),
                if target == 1 {
                    "source-patch"
                } else {
                    "execution-record"
                }
                .into(),
                if target == 1 {
                    "application/vnd.celetixo.provenance-v1"
                } else {
                    "application/vnd.celetixo.source-check-v1"
                }
                .into(),
                &altered,
                d("retention"),
            );
            address = object.content;
            if target == 1 {
                provenance.insert("source-patch".into(), object);
            } else {
                input.artifacts.insert("execution-record".into(), object);
            }
        }
        let mut bundle = h.seal_with(&attempt, input, provenance, target != 2);
        let canonical = if target == 0 {
            let bytes = bundle.evidence.canonical_bytes();
            altered = bytes.clone();
            altered[0] ^= 1;
            bundle.envelope = SourceArtifactReference::for_bytes(
                scope(),
                "sealed-evidence".into(),
                SOURCE_EVIDENCE_MEDIA_TYPE.into(),
                &altered,
                d("retention"),
            );
            address = bundle.envelope.content;
            bytes
        } else {
            canonical
        };
        let reader = Arc::new(InconsistentReader {
            backing: h.reader.clone(),
            address,
            first: altered,
            later: canonical,
            target_reads: AtomicUsize::new(0),
        });
        let storage = h
            .admin
            .register_source_artifact_store(scope(), reader.clone())
            .unwrap();
        assert!(matches!(
            h.engine.record_source_evidence(
                &h.executor,
                &attempt.admission,
                &storage,
                bundle.evidence.clone(),
                bundle.envelope.clone(),
            ),
            Err(EngineError::SourceEvidence(SourceEvidenceError::Artifact(
                SourceArtifactError::Integrity
            )))
        ));
        assert_eq!(reader.target_reads.load(Ordering::SeqCst), 1);
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn every_canonical_provenance_object_is_required_and_must_be_retrievable() {
    let h = Harness::new();
    let attempt = h.start();
    let names = h.provenance(&attempt).keys().cloned().collect::<Vec<_>>();
    assert_eq!(names.len(), 10); // Nine global records plus this check definition.
    for name in names {
        for missing_from_record in [true, false] {
            let h = Harness::new();
            let attempt = h.start();
            let mut provenance = h.provenance(&attempt);
            let address = provenance[&name].content;
            if missing_from_record {
                provenance.remove(&name);
            }
            let bundle = h.seal_with(&attempt, h.input(&attempt), provenance, true);
            if !missing_from_record {
                h.reader.objects.lock().unwrap().remove(&address);
            }
            assert!(
                h.deliver(&attempt, &bundle).is_err(),
                "accepted missing provenance {name}, missing_from_record={missing_from_record}"
            );
            h.no_retained_evidence(&bundle);
        }
    }
}

#[test]
fn every_required_check_object_must_exist_under_its_verified_address() {
    for name in ["stdout", "stderr", "diagnostics", "execution-record"] {
        let h = Harness::new();
        let attempt = h.start();
        let bundle = h.seal(&attempt, h.input(&attempt));
        let address = bundle.evidence.checks()[&cid()].input().artifacts[name].content;
        h.reader.objects.lock().unwrap().remove(&address);
        assert!(
            h.deliver(&attempt, &bundle).is_err(),
            "accepted unpersisted check artifact {name}"
        );
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn missing_required_artifact_persistence_and_injected_io_failures_retain_no_green_state() {
    for fault in [
        SourceEvidenceFault::BeforeArtifactRead,
        SourceEvidenceFault::AfterArtifactRead,
    ] {
        let h = Harness::new();
        let attempt = h.start();
        let bundle = h.seal(&attempt, h.input(&attempt));
        assert!(
            h.engine
                .record_source_evidence_with_fault(
                    &h.executor,
                    &attempt.admission,
                    &h.storage,
                    bundle.evidence.clone(),
                    bundle.envelope.clone(),
                    fault
                )
                .is_err()
        );
        h.no_retained_evidence(&bundle);
        assert_eq!(
            h.deliver(&attempt, &bundle).unwrap().assessment,
            SourceEvidenceAssessment::Passed
        );
    }
    let h = Harness::new();
    let attempt = h.start();
    let bundle = h.seal(&attempt, h.input(&attempt));
    h.reader.unavailable.store(true, Ordering::SeqCst);
    assert!(h.deliver(&attempt, &bundle).is_err());
    h.no_retained_evidence(&bundle);
    h.reader.unavailable.store(false, Ordering::SeqCst);
    assert_eq!(
        h.deliver(&attempt, &bundle).unwrap().assessment,
        SourceEvidenceAssessment::Passed
    );
}

#[test]
fn executor_and_artifact_store_scope_or_origin_cannot_be_substituted() {
    for mutation in 0..5 {
        let h = Harness::new();
        let attempt = h.start();
        let bundle = h.seal(&attempt, h.input(&attempt));
        let other = Harness::new();
        let mut wrong_scope = scope();
        wrong_scope.workspace = WorkspaceId::new("other").unwrap();
        let result = match mutation {
            0 => h.engine.record_source_evidence(
                &other.executor,
                &attempt.admission,
                &h.storage,
                bundle.evidence.clone(),
                bundle.envelope.clone(),
            ),
            1 => h.engine.record_source_evidence(
                &h.executor,
                &attempt.admission,
                &other.storage,
                bundle.evidence.clone(),
                bundle.envelope.clone(),
            ),
            2 => {
                let storage = h
                    .admin
                    .register_source_artifact_store(wrong_scope, h.reader.clone())
                    .unwrap();
                h.engine.record_source_evidence(
                    &h.executor,
                    &attempt.admission,
                    &storage,
                    bundle.evidence.clone(),
                    bundle.envelope.clone(),
                )
            }
            3 => {
                h.admin.revoke_source_artifact_store(&h.storage).unwrap();
                h.deliver(&attempt, &bundle)
            }
            _ => {
                let unclaimed = h
                    .admin
                    .grant_source_executor(
                        PrincipalId::new("another-executor").unwrap(),
                        scope(),
                        backend(),
                        [ExecutionTier::T1].into(),
                        resources(),
                        10_000,
                    )
                    .unwrap();
                h.engine.record_source_evidence(
                    &unclaimed,
                    &attempt.admission,
                    &h.storage,
                    bundle.evidence.clone(),
                    bundle.envelope.clone(),
                )
            }
        };
        assert!(
            result.is_err(),
            "accepted substituted capability {mutation}"
        );
        assert_eq!(h.reader.reads.load(Ordering::SeqCst), 0);
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn authority_expiry_revocation_policy_and_cancellation_during_storage_io_are_rechecked() {
    for mutation in 0..7 {
        let h = Harness::new();
        let attempt = h.start();
        let bundle = h.seal(&attempt, h.input(&attempt));
        let admin = h.admin.clone();
        let engine = h.engine.clone();
        let owner = h.owner.clone();
        let executor = h.executor.clone();
        let storage = h.storage.clone();
        let tx = attempt.transaction.clone();
        let policy = h.policy.clone();
        let base = h.base.clone();
        h.reader.on_read(move || match mutation {
            0 => {
                admin.advance_time(500).unwrap();
            }
            1 => admin.revoke(&owner).unwrap(),
            2 => admin.revoke_source_executor(&executor).unwrap(),
            3 => admin.revoke_source_artifact_store(&storage).unwrap(),
            4 => {
                let mut input = policy.input().clone();
                input.generation += 1;
                engine
                    .install_source_policy(
                        &owner,
                        &scope(),
                        Some(policy.digest()),
                        SourceExecutionPolicy::new(input).unwrap(),
                    )
                    .unwrap();
            }
            5 => engine
                .abort(
                    &owner,
                    &scope(),
                    &tx,
                    "cancelled during artifact retrieval".into(),
                )
                .unwrap(),
            _ => {
                let replacement = RepositoryAnalysis::new(
                    &base,
                    RepositoryAnalysisInput {
                        index_generation: d("new-index"),
                        analyzer_version: "complete-finite-reference/v2".into(),
                        configuration: d("config"),
                        elements: [element()].into(),
                        dependencies: BTreeMap::new(),
                        checks: [(element(), [cid()].into())].into(),
                        gaps: BTreeSet::new(),
                    },
                )
                .unwrap();
                engine
                    .install_repository_analysis(
                        &owner,
                        &scope(),
                        policy.input().analysis_artifact,
                        replacement,
                    )
                    .unwrap();
            }
        });
        assert!(
            h.deliver(&attempt, &bundle).is_err(),
            "stale authority accepted after I/O mutation {mutation}"
        );
        assert!(h.reader.reads.load(Ordering::SeqCst) > 0);
        h.no_retained_evidence(&bundle);
    }
}

#[test]
fn immutable_duplicate_delivery_and_lost_retention_response_return_the_original_outcome() {
    let h = Harness::new();
    let attempt = h.start();
    let input = h.input(&attempt);
    let first = h.seal(&attempt, input.clone());
    let mut changed = input;
    changed.diagnostics.push("different diagnostics".into());
    let conflicting = h.seal(&attempt, changed);
    assert_eq!(
        h.engine.record_source_evidence_with_fault(
            &h.executor,
            &attempt.admission,
            &h.storage,
            first.evidence.clone(),
            first.envelope.clone(),
            SourceEvidenceFault::AfterRetention
        ),
        Err(EngineError::ResponseLost)
    );
    let retained = h
        .engine
        .source_evidence_record(&h.observer, &scope(), first.evidence.id())
        .unwrap()
        .1;
    assert_eq!(h.deliver(&attempt, &first).unwrap(), retained);
    assert_eq!(h.deliver(&attempt, &first).unwrap(), retained);
    assert_eq!(
        h.deliver(&attempt, &conflicting),
        Err(EngineError::IdempotencyConflict)
    );
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

#[test]
fn duplicate_acknowledgement_requires_the_original_claim_and_storage_registration() {
    let h = Harness::new();
    let attempt = h.start();
    let bundle = h.seal(&attempt, h.input(&attempt));
    let original = h.deliver(&attempt, &bundle).unwrap();
    let reads = h.reader.reads.load(Ordering::SeqCst);
    let unclaimed = h
        .admin
        .grant_source_executor(
            h.executor.principal().clone(),
            scope(),
            backend(),
            [ExecutionTier::T1].into(),
            resources(),
            10_000,
        )
        .unwrap();
    assert!(
        h.engine
            .record_source_evidence(
                &unclaimed,
                &attempt.admission,
                &h.storage,
                bundle.evidence.clone(),
                bundle.envelope.clone(),
            )
            .is_err()
    );
    let replacement_store = h
        .admin
        .register_source_artifact_store(scope(), h.reader.clone())
        .unwrap();
    assert_eq!(
        h.engine.record_source_evidence(
            &h.executor,
            &attempt.admission,
            &replacement_store,
            bundle.evidence.clone(),
            bundle.envelope.clone(),
        ),
        Err(EngineError::IdempotencyConflict)
    );
    assert_eq!(h.reader.reads.load(Ordering::SeqCst), reads);
    assert_eq!(h.deliver(&attempt, &bundle).unwrap(), original);
}

#[test]
fn an_exact_replay_only_acknowledges_history_after_owner_and_lease_invalidation() {
    let h = Harness::new();
    let attempt = h.start();
    let bundle = h.seal(&attempt, h.input(&attempt));
    let original = h.deliver(&attempt, &bundle).unwrap();
    let view = h
        .engine
        .source_attempt(
            &h.observer,
            &scope(),
            &attempt.admission.binding().source_attempt,
        )
        .unwrap();
    h.admin.revoke(&h.owner).unwrap();
    h.admin.advance_time(1000).unwrap();
    assert_eq!(h.deliver(&attempt, &bundle).unwrap(), original);
    assert!(
        h.engine
            .source_application(&h.executor, &attempt.admission)
            .is_err()
    );
    assert_eq!(
        h.engine
            .source_attempt(
                &h.observer,
                &scope(),
                &attempt.admission.binding().source_attempt,
            )
            .unwrap(),
        view
    );
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

#[test]
fn competing_delivery_rechecks_state_and_retains_one_immutable_outcome() {
    let h = Harness::new();
    let attempt = h.start();
    let first = h.seal(&attempt, h.input(&attempt));
    let mut changed = h.input(&attempt);
    changed.diagnostics.push("different sealed result".into());
    let second = h.seal(&attempt, changed);
    let storage = h
        .admin
        .register_source_artifact_store(
            scope(),
            Arc::new(RacingReader {
                backing: h.reader.clone(),
                first_reads: Mutex::new(0),
                both_reading: Condvar::new(),
            }),
        )
        .unwrap();
    let outcomes = std::thread::scope(|threads| {
        let first_delivery = threads.spawn(|| {
            h.engine.record_source_evidence(
                &h.executor,
                &attempt.admission,
                &storage,
                first.evidence.clone(),
                first.envelope.clone(),
            )
        });
        let second_delivery = threads.spawn(|| {
            h.engine.record_source_evidence(
                &h.executor,
                &attempt.admission,
                &storage,
                second.evidence.clone(),
                second.envelope.clone(),
            )
        });
        [
            first_delivery.join().unwrap(),
            second_delivery.join().unwrap(),
        ]
    });
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == Err(EngineError::IdempotencyConflict))
            .count(),
        1
    );
    let winner = outcomes.into_iter().find_map(Result::ok).unwrap();
    let (retained, loser) = if winner.evidence == first.evidence.id() {
        (&first, &second)
    } else {
        assert_eq!(winner.evidence, second.evidence.id());
        (&second, &first)
    };
    assert_eq!(
        h.engine
            .source_evidence_record(&h.observer, &scope(), winner.evidence)
            .unwrap(),
        (retained.evidence.clone(), winner)
    );
    h.no_retained_evidence(loser);
    assert_eq!(
        h.engine
            .source_attempt(
                &h.observer,
                &scope(),
                &attempt.admission.binding().source_attempt,
            )
            .unwrap()
            .events
            .iter()
            .filter(|event| event.to == SourceAttemptState::EvidenceSealed)
            .count(),
        1
    );
}

#[test]
fn terminal_failed_attempts_cannot_accept_late_success_even_after_a_retry_starts() {
    let h = Harness::new();
    let first = h.start();
    let delayed = h.seal(&first, h.input(&first));
    let terminal = h
        .engine
        .finish_source_attempt(
            &h.executor,
            &first.admission,
            SourceAttemptState::ExecutionFailed,
            "executor failed before evidence sealing".into(),
        )
        .unwrap();
    let retry = h.retry(&first.transaction, "attempt-2");
    assert!(h.deliver(&first, &delayed).is_err());
    h.no_retained_evidence(&delayed);
    assert_eq!(
        h.engine
            .source_attempt(
                &h.observer,
                &scope(),
                &first.admission.binding().source_attempt,
            )
            .unwrap(),
        terminal
    );
    let current = h.seal(&retry, h.input(&retry));
    assert_eq!(
        h.deliver(&retry, &current).unwrap().assessment,
        SourceEvidenceAssessment::Passed
    );
}

#[test]
fn aggregate_persisted_payload_budget_is_checked_before_storage_io() {
    let h = Harness::new();
    let attempt = h.start();
    let original = h.seal(&attempt, h.input(&attempt));
    let evidence = SourceEvidence::new(
        SourceEvidenceBinding::for_admission(&attempt.admission, h.executor.principal().clone()),
        original.evidence.started_at_ms(),
        original.evidence.finished_at_ms(),
        original.evidence.checks().values().cloned().collect(),
        original.evidence.provenance().clone(),
        vec!["x".repeat(60 * 1024); 18],
    )
    .unwrap();
    let envelope = h.reader.put(
        "sealed-evidence",
        SOURCE_EVIDENCE_MEDIA_TYPE,
        &evidence.canonical_bytes(),
    );
    let bundle = Bundle { evidence, envelope };
    assert!(matches!(
        h.deliver(&attempt, &bundle),
        Err(EngineError::SourceEvidence(SourceEvidenceError::Artifact(
            SourceArtifactError::Limit
        )))
    ));
    assert_eq!(h.reader.reads.load(Ordering::SeqCst), 0);
    h.no_retained_evidence(&bundle);
}

#[test]
fn failed_attempt_evidence_remains_immutable_and_cannot_satisfy_a_fresh_retry() {
    let h = Harness::new();
    let first = h.start();
    let mut failed_input = h.input(&first);
    failed_input.outcome = SourceCheckOutcome::Exited { code: 1 };
    let failure = h.seal(&first, failed_input);
    let failed_receipt = h.deliver(&first, &failure).unwrap();
    assert!(matches!(
        failed_receipt.assessment,
        SourceEvidenceAssessment::Failed { .. }
    ));
    let old_view = h
        .engine
        .source_attempt(
            &h.observer,
            &scope(),
            &first.admission.binding().source_attempt,
        )
        .unwrap();
    let retry = h.retry(&first.transaction, "attempt-2");
    assert_ne!(first.admission.id(), retry.admission.id());
    assert_ne!(
        first.admission.resulting_snapshot(),
        retry.admission.resulting_snapshot()
    );
    assert!(
        h.engine
            .record_source_evidence(
                &h.executor,
                &retry.admission,
                &h.storage,
                failure.evidence.clone(),
                failure.envelope.clone()
            )
            .is_err()
    );
    let successful = h.seal(&retry, h.input(&retry));
    assert_eq!(
        h.deliver(&retry, &successful).unwrap().assessment,
        SourceEvidenceAssessment::Passed
    );
    assert_eq!(
        h.engine
            .source_evidence_record(&h.observer, &scope(), failure.evidence.id())
            .unwrap(),
        (failure.evidence, failed_receipt)
    );
    assert_eq!(
        h.engine
            .source_attempt(
                &h.observer,
                &scope(),
                &first.admission.binding().source_attempt
            )
            .unwrap(),
        old_view
    );
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

mod source_authority_recovery {
    use super::*;
    use celetixo_engine::engine::authority::{AuthorityState, NativeRecordLimits, SourceRecords};
    fn check(h: &Harness) -> SourceRecords {
        let before = h.reader.reads.load(Ordering::SeqCst);
        let records = h
            .admin
            .inspect_source_records(&scope(), NativeRecordLimits::default())
            .unwrap();
        let state =
            AuthorityState::from_source_records(records.clone(), NativeRecordLimits::default())
                .unwrap();
        assert_eq!(
            state.source_records(NativeRecordLimits::default()).unwrap(),
            records
        );
        assert_eq!(
            before,
            h.reader.reads.load(Ordering::SeqCst),
            "record recovery invoked artifact I/O"
        );
        records
    }
    #[test]
    fn configured_artifact_adapter_survives_recovery_and_is_bound_to_provenance() {
        let h = Harness::new();
        let adapter = celetixo_engine::engine::SourceArtifactAdapter {
            identity: SourceArtifactAdapterId::new("test-artifact-service").unwrap(),
            configuration: d("reviewed-adapter-configuration-v1"),
        };
        h.admin
            .register_configured_source_artifact_store(scope(), adapter.clone(), h.reader.clone())
            .unwrap();
        let records = check(&h);
        let (&registration, configured) = records
            .artifact_stores
            .iter()
            .find(|(_, record)| record.adapter.is_some())
            .unwrap();
        assert_eq!(configured.adapter.as_ref(), Some(&adapter));
        assert!(records.artifact_stores[&1].adapter.is_none());
        h.admin
            .resolve_existing_source_artifact_store(&scope(), registration, |actual, record| {
                assert_eq!(actual, &scope());
                assert_eq!(record.adapter.as_ref(), Some(&adapter));
                Ok(h.reader.clone())
            })
            .unwrap();
        let mut substituted = records.clone();
        substituted
            .artifact_stores
            .get_mut(&registration)
            .unwrap()
            .adapter
            .as_mut()
            .unwrap()
            .configuration = d("different-configuration");
        assert!(
            AuthorityState::from_source_records(substituted, NativeRecordLimits::default())
                .is_err()
        );
        let mut removed = records;
        removed
            .artifact_stores
            .get_mut(&registration)
            .unwrap()
            .adapter = None;
        assert!(
            AuthorityState::from_source_records(removed, NativeRecordLimits::default()).is_err()
        );
    }
    #[test]
    fn passed_failed_and_incomplete_receipts_recover_with_missing_readers_and_revoked_grants() {
        for case in 0..3 {
            let h = Harness::new();
            let attempt = h.start();
            check(&h);
            let mut input = h.input(&attempt);
            match case {
                1 => input.outcome = SourceCheckOutcome::Exited { code: 7 },
                2 => {
                    input.cleanup = SourceCleanup::Quarantined {
                        reason: "retained for operator".into(),
                    }
                }
                _ => {}
            }
            let bundle = h.seal(&attempt, input);
            let receipt = h.deliver(&attempt, &bundle).unwrap();
            let r = check(&h);
            match case {
                0 => assert_eq!(receipt.assessment, SourceEvidenceAssessment::Passed),
                1 => assert!(matches!(
                    receipt.assessment,
                    SourceEvidenceAssessment::Failed { .. }
                )),
                _ => assert!(matches!(
                    receipt.assessment,
                    SourceEvidenceAssessment::Incomplete { .. }
                )),
            }
            h.reader.unavailable.store(true, Ordering::SeqCst);
            h.admin.revoke_source_artifact_store(&h.storage).unwrap();
            h.admin.revoke_source_executor(&h.executor).unwrap();
            h.admin.revoke(&h.owner).unwrap();
            h.admin.advance_time(20_000).unwrap();
            let after = check(&h);
            assert_eq!(r.evidence, after.evidence);
            assert_eq!(after.evidence[&bundle.evidence.id()].receipt, receipt);
            assert!(!after.artifact_stores.values().next().unwrap().active);
            assert!(!after.executors.values().next().unwrap().active);
            // Typed metadata has no reader, and cannot revive the original
            // executor or make a lost physical artifact available.
            assert!(h.deliver(&attempt, &bundle).is_err());
        }
    }
    #[test]
    fn receipt_selection_assessment_provenance_and_source_transaction_state_are_checked() {
        let h = Harness::new();
        let attempt = h.start();
        let bundle = h.seal(&attempt, h.input(&attempt));
        h.deliver(&attempt, &bundle).unwrap();
        let r = check(&h);
        fn reject(r: SourceRecords) {
            assert!(AuthorityState::from_source_records(r, NativeRecordLimits::default()).is_err());
        }
        let mut bad = r.clone();
        bad.evidence.values_mut().next().unwrap().receipt.assessment =
            SourceEvidenceAssessment::Failed {
                checks: [cid()].into(),
            };
        reject(bad);
        let mut bad = r.clone();
        bad.evidence
            .values_mut()
            .next()
            .unwrap()
            .receipt
            .storage_provenance = d("other-store");
        reject(bad);
        let mut bad = r.clone();
        bad.evidence.values_mut().next().unwrap().store_registration += 1;
        reject(bad);
        let recorded_at = r.evidence.values().next().unwrap().receipt.recorded_at_ms;
        let mut boundary = r.clone();
        boundary
            .executors
            .values_mut()
            .next()
            .unwrap()
            .expires_at_ms = recorded_at + 1;
        AuthorityState::from_source_records(boundary, NativeRecordLimits::default()).unwrap();
        let mut bad = r.clone();
        bad.executors.values_mut().next().unwrap().expires_at_ms = recorded_at;
        reject(bad);
        let mut bad = r.clone();
        bad.evidence
            .values_mut()
            .next()
            .unwrap()
            .receipt
            .recorded_at_ms += 1;
        reject(bad);
        let mut bad = r.clone();
        bad.evidence
            .values_mut()
            .next()
            .unwrap()
            .receipt
            .envelope
            .bytes += 1;
        reject(bad);
        let mut bad = r.clone();
        bad.evidence.clear();
        reject(bad);
        let mut bad = r.clone();
        bad.attempts.values_mut().next().unwrap().evidence = None;
        reject(bad);
        let mut bad = r.clone();
        bad.attempts.values_mut().next().unwrap().executor_grant = None;
        reject(bad);
        let mut bad = r.clone();
        bad.base
            .transactions
            .get_mut(&attempt.transaction)
            .unwrap()
            .state = TransactionState::Executing;
        reject(bad);
        let mut bad = r.clone();
        bad.base
            .transactions
            .get_mut(&attempt.transaction)
            .unwrap()
            .attempts[0]
            .effective
            .as_mut()
            .unwrap()
            .writes
            .clear();
        reject(bad);
        let mut bad = r.clone();
        bad.artifact_stores.values_mut().next().unwrap().retention = d("changed-retention");
        reject(bad);
    }
}

mod recovered_runtime {
    use super::*;
    use celetixo_engine::engine::authority::*;
    #[derive(Default)]
    pub(super) struct RecordBackend {
        pub(super) records: Mutex<BTreeMap<Scope, SourceRecords>>,
        contexts: Mutex<Vec<AuthorityAccess>>,
    }
    impl AuthorityBackend for RecordBackend {
        fn access(
            &self,
            context: &AuthorityAccess,
            evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
        ) -> Result<(), AuthorityFailure> {
            self.contexts.lock().unwrap().push(context.clone());
            let mut rows = self.records.lock().unwrap();
            let records = rows
                .get(context.scope())
                .ok_or(AuthorityFailure::ScopeDenied)?
                .clone();
            let mut state =
                AuthorityState::from_source_records(records.clone(), NativeRecordLimits::default())
                    .map_err(|_| AuthorityFailure::Integrity)?;
            evaluate(&mut state)?;
            let next = state
                .source_records(NativeRecordLimits::default())
                .map_err(|_| AuthorityFailure::Integrity)?;
            if matches!(
                context.mode(),
                AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
            ) {
                rows.insert(context.scope().clone(), next);
            } else if next != records {
                return Err(AuthorityFailure::Protocol);
            }
            Ok(())
        }
    }
    fn backend(records: SourceRecords) -> Arc<RecordBackend> {
        Arc::new(RecordBackend {
            records: Mutex::new([(records.base.scope.clone(), records)].into()),
            ..Default::default()
        })
    }
    #[test]
    fn fresh_embedding_resumes_claimed_work_only_after_independent_authentication_and_reader_resolution()
     {
        let h = Harness::new();
        let attempt = h.start();
        let records = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        let service = backend(records.clone());
        let (engine, admin) = Engine::with_authority_backend(service.clone());
        let executor_id = *records.executors.keys().next().unwrap();
        let storage_id = *records.artifact_stores.keys().next().unwrap();
        assert!(
            engine
                .validate_source_attempt(&h.executor, &attempt.admission)
                .is_err()
        );
        assert!(
            engine
                .source_evidence_record(&h.observer, &scope(), SourceEvidenceId(d("none")))
                .is_err()
        );
        let executor = admin
            .authenticate_existing_source_executor(&scope(), executor_id, |s, g| {
                assert_eq!(*s, scope());
                assert_eq!(g.principal, *h.executor.principal());
                // This reentrant state read proves authentication has no state lock.
                assert_eq!(
                    admin
                        .inspect_source_records(s, Default::default())
                        .unwrap()
                        .executors[&executor_id],
                    *g
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(
            engine
                .validate_source_attempt(&executor, &attempt.admission)
                .unwrap(),
            SourceAttemptState::Verifying
        );
        assert_eq!(
            engine.claim_source_attempt(&executor, &attempt.admission),
            Err(EngineError::SourceAttempt(
                SourceAttemptError::AlreadyClaimed
            ))
        );
        assert_eq!(
            admin
                .authenticate_existing_source_executor(&scope(), executor_id, |_, _| Err(
                    EngineError::Unauthorized
                ))
                .err(),
            Some(EngineError::Unauthorized)
        );
        let reader = h.reader.clone();
        let storage = admin
            .resolve_existing_source_artifact_store(&scope(), storage_id, |s, g| {
                assert_eq!(
                    admin
                        .inspect_source_records(s, Default::default())
                        .unwrap()
                        .artifact_stores[&storage_id],
                    *g
                );
                Ok(reader)
            })
            .unwrap();
        // Generate exact immutable bytes through the original independent test
        // fixture. The newly recovered Engine must perform its own object reads.
        let bundle = h.seal(&attempt, h.input(&attempt));
        {
            let mut rows = service.records.lock().unwrap();
            let row = rows.get_mut(&scope()).unwrap();
            let mut state =
                AuthorityState::from_source_records(row.clone(), Default::default()).unwrap();
            state.advance_authoritative_clock(10).unwrap();
            *row = state.source_records(Default::default()).unwrap();
        }
        let reenter = admin.clone();
        h.reader.on_read(move || {
            reenter
                .inspect_source_records(&scope(), Default::default())
                .unwrap();
        });
        let receipt = engine
            .record_source_evidence(
                &executor,
                &attempt.admission,
                &storage,
                bundle.evidence.clone(),
                bundle.envelope.clone(),
            )
            .unwrap();
        assert_eq!(receipt.assessment, SourceEvidenceAssessment::Passed);
        let new_records = admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        assert_eq!(
            new_records.base.transactions[&attempt.transaction].state,
            TransactionState::Validating
        );
        assert!(
            new_records.base.transactions[&attempt.transaction].attempts[0]
                .patch
                .is_none()
        );
        let (engine2, admin2) = Engine::with_authority_backend(backend(new_records));
        let owner_id = records
            .base
            .grants
            .iter()
            .find(|(_, g)| g.principal == *h.owner.principal())
            .unwrap()
            .0;
        let owner = admin2
            .authenticate_existing_session(&scope(), *owner_id, |_, _| Ok(()))
            .unwrap();
        assert_eq!(
            engine2
                .source_evidence_record(&owner, &scope(), receipt.evidence)
                .unwrap()
                .1,
            receipt
        );
        assert!(
            engine2
                .record_source_evidence(
                    &executor,
                    &attempt.admission,
                    &storage,
                    bundle.evidence,
                    bundle.envelope
                )
                .is_err()
        );
        assert!(
            service
                .contexts
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c.actor(), AuthorityActor::SourceExecutor { .. }))
        );
    }
    #[test]
    fn resolver_rechecks_revocation_after_external_authentication() {
        let h = Harness::new();
        let records = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        let id = *records.executors.keys().next().unwrap();
        let service = backend(records);
        let (_, admin) = Engine::with_authority_backend(service.clone());
        assert!(
            admin
                .authenticate_existing_source_executor(&scope(), id, |_, _| {
                    service
                        .records
                        .lock()
                        .unwrap()
                        .get_mut(&scope())
                        .unwrap()
                        .executors
                        .get_mut(&id)
                        .unwrap()
                        .active = false;
                    Ok(())
                })
                .is_err()
        );
        let store_id = *service.records.lock().unwrap()[&scope()]
            .artifact_stores
            .keys()
            .next()
            .unwrap();
        assert!(
            admin
                .resolve_existing_source_artifact_store(&scope(), store_id, |_, _| {
                    service
                        .records
                        .lock()
                        .unwrap()
                        .get_mut(&scope())
                        .unwrap()
                        .artifact_stores
                        .get_mut(&store_id)
                        .unwrap()
                        .active = false;
                    Ok(h.reader.clone())
                })
                .is_err()
        );
    }
}

// Reuse the scoped test adapter while keeping this separate from its execution
// harness so two source scopes can allocate the same numeric registration IDs.
#[test]
fn recovered_scopes_can_reuse_executor_cleanup_and_storage_ids_without_runtime_aliasing() {
    use celetixo_engine::engine::authority::*;
    let first = scope();
    let mut second = scope();
    second.repository = RepositoryId::new("second").unwrap();
    let service = Arc::new(recovered_runtime::RecordBackend::default());
    // The adapter is test-only; real PostgreSQL source publication is not enabled.
    service
        .records
        .lock()
        .unwrap()
        .insert(first.clone(), SourceRecords::empty(first.clone()));
    service
        .records
        .lock()
        .unwrap()
        .insert(second.clone(), SourceRecords::empty(second.clone()));
    let (engine, admin) = Engine::with_authority_backend(service);
    let grant = |s: Scope| {
        admin
            .grant_source_executor(
                PrincipalId::new("executor").unwrap(),
                s,
                backend(),
                [ExecutionTier::T1].into(),
                resources(),
                1000,
            )
            .unwrap()
    };
    let e1 = grant(first.clone());
    let _e2 = grant(second.clone());
    let cleanup = |s: Scope| {
        admin
            .grant_source_cleanup(
                PrincipalId::new("cleanup").unwrap(),
                s,
                backend().profile,
                1000,
            )
            .unwrap()
    };
    let c1 = cleanup(first.clone());
    let c2 = cleanup(second.clone());
    let r1 = Arc::new(ReferenceReader::new());
    let r2 = Arc::new(ReferenceReader::new());
    let s1 = admin
        .register_source_artifact_store(first.clone(), r1.clone())
        .unwrap();
    let _s2 = admin
        .register_source_artifact_store(second.clone(), r2.clone())
        .unwrap();
    for s in [&first, &second] {
        let r = admin.inspect_source_records(s, Default::default()).unwrap();
        assert_eq!(r.executors.keys().copied().collect::<Vec<_>>(), vec![1]);
        assert_eq!(r.cleanup.keys().copied().collect::<Vec<_>>(), vec![1]);
        assert_eq!(
            r.artifact_stores.keys().copied().collect::<Vec<_>>(),
            vec![1]
        );
    }
    admin.revoke_source_executor(&e1).unwrap();
    admin.revoke_source_cleanup(&c1).unwrap();
    admin.revoke_source_artifact_store(&s1).unwrap();
    assert!(
        admin
            .authenticate_existing_source_executor(&first, 1, |_, _| Ok(()))
            .is_err()
    );
    assert!(
        admin
            .authenticate_existing_source_executor(&second, 1, |_, _| Ok(()))
            .is_ok()
    );
    assert!(
        engine
            .validate_source_cleanup(&c1, &first, backend().profile)
            .is_err()
    );
    assert!(
        engine
            .validate_source_cleanup(&c2, &second, backend().profile)
            .is_ok()
    );
    assert!(
        engine
            .validate_source_cleanup(&c2, &first, backend().profile)
            .is_err()
    );
    assert!(
        admin
            .resolve_existing_source_artifact_store(&first, 1, |_, _| Ok(r1))
            .is_err()
    );
    assert!(
        admin
            .resolve_existing_source_artifact_store(&second, 1, |_, _| Ok(r2))
            .is_ok()
    );
    assert!(
        admin
            .authenticate_existing_source_cleanup(&first, 1, |_, _| Ok(()))
            .is_err()
    );
    assert!(
        admin
            .authenticate_existing_source_cleanup(&second, 1, |_, _| Ok(()))
            .is_ok()
    );
}
