//! Integrated stale-base rejection, interrupted attempt recovery and fresh retry.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod support;
use celetixo_engine::engine::{EngineError, SourceAttemptState};
use celetixo_engine::ids::{IdempotencyKey, PrincipalId, SourceAttemptId};
use celetixo_engine::snapshot::Snapshot;
use celetixo_engine::source_evidence::SourceEvidenceAssessment;
use celetixo_engine::source_patch::SourcePatch;
use celetixo_engine::transaction::TransactionState;
use celetixo_execution::artifacts::LocalSourceArtifactStore;
use celetixo_execution::backend::{ExecutionCheckpoint, ExecutionGuard, GuardDecision};
use celetixo_execution::coordinator::{
    ExecutionStage, MonotonicEngineClock, SourceRunCoordinator, SourceRunError, SourceRunRequest,
};
use celetixo_execution::materialize::CleanupOutcome;
use celetixo_execution::oci::execution_identity;
use std::sync::Arc;
use support::*;

struct InterruptOnce {
    clock: MonotonicEngineClock,
    interrupted: bool,
}
impl ExecutionGuard for InterruptOnce {
    fn revalidate(&mut self) -> Result<GuardDecision, String> {
        self.clock.revalidate()
    }
    fn checkpoint(&mut self, point: ExecutionCheckpoint) -> Result<(), String> {
        if point == ExecutionCheckpoint::AfterClaimPublication && !self.interrupted {
            self.interrupted = true;
            return Err("controlled first attempt controller loss".into());
        }
        Ok(())
    }
}

#[test]
#[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
fn actual_stale_base_rejection_recovery_and_fresh_retry_preserve_history() {
    let backend = real_backend();
    let f = admitted_command(
        &backend,
        b"package main\nfunc answer() int { return 42 }\n",
        "/toolchain/gofmt",
        &["-e", "/src/main.go"],
        limits(),
    );
    let directory = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(
        LocalSourceArtifactStore::create(
            directory.path().join("immutable-evidence"),
            scope(),
            f.admission.retention_policy(),
            Default::default(),
        )
        .unwrap(),
    );
    let storage = f
        .admin
        .register_source_artifact_store(scope(), artifacts.clone())
        .unwrap();
    let workspaces = workspace_registry();
    let coordinator = SourceRunCoordinator {
        engine: &f.engine,
        executor: &f.executor,
        backend: &backend,
        artifacts: &artifacts,
        storage: &storage,
        workspaces: &workspaces,
    };
    let request = || SourceRunRequest {
        base: &f.base,
        patch: &f.patch,
        policy: &f.policy,
        admission: &f.admission,
    };
    let mut guard = InterruptOnce {
        clock: MonotonicEngineClock::new(f.admin.clone()),
        interrupted: false,
    };
    let mut stale_manifest = f.base.manifest().clone();
    stale_manifest
        .environment
        .insert("controlled_stale_input".into(), "different".into());
    let stale = Snapshot::new(
        scope(),
        f.base.revision(),
        stale_manifest,
        Default::default(),
    )
    .unwrap();
    let rejected = coordinator
        .execute(
            SourceRunRequest {
                base: &stale,
                ..request()
            },
            &mut guard,
        )
        .unwrap_err();
    assert_eq!(rejected.stage, ExecutionStage::Admission);
    assert!(matches!(rejected.cause, SourceRunError::Binding(_)));
    assert!(rejected.terminal_recording.is_none() && rejected.sealed.is_none());
    assert!(!guard.interrupted);
    let old_id = &f.admission.binding().source_attempt;
    assert_eq!(
        f.engine
            .source_attempt(&f.owner, &scope(), old_id)
            .unwrap()
            .state,
        SourceAttemptState::Admitted
    );
    let failed = coordinator.execute(request(), &mut guard).unwrap_err();
    assert!(guard.interrupted);
    assert_eq!(failed.stage, ExecutionStage::Verification);
    assert!(failed.sealed.is_none());
    assert!(matches!(
        failed.cleanup,
        Some(CleanupOutcome::Quarantined { .. })
    ));
    assert!(matches!(failed.terminal_recording, Some(Ok(()))));
    let history = f.engine.source_attempt(&f.owner, &scope(), old_id).unwrap();
    assert_eq!(history.state, SourceAttemptState::ExecutionFailed);
    assert!(history.evidence.is_none());
    let cleanup = f
        .admin
        .grant_source_cleanup(
            PrincipalId::new("retry-recovery").unwrap(),
            scope(),
            backend.identity().digest(),
            100_000,
        )
        .unwrap();
    let execution = execution_identity(&f.admission, &f.definition);
    let recovered = backend
        .reconcile_execution(&f.engine, &cleanup, &scope(), execution)
        .unwrap();
    assert!(recovered.termination_confirmed);
    assert_eq!(recovered.cleanup, CleanupOutcome::Removed);
    assert!(
        backend
            .recover_execution_record(&f.engine, &cleanup, &scope(), execution)
            .unwrap()
            .is_none()
    );
    guard.clock.revalidate().unwrap();
    let binding = f
        .engine
        .source_patch_context(&f.owner, &scope(), &f.transaction)
        .unwrap()
        .bind(
            SourceAttemptId::new(format!("{}-retry", old_id.as_str())).unwrap(),
            "controlled fresh retry".into(),
            30_000,
        )
        .unwrap();
    let patch = SourcePatch::new(
        binding,
        f.patch.policy().clone(),
        f.patch.operations().to_vec(),
    )
    .unwrap();
    let admission = f
        .engine
        .admit_source_attempt(
            &f.owner,
            &scope(),
            patch.clone(),
            IdempotencyKey::new("new-retry-attempt").unwrap(),
        )
        .unwrap();
    assert_ne!(admission.id(), f.admission.id());
    assert_ne!(admission.plan().id(), f.admission.plan().id());
    let result = coordinator
        .execute(
            SourceRunRequest {
                base: &f.base,
                patch: &patch,
                policy: &f.policy,
                admission: &admission,
            },
            &mut guard.clock,
        )
        .unwrap();
    assert_eq!(result.receipt.assessment, SourceEvidenceAssessment::Passed);
    assert_eq!(result.cleanup, CleanupOutcome::Removed);
    assert_eq!(
        coordinator.deliver(&admission, &result.sealed).unwrap(),
        result.receipt
    );
    assert!(coordinator.deliver(&f.admission, &result.sealed).is_err());
    assert_eq!(
        f.engine.source_attempt(&f.owner, &scope(), old_id).unwrap(),
        history
    );
    assert_eq!(
        f.engine
            .source_attempt(&f.owner, &scope(), &admission.binding().source_attempt)
            .unwrap()
            .state,
        SourceAttemptState::EvidenceSealed
    );
    assert_eq!(f.engine.current(&f.owner, &scope()).unwrap(), f.base);
    assert_eq!(
        f.engine
            .transaction(&f.owner, &scope(), &f.transaction)
            .unwrap()
            .state,
        TransactionState::Validating
    );
    assert_eq!(
        f.engine.commit(
            &f.owner,
            &scope(),
            &f.transaction,
            IdempotencyKey::new("still-no-source-publication").unwrap()
        ),
        Err(EngineError::SourceMutationUnsupported)
    );
}
