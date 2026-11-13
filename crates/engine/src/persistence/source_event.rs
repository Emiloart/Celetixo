//! Ordered source lifecycle facts, with separate admission/rejection namespaces.
use super::*;
use crate::engine::{SourceAttemptEvent, SourceAttemptState};
use crate::ids::{Canonical, SourceAttemptId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEventKind {
    Attempt,
    Rejection,
}
impl SourceEventKind {
    pub fn format(self) -> &'static str {
        match self {
            Self::Attempt => "celetixo/source-attempt-event/v1",
            Self::Rejection => "celetixo/source-rejection-event/v1",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEventRecord {
    kind: SourceEventKind,
    scope: Scope,
    source_attempt: SourceAttemptId,
    event: SourceAttemptEvent,
}
impl SourceEventRecord {
    pub fn new(
        kind: SourceEventKind,
        scope: Scope,
        source_attempt: SourceAttemptId,
        event: SourceAttemptEvent,
    ) -> Result<Self, RestoreError> {
        if !event.from.allows(event.to) || event.reason.trim().is_empty() {
            return Err(RestoreError::InvalidDomainValue);
        }
        match kind {
            SourceEventKind::Attempt => {
                if event.reason.len() > 4096
                    || event.reason.chars().any(char::is_control)
                    || (event.sequence == 0
                        && (event.from != SourceAttemptState::Proposed
                            || event.to != SourceAttemptState::Admitted))
                    || (event.sequence != 0 && event.from == SourceAttemptState::Proposed)
                {
                    return Err(RestoreError::InvalidDomainValue);
                }
            }
            SourceEventKind::Rejection => {
                if event.sequence != 0
                    || event.from != SourceAttemptState::Proposed
                    || event.to != SourceAttemptState::Rejected
                {
                    return Err(RestoreError::InvalidDomainValue);
                }
            }
        }
        Ok(Self {
            kind,
            scope,
            source_attempt,
            event,
        })
    }
    pub fn kind(&self) -> SourceEventKind {
        self.kind
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn source_attempt(&self) -> &SourceAttemptId {
        &self.source_attempt
    }
    pub fn event(&self) -> &SourceAttemptEvent {
        &self.event
    }
    pub fn from_label(&self) -> &'static str {
        state_name(self.event.from)
    }
    pub fn to_label(&self) -> &'static str {
        state_name(self.event.to)
    }
    pub fn digest(&self) -> Digest {
        Digest::of(self.kind.format(), &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(self.scope.organization.as_str());
        c.str(self.scope.workspace.as_str());
        c.str(self.scope.repository.as_str());
        c.str(self.source_attempt.as_str());
        c.u64(self.event.sequence);
        c.u64(self.event.at_ms);
        c.str(self.from_label());
        c.str(self.to_label());
        c.str(&self.event.reason);
        c.into_bytes()
    }
}
#[allow(clippy::too_many_arguments)]
pub fn restore_source_event(
    kind: SourceEventKind,
    scope: &Scope,
    source_attempt: &SourceAttemptId,
    sequence: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceEventRecord, RestoreError> {
    let mut r = Reader::new(expected, kind.format(), bytes, limits)?;
    if r.scope()? != *scope || r.text()? != source_attempt.as_str() || r.number()? != sequence {
        return Err(RestoreError::BindingMismatch);
    }
    let event = SourceAttemptEvent {
        sequence,
        at_ms: r.number()?,
        from: state(&r.text()?)?,
        to: state(&r.text()?)?,
        reason: r.text()?,
    };
    r.end()?;
    let record = SourceEventRecord::new(kind, scope.clone(), source_attempt.clone(), event)?;
    lifecycle::canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}
fn state_name(value: SourceAttemptState) -> &'static str {
    value.name()
}
pub(super) fn state(value: &str) -> Result<SourceAttemptState, RestoreError> {
    use SourceAttemptState::*;
    Ok(match value {
        "Proposed" => Proposed,
        "Admitted" => Admitted,
        "Patching" => Patching,
        "PatchApplied" => PatchApplied,
        "Verifying" => Verifying,
        "EvidenceSealed" => EvidenceSealed,
        "Rejected" => Rejected,
        "PatchConflict" => PatchConflict,
        "ExecutionFailed" => ExecutionFailed,
        "TimedOut" => TimedOut,
        "Cancelled" => Cancelled,
        "EvidenceIncomplete" => EvidenceIncomplete,
        "RevalidationRequired" => RevalidationRequired,
        _ => return Err(RestoreError::Encoding),
    })
}
