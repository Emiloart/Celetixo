//! Full fresh-attempt metadata decision, without publication or caller auth.
use super::*;

pub(in crate::engine) struct Selection {
    pub transaction: Transaction,
    pub receipt: SourceRevalidationReceipt,
}

/// Caller authentication remains an independent opaque-Session requirement.
/// No old patch, candidate, check result or source admission enters the attempt.
pub(in crate::engine) fn select(
    db: &Database,
    scope: &Scope,
    actor: &MutationActor,
    request: &SourceRevalidationRequest,
) -> Result<Selection, EngineError> {
    let MutationActor::Session { principal, grant } = actor else {
        return Err(EngineError::Unauthorized);
    };
    let caller = db.grants.get(grant).ok_or(EngineError::Unauthorized)?;
    if !caller.active
        || caller.scope != *scope
        || caller.principal != *principal
        || ![
            Permission::Analyze,
            Permission::Reserve,
            Permission::Execute,
        ]
        .iter()
        .all(|p| caller.permissions.contains(p))
        || request.scope != *scope
        || request.principal != *principal
        || request.authorization_grant != *grant
    {
        return Err(EngineError::Unauthorized);
    }
    let old = db
        .transactions
        .get(&(scope.clone(), request.transaction.clone()))
        .ok_or(EngineError::NotFound)?;
    if old.owner != *principal {
        return Err(EngineError::Unauthorized);
    }
    if !matches!(
        old.state,
        TransactionState::Executing
            | TransactionState::Validating
            | TransactionState::RevalidationRequired
    ) {
        return Err(SourceAttemptError::InvalidState.into());
    }
    let original = old.attempt();
    if request.previous_attempt != original.number
        || request.previous_base != original.base.id()
        || request.previous_model_generation != original.model.generation
        || original.analysis.as_ref().map(|analysis| analysis.digest())
            != Some(request.previous_impact)
        || request.intent != source_attempts::intent_digest(&old.intent)
    {
        return Err(EngineError::CommitRequestMismatch);
    }
    let lease = old.lease.as_ref().ok_or(EngineError::InvalidLease)?;
    if lease.owner != *grant || lease.fence != request.fence || !lease_active(db, lease) {
        return Err(EngineError::InvalidLease);
    }
    let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
    let analysis = repo
        .source_analysis
        .as_ref()
        .ok_or(SourceAttemptError::SourceRepositoryRequired)?;
    let policy = db
        .source_policies
        .get(scope)
        .ok_or(SourceAttemptError::PolicyChanged)?;
    if request.head != repo.current.id()
        || request.revision != repo.current.revision()
        || request.model_generation != repo.model.generation
        || request.analysis != analysis.digest()
        || request.index_generation != analysis.index_generation()
        || request.policy != policy.digest()
        || request.policy_generation != policy.input().generation
    {
        return Err(EngineError::CommitRequestMismatch);
    }
    analysis.validate(&repo.current, analysis.index_generation())?;
    if !analysis.is_complete() || analysis.model(repo.model.generation) != repo.model {
        return Err(EngineError::IncompleteAnalysis);
    }
    if policy.input().expires_at_ms <= db.now {
        return Err(SourceAttemptError::Expired.into());
    }
    if policy.input().scope != *scope
        || policy.input().snapshot != repo.current.id()
        || policy.input().analysis_artifact != analysis.digest()
        || policy.input().index_generation != analysis.index_generation()
    {
        return Err(SourceAttemptError::PolicyChanged.into());
    }
    policy
        .validate_snapshot(&repo.current)
        .map_err(SourceAttemptError::from)?;
    if original.base.id() == repo.current.id()
        && original.model == repo.model
        && original.source_analysis == Some(analysis.digest())
        && original.source_policy == Some(policy.digest())
    {
        return Err(EngineError::InvalidIntent(
            "source revalidation requires changed authoritative inputs".into(),
        ));
    }
    let number = original
        .number
        .checked_add(1)
        .ok_or(EngineError::CounterExhausted)?;
    let mut next = old.clone();
    next.attempts.push(Attempt {
        number,
        base: repo.current.clone(),
        model: repo.model.clone(),
        effective: None,
        analysis: None,
        source_analysis: Some(analysis.digest()),
        source_policy: Some(policy.digest()),
        patch: None,
        candidate: None,
        evidence: BTreeMap::new(),
    });
    analyze_attempt(&mut next, Some(analysis), Some(policy))?;
    let accesses = next
        .attempt()
        .effective
        .clone()
        .ok_or(EngineError::IncompleteAnalysis)?;
    for ((other_scope, id), other) in &db.transactions {
        if other_scope == scope
            && id != &old.id
            && let Some(held) = &other.lease
            && lease_active(db, held)
            && !held.writes.is_disjoint(&accesses.writes)
        {
            return Err(EngineError::ReservationConflict);
        }
    }
    let expires_at = expiration(db.now, request.ttl_ms)?.min(policy.input().expires_at_ms);
    let fence = db
        .reservation_fences
        .get(scope)
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(EngineError::CounterExhausted)?;
    let invalidated_source_attempts = db
        .source_attempts
        .iter()
        .filter_map(|((s, id), a)| {
            (*s == *scope && a.admission.binding().transaction == old.id && !a.state.is_terminal())
                .then_some(id.clone())
        })
        .collect();
    let receipt = SourceRevalidationReceipt {
        scope: scope.clone(),
        transaction: old.id.clone(),
        previous_attempt: original.number,
        attempt: number,
        previous_base: original.base.id(),
        base: repo.current.id(),
        revision: repo.current.revision(),
        model_generation: repo.model.generation,
        analysis: analysis.digest(),
        index_generation: analysis.index_generation(),
        policy: policy.digest(),
        policy_generation: policy.input().generation,
        previous_fence: lease.fence,
        fence,
        expires_at_ms: expires_at,
        accesses: accesses.clone(),
        impact: next
            .attempt()
            .analysis
            .as_ref()
            .ok_or(EngineError::IncompleteAnalysis)?
            .digest(),
        invalidated_source_attempts,
        intervening_revisions: repo
            .history
            .iter()
            .filter(|change| change.revision > original.base.revision())
            .map(|change| change.revision)
            .collect(),
        request: request.digest(scope, principal),
        revalidated_at_ms: db.now,
    };
    next.lease_history.push(lease.event(
        db.now,
        principal.clone(),
        LeaseEventKind::Released {
            reason: "fresh source revalidation".into(),
        },
    ));
    let fresh = Lease {
        fence,
        owner: *grant,
        acquired_at: db.now,
        acquired_attempt: number,
        expires_at,
        writes: accesses.writes,
    };
    next.lease_history.push(fresh.event(
        db.now,
        principal.clone(),
        LeaseEventKind::Acquired { expires_at },
    ));
    next.lease = Some(fresh);
    if next.state != TransactionState::RevalidationRequired {
        next.transition(TransactionState::RevalidationRequired)?;
    }
    next.transition(TransactionState::Analyzed)?;
    next.transition(TransactionState::Reserved)?;
    next.transition(TransactionState::Executing)?;
    next.last_reason =
        Some("fresh source revalidation; new explicit patch and all new evidence required".into());
    Ok(Selection {
        transaction: next,
        receipt,
    })
}
