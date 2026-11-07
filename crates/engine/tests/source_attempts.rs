//! Source attempts use an explicitly finite complete reference analysis.
//! Passing these tests does not upgrade structural parser coverage.
use celetixo_engine::access::{AccessSet, Condition, Invariant};
use celetixo_engine::credentials::CredentialError;
use celetixo_engine::engine::*;
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::repository::*;
use celetixo_engine::snapshot::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use celetixo_engine::transaction::*;
use std::collections::{BTreeMap, BTreeSet};

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
