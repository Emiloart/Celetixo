//! OCI-specific projection into common subordinate execution facts.
use crate::backend::{
    self, BackendCheckResult, BackendError, ExecutionGuard, IsolationObservation,
    VerificationBackend,
};
use crate::materialize::{CleanupOutcome, PrivateWorkspace};
use crate::oci::{OciBackend, OciCheckRecord, OciError};
use celetixo_engine::engine::{Engine, SourceAdmission, SourceExecutorCapability};
use celetixo_engine::ids::Digest;
use celetixo_engine::source_evidence::{
    SourceCheckOutcome, SourceCheckRecordInput, SourceCleanup, SourceTermination,
};
use celetixo_engine::source_execution::{BackendIdentity, SourceCheckDefinition};
use std::collections::BTreeMap;

impl backend::sealed::Backend for OciBackend {}
impl VerificationBackend for OciBackend {
    fn identity(&self) -> &BackendIdentity {
        OciBackend::identity(self)
    }
    fn execution_identity(
        &self,
        admission: &SourceAdmission,
        check: &SourceCheckDefinition,
    ) -> Digest {
        crate::oci::execution_identity(admission, check)
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
        let raw = OciBackend::execute(self, engine, executor, admission, workspace, check, guard)
            .map_err(BackendError::from)?;
        project_oci(admission, check, raw)
    }
}

impl From<OciError> for BackendError {
    fn from(error: OciError) -> Self {
        match error {
            OciError::Unsupported(feature) => Self::Unsupported(feature),
            OciError::UnsupportedPlatform => Self::Unsupported("Linux x86_64 rootless OCI"),
            OciError::Binding => Self::Binding,
            OciError::Guard(reason) => Self::Authority(reason),
            OciError::AlreadyClaimed(id) => Self::AlreadyClaimed(id),
            OciError::Busy => Self::Busy,
            other => Self::Runtime(Box::new(other)),
        }
    }
}

fn project_oci(
    admission: &SourceAdmission,
    definition: &SourceCheckDefinition,
    raw: OciCheckRecord,
) -> Result<BackendCheckResult, BackendError> {
    if raw.execution != crate::oci::execution_identity(admission, definition)
        || raw.rootfs != definition.input().toolchain.runtime_closure
        || raw.check != *definition.id()
        || raw.definition != definition.digest()
        || raw.plan != admission.plan().id()
        || raw.snapshot != admission.resulting_snapshot()
        || raw.manifest != admission.resulting_manifest()
        || raw.backend != definition.input().backend
    {
        return Err(BackendError::Binding);
    }
    // The control observations are retained as bytes, not only as a digest
    // pointing at a process-local journal. The OCI result also binds that
    // observation digest to the exact command and admission.
    let execution = serde_json::from_slice::<serde_json::Value>(&raw.canonical_bytes())
        .map_err(|_| BackendError::Binding)?;
    let isolation = raw
        .isolation
        .as_ref()
        .map(|bytes| {
            serde_json::from_slice::<serde_json::Value>(bytes).map_err(|_| BackendError::Binding)
        })
        .transpose()?;
    if raw.outcome == (SourceCheckOutcome::Exited { code: 0 }) && isolation.is_none() {
        return Err(BackendError::Binding);
    }
    let isolation_record = serde_json::to_vec(&serde_json::json!({
        "version":1,"execution":execution,"effective_isolation":isolation,
    }))
    .expect("isolation artifact JSON");
    let policy = definition.input();
    Ok(BackendCheckResult {
        execution: raw.execution,
        record: SourceCheckRecordInput {
            admission: admission.id(),
            plan: admission.plan().id(),
            candidate: admission.resulting_snapshot(),
            check: definition.id().clone(),
            definition: definition.digest(),
            backend: policy.backend.digest(),
            executable: policy.toolchain.executable_digest,
            runtime_closure: policy.toolchain.runtime_closure,
            environment: policy.check.environment(),
            limits: policy.resources,
            started_at_ms: raw.started_at_ms,
            finished_at_ms: raw.finished_at_ms,
            outcome: raw.outcome,
            usage: raw.usage,
            post_source_manifest: raw.post_source_manifest,
            termination: if raw.termination_confirmed {
                SourceTermination::Confirmed
            } else {
                SourceTermination::Unknown {
                    reason: "backend could not establish whole-process-tree termination".into(),
                }
            },
            cleanup: match raw.cleanup {
                CleanupOutcome::Removed => SourceCleanup::Confirmed,
                CleanupOutcome::Quarantined { reason, .. } => SourceCleanup::Quarantined { reason },
            },
            stdout_truncated: raw.stdout_truncated,
            stderr_truncated: raw.stderr_truncated,
            artifacts: BTreeMap::new(),
            diagnostics: raw.diagnostics,
        },
        stdout: raw.stdout,
        stderr: raw.stderr,
        isolation: Some(IsolationObservation {
            media_type: "application/json".into(),
            bytes: isolation_record,
        }),
    })
}
