//! Effects are staged by the authenticated shared journal, never by raw IDs.
use super::*;

pub(in crate::engine) fn apply(
    db: &mut Database,
    scope: &Scope,
    actor: &MutationActor,
    request: &SourceRevalidationRequest,
) -> Result<SourceRevalidationReceipt, EngineError> {
    let selected = selection::select(db, scope, actor, request)?;
    // Check every invalidation before changing the authoritative stage. Sealed
    // source evidence/execution stays immutable and is absent from the new frame.
    let mut invalidated = Vec::new();
    for id in &selected.receipt.invalidated_source_attempts {
        let mut source = db
            .source_attempts
            .get(&(scope.clone(), id.clone()))
            .ok_or(EngineError::NotFound)?
            .clone();
        source.transition(
            db.now,
            SourceAttemptState::RevalidationRequired,
            "fresh source attempt invalidates prior execution eligibility",
        )?;
        invalidated.push((id.clone(), source));
    }
    for (id, source) in invalidated {
        db.source_attempts.insert((scope.clone(), id), source);
    }
    db.reservation_fences
        .insert(scope.clone(), selected.receipt.fence);
    db.transactions.insert(
        (scope.clone(), request.transaction.clone()),
        selected.transaction,
    );
    Ok(selected.receipt)
}

pub(in crate::engine) fn observe(
    db: &Database,
    scope: &Scope,
    request: &SourceRevalidationRequest,
) -> SourceRevalidationContext {
    let repo = db.repositories.get(scope);
    SourceRevalidationContext {
        head: repo.map(|r| r.current.id()),
        model_generation: repo.map(|r| r.model.generation),
        analysis: repo.and_then(|r| r.source_analysis.as_ref().map(RepositoryAnalysis::digest)),
        policy: db.source_policies.get(scope).map(|p| p.digest()),
        reservation_fence_before: db.reservation_fences.get(scope).copied().unwrap_or(0),
        leases: db
            .transactions
            .iter()
            .filter(|((s, _), _)| s == scope)
            .map(|((_, id), tx)| {
                (
                    id.clone(),
                    SourceLeaseObservation {
                        event_count: tx.lease_history.len() as u64,
                        grant_active: tx
                            .lease
                            .as_ref()
                            .and_then(|lease| db.grants.get(&lease.owner).map(|g| g.active)),
                    },
                )
            })
            .collect(),
        source_event_counts: db
            .source_attempts
            .iter()
            .filter_map(|((s, id), a)| {
                (*s == *scope && a.admission.binding().transaction == request.transaction)
                    .then_some((id.clone(), a.events.len() as u64))
            })
            .collect(),
    }
}
