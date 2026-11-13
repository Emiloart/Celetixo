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

#[path = "coordinator_tests/mutation_recovery.rs"]
mod mutation_recovery;

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
    workspaces: WorkspaceRegistry,
    _workspace_parent: tempfile::TempDir,
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
        let workspace_parent = tempfile::tempdir().unwrap();
        let workspaces = WorkspaceRegistry::create(
            workspace_parent.path().join("workspaces"),
            scope(),
            Default::default(),
        )
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
            workspaces,
            _workspace_parent: workspace_parent,
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
            workspaces: &self.workspaces,
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
    RuntimeClosure,
    Check,
    Definition,
    Plan,
    Snapshot,
    Executable,
    Environment,
    Limits,
    Admission,
    Backend,
}

struct FiniteBackend {
    identity: BackendIdentity,
    exit: i32,
    forged: Option<ForgedBinding>,
    no_observation: bool,
    calls: RefCell<Vec<CheckId>>,
    materializations: RefCell<Vec<PathBuf>>,
}
impl FiniteBackend {
    fn new(exit: i32) -> Self {
        Self {
            identity: backend_identity(),
            exit,
            forged: None,
            no_observation: false,
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
    fn execution_identity(
        &self,
        admission: &SourceAdmission,
        check: &SourceCheckDefinition,
    ) -> Digest {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(admission.id().0.as_bytes());
        bytes.extend_from_slice(check.digest().as_bytes());
        Digest::of("celetixo/finite-backend-execution/v1", &bytes)
    }
    fn execute(
        &self,
        engine: &Engine,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        workspace: &mut PrivateWorkspace,
        check: &SourceCheckDefinition,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<BackendCheckResult, BackendError> {
        if guard.revalidate().map_err(BackendError::Authority)? != GuardDecision::Continue {
            return Err(BackendError::Authority("finite test guard stopped".into()));
        }
        engine
            .validate_source_attempt(executor, admission)
            .map_err(|error| BackendError::Authority(error.to_string()))?;
        workspace
            .validate_contents()
            .map_err(|error| BackendError::Authority(error.to_string()))?;
        self.calls.borrow_mut().push(check.id().clone());
        self.materializations
            .borrow_mut()
            .push(workspace.source_path().to_path_buf());
        let now = engine
            .source_execution_time(executor, admission)
            .map_err(|error| BackendError::Authority(error.to_string()))?;
        let policy = check.input();
        let mut raw = BackendCheckResult {
            execution: self.execution_identity(admission, check),
            record: SourceCheckRecordInput {
                admission: admission.id(),
                plan: admission.plan().id(),
                candidate: admission.resulting_snapshot(),
                check: check.id().clone(),
                definition: check.digest(),
                backend: policy.backend.digest(),
                executable: policy.toolchain.executable_digest,
                runtime_closure: policy.toolchain.runtime_closure,
                environment: policy.check.environment(),
                limits: policy.resources,
                started_at_ms: now,
                finished_at_ms: now,
                outcome: SourceCheckOutcome::Exited { code: self.exit },
                usage: SourceResourceUsage {
                    wall_time_ms: 0,
                    cpu_time_us: 0,
                    peak_memory_bytes: 0,
                },
                post_source_manifest: Some(admission.resulting_manifest()),
                termination: SourceTermination::Confirmed,
                cleanup: SourceCleanup::Confirmed,
                stdout_truncated: false,
                stderr_truncated: false,
                artifacts: BTreeMap::new(),
                diagnostics: vec!["finite fixture; no physical sandbox was exercised".into()],
            },
            stdout: b"finite reference output\n".to_vec(),
            stderr: vec![],
            isolation: Some(crate::backend::IsolationObservation {
                media_type: "application/json".into(),
                bytes: br#"{"version":1,"kind":"finite-reference-no-physical-isolation"}"#.to_vec(),
            }),
        };
        match self.forged {
            Some(ForgedBinding::Execution) => raw.execution = d("foreign-execution"),
            Some(ForgedBinding::RuntimeClosure) => {
                raw.record.runtime_closure = d("foreign-closure")
            }
            Some(ForgedBinding::Check) => raw.record.check = id("foreign"),
            Some(ForgedBinding::Definition) => raw.record.definition = d("foreign-definition"),
            Some(ForgedBinding::Plan) => raw.record.plan = SourcePlanId(d("foreign-plan")),
            Some(ForgedBinding::Snapshot) => {
                raw.record.candidate = SnapshotId(d("foreign-snapshot"))
            }
            Some(ForgedBinding::Admission) => {
                raw.record.admission = SourceAdmissionId(d("foreign-admission"))
            }
            Some(ForgedBinding::Backend) => raw.record.backend = d("foreign-backend"),
            Some(ForgedBinding::Executable) => raw.record.executable = d("foreign-executable"),
            Some(ForgedBinding::Environment) => raw.record.environment = d("foreign-environment"),
            Some(ForgedBinding::Limits) => raw.record.limits.wall_time_ms += 1,
            None => {}
        }
        if self.no_observation {
            raw.isolation = None;
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
        ForgedBinding::RuntimeClosure,
        ForgedBinding::Check,
        ForgedBinding::Definition,
        ForgedBinding::Plan,
        ForgedBinding::Snapshot,
        ForgedBinding::Executable,
        ForgedBinding::Environment,
        ForgedBinding::Limits,
        ForgedBinding::Admission,
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

#[test]
fn a_neutral_backend_cannot_replace_missing_observations_with_a_passing_status() {
    let h = Fixture::new(false);
    let backend = FiniteBackend {
        no_observation: true,
        ..FiniteBackend::new(0)
    };
    let failure = h
        .coordinator(&backend)
        .execute(h.request(), &mut Continue)
        .unwrap_err();
    assert!(matches!(
        failure.cause,
        SourceRunError::Binding("successful check lacks effective isolation observations")
    ));
    assert_eq!(failure.stage, ExecutionStage::Verification);
    assert_eq!(objects(&h.artifacts), 0);
    assert_eq!(h.state(), SourceAttemptState::ExecutionFailed);
    backend.assert_cleaned();
}

#[test]
fn a_neutral_backend_missing_required_observations_retains_incomplete_failure_evidence() {
    let h = Fixture::new(false);
    let backend = FiniteBackend {
        no_observation: true,
        ..FiniteBackend::new(1)
    };
    let result = h
        .coordinator(&backend)
        .execute(h.request(), &mut Continue)
        .unwrap();
    assert!(matches!(
        result.receipt.assessment,
        SourceEvidenceAssessment::Incomplete { .. }
    ));
    assert_eq!(h.state(), SourceAttemptState::EvidenceIncomplete);
    assert!(
        !result.sealed.evidence.checks()[&id("first")]
            .input()
            .artifacts
            .contains_key("isolation-record")
    );
    assert_eq!(result.cleanup, CleanupOutcome::Removed);
    backend.assert_cleaned();
    h.assert_base_unchanged();
}

fn workspace_cleanup(
    h: &Fixture,
    registry: &WorkspaceRegistry,
) -> celetixo_engine::engine::SourceCleanupCapability {
    h.admin
        .grant_source_cleanup(
            PrincipalId::new("workspace-operator").unwrap(),
            scope(),
            registry.identity(),
            100_000,
        )
        .unwrap()
}
fn registered_workspace(h: &Fixture, registry: &WorkspaceRegistry) -> PrivateWorkspace {
    registry
        .materialize(
            &h.engine,
            &h.executor,
            &h.admission,
            &h.base,
            &h.patch,
            |_| Ok(()),
        )
        .unwrap()
}
fn claim(h: &Fixture) {
    h.engine
        .claim_source_attempt(&h.executor, &h.admission)
        .unwrap();
}

#[test]
fn registry_recovers_only_dead_unretained_owned_copies_without_restoring_execution() {
    let h = Fixture::new(false);
    claim(&h);
    let cap = workspace_cleanup(&h, &h.workspaces);
    let live = registered_workspace(&h, &h.workspaces);
    let path = live.source_path().parent().unwrap().to_path_buf();
    let reopened =
        WorkspaceRegistry::open(h.workspaces.path(), scope(), Default::default()).unwrap();
    assert_eq!(reopened.identity(), h.workspaces.identity());
    let report = reopened.recover(&h.engine, &cap).unwrap();
    assert_eq!(report.live, vec![path.clone()]);
    assert!(report.removed.is_empty());
    live.validate_contents().unwrap();
    assert!(matches!(
        live.quarantine("controlled T0 owner exit"),
        CleanupOutcome::Quarantined { .. }
    ));
    // Recovery also works after reference Engine state loss, using a fresh,
    // explicitly scoped cleanup grant. It never resumes the lost transaction.
    let (store, admin) = celetixo_engine::engine::InMemoryStore::new();
    let engine = Engine::new(store);
    let fresh = admin
        .grant_source_cleanup(
            PrincipalId::new("recovery-operator").unwrap(),
            scope(),
            reopened.identity(),
            1000,
        )
        .unwrap();
    assert_eq!(
        reopened.recover(&engine, &fresh).unwrap().removed,
        vec![path.clone()]
    );
    assert!(!path.exists());
    assert_eq!(
        reopened.recover(&engine, &fresh).unwrap(),
        Default::default()
    );
    assert_eq!(h.state(), SourceAttemptState::Patching);
    h.assert_base_unchanged();
}

#[test]
fn registry_cleanup_rechecks_scope_store_profile_revocation_expiry_and_path_identity() {
    let h = Fixture::new(false);
    claim(&h);
    let orphan = registered_workspace(&h, &h.workspaces);
    let path = orphan.source_path().parent().unwrap().to_path_buf();
    orphan.quarantine("controlled T0 exit");
    let (other_store, _) = celetixo_engine::engine::InMemoryStore::new();
    let other = Engine::new(other_store);
    let cap = workspace_cleanup(&h, &h.workspaces);
    assert!(h.workspaces.recover(&other, &cap).is_err());
    let mut wrong_scope = scope();
    wrong_scope.repository = RepositoryId::new("other-repository").unwrap();
    for (scope, identity) in [
        (wrong_scope, h.workspaces.identity()),
        (scope(), d("wrong-registry")),
    ] {
        let wrong = h
            .admin
            .grant_source_cleanup(
                PrincipalId::new("operator").unwrap(),
                scope,
                identity,
                100_000,
            )
            .unwrap();
        assert!(h.workspaces.recover(&h.engine, &wrong).is_err());
    }
    h.admin.revoke_source_cleanup(&cap).unwrap();
    assert!(h.workspaces.recover(&h.engine, &cap).is_err());
    let expiring = h
        .admin
        .grant_source_cleanup(
            PrincipalId::new("operator").unwrap(),
            scope(),
            h.workspaces.identity(),
            1,
        )
        .unwrap();
    assert!(
        h.workspaces
            .recover_with_hook(&h.engine, &expiring, |_| {
                h.admin.advance_time(1).unwrap();
                Ok(())
            })
            .is_err()
    );
    assert!(path.exists());
    let cap = workspace_cleanup(&h, &h.workspaces);
    let moved = path.with_file_name("workspace-original-moved");
    let report = h
        .workspaces
        .recover_with_hook(&h.engine, &cap, |candidate| {
            std::fs::rename(candidate, &moved)?;
            std::fs::create_dir(candidate)?;
            std::fs::write(candidate.join("replacement.txt"), b"unrelated")
        })
        .unwrap();
    assert!(report.removed.is_empty());
    assert_eq!(report.quarantined.len(), 1);
    assert_eq!(
        std::fs::read(path.join("replacement.txt")).unwrap(),
        b"unrelated"
    );
    assert!(moved.join("source/main.txt").exists());
}

#[test]
fn registry_retention_corruption_symlinks_and_limits_fail_closed() {
    use crate::workspaces::{WorkspaceError, WorkspaceLimits};
    let h = Fixture::new(false);
    claim(&h);
    let limits = WorkspaceLimits {
        max_roots: 1,
        max_total_input_bytes: 4,
        max_entries_per_root: 16,
    };
    let path = h._workspace_parent.path().join("bounded-registry");
    let registry = WorkspaceRegistry::create(&path, scope(), limits).unwrap();
    let cap = workspace_cleanup(&h, &registry);
    let live = registered_workspace(&h, &registry);
    assert!(matches!(
        registry.materialize(
            &h.engine,
            &h.executor,
            &h.admission,
            &h.base,
            &h.patch,
            |_| Ok(())
        ),
        Err(WorkspaceError::Limit)
    ));
    assert!(WorkspaceRegistry::open(&path, scope(), Default::default()).is_err());
    let root = live.source_path().parent().unwrap().to_path_buf();
    std::fs::write(
        root.join("execution-retention.json"),
        b"unknown runtime state",
    )
    .unwrap();
    live.quarantine("runtime retained");
    let report = registry.recover(&h.engine, &cap).unwrap();
    assert_eq!(report.quarantined.len(), 1);
    assert!(root.exists());
    std::fs::remove_file(root.join("execution-retention.json")).unwrap();
    std::fs::write(root.join("workspace.json"), b"corrupt ownership").unwrap();
    // Concurrent subprocess creation can briefly inherit an open owner file
    // description until exec closes CLOEXEC descriptors. Recovery must retain a
    // held owner as live; wait boundedly for that ownership to end before testing
    // corrupted-record classification. Never delete or ignore a live result.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let corrupt_recovery = loop {
        let report = registry.recover(&h.engine, &cap).unwrap();
        if report.live.is_empty() {
            break report;
        }
        assert_eq!(report.live, vec![root.clone()]);
        assert!(report.removed.is_empty());
        assert!(report.quarantined.is_empty());
        assert!(root.exists());
        assert!(
            std::time::Instant::now() < deadline,
            "owner remained live: {report:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    assert_eq!(
        corrupt_recovery.quarantined.len(),
        1,
        "{corrupt_recovery:?}"
    );
    // A symlink occupying a workspace name never authorizes outside deletion.
    let moved = h._workspace_parent.path().join("outside");
    std::fs::rename(&root, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &root).unwrap();
    assert_eq!(
        registry.recover(&h.engine, &cap).unwrap().quarantined.len(),
        1
    );
    assert!(moved.join("source/main.txt").exists());
    assert!(WorkspaceRegistry::open(root, scope(), limits).is_err());
    // Equal policy and scope do not confer recovery over another namespace.
    let second = WorkspaceRegistry::create(
        h._workspace_parent.path().join("second-registry"),
        scope(),
        limits,
    )
    .unwrap();
    assert_ne!(registry.identity(), second.identity());
    assert!(second.recover(&h.engine, &cap).is_err());
    // Byte reservation happens before creating an allocation.
    let small = WorkspaceRegistry::create(
        h._workspace_parent.path().join("small-registry"),
        scope(),
        WorkspaceLimits {
            max_total_input_bytes: 3,
            ..limits
        },
    )
    .unwrap();
    assert!(matches!(
        small.materialize(
            &h.engine,
            &h.executor,
            &h.admission,
            &h.base,
            &h.patch,
            |_| Ok(())
        ),
        Err(WorkspaceError::Limit)
    ));
    assert_eq!(std::fs::read_dir(small.path()).unwrap().count(), 2);
}

#[test]
fn aliased_owner_descriptor_retains_source_until_release_then_corruption_quarantines() {
    let h = Fixture::new(false);
    claim(&h);
    let cap = workspace_cleanup(&h, &h.workspaces);
    let workspace = registered_workspace(&h, &h.workspaces);
    let root = workspace.source_path().parent().unwrap().to_path_buf();
    // dup and pre-exec inherited descriptors share the same open description
    // and flock lifetime. A second path-based open would not model that lifetime.
    let inherited_owner = workspace.duplicate_owner_for_test().unwrap();
    workspace.quarantine("controller relinquished this copy");
    std::fs::write(root.join("workspace.json"), b"corrupt ownership").unwrap();
    let held = h.workspaces.recover(&h.engine, &cap).unwrap();
    assert_eq!(held.live, vec![root.clone()]);
    assert!(held.removed.is_empty());
    assert!(held.quarantined.is_empty());
    assert!(root.join("source/main.txt").exists());
    drop(inherited_owner);
    let released = h.workspaces.recover(&h.engine, &cap).unwrap();
    assert!(released.live.is_empty());
    assert!(released.removed.is_empty());
    assert_eq!(released.quarantined.len(), 1);
    assert_eq!(released.quarantined[0].0, root);
    assert!(root.join("source/main.txt").exists());
}

#[test]
fn registry_rechecks_engine_authority_during_copy_without_trusting_the_caller_hook() {
    use crate::materialize::MaterializationEvent;
    for before_publication in [false, true] {
        let h = Fixture::new(false);
        claim(&h);
        let mut cancelled = false;
        let result = h.workspaces.materialize(
            &h.engine,
            &h.executor,
            &h.admission,
            &h.base,
            &h.patch,
            |event| {
                let boundary = if before_publication {
                    matches!(event, MaterializationEvent::RegistryRootCreated(_))
                } else {
                    matches!(event, MaterializationEvent::BeforeEntry { .. })
                };
                if boundary {
                    h.engine
                        .abort(
                            &h.owner,
                            &scope(),
                            &h.admission.binding().transaction,
                            "cancelled during controlled copy".into(),
                        )
                        .unwrap();
                    cancelled = true;
                }
                // The caller deliberately does not reject or check authority.
                Ok(())
            },
        );
        assert!(cancelled && result.is_err());
        assert_eq!(h.state(), SourceAttemptState::Cancelled);
        let report = h
            .workspaces
            .recover(&h.engine, &workspace_cleanup(&h, &h.workspaces))
            .unwrap();
        assert_eq!(report.removed.len(), usize::from(before_publication));
        assert!(report.live.is_empty() && report.quarantined.is_empty());
        h.assert_base_unchanged();
    }
}

#[test]
fn interrupted_removal_that_loses_ownership_quarantines_instead_of_inventing_authority() {
    let h = Fixture::new(false);
    claim(&h);
    let copy = registered_workspace(&h, &h.workspaces);
    let path = copy.source_path().parent().unwrap().to_path_buf();
    copy.quarantine("controlled T0 owner exit");
    let authority = workspace_cleanup(&h, &h.workspaces);
    let interrupted = h
        .workspaces
        .recover_with_hook(&h.engine, &authority, |path| {
            std::fs::remove_file(path.join("workspace.json"))?;
            Err(std::io::Error::other("controlled interrupted removal"))
        });
    assert!(interrupted.is_err());
    let result = h.workspaces.recover(&h.engine, &authority).unwrap();
    assert!(result.removed.is_empty() && result.live.is_empty());
    assert_eq!(result.quarantined.len(), 1);
    assert!(path.join("source/main.txt").is_file());
    h.assert_base_unchanged();
}

#[test]
#[ignore = "internal hard-kill helper; invoked by registry_survives_hard_kills_before_and_during_copy"]
fn registry_hard_kill_child() {
    use crate::materialize::MaterializationEvent;
    let root = PathBuf::from(std::env::var_os("CELETIXO_COPY_KILL_ROOT").unwrap());
    let stage = std::env::var("CELETIXO_COPY_KILL_STAGE").unwrap();
    let h = Fixture::new(false);
    claim(&h);
    let registry =
        WorkspaceRegistry::create(root.join("owned"), scope(), Default::default()).unwrap();
    let _copy = registry
        .materialize(
            &h.engine,
            &h.executor,
            &h.admission,
            &h.base,
            &h.patch,
            |event| {
                let stop = matches!(
                    (&event, stage.as_str()),
                    (MaterializationEvent::RegistryRootCreated(_), "allocation")
                        | (
                            MaterializationEvent::BeforeOwnershipPublication(_),
                            "publication"
                        )
                        | (MaterializationEvent::BeforeEntry { .. }, "copy")
                        | (MaterializationEvent::BeforeValidation(_), "validation")
                );
                if stop {
                    std::fs::write(root.join("ready"), b"paused before completion")?;
                    loop {
                        std::thread::park();
                    }
                }
                Ok(())
            },
        )
        .unwrap();
    panic!("hard-kill checkpoint was missed");
}

#[test]
fn registry_survives_hard_kills_before_and_during_copy() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for stage in ["allocation", "publication", "copy", "validation"] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = KillOnDrop(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "coordinator::tests::registry_hard_kill_child",
                ])
                .env("CELETIXO_COPY_KILL_ROOT", directory.path())
                .env("CELETIXO_COPY_KILL_STAGE", stage)
                .env("TMPDIR", directory.path())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while !directory.path().join("ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "copy helper exited at {stage}"
            );
            assert!(
                Instant::now() < deadline,
                "copy helper did not reach {stage}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        child.0.kill().unwrap();
        let status = child.0.wait().unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
        let registry =
            WorkspaceRegistry::open(directory.path().join("owned"), scope(), Default::default())
                .unwrap();
        let (store, admin) = celetixo_engine::engine::InMemoryStore::new();
        let engine = Engine::new(store);
        let cap = admin
            .grant_source_cleanup(
                PrincipalId::new("recovery-operator").unwrap(),
                scope(),
                registry.identity(),
                1000,
            )
            .unwrap();
        let report = registry.recover(&engine, &cap).unwrap();
        assert_eq!(report.removed.len(), 1, "stage {stage}: {report:?}");
        assert!(
            report.live.is_empty() && report.quarantined.is_empty(),
            "{report:?}"
        );
        assert_eq!(registry.recover(&engine, &cap).unwrap(), Default::default());
    }
}
