//! Isolated historical metadata frame. Never installed into the live store.
use super::*;

pub(super) fn at_observation(
    db: &Database,
    scope: &Scope,
    key: &MutationKey,
    request: &SourceRevalidationRequest,
    r: &MutationRecord,
) -> Result<Database, RestoreError> {
    let invalid = RestoreError::BindingMismatch;
    let c = r.source_revalidation.as_ref().ok_or(invalid.clone())?;
    let MutationActor::Session { grant, .. } = key.actor else {
        return Err(invalid);
    };
    let mut old = db.clone();
    old.now = r.recorded_at_ms;
    old.grants.get_mut(&grant).ok_or(invalid.clone())?.active = true;
    match c.head {
        Some(head) => {
            let facts = db
                .source_analysis_records
                .get(&(scope.clone(), c.analysis.ok_or(invalid.clone())?))
                .ok_or(invalid.clone())?;
            let repo = old.repositories.get_mut(scope).ok_or(invalid.clone())?;
            let snapshot = repo.snapshots.get(&head).ok_or(invalid.clone())?.clone();
            facts.validate(&snapshot, facts.index_generation())?;
            let generation = c.model_generation.ok_or(invalid.clone())?;
            if generation > repo.model.generation {
                return Err(invalid);
            }
            repo.history
                .retain(|change| change.revision <= snapshot.revision());
            repo.current = snapshot;
            repo.source_analysis = Some(facts.clone());
            repo.model = facts.model(generation);
        }
        None => {
            old.repositories.remove(scope);
        }
    }
    match c.policy {
        Some(id) => {
            old.source_policies.insert(
                scope.clone(),
                db.source_policy_records
                    .get(&(scope.clone(), id))
                    .ok_or(invalid.clone())?
                    .clone(),
            );
        }
        None => {
            old.source_policies.remove(scope);
        }
    }
    old.reservation_fences
        .insert(scope.clone(), c.reservation_fence_before);
    // Every previously acquired scope participates. A later transaction cannot
    // be invented as an original conflict, or an earlier acquisition omitted.
    for ((s, id), tx) in &db.transactions {
        if s == scope
            && tx
                .lease_history
                .iter()
                .any(|e| e.fence <= c.reservation_fence_before)
            && !c.leases.contains_key(id)
        {
            return Err(invalid);
        }
    }
    old.transactions
        .retain(|(s, id), _| s != scope || c.leases.contains_key(id));
    for (id, observed) in &c.leases {
        let tx = old
            .transactions
            .get_mut(&(scope.clone(), id.clone()))
            .ok_or(invalid.clone())?;
        let count = usize::try_from(observed.event_count).map_err(|_| RestoreError::Limit)?;
        if count > tx.lease_history.len()
            || tx.lease_history[..count]
                .iter()
                .any(|e| e.at > r.recorded_at_ms || e.fence > c.reservation_fence_before)
            || tx.lease_history[count..].iter().any(|e| {
                matches!(e.kind, LeaseEventKind::Acquired { .. })
                    && e.fence <= c.reservation_fence_before
            })
        {
            return Err(invalid);
        }
        tx.lease_history.truncate(count);
        tx.lease = super::super::source_commit::history::observed_lease(tx)?;
        if tx.lease.is_some() != observed.grant_active.is_some() {
            return Err(invalid);
        }
        if let Some(lease) = &tx.lease {
            let active = observed.grant_active.ok_or(invalid.clone())?;
            let g = old.grants.get_mut(&lease.owner).ok_or(invalid.clone())?;
            if !active && g.active {
                return Err(invalid);
            }
            g.active = active;
        }
    }
    let target = (scope.clone(), request.transaction.clone());
    if let Some(number) = r.resolved_attempt {
        let tx = old.transactions.get_mut(&target).ok_or(invalid.clone())?;
        let count = usize::try_from(number)
            .map_err(|_| RestoreError::Limit)?
            .checked_add(1)
            .ok_or(RestoreError::Limit)?;
        if count > tx.attempts.len() || tx.lease_history.len() as u64 != r.lease_events_before {
            return Err(invalid);
        }
        tx.attempts.truncate(count);
        tx.state = r.state_before.ok_or(invalid.clone())?;
    } else if c.leases.contains_key(&request.transaction) {
        return Err(invalid);
    } else {
        old.transactions.remove(&target);
    }
    for (id, count) in &c.source_event_counts {
        let source = old
            .source_attempts
            .get_mut(&(scope.clone(), id.clone()))
            .ok_or(invalid.clone())?;
        let count = usize::try_from(*count).map_err(|_| RestoreError::Limit)?;
        if source.admission.binding().transaction != request.transaction
            || count == 0
            || count > source.events.len()
            || source.events[..count]
                .iter()
                .any(|e| e.at_ms > r.recorded_at_ms)
        {
            return Err(invalid);
        }
        source.events.truncate(count);
        source.state = source.events.last().ok_or(invalid.clone())?.to;
    }
    old.source_attempts.retain(|(s, id), a| {
        s != scope
            || a.admission.binding().transaction != request.transaction
            || c.source_event_counts.contains_key(id)
    });
    Ok(old)
}
