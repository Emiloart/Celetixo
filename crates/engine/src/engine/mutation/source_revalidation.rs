//! Recheck the original fresh-attempt decision without reviving any capability.
use super::*;
use crate::persistence::RestoreError;
mod history;

pub(super) fn domain_error(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::NotFound
            | EngineError::Unauthorized
            | EngineError::InvalidLease
            | EngineError::InvalidTime
            | EngineError::RepositoryAnalysis(_)
            | EngineError::IncompleteAnalysis
            | EngineError::ReservationConflict
            | EngineError::CounterExhausted
            | EngineError::CommitRequestMismatch
            | EngineError::SourceAttempt(
                SourceAttemptError::InvalidState
                    | SourceAttemptError::PolicyChanged
                    | SourceAttemptError::Expired
                    | SourceAttemptError::SourceRepositoryRequired
            )
    ) || matches!(error,EngineError::InvalidIntent(reason) if reason=="source revalidation requires changed authoritative inputs")
}

pub(super) fn check_record(
    request: &SourceRevalidationRequest,
    r: &MutationRecord,
    actor: &MutationActor,
) -> Result<(), RestoreError> {
    let invalid = RestoreError::BindingMismatch;
    let MutationActor::Session { principal, grant } = actor else {
        return Err(invalid);
    };
    let c = r.source_revalidation.as_ref().ok_or(invalid.clone())?;
    if *grant == 0
        || r.source_commit.is_some()
        || r.source_execution.is_some()
        || r.administration.is_some()
        || r.resolved_attempt.is_some() != r.state_before.is_some()
        || (r.resolved_attempt.is_none() && r.lease_events_before != 0)
        || c.head.is_some() != c.analysis.is_some()
        || c.head.is_some() != c.model_generation.is_some()
        || c.model_generation == Some(0)
        || (c.head.is_none() && c.policy.is_some())
        || c.source_event_counts.values().any(|n| *n == 0)
    {
        return Err(invalid);
    }
    match &r.outcome {
        Ok(MutationValue::SourceRevalidated(receipt)) => {
            receipt.validate_request(&receipt.scope, principal, request)?;
            if r.resolved_attempt != Some(request.previous_attempt)
                || r.lease_events_before == 0
                || !matches!(
                    r.state_before,
                    Some(
                        TransactionState::Executing
                            | TransactionState::Validating
                            | TransactionState::RevalidationRequired
                    )
                )
                || request.authorization_grant != *grant
                || c.head != Some(request.head)
                || c.model_generation != Some(request.model_generation)
                || c.analysis != Some(request.analysis)
                || c.policy != Some(request.policy)
                || c.reservation_fence_before.checked_add(1) != Some(receipt.fence)
                || r.recorded_at_ms != receipt.revalidated_at_ms
            {
                return Err(invalid);
            }
            Ok(())
        }
        Err(error) if domain_error(error) => Ok(()),
        _ => Err(invalid),
    }
}

pub(super) fn validate(
    db: &Database,
    scope: &Scope,
    key: &MutationKey,
    request: &SourceRevalidationRequest,
    r: &MutationRecord,
) -> Result<(), RestoreError> {
    check_record(request, r, &key.actor)?;
    let invalid = RestoreError::BindingMismatch;
    let MutationActor::Session { principal, grant } = &key.actor else {
        return Err(invalid);
    };
    let actor = db.grants.get(grant).ok_or(invalid.clone())?;
    let c = r.source_revalidation.as_ref().ok_or(invalid.clone())?;
    if request.key != key.key
        || r.recorded_at_ms > db.now
        || actor.scope != *scope
        || actor.principal != *principal
        || !actor.permissions.contains(&Permission::Analyze)
        || c.reservation_fence_before > db.reservation_fences.get(scope).copied().unwrap_or(0)
    {
        return Err(invalid);
    }
    let historical = history::at_observation(db, scope, key, request, r)?;
    let expected =
        super::super::source_revalidation::select(&historical, scope, &key.actor, request);
    let selected = match (&r.outcome, expected) {
        (Err(actual), Err(expected)) if *actual == expected => return Ok(()),
        (Ok(MutationValue::SourceRevalidated(receipt)), Ok(selected))
            if **receipt == selected.receipt =>
        {
            selected
        }
        _ => return Err(invalid),
    };
    let tx = db
        .transactions
        .get(&(scope.clone(), request.transaction.clone()))
        .ok_or(invalid.clone())?;
    let index = usize::try_from(selected.receipt.attempt).map_err(|_| RestoreError::Limit)?;
    let actual = tx.attempts.get(index).ok_or(invalid.clone())?;
    let expected = selected.transaction.attempt();
    // Subsequent source admissions never attach native patches or native evidence.
    if actual.number != expected.number
        || actual.base != expected.base
        || actual.model != expected.model
        || actual.effective != expected.effective
        || actual.analysis != expected.analysis
        || actual.source_analysis != expected.source_analysis
        || actual.source_policy != expected.source_policy
        || actual.patch.is_some()
        || actual.candidate.is_some()
        || !actual.evidence.is_empty()
    {
        return Err(invalid);
    }
    let before = usize::try_from(r.lease_events_before).map_err(|_| RestoreError::Limit)?;
    if tx
        .lease_history
        .get(before..before.checked_add(2).ok_or(RestoreError::Limit)?)
        != selected.transaction.lease_history.get(before..)
    {
        return Err(invalid);
    }
    for id in &selected.receipt.invalidated_source_attempts {
        let source = db
            .source_attempts
            .get(&(scope.clone(), id.clone()))
            .ok_or(invalid.clone())?;
        let count = usize::try_from(*c.source_event_counts.get(id).ok_or(invalid.clone())?)
            .map_err(|_| RestoreError::Limit)?;
        let expected_source = historical
            .source_attempts
            .get(&(scope.clone(), id.clone()))
            .ok_or(invalid.clone())?;
        let event = source.events.get(count).ok_or(invalid.clone())?;
        if event.at_ms != r.recorded_at_ms
            || event.from != expected_source.state
            || event.to != SourceAttemptState::RevalidationRequired
            || event.reason != "fresh source attempt invalidates prior execution eligibility"
            || event.sequence != count as u64
        {
            return Err(invalid);
        }
    }
    Ok(())
}
