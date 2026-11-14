use super::*;
use postgres::types::ToSql;

mod definitions;
mod execution;

pub(in crate::authority) fn publish(
    tx: &mut Transaction<'_>,
    context: &AuthorityAccess,
    login: &Login,
    generation: u64,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    preserve(before, after)?;
    let next = generation
        .checked_add(1)
        .ok_or(AuthorityFailure::CounterExhausted)?;
    publish_session_grants(tx, context, next, &before.base, &after.base)?;
    definitions::grants(tx, context, next, before, after)?;
    native::publish_source_base(tx, next, &before.base, &after.base)?;
    definitions::catalogs(tx, before, after)?;
    definitions::heads(tx, next, before, after)?;
    execution::publish(tx, before, after)?;
    super::super::mutations::publish(tx, &before.base, &after.base)?;
    super::publication::publish(tx, before, after)?;
    super::revalidation::publish(tx, before, after)?;
    publish_envelope(tx, context, login, generation, next, &after.base, true)?;
    super::super::events::publish_source(tx, next, before, after)?;
    one(execute(
        tx,
        &after.base.scope,
        &format!(
            "UPDATE celetixo.engine_counters SET next_executor_grant=$4::text::numeric,next_cleanup_grant=$5::text::numeric,next_artifact_store=$6::text::numeric WHERE {FILTER}"
        ),
        &[
            &after.executor_counter.to_string(),
            &after.cleanup_counter.to_string(),
            &after.artifact_counter.to_string(),
        ],
    )?)
}
fn execute(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    sql: &str,
    args: &[&(dyn ToSql + Sync)],
) -> SqlResult<u64> {
    let a = snapshots::scope_parameters(scope);
    let mut parameters: Vec<&(dyn ToSql + Sync)> = vec![&a[0], &a[1], &a[2]];
    parameters.extend_from_slice(args);
    tx.execute(sql, &parameters).map_err(database)
}
fn put(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    id: Digest,
    format: &str,
    bytes: &[u8],
) -> SqlResult<()> {
    semantic::put_payload(tx, scope, id, format, bytes).map_err(storage)
}
fn one(count: u64) -> SqlResult<()> {
    if count == 1 {
        Ok(())
    } else {
        Err(AuthorityFailure::Contention)
    }
}
fn immutable<K: Ord, V: PartialEq>(
    before: &BTreeMap<K, V>,
    after: &BTreeMap<K, V>,
) -> SqlResult<()> {
    if before.iter().any(|(k, v)| after.get(k) != Some(v)) {
        Err(AuthorityFailure::Protocol)
    } else {
        Ok(())
    }
}
fn preserve(before: &SourceRecords, after: &SourceRecords) -> SqlResult<()> {
    if before.base.scope != after.base.scope
        || !after.history.starts_with(&before.history)
        || before.executor_counter > after.executor_counter
        || before.cleanup_counter > after.cleanup_counter
        || before.artifact_counter > after.artifact_counter
    {
        return Err(AuthorityFailure::Protocol);
    }
    immutable(&before.analyses, &after.analyses)?;
    immutable(&before.policies, &after.policies)?;
    immutable(&before.operations, &after.operations)?;
    immutable(&before.rejections, &after.rejections)?;
    immutable(&before.evidence, &after.evidence)?;
    immutable(&before.unprepared_commits, &after.unprepared_commits)?;
    for (key, old) in &before.attempt_contexts {
        let new = after
            .attempt_contexts
            .get(key)
            .ok_or(AuthorityFailure::Protocol)?;
        if old.analysis != new.analysis || old.policy.is_some_and(|id| new.policy != Some(id)) {
            return Err(AuthorityFailure::Protocol);
        }
    }
    for (id, old) in &before.attempts {
        let new = after.attempts.get(id).ok_or(AuthorityFailure::Protocol)?;
        if old.admission != new.admission
            || old.patch != new.patch
            || old.options != new.options
            || old.admission_fence != new.admission_fence
            || old.admission_lease_event_count != new.admission_lease_event_count
            || old.owner_grant != new.owner_grant
            || !new.events.starts_with(&old.events)
            || old
                .executor_grant
                .is_some_and(|id| new.executor_grant != Some(id))
            || old.evidence.is_some_and(|id| new.evidence != Some(id))
            || (old.state.is_terminal() && old != new)
        {
            return Err(AuthorityFailure::Protocol);
        }
    }
    Ok(())
}
