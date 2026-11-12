//! Source attempts use an explicitly finite complete reference analysis.
//! Passing these tests does not upgrade structural parser coverage.
use celetixo_engine::access::{AccessSet, Condition, Invariant};
use celetixo_engine::credentials::CredentialError;
use celetixo_engine::engine::*;
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::persistence::{
    DomainDecodeLimits, RestoreError, SourceAdmissionDependencies, SourceAdmissionRecord,
    restore_source_admission,
};
use celetixo_engine::repository::*;
use celetixo_engine::snapshot::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use celetixo_engine::transaction::*;
use std::collections::{BTreeMap, BTreeSet};

mod admission_persistence {
    use super::*;

    fn fixture() -> (Harness, SourcePatch, SourceAdmission, BTreeSet<ElementId>) {
        let h = Harness::new(false);
        let tx = h.execute("durable-admission");
        let patch = h.patch(&tx, "historical-attempt");
        let admission = h
            .engine
            .admit_source_attempt(
                &h.owner,
                &scope(),
                patch.clone(),
                IdempotencyKey::new("historical-operation").unwrap(),
            )
            .unwrap();
        (h, patch, admission, [element("file:main")].into())
    }

    // Independent layout encoder: deliberately does not use Canonical or the
    // record writer. It names every existing source-admission/v1 field.
    fn fields(a: &SourceAdmission) -> Vec<Vec<u8>> {
        vec![
            a.patch().0.as_bytes().to_vec(),
            a.operation_digest().as_bytes().to_vec(),
            a.resulting_snapshot().0.as_bytes().to_vec(),
            a.resulting_manifest().as_bytes().to_vec(),
            a.application_report().as_bytes().to_vec(),
            a.policy().as_bytes().to_vec(),
            a.policy_generation().to_be_bytes().to_vec(),
            a.plan().id().0.as_bytes().to_vec(),
            a.authorization().principal.as_str().as_bytes().to_vec(),
            a.authorization().capability_reference.as_bytes().to_vec(),
            a.admitted_at_ms().to_be_bytes().to_vec(),
            a.expires_at_ms().to_be_bytes().to_vec(),
            a.retention_policy().as_bytes().to_vec(),
        ]
    }
    fn encode(fields: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for field in fields {
            bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
            bytes.extend_from_slice(field);
        }
        bytes
    }
    fn restore(
        h: &Harness,
        patch: &SourcePatch,
        admission: &SourceAdmission,
        writes: &BTreeSet<ElementId>,
        bytes: &[u8],
        limits: DomainDecodeLimits,
    ) -> Result<SourceAdmission, RestoreError> {
        restore_source_admission(
            SourceAdmissionDependencies {
                base: &h.base,
                patch,
                policy: &h.policy,
                plan: admission.plan(),
                effective_writes: writes,
                authorization: admission.authorization(),
            },
            SourceAdmissionId(Digest::of(SourceAdmissionRecord::FORMAT, bytes)),
            bytes,
            limits,
        )
    }

    #[test]
    fn historical_admission_replays_exact_layout_after_revocation_and_expiry() {
        let (h, patch, admission, writes) = fixture();
        let bytes = encode(&fields(&admission));
        assert_eq!(bytes, admission.canonical_bytes());
        assert_eq!(
            Digest::of(SourceAdmissionRecord::FORMAT, &bytes),
            admission.id().0
        );
        let record = SourceAdmissionRecord::new(admission.clone());
        assert_eq!(record.admission(), &admission);
        assert_eq!(record.digest(), admission.id().0);
        assert_eq!(record.canonical_bytes(), bytes);
        h.admin.revoke(&h.owner).unwrap();
        h.admin.advance_time(20_000).unwrap();
        assert_eq!(
            restore(&h, &patch, &admission, &writes, &bytes, Default::default()).unwrap(),
            admission
        );
        // Metadata reconstruction cannot restore revoked session authority.
        assert_eq!(
            h.engine.current(&h.owner, &scope()),
            Err(EngineError::Unauthorized)
        );
        assert!(
            h.engine
                .claim_source_attempt(&h.executor, &admission)
                .is_err()
        );
    }

    #[test]
    fn every_admission_dependency_and_historical_grant_is_bound() {
        let (h, patch, admission, writes) = fixture();
        let original = fields(&admission);
        for index in [0, 1, 2, 3, 4, 5, 7, 9, 12] {
            let mut changed = original.clone();
            changed[index] = vec![42; 32];
            assert!(
                matches!(
                    restore(
                        &h,
                        &patch,
                        &admission,
                        &writes,
                        &encode(&changed),
                        Default::default()
                    ),
                    Err(RestoreError::BindingMismatch)
                ),
                "field {index}"
            );
        }
        for (index, value) in [
            (6, 2),
            (10, admission.expires_at_ms()),
            (11, admission.expires_at_ms() + 1),
        ] {
            let mut changed = original.clone();
            changed[index] = value.to_be_bytes().to_vec();
            assert!(
                restore(
                    &h,
                    &patch,
                    &admission,
                    &writes,
                    &encode(&changed),
                    Default::default()
                )
                .is_err(),
                "field {index}"
            );
        }
        let mut changed = original.clone();
        changed[8] = b"another-principal".to_vec();
        assert_eq!(
            restore(
                &h,
                &patch,
                &admission,
                &writes,
                &encode(&changed),
                Default::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
        assert!(matches!(
            restore(
                &h,
                &patch,
                &admission,
                &BTreeSet::new(),
                &encode(&original),
                Default::default()
            ),
            Err(RestoreError::SourceAttempt(_))
        ));
        let mut policy_input = h.policy.input().clone();
        policy_input.generation += 1;
        let other_policy = SourceExecutionPolicy::new(policy_input).unwrap();
        let bytes = encode(&original);
        assert_eq!(
            restore_source_admission(
                SourceAdmissionDependencies {
                    base: &h.base,
                    patch: &patch,
                    policy: &other_policy,
                    plan: admission.plan(),
                    effective_writes: &writes,
                    authorization: admission.authorization(),
                },
                admission.id(),
                &bytes,
                Default::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
    }

    #[test]
    fn malformed_admission_and_large_checked_dependencies_fail_with_explicit_budgets() {
        let (h, patch, admission, writes) = fixture();
        let bytes = encode(&fields(&admission));
        for end in 0..bytes.len() {
            assert!(
                restore(
                    &h,
                    &patch,
                    &admission,
                    &writes,
                    &bytes[..end],
                    Default::default()
                )
                .is_err(),
                "prefix {end}"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            restore(
                &h,
                &patch,
                &admission,
                &writes,
                &trailing,
                Default::default()
            ),
            Err(RestoreError::NonCanonical)
        );
        for limits in [
            DomainDecodeLimits {
                encoded_bytes: bytes.len() - 1,
                ..Default::default()
            },
            // The admission fits; its source/plan dependency graph does not.
            DomainDecodeLimits {
                encoded_bytes: bytes.len() + 1,
                ..Default::default()
            },
            DomainDecodeLimits {
                collection_entries: 1,
                ..Default::default()
            },
            DomainDecodeLimits {
                field_bytes: 31,
                ..Default::default()
            },
        ] {
            assert_eq!(
                restore(&h, &patch, &admission, &writes, &bytes, limits),
                Err(RestoreError::Limit)
            );
        }
        let mut malformed = fields(&admission);
        malformed[6] = vec![0; 9];
        assert_eq!(
            restore(
                &h,
                &patch,
                &admission,
                &writes,
                &encode(&malformed),
                Default::default()
            ),
            Err(RestoreError::Encoding)
        );
        let mut malformed = fields(&admission);
        malformed[8] = vec![0xff];
        assert_eq!(
            restore(
                &h,
                &patch,
                &admission,
                &writes,
                &encode(&malformed),
                Default::default()
            ),
            Err(RestoreError::Encoding)
        );
        let mut malformed = fields(&admission);
        malformed[8] = b"bad\0principal".to_vec();
        assert_eq!(
            restore(
                &h,
                &patch,
                &admission,
                &writes,
                &encode(&malformed),
                Default::default()
            ),
            Err(RestoreError::Identity)
        );
        assert_eq!(
            restore_source_admission(
                SourceAdmissionDependencies {
                    base: &h.base,
                    patch: &patch,
                    policy: &h.policy,
                    plan: admission.plan(),
                    effective_writes: &writes,
                    authorization: admission.authorization(),
                },
                SourceAdmissionId(d("wrong-record-address")),
                &bytes,
                Default::default()
            ),
            Err(RestoreError::DigestMismatch)
        );
    }
}

fn d(value: &str) -> Digest {
    Digest::of("source-attempt-fixture/v1", value.as_bytes())
}
fn element(value: &str) -> ElementId {
    ElementId::new(value).unwrap()
}
fn check_id(value: &str) -> CheckId {
    CheckId::new(value).unwrap()
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("org").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    }
}
fn resources() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 500,
        memory_bytes: 16 * 1024 * 1024,
        disk_bytes: 1024 * 1024,
        process_count: 8,
        file_descriptors: 64,
        stdout_bytes: 4096,
        stderr_bytes: 4096,
        artifact_bytes: 65_536,
    }
}
fn backend() -> BackendIdentity {
    BackendIdentity {
        name: "reference-executor-only".into(),
        version: "v1".into(),
        image: d("image"),
        profile: d("profile"),
    }
}
fn check(name: &str) -> SourceCheckDefinition {
    SourceCheckDefinition::new(SourceCheckInput {
        check: CheckSpec::new(
            check_id(name),
            vec!["/tools/reference-check".into(), name.into()],
            "reference-toolchain/v1".into(),
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
            version: "reference-toolchain/v1".into(),
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
        expected_artifacts: ["stdout", "stderr", "diagnostics"]
            .into_iter()
            .map(|name| ArtifactRequirement {
                name: name.into(),
                media_type: "text/plain".into(),
                max_bytes: 4096,
                required: true,
            })
            .collect(),
        depends_on: BTreeSet::new(),
        failure_policy: FailurePolicy::FailAttempt,
    })
    .unwrap()
}
fn source_snapshot() -> Snapshot {
    Snapshot::new(
        scope(),
        Revision(0),
        SnapshotManifest {
            base_commit: "finite-source-reference".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "celetixo/reference-source/v1".into(),
            inputs: ["main", "sibling", "consumer", "other"]
                .into_iter()
                .map(|name| WorkspaceInput {
                    path: format!("src/{name}.rs"),
                    layer: InputLayer::Base,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(b"old".to_vec()),
                })
                .collect(),
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap()
}
fn analysis_input(incomplete: bool) -> RepositoryAnalysisInput {
    RepositoryAnalysisInput {
        index_generation: d("index"),
        analyzer_version: "complete-finite-reference/v1".into(),
        configuration: d("analysis-config"),
        elements: ["file:main", "file:sibling", "file:consumer", "file:other"]
            .into_iter()
            .map(element)
            .collect(),
        // The sibling read introduced by the invariant itself reads this
        // prerequisite. Read dependencies do not imply downstream mutation.
        dependencies: [(element("file:sibling"), [element("file:consumer")].into())].into(),
        checks: [
            (element("file:main"), [check_id("verify")].into()),
            (element("file:consumer"), [check_id("downstream")].into()),
        ]
        .into(),
        gaps: if incomplete {
            [AnalysisGap {
                stage: "semantic".into(),
                path: Some("src/main.rs".into()),
                detail: "unresolved dynamic reference".into(),
            }]
            .into()
        } else {
            BTreeSet::new()
        },
    }
}

struct Harness {
    admin: Administrator,
    engine: Engine,
    owner: Session,
    executor: SourceExecutorCapability,
    base: Snapshot,
    analysis: RepositoryAnalysis,
    policy: SourceExecutionPolicy,
}

impl Harness {
    fn new(invariants: bool) -> Self {
        Self::with_base(invariants, source_snapshot())
    }

    fn with_base(invariants: bool, base: Snapshot) -> Self {
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
        let owner = admin
            .grant(
                PrincipalId::new("human").unwrap(),
                scope(),
                Permission::ALL.into(),
            )
            .unwrap();
        let analysis = RepositoryAnalysis::new(&base, analysis_input(false)).unwrap();
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
            paths: base
                .manifest()
                .inputs
                .iter()
                .map(|input| {
                    (
                        input.path.clone(),
                        SourcePathRule {
                            allowed_operations: [OperationKind::Replace, OperationKind::Delete]
                                .into(),
                            required_writes: [element(&format!(
                                "file:{}",
                                input
                                    .path
                                    .strip_prefix("src/")
                                    .unwrap()
                                    .strip_suffix(".rs")
                                    .unwrap()
                            ))]
                            .into(),
                            expected_base: Some(Preimage::of(input).unwrap()),
                        },
                    )
                })
                .collect(),
            checks: ["verify", "invariant", "downstream"]
                .into_iter()
                .map(check)
                .collect(),
            invariants: if invariants {
                vec![
                    SourceInvariantSpec {
                        id: InvariantId::new("source-contract").unwrap(),
                        scope: [element("file:main"), element("file:sibling")].into(),
                        contract_digest: d("source-contract"),
                        required_checks: [check_id("invariant")].into(),
                    },
                    SourceInvariantSpec {
                        id: InvariantId::new("a-prerequisite-contract").unwrap(),
                        scope: [element("file:consumer"), element("file:other")].into(),
                        contract_digest: d("prerequisite-contract"),
                        required_checks: [check_id("invariant")].into(),
                    },
                ]
            } else {
                vec![]
            },
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
        Self {
            admin,
            engine,
            owner,
            executor,
            base,
            analysis,
            policy,
        }
    }

    fn propose(&self, name: &str, writes: &[&str]) -> TransactionId {
        let id = TransactionId::new(name).unwrap();
        self.engine
            .propose(
                &self.owner,
                &scope(),
                id.clone(),
                self.base.id(),
                TransactionIntent {
                    objective: "finite source reference attempt".into(),
                    accesses: AccessSet {
                        writes: writes.iter().map(|value| element(value)).collect(),
                        ..Default::default()
                    },
                    invariants: vec![],
                    required_checks: vec![
                        self.policy.checks()[&check_id("verify")]
                            .input()
                            .check
                            .clone(),
                    ],
                    depends_on: BTreeSet::new(),
                },
            )
            .unwrap();
        id
    }

    fn execute(&self, name: &str) -> TransactionId {
        let id = self.propose(name, &["file:main"]);
        self.engine.analyze(&self.owner, &scope(), &id).unwrap();
        self.engine
            .reserve(&self.owner, &scope(), &id, 1000)
            .unwrap();
        self.engine.execute(&self.owner, &scope(), &id).unwrap();
        id
    }

    fn patch(&self, tx: &TransactionId, attempt: &str) -> SourcePatch {
        let context = self
            .engine
            .source_patch_context(&self.owner, &scope(), tx)
            .unwrap();
        let binding = context
            .bind(
                SourceAttemptId::new(attempt).unwrap(),
                "reference-test".into(),
                500,
            )
            .unwrap();
        SourcePatch::new(
            binding,
            self.policy.input().patch_policy.clone(),
            vec![PatchOperation::Replace {
                path: "src/main.rs".into(),
                expected: Preimage::of(
                    self.base
                        .manifest()
                        .inputs
                        .iter()
                        .find(|input| input.path == "src/main.rs")
                        .unwrap(),
                )
                .unwrap(),
                entry: PatchEntry::file(b"new".to_vec(), FileMode::Regular).unwrap(),
            }],
        )
        .unwrap()
    }
    fn admit(&self, tx: &TransactionId, attempt: &str) -> SourceAdmission {
        self.engine
            .admit_source_attempt(
                &self.owner,
                &scope(),
                self.patch(tx, attempt),
                IdempotencyKey::new(attempt).unwrap(),
            )
            .unwrap()
    }
}

#[test]
fn admission_and_legal_execution_flow_preserve_authoritative_source_and_revision() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let admission = h.admit(&tx, "attempt-1");
    assert_eq!(admission.binding().base_snapshot, h.base.id());
    assert_eq!(h.engine.current(&h.owner, &scope()).unwrap(), h.base);
    assert!(
        h.engine
            .source_application(&h.executor, &admission)
            .is_err()
    );
    assert_eq!(
        h.engine
            .claim_source_attempt(&h.executor, &admission)
            .unwrap()
            .state,
        SourceAttemptState::Patching
    );
    assert!(matches!(
        h.engine.claim_source_attempt(&h.executor, &admission),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::AlreadyClaimed
        ))
    ));
    assert!(
        h.engine
            .begin_source_verification(&h.executor, &admission)
            .is_err()
    );
    let candidate = h
        .engine
        .source_application(&h.executor, &admission)
        .unwrap();
    assert_eq!(candidate.snapshot().revision(), h.base.revision());
    assert_ne!(candidate.snapshot().id(), h.base.id());
    assert_eq!(
        h.engine
            .mark_source_patch_applied(&h.executor, &admission)
            .unwrap()
            .state,
        SourceAttemptState::PatchApplied
    );
    assert_eq!(
        h.engine
            .begin_source_verification(&h.executor, &admission)
            .unwrap()
            .state,
        SourceAttemptState::Verifying
    );
    let failed = h
        .engine
        .finish_source_attempt(
            &h.executor,
            &admission,
            SourceAttemptState::ExecutionFailed,
            "required check failed".into(),
        )
        .unwrap();
    assert_eq!(failed.state, SourceAttemptState::ExecutionFailed);
    assert!(
        h.engine
            .mark_source_patch_applied(&h.executor, &admission)
            .is_err()
    );
    assert!(
        h.engine
            .finish_source_attempt(
                &h.executor,
                &admission,
                SourceAttemptState::EvidenceSealed,
                "worker says green".into()
            )
            .is_err()
    );
    assert_eq!(
        h.engine.transaction(&h.owner, &scope(), &tx).unwrap().state,
        TransactionState::Executing
    );
    assert_eq!(
        h.engine.commit(
            &h.owner,
            &scope(),
            &tx,
            IdempotencyKey::new("never-publish").unwrap()
        ),
        Err(EngineError::SourceMutationUnsupported)
    );
    assert_eq!(h.engine.current(&h.owner, &scope()).unwrap(), h.base);
}

#[test]
fn idempotency_preserves_one_attempt_and_rejects_conflicting_or_parallel_work() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let patch = h.patch(&tx, "attempt-1");
    let key = IdempotencyKey::new("operation").unwrap();
    let first = h
        .engine
        .admit_source_attempt(&h.owner, &scope(), patch.clone(), key.clone())
        .unwrap();
    assert_eq!(
        first,
        h.engine
            .admit_source_attempt(&h.owner, &scope(), patch.clone(), key.clone())
            .unwrap()
    );
    let mut changed_binding = patch.binding().clone();
    changed_binding.producer = "changed-producer".into();
    let changed = SourcePatch::new(
        changed_binding,
        patch.policy().clone(),
        patch.operations().to_vec(),
    )
    .unwrap();
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, &scope(), changed, key),
        Err(EngineError::IdempotencyConflict)
    );
    assert!(matches!(
        h.engine.admit_source_attempt(
            &h.owner,
            &scope(),
            patch,
            IdempotencyKey::new("new-key").unwrap()
        ),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::AttemptIdentity
        ))
    ));
    assert!(matches!(
        h.engine.admit_source_attempt(
            &h.owner,
            &scope(),
            h.patch(&tx, "parallel"),
            IdempotencyKey::new("parallel").unwrap()
        ),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::AttemptActive
        ))
    ));
}

#[test]
fn retry_uses_new_identity_and_cannot_rewrite_failed_attempt_history() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let first = h.admit(&tx, "attempt-1");
    h.engine.claim_source_attempt(&h.executor, &first).unwrap();
    let failed = h
        .engine
        .finish_source_attempt(
            &h.executor,
            &first,
            SourceAttemptState::ExecutionFailed,
            "materializer failed".into(),
        )
        .unwrap();
    let second = h.admit(&tx, "attempt-2");
    assert_ne!(first.id(), second.id());
    assert_eq!(
        h.engine
            .source_attempt(&h.owner, &scope(), &first.binding().source_attempt)
            .unwrap(),
        failed
    );
    assert!(h.engine.claim_source_attempt(&h.executor, &first).is_err());
    h.engine.claim_source_attempt(&h.executor, &second).unwrap();
}

#[test]
fn worker_verifier_session_cannot_replace_separate_scoped_executor_capability() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let admission = h.admit(&tx, "attempt");
    let mut foreign_scope = scope();
    foreign_scope.workspace = WorkspaceId::new("foreign").unwrap();
    let foreign_session = h
        .admin
        .grant(
            PrincipalId::new("worker-verifier").unwrap(),
            foreign_scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    assert!(
        h.engine
            .source_patch_context(&foreign_session, &scope(), &tx)
            .is_err()
    );
    let wrong_executor = h
        .admin
        .grant_source_executor(
            PrincipalId::new("foreign-executor").unwrap(),
            foreign_scope,
            backend(),
            [ExecutionTier::T1].into(),
            resources(),
            10_000,
        )
        .unwrap();
    assert!(
        h.engine
            .claim_source_attempt(&wrong_executor, &admission)
            .is_err()
    );
    let (_, foreign_admin) = InMemoryStore::new();
    let different_store = foreign_admin
        .grant_source_executor(
            PrincipalId::new("same-label").unwrap(),
            scope(),
            backend(),
            [ExecutionTier::T1].into(),
            resources(),
            10_000,
        )
        .unwrap();
    assert!(matches!(
        h.engine.claim_source_attempt(&different_store, &admission),
        Err(EngineError::Unauthorized)
    ));
    h.engine
        .claim_source_attempt(&h.executor, &admission)
        .unwrap();
    let second_executor = h
        .admin
        .grant_source_executor(
            PrincipalId::new("other-trusted").unwrap(),
            scope(),
            backend(),
            [ExecutionTier::T1].into(),
            resources(),
            10_000,
        )
        .unwrap();
    assert!(
        h.engine
            .source_application(&second_executor, &admission)
            .is_err()
    );
}

#[test]
fn cancellation_expiry_and_revocation_prevent_continuation() {
    for scenario in 0..4 {
        let h = Harness::new(false);
        let tx = h.execute("transaction");
        let admission = h.admit(&tx, "attempt");
        h.engine
            .claim_source_attempt(&h.executor, &admission)
            .unwrap();
        match scenario {
            0 => h
                .engine
                .abort(&h.owner, &scope(), &tx, "human cancelled".into())
                .unwrap(),
            1 => {
                h.admin.advance_time(501).unwrap();
            }
            2 => h.admin.revoke(&h.owner).unwrap(),
            _ => h.admin.revoke_source_executor(&h.executor).unwrap(),
        }
        assert!(
            h.engine
                .mark_source_patch_applied(&h.executor, &admission)
                .is_err()
        );
        let reader = h
            .admin
            .grant(
                PrincipalId::new("reader").unwrap(),
                scope(),
                [Permission::Read].into(),
            )
            .unwrap();
        let view = h
            .engine
            .source_attempt(&reader, &scope(), &admission.binding().source_attempt)
            .unwrap();
        assert!(matches!(
            view.state,
            SourceAttemptState::Cancelled | SourceAttemptState::TimedOut
        ));
        assert_eq!(h.engine.current(&reader, &scope()).unwrap(), h.base);
    }
}

#[test]
fn policy_cas_invalidates_prepolicy_analysis_without_advancing_source() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let admission = h.admit(&tx, "attempt");
    let mut input = h.policy.input().clone();
    input.generation += 1;
    input.retention_policy = d("new-retention");
    let next = SourceExecutionPolicy::new(input).unwrap();
    assert!(
        h.engine
            .install_source_policy(&h.owner, &scope(), Some(d("wrong-policy")), next.clone())
            .is_err()
    );
    h.engine
        .install_source_policy(&h.owner, &scope(), Some(h.policy.digest()), next.clone())
        .unwrap();
    // A lost install response returns the same outcome and does not bump twice.
    h.engine
        .install_source_policy(&h.owner, &scope(), Some(h.policy.digest()), next)
        .unwrap();
    assert_eq!(
        h.engine.source_patch_context(&h.owner, &scope(), &tx),
        Err(EngineError::ModelChanged)
    );
    assert!(
        h.engine
            .claim_source_attempt(&h.executor, &admission)
            .is_err()
    );
    assert_eq!(
        h.engine
            .source_attempt(&h.owner, &scope(), &admission.binding().source_attempt)
            .unwrap()
            .state,
        SourceAttemptState::RevalidationRequired
    );
    let fresh = h.propose("fresh-transaction", &["file:main"]);
    h.engine.analyze(&h.owner, &scope(), &fresh).unwrap();
    assert_eq!(h.engine.current(&h.owner, &scope()).unwrap(), h.base);
}

#[test]
fn incomplete_analysis_never_becomes_execution_safe_through_a_small_patch() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let old_patch = h.patch(&tx, "attempt");
    let partial = RepositoryAnalysis::new(&h.base, analysis_input(true)).unwrap();
    h.engine
        .install_repository_analysis(&h.owner, &scope(), h.analysis.digest(), partial)
        .unwrap();
    assert!(
        h.engine
            .admit_source_attempt(
                &h.owner,
                &scope(),
                old_patch,
                IdempotencyKey::new("rejected").unwrap()
            )
            .is_err()
    );
    assert!(
        h.engine
            .source_attempt(
                &h.owner,
                &scope(),
                &SourceAttemptId::new("attempt").unwrap()
            )
            .is_err()
    );
    let fresh = h.propose("fresh", &["file:main"]);
    assert_eq!(
        h.engine.analyze(&h.owner, &scope(), &fresh),
        Err(EngineError::IncompleteAnalysis)
    );
    let mut next = h.policy.input().clone();
    next.generation += 1;
    assert_eq!(
        h.engine.install_source_policy(
            &h.owner,
            &scope(),
            Some(h.policy.digest()),
            SourceExecutionPolicy::new(next).unwrap()
        ),
        Err(EngineError::IncompleteAnalysis)
    );
}

#[test]
fn new_complete_analysis_requires_matching_policy_before_fresh_analysis() {
    let h = Harness::new(false);
    let mut changed = analysis_input(false);
    changed.index_generation = d("new-complete-index");
    let changed = RepositoryAnalysis::new(&h.base, changed).unwrap();
    h.engine
        .install_repository_analysis(&h.owner, &scope(), h.analysis.digest(), changed)
        .unwrap();
    let tx = h.propose("fresh-analysis-stale-policy", &["file:main"]);
    assert!(matches!(
        h.engine.analyze(&h.owner, &scope(), &tx),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::PolicyChanged
        ))
    ));
    assert_eq!(
        h.engine.transaction(&h.owner, &scope(), &tx).unwrap().state,
        TransactionState::Proposed
    );
}

#[test]
fn invariant_scopes_reach_sibling_dependencies_and_cannot_be_omitted_from_plan() {
    let h = Harness::new(true);
    let tx = h.execute("transaction");
    let accesses = h.engine.attempts(&h.owner, &scope(), &tx).unwrap();
    let derived = accesses[0].analysis.as_ref().unwrap().derived();
    assert!(derived.reads.contains(&element("file:sibling")));
    assert!(derived.reads.contains(&element("file:consumer")));
    assert!(derived.reads.contains(&element("file:other")));
    assert!(derived.invariant_scopes.contains(&element("file:main")));
    assert!(derived.invariant_scopes.contains(&element("file:sibling")));
    let sibling_write = AccessSet {
        writes: [element("file:sibling")].into(),
        ..Default::default()
    };
    assert!(!derived.conflicts(&sibling_write).is_empty());
    let admission = h.admit(&tx, "attempt");
    let checks: BTreeSet<_> = admission
        .plan()
        .checks()
        .iter()
        .filter(|check| check.required())
        .map(|check| check.definition().id().clone())
        .collect();
    assert_eq!(
        checks,
        [
            check_id("verify"),
            check_id("invariant"),
            check_id("downstream")
        ]
        .into()
    );
    assert_eq!(
        admission.plan().invariant_contracts()[&InvariantId::new("source-contract").unwrap()],
        d("source-contract")
    );
}

#[test]
fn stale_or_forged_patch_binding_and_undeclared_paths_are_rejected_before_attempt() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let original = h.patch(&tx, "attempt");
    for variant in 0..6 {
        let mut changed = original.binding().clone();
        changed.source_attempt =
            SourceAttemptId::new(format!("rejected-attempt-{variant}")).unwrap();
        match variant {
            0 => changed.principal = PrincipalId::new("foreign").unwrap(),
            1 => changed.analysis_artifact = d("other-analysis"),
            2 => changed.index_generation = d("other-index"),
            3 => changed.impact_request = d("other-request"),
            4 => changed.impact_result = d("other-result"),
            _ => changed.transaction_attempt += 1,
        }
        let patch = SourcePatch::new(
            changed,
            original.policy().clone(),
            original.operations().to_vec(),
        )
        .unwrap();
        assert!(
            h.engine
                .admit_source_attempt(
                    &h.owner,
                    &scope(),
                    patch,
                    IdempotencyKey::new(format!("rejected-{variant}")).unwrap()
                )
                .is_err()
        );
    }
    let wrong_path = SourcePatch::new(
        original.binding().clone(),
        original.policy().clone(),
        vec![PatchOperation::Delete {
            path: "src/sibling.rs".into(),
            expected: Preimage::of(
                h.base
                    .manifest()
                    .inputs
                    .iter()
                    .find(|input| input.path == "src/sibling.rs")
                    .unwrap(),
            )
            .unwrap(),
        }],
    )
    .unwrap();
    assert!(matches!(
        h.engine.admit_source_attempt(
            &h.owner,
            &scope(),
            wrong_path,
            IdempotencyKey::new("wrong-path").unwrap()
        ),
        Err(EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::UnauthorizedPath(_)
        )))
    ));
    assert!(
        h.engine
            .source_attempt(
                &h.owner,
                &scope(),
                &SourceAttemptId::new("attempt").unwrap()
            )
            .is_err()
    );
}

#[test]
fn numeric_reference_invariant_is_not_relabelled_as_a_source_property() {
    let h = Harness::new(false);
    let tx = TransactionId::new("numeric-invariant").unwrap();
    h.engine
        .propose(
            &h.owner,
            &scope(),
            tx.clone(),
            h.base.id(),
            TransactionIntent {
                objective: "cannot reinterpret reference invariant".into(),
                accesses: AccessSet {
                    writes: [element("file:main")].into(),
                    ..Default::default()
                },
                invariants: vec![Invariant {
                    id: InvariantId::new("numeric").unwrap(),
                    condition: Condition::Present(element("file:main")),
                }],
                required_checks: vec![h.policy.checks()[&check_id("verify")].input().check.clone()],
                depends_on: BTreeSet::new(),
            },
        )
        .unwrap();
    h.engine.analyze(&h.owner, &scope(), &tx).unwrap();
    h.engine.reserve(&h.owner, &scope(), &tx, 1000).unwrap();
    h.engine.execute(&h.owner, &scope(), &tx).unwrap();
    assert!(matches!(
        h.engine.admit_source_attempt(
            &h.owner,
            &scope(),
            h.patch(&tx, "attempt"),
            IdempotencyKey::new("no-source-proof").unwrap()
        ),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::UnsupportedInvariant
        ))
    ));
}

#[test]
fn rejected_admission_identity_and_error_survive_policy_fixes_and_lost_responses() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let template = h.patch(&tx, "rejected-attempt");
    let make_patch = |binding: PatchBinding| {
        SourcePatch::new(
            binding,
            template.policy().clone(),
            vec![PatchOperation::Replace {
                path: "src/main.rs".into(),
                expected: Preimage::of(
                    h.base
                        .manifest()
                        .inputs
                        .iter()
                        .find(|input| input.path == "src/main.rs")
                        .unwrap(),
                )
                .unwrap(),
                entry: PatchEntry::file(b"other".to_vec(), FileMode::Regular).unwrap(),
            }],
        )
        .unwrap()
    };
    let rejected_patch = make_patch(template.binding().clone());
    let key = IdempotencyKey::new("rejected-operation").unwrap();
    let original = h
        .engine
        .admit_source_attempt(&h.owner, &scope(), rejected_patch.clone(), key.clone())
        .unwrap_err();
    assert!(matches!(
        original,
        EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::CheckInputMismatch(_)
        ))
    ));
    let rejection = h
        .engine
        .source_rejection(&h.owner, &scope(), &rejected_patch.binding().source_attempt)
        .unwrap();
    assert_eq!(rejection.events[0].to, SourceAttemptState::Rejected);
    assert_eq!(rejection.request, rejected_patch.id().0);
    assert!(
        h.engine
            .source_attempt(&h.owner, &scope(), &rejected_patch.binding().source_attempt)
            .is_err()
    );
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, &scope(), template.clone(), key.clone()),
        Err(EngineError::IdempotencyConflict)
    );
    let mut next = h.policy.input().clone();
    next.generation += 1;
    next.checks = next
        .checks
        .iter()
        .map(|definition| {
            let mut input = definition.input().clone();
            input.inputs.insert(
                "src/main.rs".into(),
                Digest::of("celetixo/source-content/v1", b"other"),
            );
            SourceCheckDefinition::new(input).unwrap()
        })
        .collect();
    h.engine
        .install_source_policy(
            &h.owner,
            &scope(),
            Some(h.policy.digest()),
            SourceExecutionPolicy::new(next).unwrap(),
        )
        .unwrap();
    assert_eq!(
        h.engine
            .admit_source_attempt(&h.owner, &scope(), rejected_patch.clone(), key),
        Err(original)
    );
    assert_eq!(
        h.engine
            .source_rejection(&h.owner, &scope(), &rejected_patch.binding().source_attempt)
            .unwrap(),
        rejection
    );
    h.engine
        .abort(
            &h.owner,
            &scope(),
            &tx,
            "new policy needs fresh transaction".into(),
        )
        .unwrap();
    let fresh = h.execute("fresh-transaction");
    let context = h
        .engine
        .source_patch_context(&h.owner, &scope(), &fresh)
        .unwrap();
    let reused = make_patch(
        context
            .bind(
                SourceAttemptId::new("rejected-attempt").unwrap(),
                "fresh-policy".into(),
                500,
            )
            .unwrap(),
    );
    assert!(matches!(
        h.engine.admit_source_attempt(
            &h.owner,
            &scope(),
            reused,
            IdempotencyKey::new("different-key").unwrap()
        ),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::AttemptIdentity
        ))
    ));
    let fresh_patch = make_patch(
        context
            .bind(
                SourceAttemptId::new("fresh-attempt").unwrap(),
                "fresh-policy".into(),
                500,
            )
            .unwrap(),
    );
    h.engine
        .admit_source_attempt(
            &h.owner,
            &scope(),
            fresh_patch,
            IdempotencyKey::new("fresh-key").unwrap(),
        )
        .unwrap();
}

#[test]
fn unauthorized_principal_cannot_poison_an_owners_attempt_or_idempotency_identity() {
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let patch = h.patch(&tx, "unpoisoned-attempt");
    let other = h
        .admin
        .grant(
            PrincipalId::new("other-principal").unwrap(),
            scope(),
            [Permission::Execute, Permission::Read].into(),
        )
        .unwrap();
    let key = IdempotencyKey::new("unpoisoned-key").unwrap();
    assert_eq!(
        h.engine
            .admit_source_attempt(&other, &scope(), patch.clone(), key.clone()),
        Err(EngineError::Unauthorized)
    );
    assert!(
        h.engine
            .source_rejection(&h.owner, &scope(), &patch.binding().source_attempt)
            .is_err()
    );
    let same_principal_different_capability = h
        .admin
        .grant(h.owner.principal().clone(), scope(), Permission::ALL.into())
        .unwrap();
    assert_eq!(
        h.engine.admit_source_attempt(
            &same_principal_different_capability,
            &scope(),
            patch.clone(),
            key.clone()
        ),
        Err(EngineError::InvalidLease)
    );
    assert!(
        h.engine
            .source_rejection(&h.owner, &scope(), &patch.binding().source_attempt)
            .is_err()
    );
    h.engine
        .admit_source_attempt(&h.owner, &scope(), patch, key)
        .unwrap();
}

#[test]
fn terminal_attempts_cannot_be_revived_and_single_claim_survives_competing_handles() {
    for terminal in [
        SourceAttemptState::EvidenceSealed,
        SourceAttemptState::Rejected,
        SourceAttemptState::PatchConflict,
        SourceAttemptState::ExecutionFailed,
        SourceAttemptState::TimedOut,
        SourceAttemptState::Cancelled,
        SourceAttemptState::EvidenceIncomplete,
        SourceAttemptState::RevalidationRequired,
    ] {
        assert!(!terminal.allows(SourceAttemptState::Admitted));
        assert!(!terminal.allows(SourceAttemptState::Verifying));
    }
    let h = Harness::new(false);
    let tx = h.execute("transaction");
    let admission = h.admit(&tx, "attempt");
    let engine = h.engine.clone();
    let executor = h.executor.clone();
    let copied = admission.clone();
    let contender = std::thread::spawn(move || engine.claim_source_attempt(&executor, &copied));
    let local = h.engine.claim_source_attempt(&h.executor, &admission);
    let remote = contender.join().unwrap();
    assert_eq!(usize::from(local.is_ok()) + usize::from(remote.is_ok()), 1);
}

#[test]
fn candidate_credential_admission_covers_changed_and_unchanged_effective_files() {
    let material = format!("{}{}", "ghp_", "x".repeat(36)).into_bytes();
    for inherited in [false, true] {
        let base = if inherited {
            let original = source_snapshot();
            let mut manifest = original.manifest().clone();
            manifest
                .inputs
                .iter_mut()
                .find(|input| input.path == "src/sibling.rs")
                .unwrap()
                .bytes = Some(material.clone());
            Snapshot::new(scope(), Revision(0), manifest, BTreeMap::new()).unwrap()
        } else {
            source_snapshot()
        };
        let h = Harness::with_base(false, base);
        let tx = h.execute("credential-attempt");
        let template = h.patch(&tx, "credential-rejected");
        let patch = if inherited {
            template
        } else {
            SourcePatch::new(
                template.binding().clone(),
                template.policy().clone(),
                vec![PatchOperation::Replace {
                    path: "src/main.rs".into(),
                    expected: Preimage::of(
                        h.base
                            .manifest()
                            .inputs
                            .iter()
                            .find(|input| input.path == "src/main.rs")
                            .unwrap(),
                    )
                    .unwrap(),
                    entry: PatchEntry::file(material.clone(), FileMode::Regular).unwrap(),
                }],
            )
            .unwrap()
        };
        let expected_path = if inherited {
            "src/sibling.rs"
        } else {
            "src/main.rs"
        };
        let error = h
            .engine
            .admit_source_attempt(
                &h.owner,
                &scope(),
                patch,
                IdempotencyKey::new("credential-denial").unwrap(),
            )
            .unwrap_err();
        assert_eq!(
            error,
            EngineError::SourceAttempt(SourceAttemptError::Credential(
                CredentialError::SensitiveInput(expected_path.into())
            ))
        );
        assert!(
            !error
                .to_string()
                .contains(std::str::from_utf8(&material).unwrap())
        );
        assert!(
            h.engine
                .source_attempt(
                    &h.owner,
                    &scope(),
                    &SourceAttemptId::new("credential-rejected").unwrap()
                )
                .is_err()
        );
        assert_eq!(h.engine.current(&h.owner, &scope()).unwrap(), h.base);
    }
}

#[test]
fn only_installed_exact_public_review_can_admit_a_new_sensitive_path() {
    let h = Harness::new(false);
    let public = b"PUBLIC_API_BASE=https://example.invalid\n";
    let mut input = h.policy.input().clone();
    input.generation += 1;
    input.paths.insert(
        ".env".into(),
        SourcePathRule {
            allowed_operations: [OperationKind::Create].into(),
            required_writes: [element("file:main")].into(),
            expected_base: None,
        },
    );
    input.credentials.reviewed_public_inputs.insert(
        ".env".into(),
        [Digest::of("celetixo/source-content/v1", public)].into(),
    );
    let reviewed = SourceExecutionPolicy::new(input).unwrap();
    assert_ne!(h.policy.digest(), reviewed.digest());
    let worker = h
        .admin
        .grant(
            PrincipalId::new("worker").unwrap(),
            scope(),
            [Permission::Execute].into(),
        )
        .unwrap();
    assert_eq!(
        h.engine.install_source_policy(
            &worker,
            &scope(),
            Some(h.policy.digest()),
            reviewed.clone()
        ),
        Err(EngineError::Unauthorized)
    );
    h.engine
        .install_source_policy(&h.owner, &scope(), Some(h.policy.digest()), reviewed)
        .unwrap();
    let tx = h.execute("reviewed-public-input");
    let template = h.patch(&tx, "reviewed-attempt");
    let create = |binding: PatchBinding, bytes: Vec<u8>| {
        let mut operations = template.operations().to_vec();
        operations.push(PatchOperation::Create {
            path: ".env".into(),
            entry: PatchEntry::file(bytes, FileMode::Regular).unwrap(),
        });
        SourcePatch::new(binding, template.policy().clone(), operations).unwrap()
    };
    let admission = h
        .engine
        .admit_source_attempt(
            &h.owner,
            &scope(),
            create(template.binding().clone(), public.to_vec()),
            IdempotencyKey::new("reviewed").unwrap(),
        )
        .unwrap();
    h.engine
        .claim_source_attempt(&h.executor, &admission)
        .unwrap();
    h.engine
        .finish_source_attempt(
            &h.executor,
            &admission,
            SourceAttemptState::ExecutionFailed,
            "controlled fixture ends before execution".into(),
        )
        .unwrap();
    let context = h
        .engine
        .source_patch_context(&h.owner, &scope(), &tx)
        .unwrap();
    let changed = create(
        context
            .bind(
                SourceAttemptId::new("unreviewed-bytes").unwrap(),
                "worker".into(),
                500,
            )
            .unwrap(),
        b"PUBLIC_API_BASE=https://changed.invalid\n".to_vec(),
    );
    assert_eq!(
        h.engine.admit_source_attempt(
            &h.owner,
            &scope(),
            changed,
            IdempotencyKey::new("changed-public-input").unwrap()
        ),
        Err(EngineError::SourceAttempt(SourceAttemptError::Credential(
            CredentialError::SensitiveInput(".env".into())
        )))
    );
}

#[test]
fn optional_checks_are_bound_to_idempotency_and_cannot_remove_mandatory_checks() {
    let h = Harness::new(false);
    let tx = h.execute("optional-transaction");
    let patch = h.patch(&tx, "optional-attempt");
    let key = IdempotencyKey::new("optional-operation").unwrap();
    let options = SourceAttemptOptions {
        optional_checks: [check_id("invariant"), check_id("downstream")].into(),
    };
    let admission = h
        .engine
        .admit_source_attempt_with_options(
            &h.owner,
            &scope(),
            patch.clone(),
            options.clone(),
            key.clone(),
        )
        .unwrap();
    assert_eq!(admission.plan().checks().len(), 3);
    for check in admission.plan().checks() {
        let name = check.definition().id().as_str();
        assert_eq!(check.required(), name == "verify");
        if name != "verify" {
            assert!(
                check
                    .rationales()
                    .contains(&CheckRationale::OptionalRequest)
            );
        }
    }
    let reverse_delivery = SourceAttemptOptions {
        optional_checks: [check_id("downstream"), check_id("invariant")].into(),
    };
    assert_eq!(
        h.engine
            .admit_source_attempt_with_options(
                &h.owner,
                &scope(),
                patch.clone(),
                reverse_delivery,
                key.clone(),
            )
            .unwrap(),
        admission
    );
    for changed in [
        SourceAttemptOptions::default(),
        SourceAttemptOptions {
            optional_checks: [check_id("invariant")].into(),
        },
    ] {
        assert_eq!(
            h.engine.admit_source_attempt_with_options(
                &h.owner,
                &scope(),
                patch.clone(),
                changed,
                key.clone(),
            ),
            Err(EngineError::IdempotencyConflict)
        );
    }
    assert_eq!(
        h.engine
            .source_attempt(&h.owner, &scope(), &patch.binding().source_attempt)
            .unwrap()
            .admission,
        admission
    );
    assert_eq!(h.engine.current(&h.owner, &scope()).unwrap(), h.base);
}

#[test]
fn unknown_and_explicitly_required_optional_checks_reject_without_runnable_work() {
    for (name, expected) in [
        (
            "not-installed",
            SourcePlanError::MissingCheck(check_id("not-installed")),
        ),
        (
            "verify",
            SourcePlanError::ConflictingRequirement(check_id("verify")),
        ),
    ] {
        let h = Harness::new(false);
        let tx = h.execute("optional-rejection-transaction");
        let patch = h.patch(&tx, "optional-rejection");
        let options = SourceAttemptOptions {
            optional_checks: [check_id(name)].into(),
        };
        let key = IdempotencyKey::new("optional-rejection-operation").unwrap();
        let error = EngineError::SourceAttempt(SourceAttemptError::Plan(expected));
        assert_eq!(
            h.engine.admit_source_attempt_with_options(
                &h.owner,
                &scope(),
                patch.clone(),
                options.clone(),
                key.clone(),
            ),
            Err(error.clone())
        );
        let rejection = h
            .engine
            .source_rejection(&h.owner, &scope(), &patch.binding().source_attempt)
            .unwrap();
        assert_ne!(rejection.request, patch.id().0);
        assert_eq!(rejection.error, error);
        assert!(
            h.engine
                .source_attempt(&h.owner, &scope(), &patch.binding().source_attempt)
                .is_err()
        );
        assert_eq!(
            h.engine.admit_source_attempt_with_options(
                &h.owner,
                &scope(),
                patch.clone(),
                options,
                key,
            ),
            Err(error)
        );
        assert_eq!(
            h.engine
                .source_rejection(&h.owner, &scope(), &patch.binding().source_attempt)
                .unwrap(),
            rejection
        );
    }
}

#[test]
fn a_mandatory_dependency_promotes_an_optional_check_without_losing_provenance() {
    let h = Harness::new(false);
    let mut input = h.policy.input().clone();
    input.generation += 1;
    let configured = input
        .checks
        .iter_mut()
        .find(|check| check.id() == &check_id("verify"))
        .unwrap();
    let mut definition = configured.input().clone();
    definition.depends_on.insert(check_id("downstream"));
    *configured = SourceCheckDefinition::new(definition).unwrap();
    h.engine
        .install_source_policy(
            &h.owner,
            &scope(),
            Some(h.policy.digest()),
            SourceExecutionPolicy::new(input).unwrap(),
        )
        .unwrap();
    let tx = h.execute("promoted-optional-transaction");
    let admission = h
        .engine
        .admit_source_attempt_with_options(
            &h.owner,
            &scope(),
            h.patch(&tx, "promoted-optional-attempt"),
            SourceAttemptOptions {
                optional_checks: [check_id("downstream")].into(),
            },
            IdempotencyKey::new("promoted-optional-operation").unwrap(),
        )
        .unwrap();
    let prerequisite = &admission.plan().checks()[0];
    assert_eq!(prerequisite.definition().id(), &check_id("downstream"));
    assert!(prerequisite.required());
    assert!(
        prerequisite
            .rationales()
            .contains(&CheckRationale::OptionalRequest)
    );
    assert!(
        prerequisite
            .rationales()
            .contains(&CheckRationale::DependencyOf(check_id("verify")))
    );
}

#[test]
fn oversized_optional_selection_reserves_neither_an_attempt_nor_an_idempotency_key() {
    let h = Harness::new(false);
    let tx = h.execute("oversized-options-transaction");
    let patch = h.patch(&tx, "oversized-options-attempt");
    let key = IdempotencyKey::new("oversized-options-operation").unwrap();
    assert_eq!(
        h.engine.admit_source_attempt_with_options(
            &h.owner,
            &scope(),
            patch.clone(),
            SourceAttemptOptions {
                optional_checks: (0..4097)
                    .map(|number| check_id(&format!("extra-{number}")))
                    .collect(),
            },
            key.clone(),
        ),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::InvalidOptions
        ))
    );
    assert!(
        h.engine
            .source_attempt(&h.owner, &scope(), &patch.binding().source_attempt)
            .is_err()
    );
    assert!(
        h.engine
            .source_rejection(&h.owner, &scope(), &patch.binding().source_attempt)
            .is_err()
    );
    // The original empty-options entry point retains its existing behavior.
    let admission = h
        .engine
        .admit_source_attempt(&h.owner, &scope(), patch.clone(), key.clone())
        .unwrap();
    assert_eq!(
        h.engine
            .admit_source_attempt_with_options(
                &h.owner,
                &scope(),
                patch,
                SourceAttemptOptions::default(),
                key,
            )
            .unwrap(),
        admission
    );
}

mod complete_source_reconstruction {
    use super::*;
    use celetixo_engine::engine::authority::{AuthorityState, NativeRecordLimits, SourceRecords};

    fn records(h: &Harness) -> SourceRecords {
        h.admin
            .inspect_source_records(&scope(), NativeRecordLimits::default())
            .unwrap()
    }
    fn check(h: &Harness) -> SourceRecords {
        let r = records(h);
        let state =
            AuthorityState::from_source_records(r.clone(), NativeRecordLimits::default()).unwrap();
        assert_eq!(
            state.source_records(NativeRecordLimits::default()).unwrap(),
            r
        );
        assert!(state.native_records(NativeRecordLimits::default()).is_err());
        r
    }
    fn rejects(r: SourceRecords) {
        assert!(AuthorityState::from_source_records(r, NativeRecordLimits::default()).is_err());
    }

    #[test]
    fn source_transaction_and_execution_states_reopen_without_synthesizing_native_patches() {
        let h = Harness::new(false);
        check(&h);
        let tx = h.propose("phases", &["file:main"]);
        check(&h);
        h.engine.analyze(&h.owner, &scope(), &tx).unwrap();
        check(&h);
        h.engine.reserve(&h.owner, &scope(), &tx, 1000).unwrap();
        check(&h);
        h.engine.execute(&h.owner, &scope(), &tx).unwrap();
        check(&h);
        let patch = h.patch(&tx, "source-phases");
        let a = h
            .engine
            .admit_source_attempt(
                &h.owner,
                &scope(),
                patch,
                IdempotencyKey::new("source-phases").unwrap(),
            )
            .unwrap();
        check(&h);
        h.engine.claim_source_attempt(&h.executor, &a).unwrap();
        check(&h);
        h.engine.mark_source_patch_applied(&h.executor, &a).unwrap();
        check(&h);
        h.engine.begin_source_verification(&h.executor, &a).unwrap();
        check(&h);
        h.engine
            .finish_source_attempt(
                &h.executor,
                &a,
                SourceAttemptState::ExecutionFailed,
                "failed check".into(),
            )
            .unwrap();
        check(&h);
        h.engine
            .abort(
                &h.owner,
                &scope(),
                &tx,
                "cancelled\0with binary reason".into(),
            )
            .unwrap();
        check(&h);
    }
    #[test]
    fn expired_unreconciled_attempt_and_same_tick_revocation_preserve_history() {
        for revoke in [false, true] {
            let h = Harness::new(false);
            let tx = h.execute("history");
            let a = h
                .engine
                .admit_source_attempt(
                    &h.owner,
                    &scope(),
                    h.patch(&tx, "history"),
                    IdempotencyKey::new("history").unwrap(),
                )
                .unwrap();
            let initial = check(&h);
            if revoke {
                h.admin.revoke(&h.owner).unwrap();
            } else {
                h.admin.advance_time(501).unwrap();
            }
            let restored = check(&h);
            assert_eq!(
                restored.attempts[&a.binding().source_attempt].state,
                SourceAttemptState::Admitted
            );
            assert_eq!(
                restored.attempts[&a.binding().source_attempt].admission,
                initial.attempts[&a.binding().source_attempt].admission
            );
            // Explicit reconciliation creates the expiry/revocation transition;
            // importing rows must not invent that transition on its own.
            if !revoke {
                h.engine
                    .source_attempt(&h.owner, &scope(), &a.binding().source_attempt)
                    .unwrap();
                check(&h);
            }
        }
    }
    #[test]
    fn replaced_heads_retain_original_policy_analysis_and_exact_optional_selection() {
        let h = Harness::new(false);
        let tx = h.execute("historical-definitions");
        let patch = h.patch(&tx, "old-attempt");
        let a = h
            .engine
            .admit_source_attempt_with_options(
                &h.owner,
                &scope(),
                patch.clone(),
                SourceAttemptOptions {
                    optional_checks: [check_id("invariant")].into(),
                },
                IdempotencyKey::new("old-operation").unwrap(),
            )
            .unwrap();
        let initial = check(&h);
        let mut input = h.policy.input().clone();
        input.generation = 2;
        input.retention_policy = d("second-retention");
        let policy = SourceExecutionPolicy::new(input).unwrap();
        h.engine
            .install_source_policy(&h.owner, &scope(), Some(h.policy.digest()), policy.clone())
            .unwrap();
        let mut input = analysis_input(false);
        input.index_generation = d("second-index");
        let analysis = RepositoryAnalysis::new(&h.base, input).unwrap();
        h.engine
            .install_repository_analysis(&h.owner, &scope(), h.analysis.digest(), analysis.clone())
            .unwrap();
        let r = check(&h);
        assert_eq!(r.policies.len(), 2);
        assert_eq!(r.analyses.len(), 2);
        assert_eq!(r.policy_head, Some(policy.digest()));
        assert_eq!(r.analysis_head, Some(analysis.digest()));
        assert_eq!(
            r.attempts[&patch.binding().source_attempt],
            initial.attempts[&patch.binding().source_attempt]
        );
        assert_eq!(r.attempts[&patch.binding().source_attempt].admission, a);
        // An installed but stale policy head is valid stored state. Live source
        // context still rejects it rather than silently rebinding old work.
        assert!(
            h.engine
                .source_patch_context(&h.owner, &scope(), &tx)
                .is_err()
        );
    }
    #[test]
    fn invalid_submitted_attempt_and_identity_collision_keep_original_requests_and_outcomes() {
        let h = Harness::new(false);
        let tx = h.execute("rejected-binding");
        let valid = h.patch(&tx, "accepted");
        let mut binding = valid.binding().clone();
        binding.transaction_attempt = 999;
        binding.source_attempt = SourceAttemptId::new("rejected-999").unwrap();
        let bad =
            SourcePatch::new(binding, valid.policy().clone(), valid.operations().to_vec()).unwrap();
        assert!(
            h.engine
                .admit_source_attempt(
                    &h.owner,
                    &scope(),
                    bad.clone(),
                    IdempotencyKey::new("invalid-binding").unwrap()
                )
                .is_err()
        );
        let accepted = h
            .engine
            .admit_source_attempt(
                &h.owner,
                &scope(),
                valid.clone(),
                IdempotencyKey::new("accepted").unwrap(),
            )
            .unwrap();
        let error = h
            .engine
            .admit_source_attempt(
                &h.owner,
                &scope(),
                valid.clone(),
                IdempotencyKey::new("collision").unwrap(),
            )
            .unwrap_err();
        assert_eq!(
            error,
            EngineError::SourceAttempt(SourceAttemptError::AttemptIdentity)
        );
        assert_eq!(
            h.engine.commit(
                &h.owner,
                &scope(),
                &tx,
                IdempotencyKey::new("unprepared-source").unwrap()
            ),
            Err(EngineError::SourceMutationUnsupported)
        );
        let r = check(&h);
        let rejected = &r.operations[&IdempotencyKey::new("invalid-binding").unwrap()];
        assert_eq!(rejected.target_attempt, 0);
        assert_eq!(rejected.request.patch(), &bad);
        assert_eq!(
            r.rejections[&SourceAttemptId::new("rejected-999").unwrap()].transaction_attempt,
            999
        );
        assert!(!r.rejections.contains_key(&valid.binding().source_attempt));
        assert_eq!(r.operations.len(), 3);
        assert_eq!(r.unprepared_commits.len(), 1);
        assert_eq!(
            r.attempts[&valid.binding().source_attempt].admission,
            accepted
        );
    }
    #[test]
    fn orphaned_substituted_and_weakened_source_graphs_reject() {
        let h = Harness::new(false);
        let tx = h.execute("corruption");
        let a = h
            .engine
            .admit_source_attempt(
                &h.owner,
                &scope(),
                h.patch(&tx, "corruption"),
                IdempotencyKey::new("corruption").unwrap(),
            )
            .unwrap();
        let r = check(&h);
        let mut bad = r.clone();
        bad.analyses.clear();
        rejects(bad);
        let mut bad = r.clone();
        bad.policies.clear();
        rejects(bad);
        let mut bad = r.clone();
        bad.attempt_contexts.clear();
        rejects(bad);
        let mut bad = r.clone();
        bad.attempt_contexts
            .get_mut(&(tx.clone(), 0))
            .unwrap()
            .policy = None;
        rejects(bad);
        let mut bad = r.clone();
        bad.attempts
            .get_mut(&a.binding().source_attempt)
            .unwrap()
            .admission_fence += 1;
        rejects(bad);
        let mut bad = r.clone();
        bad.attempts
            .get_mut(&a.binding().source_attempt)
            .unwrap()
            .admission_lease_event_count = 0;
        rejects(bad);
        let mut bad = r.clone();
        bad.attempts
            .get_mut(&a.binding().source_attempt)
            .unwrap()
            .events[0]
            .at_ms += 1;
        rejects(bad);
        let mut bad = r.clone();
        bad.operations.clear();
        rejects(bad);
        let mut bad = r.clone();
        bad.operations.values_mut().next().unwrap().target_attempt = 999;
        rejects(bad);
        let mut bad = r.clone();
        bad.operations.values_mut().next().unwrap().observed_policy = None;
        rejects(bad);
        let mut bad = r.clone();
        bad.base.transactions.get_mut(&tx).unwrap().state = TransactionState::Validating;
        rejects(bad);
        let mut bad = r.clone();
        bad.base.repository.as_mut().unwrap().registered.clear();
        rejects(bad);
        let mut bad = r.clone();
        bad.base.scope.repository = RepositoryId::new("other").unwrap();
        rejects(bad);
        let mut bad = r.clone();
        bad.executor_counter = 0;
        rejects(bad);
        for limits in [
            NativeRecordLimits {
                entries: 1,
                ..Default::default()
            },
            NativeRecordLimits {
                total_value_bytes: 1,
                ..Default::default()
            },
            NativeRecordLimits {
                value_bytes: 1,
                ..Default::default()
            },
        ] {
            assert!(AuthorityState::from_source_records(r.clone(), limits).is_err());
        }
    }
}

mod source_request_persistence {
    use super::*;
    use celetixo_engine::persistence::{SourceRequestRecord, restore_source_request};
    #[test]
    fn exact_full_request_keeps_empty_operation_identity_and_optional_selection() {
        let h = Harness::new(false);
        let tx = h.execute("request-codec");
        let patch = h.patch(&tx, "request-codec");
        for selection in [
            BTreeSet::new(),
            [check_id("downstream"), check_id("invariant")].into(),
        ] {
            let options = SourceAttemptOptions {
                optional_checks: selection,
            };
            let record = SourceRequestRecord::new(patch.clone(), options.clone()).unwrap();
            let bytes = record.canonical_bytes();
            let restored =
                restore_source_request(&scope(), record.digest(), &bytes, Default::default())
                    .unwrap();
            assert_eq!(restored, record);
            assert_eq!(restored.patch(), &patch);
            assert_eq!(restored.options(), &options);
            if options.optional_checks.is_empty() {
                assert_eq!(record.request_digest(), patch.id().0);
            } else {
                assert_ne!(record.request_digest(), patch.id().0);
            }
            for end in 0..bytes.len() {
                let prefix = &bytes[..end];
                assert!(
                    restore_source_request(
                        &scope(),
                        Digest::of(SourceRequestRecord::FORMAT, prefix),
                        prefix,
                        Default::default()
                    )
                    .is_err()
                );
            }
            let mut extra = bytes.clone();
            extra.push(0);
            assert!(
                restore_source_request(
                    &scope(),
                    Digest::of(SourceRequestRecord::FORMAT, &extra),
                    &extra,
                    Default::default()
                )
                .is_err()
            );
            let entries = patch.operations().len() + options.optional_checks.len();
            assert_eq!(
                restore_source_request(
                    &scope(),
                    record.digest(),
                    &bytes,
                    DomainDecodeLimits {
                        collection_entries: entries - 1,
                        ..Default::default()
                    }
                ),
                Err(RestoreError::Limit)
            );
            assert_eq!(
                restore_source_request(
                    &scope(),
                    record.digest(),
                    &bytes,
                    DomainDecodeLimits {
                        collection_entries: entries,
                        ..Default::default()
                    }
                )
                .unwrap(),
                record
            );
            let mut other = scope();
            other.repository = RepositoryId::new("other").unwrap();
            assert_eq!(
                restore_source_request(&other, record.digest(), &bytes, Default::default()),
                Err(RestoreError::BindingMismatch)
            );
        }
    }
}

#[test]
fn source_recovery_keeps_full_engine_rejection_diagnostics_beyond_worker_reason_limit() {
    use celetixo_engine::engine::authority::*;
    let h = Harness::new(false);
    let tx = h.execute("long-rejection");
    let original = h.patch(&tx, "long-rejection");
    let path = "a".repeat(4096);
    let patch = SourcePatch::new(
        original.binding().clone(),
        original.policy().clone(),
        vec![PatchOperation::Create {
            path: path.clone(),
            entry: PatchEntry::file(b"public".to_vec(), FileMode::Regular).unwrap(),
        }],
    )
    .unwrap();
    let error = h
        .engine
        .admit_source_attempt(
            &h.owner,
            &scope(),
            patch,
            IdempotencyKey::new("long-rejection").unwrap(),
        )
        .unwrap_err();
    assert!(error.to_string().len() > 4096);
    let r = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    assert_eq!(
        r.rejections.values().next().unwrap().events[0].reason,
        error.to_string()
    );
    let state = AuthorityState::from_source_records(r.clone(), Default::default()).unwrap();
    assert_eq!(state.source_records(Default::default()).unwrap(), r);
}
