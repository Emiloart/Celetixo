//! Original executor transitions. These values describe history; execution
//! still requires fresh admission validation and independent launch recovery.
use super::*;
use crate::persistence::{RecoverySize, RestoreError};
use crate::source_patch::PatchBinding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceExecutorAction {
    Claim,
    PatchApplied,
    BeginVerification,
    Finish {
        outcome: SourceAttemptState,
        reason: String,
    },
}
impl SourceExecutorAction {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Claim => "ClaimSourceAttempt",
            Self::PatchApplied => "SourcePatchApplied",
            Self::BeginVerification => "BeginSourceVerification",
            Self::Finish { .. } => "FinishSourceAttempt",
        }
    }
    pub(crate) fn transition(&self) -> (SourceAttemptState, &str) {
        match self {
            Self::Claim => (
                SourceAttemptState::Patching,
                "trusted executor claimed attempt",
            ),
            Self::PatchApplied => (
                SourceAttemptState::PatchApplied,
                "exact candidate materialized",
            ),
            Self::BeginVerification => {
                (SourceAttemptState::Verifying, "policy verification started")
            }
            Self::Finish { outcome, reason } => (*outcome, reason),
        }
    }
    fn invalid_input(&self) -> bool {
        matches!(self, Self::Finish { outcome, reason } if !source_attempts::valid_text(reason)
            || !outcome.is_terminal() || *outcome == SourceAttemptState::EvidenceSealed)
    }
}

/// Exact immutable admission selector and original binding. A digest does not
/// authenticate an executor or select an unchecked admission dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceExecutorMutation {
    pub admission: SourceAdmissionId,
    pub binding: PatchBinding,
    pub action: SourceExecutorAction,
}
impl SourceExecutorMutation {
    pub fn new(admission: &SourceAdmission, action: SourceExecutorAction) -> Self {
        Self {
            admission: admission.id(),
            binding: admission.binding().clone(),
            action,
        }
    }
    pub(super) fn encode(&self, c: &mut Canonical) {
        c.str(self.action.kind());
        c.digest(self.admission.0);
        crate::source_patch::encode_binding(c, &self.binding);
        if let SourceExecutorAction::Finish { outcome, reason } = &self.action {
            c.str(outcome.name());
            c.str(reason);
        }
    }
    fn attempt<'a>(
        &self,
        db: &'a Database,
        scope: &Scope,
    ) -> Option<&'a source_attempts::SourceAttemptRecord> {
        db.source_attempts
            .get(&(scope.clone(), self.binding.source_attempt.clone()))
    }
    pub(super) fn observe(&self, db: &Database, scope: &Scope) -> SourceMutationContext {
        let record = self.attempt(db, scope);
        SourceMutationContext {
            admission: record.map(|r| r.admission.id()),
            state_before: record.map(|r| r.state),
            executor_before: record.and_then(|r| r.executor_capability),
            events_before: record.map_or(0, |r| r.events.len() as u64),
            events_after: record.map_or(0, |r| r.events.len() as u64),
        }
    }
    pub(super) fn observe_after(
        &self,
        db: &Database,
        scope: &Scope,
        context: &mut SourceMutationContext,
    ) {
        context.events_after = self.attempt(db, scope).map_or(0, |r| r.events.len() as u64);
    }
    pub(super) fn apply(
        &self,
        engine: &Engine,
        db: &mut Database,
        executor: &SourceExecutorCapability,
        scope: &Scope,
    ) -> Result<MutationValue, EngineError> {
        if self.action.invalid_input() {
            return Err(SourceAttemptError::InvalidReason.into());
        }
        let admission = self
            .attempt(db, scope)
            .ok_or(EngineError::NotFound)?
            .admission
            .clone();
        if self.admission != admission.id() || self.binding != *admission.binding() {
            // Ordinary callbacks reconcile before checking the submitted binding.
            source_attempts::reconcile_attempt(db, &source_attempts::attempt_key(&admission))?;
            return Err(SourceAttemptError::BindingMismatch.into());
        }
        let view = if self.action == SourceExecutorAction::Claim {
            engine.claim_source_attempt_in(db, executor, &admission)?
        } else {
            let (next, reason) = self.action.transition();
            engine.transition_source_attempt_in(db, executor, &admission, next, reason)?
        };
        Ok(MutationValue::SourceAttempt(Box::new(
            SourceMutationResult {
                admission: view.admission.id(),
                binding: view.admission.binding().clone(),
                state: view.state,
                executor: view.executor,
                evidence: view.evidence,
                events: view.events,
            },
        )))
    }
    pub(super) fn domain_error(&self, error: &EngineError) -> bool {
        matches!(
            error,
            EngineError::NotFound
                | EngineError::SourceAttempt(
                    SourceAttemptError::InvalidReason
                        | SourceAttemptError::BindingMismatch
                        | SourceAttemptError::UntrustedExecutor
                        | SourceAttemptError::AlreadyClaimed
                        | SourceAttemptError::InvalidState
                )
        )
    }
    pub(super) fn check_record(&self, r: &MutationRecord) -> Result<(), RestoreError> {
        let invalid = RestoreError::BindingMismatch;
        let context = r.source_execution.as_ref().ok_or(invalid.clone())?;
        if r.administration.is_some()
            || r.resolved_attempt.is_some() != r.state_before.is_some()
            || (r.resolved_attempt.is_none() && r.lease_events_before != 0)
            || context.admission.is_some() != context.state_before.is_some()
            || context.events_after < context.events_before
            || context.events_after
                > context
                    .events_before
                    .checked_add(1)
                    .ok_or(invalid.clone())?
            || (context.admission.is_none()
                && (context.events_before != 0
                    || context.events_after != 0
                    || context.executor_before.is_some()))
            || (context.admission.is_some() && context.events_before == 0)
        {
            return Err(invalid);
        }
        if self.action.invalid_input() {
            return if r.outcome == Err(SourceAttemptError::InvalidReason.into())
                && context.events_after == context.events_before
            {
                Ok(())
            } else {
                Err(invalid)
            };
        }
        match &r.outcome {
            Ok(MutationValue::SourceAttempt(value))
                if value.admission == self.admission
                    && value.binding == self.binding
                    && value.state == self.action.transition().0
                    && value.events.len() as u64 == context.events_after
                    && context.events_after == context.events_before + 1
                    && value.evidence.is_none()
                    && value.events.last().is_some_and(|e| {
                        e.at_ms == r.recorded_at_ms
                            && e.to == value.state
                            && e.reason == self.action.transition().1
                    }) =>
            {
                Ok(())
            }
            Err(error) if self.domain_error(error) => Ok(()),
            _ => Err(invalid),
        }
    }
    pub(super) fn validate(
        &self,
        db: &Database,
        scope: &Scope,
        key: &MutationKey,
        r: &MutationRecord,
    ) -> Result<(), RestoreError> {
        self.check_record(r)?;
        let invalid = RestoreError::BindingMismatch;
        let MutationActor::SourceExecutor { principal, grant } = &key.actor else {
            return Err(invalid);
        };
        let actor = db.source_executors.get(grant).ok_or(invalid.clone())?;
        if *grant == 0
            || actor.scope != *scope
            || actor.principal != *principal
            || actor.expires_at_ms <= r.recorded_at_ms
            || r.recorded_at_ms > db.now
            || self.binding.scope != *scope
        {
            return Err(invalid);
        }
        let native = db
            .transactions
            .get(&(scope.clone(), self.binding.transaction.clone()));
        if let Some(attempt) = r.resolved_attempt {
            let native = native.ok_or(invalid.clone())?;
            if attempt > native.attempt().number
                || r.lease_events_before > native.lease_history.len() as u64
                || r.lease_events_before
                    .checked_sub(1)
                    .is_some_and(|i| native.lease_history[i as usize].at > r.recorded_at_ms)
            {
                return Err(invalid);
            }
        }
        let context = r.source_execution.as_ref().ok_or(invalid.clone())?;
        let Some(original) = context.admission else {
            return if self.action.invalid_input() || r.outcome == Err(EngineError::NotFound) {
                Ok(())
            } else {
                Err(invalid)
            };
        };
        let actual = self.attempt(db, scope).ok_or(invalid.clone())?;
        let before = usize::try_from(context.events_before).map_err(|_| invalid.clone())?;
        let after = usize::try_from(context.events_after).map_err(|_| invalid.clone())?;
        if original != actual.admission.id()
            || after > actual.events.len()
            || context.state_before != actual.events.get(before - 1).map(|e| e.to)
            || actual.events[before - 1].at_ms > r.recorded_at_ms
        {
            return Err(invalid);
        }
        let claim = actual
            .events
            .iter()
            .position(|e| e.to == SourceAttemptState::Patching);
        let claimed_before = claim
            .filter(|index| *index < before)
            .and(actual.executor_capability);
        if context.executor_before != claimed_before {
            return Err(invalid);
        }
        if self.action.invalid_input() {
            return Ok(());
        }
        let binding_matches =
            self.admission == actual.admission.id() && self.binding == *actual.admission.binding();
        let compatible = actual.admission.plan().checks().iter().all(|check| {
            let input = check.definition().input();
            input.backend == actor.backend
                && actor.tiers.contains(&input.tier)
                && input.resources.is_within(&actor.resource_ceiling)
        });
        let claimed =
            self.action == SourceExecutorAction::Claim || context.executor_before == Some(*grant);
        let grew = after != before;
        let last = &actual.events[after - 1];
        if grew
            && (last.from != context.state_before.ok_or(invalid.clone())?
                || last.at_ms != r.recorded_at_ms)
        {
            return Err(invalid);
        }
        match &r.outcome {
            Ok(MutationValue::SourceAttempt(value))
                if binding_matches
                    && compatible
                    && claimed
                    && r.resolved_attempt == Some(self.binding.transaction_attempt)
                    && r.state_before == Some(TransactionState::Executing)
                    && actual.executor_capability == Some(*grant)
                    && (self.action != SourceExecutorAction::Claim
                        || context.executor_before.is_none())
                    && value.executor.as_ref() == Some(principal)
                    && value.events == actual.events[..after]
                    && last.to == self.action.transition().0
                    && last.reason == self.action.transition().1 =>
            {
                Ok(())
            }
            Err(error) => {
                // A failed ordinary call may append one lazy reconciliation event.
                if grew
                    && !matches!(
                        last.to,
                        SourceAttemptState::Cancelled
                            | SourceAttemptState::TimedOut
                            | SourceAttemptState::RevalidationRequired
                    )
                {
                    return Err(invalid);
                }
                let expected = if !binding_matches {
                    SourceAttemptError::BindingMismatch
                } else if !compatible || !claimed {
                    SourceAttemptError::UntrustedExecutor
                } else if last.to.is_terminal() {
                    SourceAttemptError::InvalidState
                } else if self.action == SourceExecutorAction::Claim
                    && context.executor_before.is_some()
                {
                    SourceAttemptError::AlreadyClaimed
                } else if !last.to.allows(self.action.transition().0) {
                    SourceAttemptError::InvalidState
                } else {
                    return Err(invalid);
                };
                if *error == EngineError::SourceAttempt(expected) {
                    Ok(())
                } else {
                    Err(invalid)
                }
            }
            _ => Err(invalid),
        }
    }
    pub(super) fn recovery_size(&self) -> Result<RecoverySize, RestoreError> {
        let mut size = binding_size(&self.binding)?;
        if let SourceExecutorAction::Finish { reason, .. } = &self.action {
            size.bytes = size
                .bytes
                .checked_add(reason.len())
                .ok_or(RestoreError::Limit)?;
        }
        Ok(size)
    }
}

/// Engine-observed original source state, independent of today's mutable head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMutationContext {
    pub admission: Option<SourceAdmissionId>,
    pub state_before: Option<SourceAttemptState>,
    pub executor_before: Option<u64>,
    pub events_before: u64,
    pub events_after: u64,
}
/// Historical view header and event prefix. Resolve its admission through the
/// checked immutable graph; this result never authorizes a launch or a receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMutationResult {
    pub admission: SourceAdmissionId,
    pub binding: PatchBinding,
    pub state: SourceAttemptState,
    pub executor: Option<PrincipalId>,
    pub evidence: Option<SourceEvidenceId>,
    pub events: Vec<SourceAttemptEvent>,
}
impl SourceMutationResult {
    pub(super) fn recovery_size(&self) -> Result<RecoverySize, RestoreError> {
        let mut size = binding_size(&self.binding)?;
        size.entries = size
            .entries
            .checked_add(self.events.len())
            .ok_or(RestoreError::Limit)?;
        for event in &self.events {
            size.bytes = size
                .bytes
                .checked_add(256)
                .and_then(|n| n.checked_add(event.reason.len()))
                .ok_or(RestoreError::Limit)?;
        }
        if let Some(principal) = &self.executor {
            size.bytes = size
                .bytes
                .checked_add(principal.as_str().len())
                .ok_or(RestoreError::Limit)?;
        }
        Ok(size)
    }
}
fn binding_size(binding: &PatchBinding) -> Result<RecoverySize, RestoreError> {
    let mut bytes = 4096usize;
    for text in [
        binding.scope.organization.as_str(),
        binding.scope.workspace.as_str(),
        binding.scope.repository.as_str(),
        binding.principal.as_str(),
        binding.transaction.as_str(),
        binding.source_attempt.as_str(),
        &binding.producer,
    ] {
        bytes = bytes.checked_add(text.len()).ok_or(RestoreError::Limit)?;
    }
    Ok(RecoverySize { bytes, entries: 16 })
}
fn authorize(
    engine: &Engine,
    db: &Database,
    executor: &SourceExecutorCapability,
    scope: &Scope,
) -> Result<(), EngineError> {
    source_attempts::validate_executor_store(engine, executor)?;
    let grant = db
        .source_executors
        .get(&executor.token())
        .ok_or(SourceAttemptError::UntrustedExecutor)?;
    if executor.scope() != scope
        || grant.scope != *scope
        || grant.principal != *executor.principal()
        || grant.backend != *executor.backend()
        || !grant.active
        || grant.expires_at_ms <= db.now
    {
        return Err(SourceAttemptError::UntrustedExecutor.into());
    }
    Ok(())
}
impl Engine {
    pub fn mutate_source_executor(
        &self,
        executor: &SourceExecutorCapability,
        scope: &Scope,
        key: IdempotencyKey,
        request: SourceExecutorMutation,
    ) -> Result<MutationResponse, EngineError> {
        self.store.source_executor(executor, scope).atomic(|db| {
            if let Err(error) = authorize(self, db, executor, scope) {
                return Ok(Err(error));
            }
            if request.binding.scope != *scope {
                return Ok(Err(EngineError::Unauthorized));
            }
            let actor = MutationActor::SourceExecutor {
                principal: executor.principal().clone(),
                grant: executor.token(),
            };
            apply_once(
                db,
                scope,
                actor,
                key,
                MutationRequest::SourceExecutor(Box::new(request)),
                |db, request| {
                    let MutationRequest::SourceExecutor(request) = request else {
                        return Err(EngineError::Unauthorized);
                    };
                    request.apply(self, db, executor, scope)
                },
            )
        })?
    }
    pub fn source_executor_mutation_outcome(
        &self,
        executor: &SourceExecutorCapability,
        scope: &Scope,
        key: &IdempotencyKey,
    ) -> Result<Option<MutationResponse>, EngineError> {
        self.store.source_executor(executor, scope).read(|db| {
            authorize(self, db, executor, scope)?;
            let key = MutationKey {
                actor: MutationActor::SourceExecutor {
                    principal: executor.principal().clone(),
                    grant: executor.token(),
                },
                key: key.clone(),
            };
            Ok(db
                .mutations
                .get(&(scope.clone(), key.clone()))
                .map(|r| r.response(scope, &key.actor, true)))
        })
    }
}
