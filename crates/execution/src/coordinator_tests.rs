//! Finite coordinator contract fixtures. The private fake backend below starts
//! no process and validates no physical isolation property. Real OCI integration
//! probes live in the execution integration tests.
use super::*;
use crate::artifacts::ArtifactStoreLimits;
use celetixo_engine::access::AccessSet;
use celetixo_engine::engine::{Permission, Session, SourceAttemptError};
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::repository::{RepositoryAnalysis, RepositoryAnalysisInput};
use celetixo_engine::snapshot::*;
use celetixo_engine::source_evidence::SourceEvidenceAssessment;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use celetixo_engine::transaction::{TransactionIntent, TransactionState};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

fn d(value: &str) -> Digest {
    Digest::of("finite-coordinator-reference/v1", value.as_bytes())
}
fn id(value: &str) -> CheckId {
    CheckId::new(value).unwrap()
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("coordinator-org").unwrap(),
        workspace: WorkspaceId::new("coordinator-workspace").unwrap(),
        repository: RepositoryId::new("coordinator-repository").unwrap(),
    }
}
fn limits() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 500,
        memory_bytes: 16 * 1024 * 1024,
        disk_bytes: 1024 * 1024,
        process_count: 8,
        file_descriptors: 64,
        stdout_bytes: 65_536,
        stderr_bytes: 65_536,
        artifact_bytes: 2 * 1024 * 1024,
    }
}
fn backend_identity() -> BackendIdentity {
    BackendIdentity {
        name: "finite-reference-no-physical-isolation".into(),
        version: "v1".into(),
        image: d("rootfs"),
        profile: d("profile"),
    }
}
fn definition(name: &str, prerequisites: &[&str]) -> SourceCheckDefinition {
    SourceCheckDefinition::new(SourceCheckInput {
        check: CheckSpec::new(
            id(name),
            vec!["/tools/finite-reference".into(), name.into()],
            "finite-reference/v1".into(),
            d("environment"),
            d("configuration"),
        )
        .unwrap(),
        working_directory: WorkingDirectory::WorkspaceRoot,
        inputs: [(
            "main.txt".into(),
            Digest::of("celetixo/source-content/v1", b"new\n"),
        )]
        .into(),
        outputs: BTreeSet::new(),
        toolchain: ToolchainIdentity {
            executable: "/tools/finite-reference".into(),
            executable_digest: d("executable"),
            runtime_closure: d("rootfs"),
            version: "finite-reference/v1".into(),
            platform: "linux".into(),
            architecture: "x86_64".into(),
        },
        dependency_inputs: BTreeMap::new(),
        environment: BTreeMap::new(),
        tier: ExecutionTier::T1,
        backend: backend_identity(),
        resources: limits(),
        network: NetworkPolicy::DenyAll,
        credential_references: BTreeSet::new(),
        expected_artifacts: [
            ("stdout", "text/plain"),
            ("stderr", "text/plain"),
            ("diagnostics", "application/json"),
            ("isolation-record", "application/json"),
        ]
        .into_iter()
        .map(|(name, media_type)| ArtifactRequirement {
            name: name.into(),
            media_type: media_type.into(),
            max_bytes: 65_536,
            required: true,
        })
        .collect(),
        depends_on: prerequisites.iter().map(|name| id(name)).collect(),
        failure_policy: FailurePolicy::FailAttempt,
    })
    .unwrap()
}

struct Fixture {
    admin: Administrator,
    engine: Engine,
    observer: Session,
    owner: Session,
    executor: SourceExecutorCapability,
    artifacts: Arc<LocalSourceArtifactStore>,
    storage: SourceArtifactStoreCapability,
    base: Snapshot,
    policy: SourceExecutionPolicy,
    patch: SourcePatch,
    admission: SourceAdmission,
}

impl Fixture {
    fn new(dependency: bool) -> Self {
        let (store, admin) = celetixo_engine::engine::InMemoryStore::new();
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
                PrincipalId::new("audit-reader").unwrap(),
                scope(),
                [Permission::Read].into(),
            )
            .unwrap();
        let base = Snapshot::new(
            scope(),
            Revision(0),
            SnapshotManifest {
                base_commit: "finite-coordinator-reference".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "celetixo/reference-source/coordinator-v1".into(),
                inputs: vec![WorkspaceInput {
                    path: "main.txt".into(),
                    layer: InputLayer::Base,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(b"old\n".to_vec()),
                }],
                environment: BTreeMap::new(),
            },
            BTreeMap::new(),
        )
        .unwrap();
        let element = ElementId::new("file:main").unwrap();
        let mut checks = vec![definition("first", &[])];
        if dependency {
            checks.push(definition("dependent", &["first"]));
        }
        let analysis = RepositoryAnalysis::new(
            &base,
            RepositoryAnalysisInput {
                index_generation: d("index"),
                analyzer_version: "complete-finite-reference-only/v1".into(),
                configuration: d("analysis-configuration"),
                elements: [element.clone()].into(),
                dependencies: BTreeMap::new(),
                checks: [(
                    element.clone(),
                    checks.iter().map(|check| check.id().clone()).collect(),
                )]
                .into(),
                gaps: BTreeSet::new(),
            },
        )
        .unwrap();
        engine
            .register_repository(&owner, base.clone(), analysis.clone())
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
                capture_policy: d("capture-policy"),
                uncertainty: SourceCaptureBinding::empty_uncertainty_digest(),
                consistency: SourceCaptureConsistency::ReferenceFixture,
                gaps: BTreeSet::new(),
            },
            patch_policy: PatchPolicy::default(),
            credentials: Default::default(),
            paths: [(
                "main.txt".into(),
                SourcePathRule {
                    allowed_operations: [OperationKind::Replace].into(),
                    required_writes: [element.clone()].into(),
                    expected_base: Some(Preimage::of(&base.manifest().inputs[0]).unwrap()),
                },
            )]
            .into(),
            checks: checks.clone(),
            invariants: vec![],
            always_required: checks.iter().map(|check| check.id().clone()).collect(),
            approved_tiers: [ExecutionTier::T1].into(),
            approved_backends: [backend_identity().digest()].into(),
            resource_ceiling: limits(),
            network_allowed: false,
            approved_credential_references: BTreeSet::new(),
            expires_at_ms: 100_000,
            retention_policy: d("retention"),
        })
        .unwrap();
        engine
            .install_source_policy(&owner, &scope(), None, policy.clone())
            .unwrap();
        let transaction = TransactionId::new("coordinator-transaction").unwrap();
        engine
            .propose(
                &owner,
                &scope(),
                transaction.clone(),
                base.id(),
                TransactionIntent {
                    objective: "finite coordinator bridge, no physical execution".into(),
                    accesses: AccessSet {
                        writes: [element].into(),
                        ..Default::default()
                    },
                    invariants: vec![],
                    required_checks: checks
                        .iter()
                        .map(|check| check.input().check.clone())
                        .collect(),
                    depends_on: BTreeSet::new(),
                },
            )
            .unwrap();
        engine.analyze(&owner, &scope(), &transaction).unwrap();
        engine
            .reserve(&owner, &scope(), &transaction, 100_000)
            .unwrap();
        engine.execute(&owner, &scope(), &transaction).unwrap();
        let binding = engine
            .source_patch_context(&owner, &scope(), &transaction)
            .unwrap()
            .bind(
                SourceAttemptId::new("coordinator-attempt").unwrap(),
                "finite coordinator fixture".into(),
                50_000,
            )
            .unwrap();
        let patch = SourcePatch::new(
            binding,
            PatchPolicy::default(),
            vec![PatchOperation::Replace {
                path: "main.txt".into(),
                expected: Preimage::of(&base.manifest().inputs[0]).unwrap(),
                entry: PatchEntry::file(b"new\n".to_vec(), FileMode::Regular).unwrap(),
            }],
        )
        .unwrap();
        let admission = engine
            .admit_source_attempt(
                &owner,
                &scope(),
                patch.clone(),
                IdempotencyKey::new("coordinator-admission").unwrap(),
            )
            .unwrap();
        let executor = admin
            .grant_source_executor(
                PrincipalId::new("trusted-finite-executor").unwrap(),
                scope(),
                backend_identity(),
                [ExecutionTier::T1].into(),
                limits(),
                100_000,
            )
            .unwrap();
        let artifacts = Arc::new(
            LocalSourceArtifactStore::temporary(
                scope(),
                d("retention"),
                ArtifactStoreLimits::default(),
            )
            .unwrap(),
        );
        let storage = admin
            .register_source_artifact_store(scope(), artifacts.clone())
            .unwrap();
        Self {
            admin,
            engine,
            observer,
            owner,
            executor,
            artifacts,
            storage,
            base,
            policy,
            patch,
            admission,
        }
    }
    fn request(&self) -> SourceRunRequest<'_> {
        SourceRunRequest {
            base: &self.base,
            patch: &self.patch,
            policy: &self.policy,
            admission: &self.admission,
        }
    }
    fn coordinator<'a>(
        &'a self,
        backend: &'a FiniteBackend,
    ) -> SourceRunCoordinator<'a, FiniteBackend> {
        SourceRunCoordinator {
            engine: &self.engine,
            executor: &self.executor,
            backend,
            artifacts: &self.artifacts,
            storage: &self.storage,
        }
    }
    fn state(&self) -> SourceAttemptState {
        self.engine
            .source_attempt(
                &self.observer,
                &scope(),
                &self.admission.binding().source_attempt,
            )
            .unwrap()
            .state
    }
    fn assert_base_unchanged(&self) {
        assert_eq!(
            self.engine.current(&self.observer, &scope()).unwrap(),
            self.base
        );
    }
}

#[derive(Clone, Copy)]
enum ForgedBinding {
    Execution,
    Rootfs,
    Check,
    Definition,
    Plan,
    Snapshot,
    Manifest,
    Backend,
}

struct FiniteBackend {
    identity: BackendIdentity,
    exit: i32,
    forged: Option<ForgedBinding>,
    calls: RefCell<Vec<CheckId>>,
    materializations: RefCell<Vec<PathBuf>>,
}
impl FiniteBackend {
    fn new(exit: i32) -> Self {
        Self {
            identity: backend_identity(),
            exit,
            forged: None,
            calls: RefCell::new(vec![]),
            materializations: RefCell::new(vec![]),
        }
    }
    fn assert_cleaned(&self) {
        assert!(
            self.materializations
                .borrow()
                .iter()
                .all(|path| !path.exists())
        );
    }
}
impl sealed::Backend for FiniteBackend {}
impl VerificationBackend for FiniteBackend {
    fn identity(&self) -> &BackendIdentity {
        &self.identity
    }
    fn execute(
        &self,
        engine: &Engine,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        workspace: &mut PrivateWorkspace,
        check: &SourceCheckDefinition,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<OciCheckRecord, OciError> {
        if guard.revalidate().map_err(OciError::Guard)? != GuardDecision::Continue {
            return Err(OciError::Guard("finite test guard stopped".into()));
        }
        engine
            .validate_source_attempt(executor, admission)
            .map_err(|error| OciError::Guard(error.to_string()))?;
        workspace
            .validate_contents()
            .map_err(|error| OciError::Guard(error.to_string()))?;
        self.calls.borrow_mut().push(check.id().clone());
        self.materializations
            .borrow_mut()
            .push(workspace.source_path().to_path_buf());
        let now = engine
            .source_execution_time(executor, admission)
            .map_err(|error| OciError::Guard(error.to_string()))?;
        let mut raw = OciCheckRecord {
            execution: crate::oci::execution_identity(admission, check), journal: d("finite-journal"), check: check.id().clone(), definition: check.digest(), plan: admission.plan().id(), snapshot: admission.resulting_snapshot(), manifest: admission.resulting_manifest(), backend: check.input().backend.clone(), runtime: d("runtime"), rootfs: d("rootfs"), launcher: d("launcher"),
            isolation: Some(serde_json::to_vec(&serde_json::json!({"version":1,"kind":"finite-reference-no-physical-isolation","runtime":d("runtime").to_string(),"rootfs":d("rootfs").to_string(),"launcher":d("launcher").to_string()})).unwrap()),
            started_at_ms: now, finished_at_ms: now, outcome: SourceCheckOutcome::Exited { code: self.exit }, usage: SourceResourceUsage { wall_time_ms: 0, cpu_time_us: 0, peak_memory_bytes: 0 }, resource_observations: BTreeMap::new(), phase_timings_us: BTreeMap::new(), stdout: b"finite reference output\n".to_vec(), stderr: vec![], stdout_truncated: false, stderr_truncated: false, termination_confirmed: true, cleanup: CleanupOutcome::Removed, post_source_manifest: Some(admission.resulting_manifest()), diagnostics: vec!["finite fixture; no physical sandbox was exercised".into()],
        };
        match self.forged {
            Some(ForgedBinding::Execution) => raw.execution = d("foreign-execution"),
            Some(ForgedBinding::Rootfs) => raw.rootfs = d("foreign-rootfs"),
            Some(ForgedBinding::Check) => raw.check = id("foreign"),
            Some(ForgedBinding::Definition) => raw.definition = d("foreign-definition"),
            Some(ForgedBinding::Plan) => raw.plan = SourcePlanId(d("foreign-plan")),
            Some(ForgedBinding::Snapshot) => raw.snapshot = SnapshotId(d("foreign-snapshot")),
            Some(ForgedBinding::Manifest) => raw.manifest = d("foreign-manifest"),
            Some(ForgedBinding::Backend) => raw.backend.profile = d("foreign-profile"),
            None => {}
        }
        Ok(raw)
    }
}

struct Continue;
impl ExecutionGuard for Continue {
    fn revalidate(&mut self) -> Result<GuardDecision, String> {
        Ok(GuardDecision::Continue)
    }
}
fn objects(store: &LocalSourceArtifactStore) -> usize {
    std::fs::read_dir(store.root_path())
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".artifact"))
        .count()
}

#[test]
fn finite_bridge_persists_exact_evidence_and_independently_admits_pass_or_failure() {
    for exit in [0, 1] {
        let h = Fixture::new(false);
        assert_eq!(h.state(), SourceAttemptState::Admitted);
        let backend = FiniteBackend::new(exit);
        let coordinator = h.coordinator(&backend);
        let result = coordinator.execute(h.request(), &mut Continue).unwrap();
        assert_eq!(*backend.calls.borrow(), vec![id("first")]);
        assert_eq!(result.cleanup, CleanupOutcome::Removed);
        backend.assert_cleaned();
        assert!(objects(&h.artifacts) > 1);
        assert_eq!(
            result.sealed.envelope.bytes as usize,
            result.sealed.evidence.canonical_bytes().len()
        );
        let retained = h
            .artifacts
            .read(
                &scope(),
                result.sealed.envelope.content,
                result.sealed.envelope.bytes,
            )
            .unwrap();
        assert_eq!(retained, result.sealed.evidence.canonical_bytes());
        let expected = if exit == 0 {
            SourceEvidenceAssessment::Passed
        } else {
            SourceEvidenceAssessment::Failed {
                checks: [id("first")].into(),
            }
        };
        assert_eq!(result.receipt.assessment, expected);
        assert_eq!(
            h.state(),
            if exit == 0 {
                SourceAttemptState::EvidenceSealed
            } else {
                SourceAttemptState::ExecutionFailed
            }
        );
        assert_eq!(
            coordinator.deliver(&h.admission, &result.sealed).unwrap(),
            result.receipt
        );
        assert_eq!(
            coordinator.deliver(&h.admission, &result.sealed).unwrap(),
            result.receipt
        );
        assert_eq!(backend.calls.borrow().len(), 1);
        h.assert_base_unchanged();
        assert_eq!(
            h.engine.commit(
                &h.owner,
                &scope(),
                &h.admission.binding().transaction,
                IdempotencyKey::new("no-source-publication").unwrap()
            ),
            Err(EngineError::SourceMutationUnsupported)
        );
    }
}

#[test]
fn failed_prerequisite_records_the_dependent_as_skipped_and_cannot_turn_green() {
    let h = Fixture::new(true);
    let backend = FiniteBackend::new(1);
    let result = h
        .coordinator(&backend)
        .execute(h.request(), &mut Continue)
        .unwrap();
    assert_eq!(*backend.calls.borrow(), vec![id("first")]);
    assert!(matches!(
        result.sealed.evidence.checks()[&id("dependent")]
            .input()
            .outcome,
        SourceCheckOutcome::Skipped { .. }
    ));
    assert!(matches!(
        result.receipt.assessment,
        SourceEvidenceAssessment::Incomplete { .. }
    ));
    assert_eq!(h.state(), SourceAttemptState::EvidenceIncomplete);
    assert_eq!(
        h.engine
            .transaction(&h.observer, &scope(), &h.admission.binding().transaction)
            .unwrap()
            .state,
        TransactionState::Executing
    );
    backend.assert_cleaned();
    h.assert_base_unchanged();
}

#[test]
fn forged_backend_bindings_are_rejected_before_artifact_publication() {
    for forged in [
        ForgedBinding::Execution,
        ForgedBinding::Rootfs,
        ForgedBinding::Check,
        ForgedBinding::Definition,
        ForgedBinding::Plan,
        ForgedBinding::Snapshot,
        ForgedBinding::Manifest,
        ForgedBinding::Backend,
    ] {
        let h = Fixture::new(false);
        let backend = FiniteBackend {
            forged: Some(forged),
            ..FiniteBackend::new(0)
        };
        let failure = h
            .coordinator(&backend)
            .execute(h.request(), &mut Continue)
            .unwrap_err();
        assert_eq!(failure.stage, ExecutionStage::Verification);
        assert!(matches!(
            failure.cause,
            SourceRunError::Binding("backend execution receipt")
        ));
        assert_eq!(failure.cleanup, Some(CleanupOutcome::Removed));
        assert_eq!(objects(&h.artifacts), 0);
        assert_eq!(h.state(), SourceAttemptState::ExecutionFailed);
        assert_eq!(backend.calls.borrow().len(), 1);
        backend.assert_cleaned();
        h.assert_base_unchanged();
    }
}

#[test]
fn duplicate_claim_and_repeat_execution_never_launch_a_second_check() {
    let h = Fixture::new(false);
    let backend = FiniteBackend::new(0);
    let coordinator = h.coordinator(&backend);
    h.engine
        .claim_source_attempt(&h.executor, &h.admission)
        .unwrap();
    let failure = coordinator.execute(h.request(), &mut Continue).unwrap_err();
    assert_eq!(failure.stage, ExecutionStage::Admission);
    assert!(matches!(
        failure.cause,
        SourceRunError::Engine(EngineError::SourceAttempt(
            SourceAttemptError::AlreadyClaimed
        ))
    ));
    assert!(failure.terminal_recording.is_none());
    assert!(backend.calls.borrow().is_empty());
    assert_eq!(objects(&h.artifacts), 0);
    let h = Fixture::new(false);
    let coordinator = h.coordinator(&backend);
    let result = coordinator.execute(h.request(), &mut Continue).unwrap();
    let failure = coordinator.execute(h.request(), &mut Continue).unwrap_err();
    assert_eq!(failure.stage, ExecutionStage::Admission);
    assert!(failure.terminal_recording.is_none());
    assert_eq!(backend.calls.borrow().len(), 1);
    assert_eq!(
        coordinator.deliver(&h.admission, &result.sealed).unwrap(),
        result.receipt
    );
    backend.assert_cleaned();
}

#[test]
fn a_mismatched_request_grants_no_claim_and_does_not_poison_the_valid_request() {
    let h = Fixture::new(false);
    let backend = FiniteBackend::new(0);
    let coordinator = h.coordinator(&backend);
    let mut binding = h.patch.binding().clone();
    binding.producer = "different request producer".into();
    let different = SourcePatch::new(
        binding,
        h.patch.policy().clone(),
        h.patch.operations().to_vec(),
    )
    .unwrap();
    let failure = coordinator
        .execute(
            SourceRunRequest {
                patch: &different,
                ..h.request()
            },
            &mut Continue,
        )
        .unwrap_err();
    assert_eq!(failure.stage, ExecutionStage::Admission);
    assert!(matches!(failure.cause, SourceRunError::Binding(_)));
    assert!(failure.terminal_recording.is_none());
    assert_eq!(h.state(), SourceAttemptState::Admitted);
    assert!(backend.calls.borrow().is_empty());
    assert_eq!(objects(&h.artifacts), 0);
    let result = coordinator.execute(h.request(), &mut Continue).unwrap();
    assert_eq!(result.receipt.assessment, SourceEvidenceAssessment::Passed);
    h.assert_base_unchanged();
}

#[derive(Clone, Copy)]
enum StopAction {
    Cancel,
    RevokeExecutor,
}
struct StopAtBoundary<'a> {
    fixture: &'a Fixture,
    action: StopAction,
    during_persistence: bool,
    calls: usize,
    fired: bool,
}
impl ExecutionGuard for StopAtBoundary<'_> {
    fn revalidate(&mut self) -> Result<GuardDecision, String> {
        self.calls += 1;
        let reached = if self.during_persistence {
            objects(&self.fixture.artifacts) > 0
        } else {
            self.calls == 2
        };
        if !self.fired && reached {
            self.fired = true;
            match self.action {
                StopAction::Cancel => {
                    self.fixture
                        .engine
                        .abort(
                            &self.fixture.owner,
                            &scope(),
                            &self.fixture.admission.binding().transaction,
                            "controlled cancellation at coordinator boundary".into(),
                        )
                        .map_err(|error| error.to_string())?;
                    return Ok(GuardDecision::Cancelled);
                }
                StopAction::RevokeExecutor => self
                    .fixture
                    .admin
                    .revoke_source_executor(&self.fixture.executor)
                    .map_err(|error| error.to_string())?,
            }
        }
        Ok(GuardDecision::Continue)
    }
}

#[test]
fn cancellation_and_executor_revocation_stop_materialization_or_partial_persistence() {
    for during_persistence in [false, true] {
        for action in [StopAction::Cancel, StopAction::RevokeExecutor] {
            let h = Fixture::new(false);
            let backend = FiniteBackend::new(0);
            let mut guard = StopAtBoundary {
                fixture: &h,
                action,
                during_persistence,
                calls: 0,
                fired: false,
            };
            let failure = h
                .coordinator(&backend)
                .execute(h.request(), &mut guard)
                .unwrap_err();
            assert!(guard.fired);
            assert_eq!(
                failure.stage,
                if during_persistence {
                    ExecutionStage::Persistence
                } else {
                    ExecutionStage::Materialization
                }
            );
            assert!(failure.sealed.is_none());
            assert_eq!(h.state(), SourceAttemptState::Cancelled);
            assert_eq!(
                backend.calls.borrow().len(),
                usize::from(during_persistence)
            );
            assert_eq!(objects(&h.artifacts), usize::from(during_persistence));
            if during_persistence {
                assert_eq!(failure.cleanup, Some(CleanupOutcome::Removed));
                backend.assert_cleaned();
            }
            let attempt = h
                .engine
                .source_attempt(&h.observer, &scope(), &h.admission.binding().source_attempt)
                .unwrap();
            assert!(attempt.evidence.is_none());
            h.assert_base_unchanged();
        }
    }
}
