//! Structural original executor request/context/result fields. Parsing never
//! resolves an admission graph or constructs a live executor capability.
use super::*;
use crate::engine::{
    SourceAttemptEvent, SourceExecutorAction, SourceExecutorMutation, SourceMutationContext,
    SourceMutationResult,
};
use crate::ids::{
    Canonical, PrincipalId, SourceAdmissionId, SourceAttemptId, SourceEvidenceId, TransactionId,
};
use crate::source_patch::PatchBinding;

pub(super) fn is_kind(kind: &str) -> bool {
    matches!(
        kind,
        "ClaimSourceAttempt"
            | "SourcePatchApplied"
            | "BeginSourceVerification"
            | "FinishSourceAttempt"
    )
}
pub(super) fn request(
    r: &mut Reader<'_>,
    kind: &str,
) -> Result<SourceExecutorMutation, RestoreError> {
    let admission = SourceAdmissionId(r.digest()?);
    let binding = binding(r)?;
    let action = match kind {
        "ClaimSourceAttempt" => SourceExecutorAction::Claim,
        "SourcePatchApplied" => SourceExecutorAction::PatchApplied,
        "BeginSourceVerification" => SourceExecutorAction::BeginVerification,
        "FinishSourceAttempt" => SourceExecutorAction::Finish {
            outcome: source_event::state(&r.text()?)?,
            reason: r.text()?,
        },
        _ => return Err(RestoreError::Encoding),
    };
    Ok(SourceExecutorMutation {
        admission,
        binding,
        action,
    })
}
fn binding(r: &mut Reader<'_>) -> Result<PatchBinding, RestoreError> {
    Ok(PatchBinding {
        scope: r.scope()?,
        principal: PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        transaction: TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        transaction_attempt: r.number()?,
        source_attempt: SourceAttemptId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        base_snapshot: SnapshotId(r.digest()?),
        base_revision: Revision(r.number()?),
        analysis_artifact: r.digest()?,
        index_generation: r.digest()?,
        impact_request: r.digest()?,
        impact_result: r.digest()?,
        created_at_ms: r.number()?,
        expires_at_ms: r.number()?,
        producer: r.text()?,
    })
}
fn optional_u64(r: &mut Reader<'_>) -> Result<Option<u64>, RestoreError> {
    match r.number()? {
        0 => Ok(None),
        1 => Ok(Some(r.number()?)),
        _ => Err(RestoreError::Encoding),
    }
}
fn optional_digest(r: &mut Reader<'_>) -> Result<Option<Digest>, RestoreError> {
    match r.number()? {
        0 => Ok(None),
        1 => Ok(Some(r.digest()?)),
        _ => Err(RestoreError::Encoding),
    }
}
fn encode_optional_u64(c: &mut Canonical, value: Option<u64>) {
    match value {
        None => c.u64(0),
        Some(value) => {
            c.u64(1);
            c.u64(value);
        }
    }
}
fn encode_optional_digest(c: &mut Canonical, value: Option<Digest>) {
    match value {
        None => c.u64(0),
        Some(value) => {
            c.u64(1);
            c.digest(value);
        }
    }
}
pub(super) fn encode_context(c: &mut Canonical, value: &SourceMutationContext) {
    encode_optional_digest(c, value.admission.map(|id| id.0));
    match value.state_before {
        None => c.u64(0),
        Some(state) => {
            c.u64(1);
            c.str(state.name());
        }
    }
    encode_optional_u64(c, value.executor_before);
    c.u64(value.events_before);
    c.u64(value.events_after);
}
pub(super) fn context(r: &mut Reader<'_>) -> Result<SourceMutationContext, RestoreError> {
    Ok(SourceMutationContext {
        admission: optional_digest(r)?.map(SourceAdmissionId),
        state_before: match r.number()? {
            0 => None,
            1 => Some(source_event::state(&r.text()?)?),
            _ => return Err(RestoreError::Encoding),
        },
        executor_before: optional_u64(r)?,
        events_before: r.number()?,
        events_after: r.number()?,
    })
}
pub(super) fn encode_result(c: &mut Canonical, value: &SourceMutationResult) {
    c.digest(value.admission.0);
    crate::source_patch::encode_binding(c, &value.binding);
    c.str(value.state.name());
    match &value.executor {
        None => c.u64(0),
        Some(principal) => {
            c.u64(1);
            c.str(principal.as_str());
        }
    }
    encode_optional_digest(c, value.evidence.map(|id| id.0));
    c.u64(value.events.len() as u64);
    for event in &value.events {
        c.u64(event.sequence);
        c.u64(event.at_ms);
        c.str(event.from.name());
        c.str(event.to.name());
        c.str(&event.reason);
    }
}
pub(super) fn result(r: &mut Reader<'_>) -> Result<SourceMutationResult, RestoreError> {
    let admission = SourceAdmissionId(r.digest()?);
    let binding = binding(r)?;
    let state = source_event::state(&r.text()?)?;
    let executor = match r.number()? {
        0 => None,
        1 => Some(PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?),
        _ => return Err(RestoreError::Encoding),
    };
    let evidence = optional_digest(r)?.map(SourceEvidenceId);
    let mut events = Vec::new();
    for _ in 0..r.count()? {
        events.push(SourceAttemptEvent {
            sequence: r.number()?,
            at_ms: r.number()?,
            from: source_event::state(&r.text()?)?,
            to: source_event::state(&r.text()?)?,
            reason: r.text()?,
        });
    }
    Ok(SourceMutationResult {
        admission,
        binding,
        state,
        executor,
        evidence,
        events,
    })
}
