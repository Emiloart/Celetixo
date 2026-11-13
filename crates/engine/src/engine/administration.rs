//! Pure administrative transitions shared by ordinary and keyed operations.
//! The caller must enter through the trusted embedding's scoped admin boundary.
use super::*;
use crate::source_execution::{BackendIdentity, ExecutionTier, ResourceLimits};

pub(super) fn grant_session(
    db: &mut Database,
    principal: PrincipalId,
    scope: &Scope,
    permissions: BTreeSet<Permission>,
) -> Result<u64, EngineError> {
    let token = db
        .next_token
        .checked_add(1)
        .ok_or(EngineError::CounterExhausted)?;
    db.next_token = token;
    db.grants.insert(
        token,
        Grant {
            principal,
            scope: scope.clone(),
            permissions,
            active: true,
        },
    );
    Ok(token)
}
pub(super) fn revoke_session(
    db: &mut Database,
    scope: &Scope,
    token: u64,
    principal: &PrincipalId,
) -> Result<(), EngineError> {
    let grant = db.grants.get_mut(&token).ok_or(EngineError::Unauthorized)?;
    if grant.scope != *scope || grant.principal != *principal {
        return Err(EngineError::Unauthorized);
    }
    grant.active = false;
    let now = db.now;
    for tx in db.transactions.values_mut() {
        if let Some(lease) = &tx.lease
            && lease.owner == token
        {
            tx.lease_history
                .push(lease.event(now, principal.clone(), LeaseEventKind::Revoked));
        }
    }
    Ok(())
}
pub(super) fn grant_executor(
    db: &mut Database,
    principal: PrincipalId,
    scope: &Scope,
    backend: BackendIdentity,
    tiers: BTreeSet<ExecutionTier>,
    resource_ceiling: ResourceLimits,
    expires_at_ms: u64,
) -> Result<u64, EngineError> {
    if tiers.is_empty()
        || tiers.contains(&ExecutionTier::T0)
        || !source_attempts::valid_text(&backend.name)
        || !source_attempts::valid_text(&backend.version)
        || expires_at_ms <= db.now
    {
        return Err(SourceAttemptError::UntrustedExecutor.into());
    }
    resource_ceiling
        .validate()
        .map_err(SourceAttemptError::from)?;
    let token = db
        .next_source_executor
        .checked_add(1)
        .ok_or(EngineError::CounterExhausted)?;
    db.next_source_executor = token;
    db.source_executors.insert(
        token,
        source_attempts::SourceExecutorGrant {
            principal,
            scope: scope.clone(),
            backend,
            tiers,
            resource_ceiling,
            expires_at_ms,
            active: true,
        },
    );
    Ok(token)
}
pub(super) fn revoke_executor(
    db: &mut Database,
    scope: &Scope,
    token: u64,
    principal: &PrincipalId,
    backend: &BackendIdentity,
) -> Result<(), EngineError> {
    let grant = db
        .source_executors
        .get_mut(&token)
        .ok_or(EngineError::Unauthorized)?;
    if grant.scope != *scope || grant.principal != *principal || grant.backend != *backend {
        return Err(EngineError::Unauthorized);
    }
    grant.active = false;
    let now = db.now;
    for record in db.source_attempts.values_mut() {
        if record.admission.binding().scope == *scope
            && record.executor_capability == Some(token)
            && !record.state.is_terminal()
        {
            record.transition(now, SourceAttemptState::Cancelled, "executor grant revoked")?;
        }
    }
    Ok(())
}
pub(super) fn grant_cleanup(
    db: &mut Database,
    principal: PrincipalId,
    scope: &Scope,
    backend: Digest,
    expires_at_ms: u64,
) -> Result<u64, EngineError> {
    if expires_at_ms <= db.now {
        return Err(EngineError::InvalidTime);
    }
    let token = db
        .next_source_cleanup
        .checked_add(1)
        .ok_or(EngineError::CounterExhausted)?;
    db.next_source_cleanup = token;
    db.source_cleanup_grants.insert(
        token,
        source_cleanup::SourceCleanupGrant {
            principal,
            scope: scope.clone(),
            backend,
            expires_at_ms,
            active: true,
        },
    );
    Ok(token)
}
pub(super) fn revoke_cleanup(
    db: &mut Database,
    scope: &Scope,
    token: u64,
    principal: &PrincipalId,
    backend: Digest,
) -> Result<(), EngineError> {
    let grant = db
        .source_cleanup_grants
        .get_mut(&token)
        .ok_or(EngineError::Unauthorized)?;
    if grant.scope != *scope || grant.principal != *principal || grant.backend != backend {
        return Err(EngineError::Unauthorized);
    }
    grant.active = false;
    Ok(())
}
pub(super) fn register_store(
    db: &mut Database,
    scope: &Scope,
    adapter: Option<SourceArtifactAdapter>,
    retention: Digest,
) -> Result<u64, EngineError> {
    let token = db
        .next_source_artifact_store
        .checked_add(1)
        .ok_or(EngineError::CounterExhausted)?;
    let provenance =
        source_evidence_gate::artifact_store_provenance(scope, token, retention, adapter.as_ref());
    db.source_artifact_stores.insert(
        token,
        source_evidence_gate::RegisteredSourceArtifactStore {
            scope: scope.clone(),
            adapter,
            retention,
            provenance,
            active: true,
        },
    );
    db.next_source_artifact_store = token;
    Ok(token)
}
pub(super) fn revoke_store(
    db: &mut Database,
    scope: &Scope,
    token: u64,
) -> Result<(), EngineError> {
    let registered = db
        .source_artifact_stores
        .get_mut(&token)
        .ok_or(EngineError::Unauthorized)?;
    if registered.scope != *scope {
        return Err(EngineError::Unauthorized);
    }
    registered.active = false;
    Ok(())
}
