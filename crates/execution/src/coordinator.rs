//! Trusted, synchronous integration of Engine admission, private materialization,
//! isolated checks and persisted evidence. A successful receipt is evidence
//! admission only; this module cannot publish source or grant commit authority.
use crate::artifacts::LocalSourceArtifactStore;
use crate::materialize::{
    CleanupOutcome, MaterializationError, MaterializationReport, PrivateWorkspace,
};
use crate::oci::{ExecutionGuard, GuardDecision, OciBackend, OciCheckRecord, OciError};
use celetixo_engine::engine::{
    Administrator, Engine, EngineError, SourceAdmission, SourceArtifactStoreCapability,
    SourceAttemptState, SourceEvidenceFault, SourceEvidenceReceipt, SourceExecutorCapability,
};
use celetixo_engine::ids::CheckId;
use celetixo_engine::snapshot::Snapshot;
use celetixo_engine::source_evidence::{
    SourceArtifactError, SourceArtifactReader, SourceArtifactReference, SourceCheckOutcome,
    SourceCheckRecord, SourceCheckRecordInput, SourceCleanup, SourceEvidence,
    SourceEvidenceBinding, SourceEvidenceError, SourceResourceUsage, SourceTermination,
};
use celetixo_engine::source_execution::{
    BackendIdentity, SourceCheckDefinition, SourceExecutionPolicy,
};
use celetixo_engine::source_patch::SourcePatch;
use std::collections::BTreeMap;
use std::time::Instant;
use thiserror::Error;

mod sealed {
    pub trait Backend {}
}

/// Implementations live in this crate and must revalidate the Engine before
/// releasing a command and throughout its lifetime. No worker-supplied runner
/// or host-command fallback can be attached to this coordinator.
pub trait VerificationBackend: sealed::Backend {
    fn identity(&self) -> &BackendIdentity;
    fn execute(
        &self,
        engine: &Engine,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        workspace: &mut PrivateWorkspace,
        check: &SourceCheckDefinition,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<OciCheckRecord, OciError>;
}

impl sealed::Backend for OciBackend {}
impl VerificationBackend for OciBackend {
    fn identity(&self) -> &BackendIdentity {
        self.identity()
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
        self.execute(engine, executor, admission, workspace, check, guard)
    }
}

/// One trusted clock driver per reference Engine store. Retain it across runs:
/// elapsed idle, materialization, execution and storage time all count toward
/// leases. Independent drivers must not advance the same development clock.
/// Production wall-clock/lease persistence belongs to the authoritative store.
pub struct MonotonicEngineClock {
    administrator: Administrator,
    started: Instant,
    accounted_ms: u64,
}

impl MonotonicEngineClock {
    pub fn new(administrator: Administrator) -> Self {
        Self {
            administrator,
            started: Instant::now(),
            accounted_ms: 0,
        }
    }
}
impl ExecutionGuard for MonotonicEngineClock {
    fn revalidate(&mut self) -> Result<GuardDecision, String> {
        let elapsed = u64::try_from(self.started.elapsed().as_millis())
            .map_err(|_| "monotonic Engine clock exhausted".to_owned())?;
        let delta = elapsed
            .checked_sub(self.accounted_ms)
            .ok_or_else(|| "monotonic Engine clock regressed".to_owned())?;
        self.administrator
            .advance_time(delta)
            .map_err(|e| e.to_string())?;
        self.accounted_ms = elapsed;
        Ok(GuardDecision::Continue)
    }
}

pub struct SourceRunRequest<'a> {
    pub base: &'a Snapshot,
    pub patch: &'a SourcePatch,
    pub policy: &'a SourceExecutionPolicy,
    pub admission: &'a SourceAdmission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStage {
    Admission,
    Materialization,
    Verification,
    Persistence,
    EvidenceAdmission,
}

#[derive(Debug, Error)]
pub enum SourceRunError {
    #[error("execution input differs from Engine admission: {0}")]
    Binding(&'static str),
    #[error("trusted execution guard failed: {0}")]
    Guard(String),
    #[error("trusted execution guard stopped execution: {0:?}")]
    Stopped(GuardDecision),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Materialization(#[from] MaterializationError),
    #[error(transparent)]
    Backend(#[from] OciError),
    #[error(transparent)]
    Artifact(#[from] SourceArtifactError),
    #[error(transparent)]
    Evidence(#[from] SourceEvidenceError),
}

/// Retained on a delivery failure so a caller can redeliver these exact objects
/// without running checks again. The Engine still decides current admissibility.
#[derive(Debug, Clone)]
pub struct SealedSourceEvidence {
    pub evidence: SourceEvidence,
    pub envelope: SourceArtifactReference,
}

#[derive(Debug, Error)]
#[error(
    "source run failed during {stage:?}: {cause}; terminal recording: {terminal_recording:?}; cleanup: {cleanup:?}"
)]
pub struct SourceRunFailure {
    pub stage: ExecutionStage,
    pub cause: SourceRunError,
    /// None means no execution was claimed. An error is preserved separately
    /// rather than hiding an authority change behind the original failure.
    pub terminal_recording: Option<Result<(), EngineError>>,
    pub cleanup: Option<CleanupOutcome>,
    pub sealed: Option<SealedSourceEvidence>,
}

/// Observed durations from this process, in microseconds. They are measurements,
/// not evidence authority or estimates of other processes' peak memory.
#[derive(Debug, Clone, Default)]
pub struct SourceRunTimings {
    pub materialization_us: u64,
    pub checks_us: u64,
    pub source_cleanup_us: u64,
    pub persistence_us: u64,
    pub evidence_admission_us: u64,
    pub total_us: u64,
}

#[derive(Debug, Clone)]
pub struct SourceRunResult {
    pub receipt: SourceEvidenceReceipt,
    pub sealed: SealedSourceEvidence,
    pub materialization: MaterializationReport,
    pub cleanup: CleanupOutcome,
    pub timings: SourceRunTimings,
}

pub struct SourceRunCoordinator<'a, B: VerificationBackend> {
    pub engine: &'a Engine,
    pub executor: &'a SourceExecutorCapability,
    pub backend: &'a B,
    pub artifacts: &'a LocalSourceArtifactStore,
    pub storage: &'a SourceArtifactStoreCapability,
}

impl<B: VerificationBackend> SourceRunCoordinator<'_, B> {
    pub fn execute(
        &self,
        request: SourceRunRequest<'_>,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<SourceRunResult, Box<SourceRunFailure>> {
        self.execute_with_evidence_fault(request, guard, SourceEvidenceFault::None)
    }

    /// Trusted embedding fault injection. Faults can lose a delivery response,
    /// but cannot weaken validation or confer evidence authority.
    pub fn execute_with_evidence_fault(
        &self,
        request: SourceRunRequest<'_>,
        guard: &mut dyn ExecutionGuard,
        fault: SourceEvidenceFault,
    ) -> Result<SourceRunResult, Box<SourceRunFailure>> {
        let total = Instant::now();
        let admission = request.admission;
        let preflight = (|| {
            poll_guard(guard)?;
            if request.patch.id() != admission.patch()
                || request.base.id() != admission.binding().base_snapshot
                || request.policy.digest() != admission.policy()
                || self.backend.identity() != self.executor.backend()
                || self.artifacts.scope() != &admission.binding().scope
                || self.artifacts.retention_policy() != admission.retention_policy()
            {
                return Err(SourceRunError::Binding(
                    "patch, base, policy, backend or artifact store",
                ));
            }
            if admission.plan().checks().iter().any(|check| {
                !check
                    .definition()
                    .input()
                    .expected_artifacts
                    .iter()
                    .any(|artifact| {
                        artifact.name == "isolation-record"
                            && artifact.required
                            && artifact.media_type == "application/json"
                    })
            }) {
                return Err(SourceRunError::Binding(
                    "required application/json isolation record",
                ));
            }
            self.engine.claim_source_attempt(self.executor, admission)?;
            Ok(())
        })();
        preflight.map_err(|cause| SourceRunFailure {
            stage: ExecutionStage::Admission,
            cause,
            terminal_recording: None,
            cleanup: None,
            sealed: None,
        })?;

        let mut timings = SourceRunTimings::default();
        let phase = Instant::now();
        let workspace =
            PrivateWorkspace::materialize_with_hook(request.base, request.patch, |_| {
                poll_guard(guard)
                    .and_then(|()| {
                        self.engine
                            .validate_source_attempt(self.executor, admission)?;
                        Ok(())
                    })
                    .map_err(std::io::Error::other)
            });
        timings.materialization_us = micros(phase);
        let mut workspace = workspace.map_err(|cause| {
            self.failure(
                admission,
                ExecutionStage::Materialization,
                cause.into(),
                None,
                None,
            )
        })?;
        let materialization = workspace.report().clone();
        let phase = Instant::now();
        let run = (|| {
            let expected = self.engine.source_application(self.executor, admission)?;
            if workspace.applied() != &expected {
                return Err(SourceRunError::Binding(
                    "materialized candidate and application report",
                ));
            }
            poll_guard(guard)?;
            self.engine
                .mark_source_patch_applied(self.executor, admission)?;
            self.engine
                .begin_source_verification(self.executor, admission)?;
            self.run_checks(
                admission,
                &mut workspace,
                guard,
                request.policy.input().resource_ceiling.artifact_bytes,
            )
        })();
        timings.checks_us = micros(phase);
        // Cleanup is part of the evidence claim, so complete it before producing
        // canonical check records. Unknown termination latches retain the tree.
        let phase = Instant::now();
        let source_validation = workspace.validate_contents();
        let cleanup = workspace.cleanup();
        timings.source_cleanup_us = micros(phase);
        let mut pending = run.map_err(|cause| {
            self.failure(
                admission,
                ExecutionStage::Verification,
                cause,
                Some(cleanup.clone()),
                None,
            )
        })?;
        if let Err(error) = source_validation {
            for check in &mut pending {
                check.input.post_source_manifest = None;
                check
                    .input
                    .diagnostics
                    .push(format!("final source integrity: {error}"));
            }
        }
        if let CleanupOutcome::Quarantined { reason, .. } = &cleanup {
            for check in &mut pending {
                check.input.cleanup = SourceCleanup::Quarantined {
                    reason: reason.clone(),
                };
            }
        }

        let phase = Instant::now();
        let sealed = self.persist(&request, pending, guard).map_err(|cause| {
            self.failure(
                admission,
                ExecutionStage::Persistence,
                cause,
                Some(cleanup.clone()),
                None,
            )
        })?;
        timings.persistence_us = micros(phase);
        let phase = Instant::now();
        poll_guard(guard).map_err(|cause| {
            self.failure(
                admission,
                ExecutionStage::EvidenceAdmission,
                cause,
                Some(cleanup.clone()),
                Some(sealed.clone()),
            )
        })?;
        let receipt = self
            .engine
            .record_source_evidence_with_fault(
                self.executor,
                admission,
                self.storage,
                sealed.evidence.clone(),
                sealed.envelope.clone(),
                fault,
            )
            .map_err(|cause| SourceRunFailure {
                // Preserve a live Verifying attempt on an I/O error or a lost
                // response. The gate may have retained this exact evidence already.
                // Current authority is rechecked on exact redelivery.
                stage: ExecutionStage::EvidenceAdmission,
                cause: cause.into(),
                terminal_recording: None,
                cleanup: Some(cleanup.clone()),
                sealed: Some(sealed.clone()),
            })?;
        timings.evidence_admission_us = micros(phase);
        timings.total_us = micros(total);
        Ok(SourceRunResult {
            receipt,
            sealed,
            materialization,
            cleanup,
            timings,
        })
    }

    pub fn deliver(
        &self,
        admission: &SourceAdmission,
        sealed: &SealedSourceEvidence,
    ) -> Result<SourceEvidenceReceipt, EngineError> {
        self.engine.record_source_evidence(
            self.executor,
            admission,
            self.storage,
            sealed.evidence.clone(),
            sealed.envelope.clone(),
        )
    }

    fn run_checks(
        &self,
        admission: &SourceAdmission,
        workspace: &mut PrivateWorkspace,
        guard: &mut dyn ExecutionGuard,
        budget: u64,
    ) -> Result<Vec<PendingCheck>, SourceRunError> {
        let mut checks = Vec::<PendingCheck>::new();
        let mut outcomes = BTreeMap::<CheckId, SourceCheckOutcome>::new();
        let mut bytes = 0_u64;
        for planned in admission.plan().checks() {
            poll_guard(guard)?;
            self.engine
                .validate_source_attempt(self.executor, admission)?;
            let definition = planned.definition();
            let blocked =
                definition.input().depends_on.iter().find(|id| {
                    outcomes.get(*id) != Some(&(SourceCheckOutcome::Exited { code: 0 }))
                });
            let pending = if let Some(dependency) = blocked {
                let now = self
                    .engine
                    .source_execution_time(self.executor, admission)?;
                PendingCheck::skipped(admission, definition, now, dependency)
            } else {
                let raw = self.backend.execute(
                    self.engine,
                    self.executor,
                    admission,
                    workspace,
                    definition,
                    guard,
                )?;
                PendingCheck::executed(admission, definition, raw)?
            };
            bytes = bytes
                .checked_add(pending.retained_bytes()?)
                .ok_or(SourceArtifactError::Limit)?;
            if bytes > budget {
                return Err(SourceArtifactError::Limit.into());
            }
            outcomes.insert(definition.id().clone(), pending.input.outcome.clone());
            checks.push(pending);
        }
        Ok(checks)
    }

    fn persist(
        &self,
        request: &SourceRunRequest<'_>,
        checks: Vec<PendingCheck>,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<SealedSourceEvidence, SourceRunError> {
        let admission = request.admission;
        let mut records = Vec::new();
        for pending in checks {
            self.revalidate(admission, guard)?;
            let mut input = pending.input;
            let definition = admission
                .plan()
                .checks()
                .iter()
                .find(|c| c.definition().id() == &input.check)
                .ok_or(SourceRunError::Binding("unplanned check record"))?
                .definition();
            for (name, media, bytes) in [
                ("stdout", "text/plain", pending.stdout),
                ("stderr", "text/plain", pending.stderr),
                (
                    "diagnostics",
                    "application/json",
                    serde_json::to_vec(&input.diagnostics).expect("diagnostics JSON"),
                ),
            ] {
                self.revalidate(admission, guard)?;
                let media = definition
                    .input()
                    .expected_artifacts
                    .iter()
                    .find(|requirement| requirement.name == name)
                    .map_or(media, |requirement| requirement.media_type.as_str());
                input
                    .artifacts
                    .insert(name.into(), self.artifacts.put_bytes(name, media, &bytes)?);
            }
            if let Some(requirement) = admission
                .plan()
                .checks()
                .iter()
                .find(|c| c.definition().id() == &input.check)
                .and_then(|c| {
                    c.definition()
                        .input()
                        .expected_artifacts
                        .iter()
                        .find(|r| r.name == "isolation-record")
                })
            {
                self.revalidate(admission, guard)?;
                input.artifacts.insert(
                    "isolation-record".into(),
                    self.artifacts.put_bytes(
                        "isolation-record",
                        &requirement.media_type,
                        &pending.isolation_record,
                    )?,
                );
            }
            let bytes = SourceCheckRecord::new(input.clone())?.execution_bytes();
            self.revalidate(admission, guard)?;
            input.artifacts.insert(
                "execution-record".into(),
                self.artifacts.put_bytes(
                    "execution-record",
                    "application/vnd.celetixo.source-check-v1",
                    &bytes,
                )?,
            );
            records.push(SourceCheckRecord::new(input)?);
        }
        let applied = self.engine.source_application(self.executor, admission)?;
        let mut provenance_bytes = BTreeMap::from([
            ("source-patch".to_owned(), request.patch.canonical_bytes()),
            ("base-snapshot".to_owned(), request.base.canonical_bytes()),
            (
                "result-snapshot".to_owned(),
                applied.snapshot().canonical_bytes(),
            ),
            (
                "base-manifest".to_owned(),
                request.base.manifest().canonical_bytes(),
            ),
            (
                "result-manifest".to_owned(),
                applied.snapshot().manifest().canonical_bytes(),
            ),
            (
                "application-report".to_owned(),
                applied.report().canonical_bytes(),
            ),
            (
                "verification-plan".to_owned(),
                admission.plan().canonical_bytes(),
            ),
            (
                "execution-admission".to_owned(),
                admission.canonical_bytes(),
            ),
            (
                "execution-policy".to_owned(),
                request.policy.canonical_bytes(),
            ),
        ]);
        for check in admission.plan().checks() {
            provenance_bytes.insert(
                format!("check-definition/{}", check.definition().id()),
                check.definition().canonical_bytes(),
            );
        }
        let mut provenance = BTreeMap::new();
        for (name, bytes) in provenance_bytes {
            self.revalidate(admission, guard)?;
            provenance.insert(
                name.clone(),
                self.artifacts.put_bytes(
                    &name,
                    "application/vnd.celetixo.provenance-v1",
                    &bytes,
                )?,
            );
        }
        let started = records
            .iter()
            .map(|c| c.input().started_at_ms)
            .min()
            .ok_or(SourceRunError::Binding("empty verification plan"))?;
        let finished = records
            .iter()
            .map(|c| c.input().finished_at_ms)
            .max()
            .ok_or(SourceRunError::Binding("empty verification plan"))?;
        let evidence = SourceEvidence::new(
            SourceEvidenceBinding::for_admission(admission, self.executor.principal().clone()),
            started,
            finished,
            records,
            provenance,
            vec![],
        )?;
        self.revalidate(admission, guard)?;
        let envelope = self.artifacts.put_bytes(
            "sealed-evidence",
            "application/vnd.celetixo.source-evidence-v1",
            &evidence.canonical_bytes(),
        )?;
        Ok(SealedSourceEvidence { evidence, envelope })
    }

    fn revalidate(
        &self,
        admission: &SourceAdmission,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<(), SourceRunError> {
        poll_guard(guard)?;
        self.engine
            .validate_source_attempt(self.executor, admission)?;
        Ok(())
    }

    fn failure(
        &self,
        admission: &SourceAdmission,
        stage: ExecutionStage,
        cause: SourceRunError,
        cleanup: Option<CleanupOutcome>,
        sealed: Option<SealedSourceEvidence>,
    ) -> SourceRunFailure {
        let outcome = match cause {
            SourceRunError::Stopped(GuardDecision::Cancelled) => SourceAttemptState::Cancelled,
            SourceRunError::Stopped(GuardDecision::Expired) => SourceAttemptState::TimedOut,
            _ => SourceAttemptState::ExecutionFailed,
        };
        let recording = self.engine.finish_source_attempt(
            self.executor,
            admission,
            outcome,
            format!("{stage:?}: {cause}"),
        );
        SourceRunFailure {
            stage,
            cause,
            terminal_recording: Some(recording.map(|_| ())),
            cleanup,
            sealed,
        }
    }
}

struct PendingCheck {
    input: SourceCheckRecordInput,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    isolation_record: Vec<u8>,
}

impl PendingCheck {
    fn executed(
        admission: &SourceAdmission,
        definition: &SourceCheckDefinition,
        raw: OciCheckRecord,
    ) -> Result<Self, SourceRunError> {
        if raw.execution != crate::oci::execution_identity(admission, definition)
            || raw.rootfs != definition.input().toolchain.runtime_closure
            || raw.check != *definition.id()
            || raw.definition != definition.digest()
            || raw.plan != admission.plan().id()
            || raw.snapshot != admission.resulting_snapshot()
            || raw.manifest != admission.resulting_manifest()
            || raw.backend != definition.input().backend
        {
            return Err(SourceRunError::Binding("backend execution receipt"));
        }
        // The control observations are retained as bytes, not only as a digest
        // pointing at a process-local journal. The OCI result also binds that
        // observation digest to the exact command and admission.
        let execution = serde_json::from_slice::<serde_json::Value>(&raw.canonical_bytes())
            .map_err(|_| SourceRunError::Binding("backend record encoding"))?;
        let isolation = raw
            .isolation
            .as_ref()
            .map(|bytes| {
                serde_json::from_slice::<serde_json::Value>(bytes)
                    .map_err(|_| SourceRunError::Binding("backend isolation encoding"))
            })
            .transpose()?;
        if raw.outcome == (SourceCheckOutcome::Exited { code: 0 }) && isolation.is_none() {
            return Err(SourceRunError::Binding(
                "successful check lacks effective isolation observations",
            ));
        }
        let isolation_record = serde_json::to_vec(&serde_json::json!({
            "version":1,"execution":execution,"effective_isolation":isolation,
        }))
        .expect("isolation artifact JSON");
        let policy = definition.input();
        Ok(Self {
            input: SourceCheckRecordInput {
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
                    CleanupOutcome::Quarantined { reason, .. } => {
                        SourceCleanup::Quarantined { reason }
                    }
                },
                stdout_truncated: raw.stdout_truncated,
                stderr_truncated: raw.stderr_truncated,
                artifacts: BTreeMap::new(),
                diagnostics: raw.diagnostics,
            },
            stdout: raw.stdout,
            stderr: raw.stderr,
            isolation_record,
        })
    }

    fn skipped(
        admission: &SourceAdmission,
        definition: &SourceCheckDefinition,
        now: u64,
        dependency: &CheckId,
    ) -> Self {
        let policy = definition.input();
        Self {
            input: SourceCheckRecordInput {
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
                started_at_ms: now,
                finished_at_ms: now,
                outcome: SourceCheckOutcome::Skipped {
                    reason: format!("prerequisite {dependency} did not pass"),
                },
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
                diagnostics: vec![format!(
                    "check not launched because prerequisite {dependency} did not pass"
                )],
            },
            stdout: vec![],
            stderr: vec![],
            isolation_record: br#"{"version":1,"kind":"not-executed"}"#.to_vec(),
        }
    }

    fn retained_bytes(&self) -> Result<u64, SourceArtifactError> {
        [
            self.stdout.len(),
            self.stderr.len(),
            self.isolation_record.len(),
        ]
        .into_iter()
        .chain(self.input.diagnostics.iter().map(String::len))
        .try_fold(0_u64, |n, size| {
            n.checked_add(size as u64).ok_or(SourceArtifactError::Limit)
        })
    }
}

fn poll_guard(guard: &mut dyn ExecutionGuard) -> Result<(), SourceRunError> {
    match guard.revalidate().map_err(SourceRunError::Guard)? {
        GuardDecision::Continue => Ok(()),
        stopped => Err(SourceRunError::Stopped(stopped)),
    }
}
fn micros(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
