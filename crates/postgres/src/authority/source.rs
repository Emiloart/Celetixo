//! Complete normalized captured-source profile. Metadata reconstruction does not
//! resolve adapters or confer a live session/executor capability.
use super::native::{Loader, optional_digest, optional_number};
use super::*;
use celetixo_engine::engine::authority::*;
use celetixo_engine::ids::{
    IdempotencyKey, SourceAdmissionId, SourceAttemptId, SourceEvidenceId, TransactionId,
};
use celetixo_engine::persistence::*;
use celetixo_engine::source_execution::{BackendIdentity, ExecutionTier, ResourceLimits};

mod definitions;
mod load;
mod publication;
mod publish;
mod revalidation;

pub(super) use load::load;
pub(super) use publish::publish;
pub(super) const PROFILE: &str = "source-execution-v1";
type SqlResult<T> = Result<T, AuthorityFailure>;

fn principal(row: &Row, column: usize) -> SqlResult<PrincipalId> {
    PrincipalId::new(row.get::<_, String>(column)).map_err(|_| AuthorityFailure::Integrity)
}
fn transaction(row: &Row, column: usize) -> SqlResult<TransactionId> {
    TransactionId::new(row.get::<_, String>(column)).map_err(|_| AuthorityFailure::Integrity)
}
fn source_attempt(row: &Row, column: usize) -> SqlResult<SourceAttemptId> {
    SourceAttemptId::new(row.get::<_, String>(column)).map_err(|_| AuthorityFailure::Integrity)
}
fn generation(value: u64, current: u64) -> SqlResult<()> {
    if value == 0 || value > current {
        Err(AuthorityFailure::Integrity)
    } else {
        Ok(())
    }
}

pub(super) fn access(
    authority: &PostgresAuthority,
    context: &AuthorityAccess,
    evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
) -> SqlResult<()> {
    let administrative = matches!(
        context.mode(),
        AuthorityMode::AdministrativeRead | AuthorityMode::AdministrativeUpdate
    );
    let update = matches!(
        context.mode(),
        AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
    );
    if administrative != matches!(context.actor(), AuthorityActor::EmbeddingAdministrator) {
        return Err(AuthorityFailure::Protocol);
    }
    let login = if administrative {
        &authority.administration
    } else {
        &authority.runtime
    };
    let mut client = login.connect()?;
    schema_lock(&mut client)?;
    authority.checkpoint(AuthorityStage::SchemaLocked)?;
    let mut tx = client
        .build_transaction()
        .isolation_level(if update {
            IsolationLevel::ReadCommitted
        } else {
            IsolationLevel::RepeatableRead
        })
        .start()
        .map_err(database)?;
    validate_schema(&mut tx).map_err(storage)?;
    login.check(&mut tx, Some(context.scope()))?;
    if administrative && update {
        bootstrap(&mut tx, context.scope())?;
    }
    let generation = lock_repository(&mut tx, context.scope(), update)?;
    profile_gate(&mut tx, context.scope(), PROFILE, update)?;

    let before = load(&mut tx, context.scope(), authority.limits)?;

    before
        .check_recovery_limits(authority.limits)
        .map_err(restore)?;
    let mut state =
        AuthorityState::from_source_records(before.clone(), authority.limits).map_err(restore)?;
    let now = database_clock(&mut tx)?;
    if update {
        state.advance_authoritative_clock(now)
    } else {
        state.observe_authoritative_clock(now)
    }
    .map_err(|_| AuthorityFailure::ClockRegression)?;
    authority.checkpoint(AuthorityStage::StateLoaded)?;
    let observed = state.source_records(authority.limits).map_err(restore)?;
    evaluate(&mut state)?;

    let after = state.source_records(authority.limits).map_err(restore)?;
    after
        .check_recovery_limits(authority.limits)
        .map_err(restore)?;
    AuthorityState::from_source_records(after.clone(), authority.limits).map_err(restore)?;
    if update {
        publish(&mut tx, context, login, generation, &before, &after)?;

        if load(&mut tx, context.scope(), authority.limits)? != after {
            return Err(AuthorityFailure::Integrity);
        }

        authority.checkpoint(AuthorityStage::BeforeCommit)?;
        login.check(&mut tx, Some(context.scope()))?;
        final_gate(&mut tx, &before, &after, authority.limits)?;
        tx.commit()
            .map_err(|_| AuthorityFailure::CommitIndeterminate)?;
        authority
            .checkpoint(AuthorityStage::AfterCommit)
            .map_err(|_| AuthorityFailure::CommitIndeterminate)
    } else {
        if after != observed {
            return Err(AuthorityFailure::Protocol);
        }
        tx.commit().map_err(database)
    }
}

fn final_gate(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
    limits: NativeRecordLimits,
) -> SqlResult<()> {
    let now = database_clock(tx)?;
    if now < after.base.clock_ms {
        return Err(AuthorityFailure::ClockRegression);
    }
    native::final_lease_gate(now, &before.base, &after.base)?;
    for (key, record) in &after.base.mutations {
        if before.base.mutations.contains_key(key) {
            continue;
        }
        if let (
            celetixo_engine::engine::MutationRequest::SourceRevalidate(request),
            Ok(celetixo_engine::engine::MutationValue::SourceRevalidated(_)),
        ) = (&record.request, &record.outcome)
        {
            let original =
                AuthorityState::from_source_records(before.clone(), limits).map_err(restore)?;
            original
                .check_source_revalidation_metadata_at(now, &key.actor, request)
                .map_err(|_| AuthorityFailure::Contention)?;
        }
    }
    if after.history.len() > before.history.len() {
        let original =
            AuthorityState::from_source_records(before.clone(), limits).map_err(restore)?;
        for change in after.history.iter().skip(before.history.len()) {
            let (key, record) = after.base.mutations.iter().find(|(_,record)|
                matches!(&record.outcome, Ok(celetixo_engine::engine::MutationValue::SourceCommitted(receipt)) if receipt.as_ref()==&change.receipt)
            ).ok_or(AuthorityFailure::Integrity)?;
            let celetixo_engine::engine::MutationRequest::SourceCommit(request) = &record.request
            else {
                return Err(AuthorityFailure::Integrity);
            };
            let context = record
                .source_commit
                .as_ref()
                .ok_or(AuthorityFailure::Integrity)?;
            let analyst = celetixo_engine::engine::MutationActor::Session {
                principal: context.analysis_principal.clone(),
                grant: context.analysis_grant,
            };
            original
                .check_source_commit_metadata_at(now, &key.actor, &analyst, request)
                .map_err(|_| AuthorityFailure::Contention)?;
        }
    }
    // Newly issued execution/cleanup grants must still be live at publication.
    // Historical allocation replies introduce no grant and cannot renew expiry.
    for (id, grant) in &after.executors {
        if !before.executors.contains_key(id) && grant.expires_at_ms <= now {
            return Err(AuthorityFailure::Contention);
        }
    }
    for (id, grant) in &after.cleanup {
        if !before.cleanup.contains_key(id) && grant.expires_at_ms <= now {
            return Err(AuthorityFailure::Contention);
        }
    }
    if before.policy_head != after.policy_head
        && let Some(head) = after.policy_head
    {
        let policy = after
            .policies
            .get(&head)
            .ok_or(AuthorityFailure::Integrity)?;
        if policy.input().expires_at_ms <= now {
            return Err(AuthorityFailure::Contention);
        }
    }
    for (id, a) in &after.attempts {
        let old = before.attempts.get(id);
        // Exact historical replay is not a new side effect. Any new admission,
        // claim, execution transition or receipt needs live original authority.
        if old == Some(a)
            || a.state == celetixo_engine::engine::SourceAttemptState::Cancelled
            || a.state == celetixo_engine::engine::SourceAttemptState::TimedOut
            || a.state == celetixo_engine::engine::SourceAttemptState::RevalidationRequired
        {
            continue;
        }
        let b = a.admission.binding();
        let transaction = after
            .base
            .transactions
            .get(&b.transaction)
            .ok_or(AuthorityFailure::Integrity)?;
        let lease = transaction
            .lease
            .as_ref()
            .ok_or(AuthorityFailure::Integrity)?;
        let grant = after
            .base
            .grants
            .get(&a.owner_grant)
            .ok_or(AuthorityFailure::Integrity)?;
        if lease.fence != a.admission_fence
            || lease.owner_grant != a.owner_grant
            || lease.expires_at <= now
            || a.admission.expires_at_ms() <= now
            || !grant.active
            || grant.principal != transaction.owner
            || !grant.permissions.contains(&Permission::Execute)
        {
            return Err(AuthorityFailure::Contention);
        }
        if let Some(executor) = a.executor_grant {
            let executor = after
                .executors
                .get(&executor)
                .ok_or(AuthorityFailure::Integrity)?;
            if !executor.active || executor.expires_at_ms <= now {
                return Err(AuthorityFailure::Contention);
            }
        }
        if let Some(evidence) = a.evidence {
            let receipt = after
                .evidence
                .get(&evidence)
                .ok_or(AuthorityFailure::Integrity)?;
            if after
                .artifact_stores
                .get(&receipt.store_registration)
                .is_none_or(|store| !store.active)
            {
                return Err(AuthorityFailure::Contention);
            }
        }
    }
    Ok(())
}
