//! Reconstruct an original observation for pure decision checking only. This
//! adapter-free clone is never installed and cannot issue a live capability.
use super::*;

pub(super) fn at_observation(
    db: &Database,
    scope: &Scope,
    key: &MutationKey,
    request: &SourceCommitRequest,
    record: &MutationRecord,
) -> Result<Database, RestoreError> {
    let invalid = RestoreError::BindingMismatch;
    let context = record.source_commit.as_ref().ok_or(invalid.clone())?;
    let MutationActor::Session { grant, .. } = key.actor else {
        return Err(invalid);
    };
    let mut historical = db.clone();
    historical.now = record.recorded_at_ms;
    // Original caller authentication was checked before an outcome was emitted.
    // Replaying metadata does not reactivate either grant in the actual store.
    historical
        .grants
        .get_mut(&grant)
        .ok_or(invalid.clone())?
        .active = true;
    historical
        .grants
        .get_mut(&context.analysis_grant)
        .ok_or(invalid.clone())?
        .active = true;
    if let Some(head) = context.head {
        let facts = db
            .source_analysis_records
            .get(&(scope.clone(), context.analysis.ok_or(invalid.clone())?))
            .ok_or(invalid.clone())?;
        let repo = historical
            .repositories
            .get_mut(scope)
            .ok_or(invalid.clone())?;
        let snapshot = repo.snapshots.get(&head).ok_or(invalid.clone())?.clone();
        repo.history
            .retain(|change| change.revision <= snapshot.revision());
        repo.current = snapshot;
        repo.source_analysis = Some(facts.clone());
        repo.model = facts.model(context.model_generation.ok_or(invalid.clone())?);
    } else {
        historical.repositories.remove(scope);
    }
    match context.policy {
        Some(id) => {
            historical.source_policies.insert(
                scope.clone(),
                db.source_policy_records
                    .get(&(scope.clone(), id))
                    .ok_or(invalid.clone())?
                    .clone(),
            );
        }
        None => {
            historical.source_policies.remove(scope);
        }
    }
    let native_key = (scope.clone(), request.binding.transaction.clone());
    if let Some(number) = record.resolved_attempt {
        let tx = historical
            .transactions
            .get_mut(&native_key)
            .ok_or(invalid.clone())?;
        let count = usize::try_from(number)
            .map_err(|_| RestoreError::Limit)?
            .checked_add(1)
            .ok_or(RestoreError::Limit)?;
        if count > tx.attempts.len() {
            return Err(invalid);
        }
        tx.attempts.truncate(count);
        tx.state = record.state_before.ok_or(invalid.clone())?;
        let before =
            usize::try_from(record.lease_events_before).map_err(|_| RestoreError::Limit)?;
        if before > tx.lease_history.len() {
            return Err(invalid);
        }
        tx.lease_history.truncate(before);
        tx.lease = observed_lease(tx)?;
    } else {
        historical.transactions.remove(&native_key);
    }
    if let Some(observed) = &context.source {
        let source = historical
            .source_attempts
            .get_mut(&(scope.clone(), observed.source_attempt.clone()))
            .ok_or(invalid.clone())?;
        let before = usize::try_from(observed.event_count).map_err(|_| RestoreError::Limit)?;
        if before > source.events.len() {
            return Err(invalid);
        }
        source.events.truncate(before);
        source.state = observed.state;
        source.evidence = observed.evidence;
        source.executor_capability = observed.executor_grant;
        if let Some(id) = observed.executor_grant {
            historical
                .source_executors
                .get_mut(&id)
                .ok_or(invalid.clone())?
                .active = observed.executor_active.ok_or(invalid.clone())?;
        }
        if let Some(id) = observed.store_registration {
            historical
                .source_artifact_stores
                .get_mut(&id)
                .ok_or(invalid.clone())?
                .active = observed.store_active.ok_or(invalid.clone())?;
        }
    } else {
        historical
            .source_attempts
            .retain(|(s, _), a| *s != *scope || a.admission.id() != request.admission);
    }
    Ok(historical)
}

pub(in crate::engine::mutation) fn observed_lease(
    tx: &Transaction,
) -> Result<Option<Lease>, RestoreError> {
    let Some(start) = tx
        .lease_history
        .iter()
        .rposition(|event| matches!(event.kind, LeaseEventKind::Acquired { .. }))
    else {
        return Ok(None);
    };
    let first = &tx.lease_history[start];
    let LeaseEventKind::Acquired {
        expires_at: mut expiry,
    } = first.kind
    else {
        return Err(RestoreError::BindingMismatch);
    };
    let mut present = true;
    for event in &tx.lease_history[start + 1..] {
        match event.kind {
            LeaseEventKind::Renewed { expires_at, .. } => expiry = expires_at,
            LeaseEventKind::Released { .. } => present = false,
            // Expiry and revocation are independently evaluated by the clock
            // and original grant metadata; neither extends the old lease.
            LeaseEventKind::Expired | LeaseEventKind::Revoked => {}
            LeaseEventKind::Acquired { .. } => return Err(RestoreError::BindingMismatch),
        }
    }
    Ok(present.then(|| Lease {
        fence: first.fence,
        owner: first.authorization_grant,
        acquired_at: first.at,
        acquired_attempt: first.acquisition_attempt,
        expires_at: expiry,
        writes: first.scopes.clone(),
    }))
}
