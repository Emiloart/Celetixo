//! Normalized projections of original Engine revalidation observations/effects.
use super::publication::{Value, execute, one, put};
use super::*;
use celetixo_engine::engine::{
    MutationActor, MutationRequest, MutationValue, SourceRevalidationReceipt,
};

const PARENT: &str = "source_revalidation_mutations";
const LEASES: &str = "source_revalidation_leases";
const SOURCES: &str = "source_revalidation_sources";
const MAIN: &[&str] = &[
    "actor_digest",
    "idempotency_key",
    "request_digest",
    "outcome_kind",
    "observed_head",
    "observed_model_generation",
    "observed_analysis",
    "observed_policy",
    "observed_fence",
    "observed_lease_count",
    "observed_source_count",
    "result_transaction",
    "result_attempt",
    "result_fence",
    "result_owner_grant",
    "result_expiry_ms",
    "result_accesses",
    "result_impact",
    "receipt_digest",
];
const HELD: &[&str] = &[
    "actor_digest",
    "idempotency_key",
    "transaction_id",
    "event_count",
    "grant_active",
    "last_fence",
    "last_event",
];
const WORK: &[&str] = &[
    "actor_digest",
    "idempotency_key",
    "source_attempt_id",
    "event_count",
    "event_sequence",
    "state",
];
type RowKey = (Digest, String, Option<String>);
type Rows = BTreeMap<RowKey, Vec<Value>>;
fn projection(records: &SourceRecords, table: &str) -> SqlResult<Rows> {
    let mut result = BTreeMap::new();
    for (key, record) in &records.base.mutations {
        if !matches!(record.request, MutationRequest::SourceRevalidate(_)) {
            continue;
        }
        let c = record
            .source_revalidation
            .as_ref()
            .ok_or(AuthorityFailure::Integrity)?;
        let actor = key.actor.digest();
        let name = key.key.as_str().to_owned();
        let prefix = || vec![Value::digest(Some(actor)), Value::Text(Some(name.clone()))];
        match table {
            PARENT => {
                let receipt = match &record.outcome {
                    Ok(MutationValue::SourceRevalidated(r)) => Some(r.as_ref()),
                    Err(_) => None,
                    _ => return Err(AuthorityFailure::Integrity),
                };
                let grant = if let MutationActor::Session { grant, .. } = key.actor {
                    Some(grant)
                } else {
                    return Err(AuthorityFailure::Integrity);
                };
                let mut values = prefix();
                values.extend([
                    Value::digest(Some(record.request.digest(&records.base.scope, &key.actor))),
                    Value::Text(Some(
                        if record.outcome.is_ok() {
                            "Accepted"
                        } else {
                            "Rejected"
                        }
                        .into(),
                    )),
                    Value::digest(c.head.map(|id| id.0)),
                    Value::number(c.model_generation),
                    Value::digest(c.analysis),
                    Value::digest(c.policy),
                    Value::number(Some(c.reservation_fence_before)),
                    Value::number(Some(c.leases.len() as u64)),
                    Value::number(Some(c.source_event_counts.len() as u64)),
                    Value::Text(receipt.map(|r| r.transaction.as_str().to_owned())),
                    Value::number(receipt.map(|r| r.attempt)),
                    Value::number(receipt.map(|r| r.fence)),
                    Value::number(receipt.and(grant)),
                    Value::number(receipt.map(|r| r.expires_at_ms)),
                    Value::digest(receipt.map(|r| r.accesses.digest())),
                    Value::digest(receipt.map(|r| r.impact)),
                    Value::digest(receipt.map(SourceRevalidationReceipt::digest)),
                ]);
                result.insert((actor, name, None), values);
            }
            LEASES => {
                for (id, observed) in &c.leases {
                    let tx = records
                        .base
                        .transactions
                        .get(id)
                        .ok_or(AuthorityFailure::Integrity)?;
                    let last = observed
                        .event_count
                        .checked_sub(1)
                        .map(|i| {
                            tx.lease_history
                                .get(usize::try_from(i).map_err(|_| AuthorityFailure::Integrity)?)
                                .ok_or(AuthorityFailure::Integrity)
                        })
                        .transpose()?;
                    let mut values = prefix();
                    values.extend([
                        Value::Text(Some(id.as_str().into())),
                        Value::number(Some(observed.event_count)),
                        Value::Bool(observed.grant_active),
                        Value::number(last.map(|e| e.fence)),
                        Value::number(observed.event_count.checked_sub(1)),
                    ]);
                    result.insert((actor, name.clone(), Some(id.as_str().into())), values);
                }
            }
            SOURCES => {
                for (id, count) in &c.source_event_counts {
                    let source = records
                        .attempts
                        .get(id)
                        .ok_or(AuthorityFailure::Integrity)?;
                    let sequence = count.checked_sub(1).ok_or(AuthorityFailure::Integrity)?;
                    let event = source
                        .events
                        .get(usize::try_from(sequence).map_err(|_| AuthorityFailure::Integrity)?)
                        .ok_or(AuthorityFailure::Integrity)?;
                    let mut values = prefix();
                    values.extend([
                        Value::Text(Some(id.as_str().into())),
                        Value::number(Some(*count)),
                        Value::number(Some(sequence)),
                        Value::Text(Some(event.to.name().into())),
                    ]);
                    result.insert((actor, name.clone(), Some(id.as_str().into())), values);
                }
            }
            _ => return Err(AuthorityFailure::Protocol),
        }
    }
    Ok(result)
}
fn selection(columns: &[&str], values: &[Value]) -> String {
    columns
        .iter()
        .zip(values)
        .map(|(name, value)| {
            format!(
                "{name}::{}",
                match value {
                    Value::Number(_) => "text",
                    _ => value.cast(),
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
pub(super) fn load(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &SourceRecords,
) -> SqlResult<()> {
    for (key, record) in &records.base.mutations {
        if let (
            MutationRequest::SourceRevalidate(request),
            Ok(MutationValue::SourceRevalidated(receipt)),
        ) = (&record.request, &record.outcome)
        {
            let MutationActor::Session { principal, .. } = &key.actor else {
                return Err(AuthorityFailure::Integrity);
            };
            let bytes = loader.payload(tx, receipt.digest(), SourceRevalidationReceipt::FORMAT)?;
            if restore_source_revalidation_receipt(
                loader.scope,
                principal,
                request,
                receipt.digest(),
                &bytes,
                loader.budget.decode_limits(),
            )
            .map_err(restore)?
                != **receipt
            {
                return Err(AuthorityFailure::Integrity);
            }
            let bytes = loader.payload(tx, receipt.accesses.digest(), "celetixo/access-set/v1")?;
            if bytes != receipt.accesses.canonical_bytes() {
                return Err(AuthorityFailure::Integrity);
            }
        }
    }
    for (table, columns) in [(PARENT, MAIN), (LEASES, HELD), (SOURCES, WORK)] {
        let mut expected = projection(records, table)?;
        // Typed projection is defined even for an empty repository.
        let select = match table {
            PARENT => {
                "actor_digest::bytea,idempotency_key::text,request_digest::bytea,outcome_kind::text,observed_head::bytea,observed_model_generation::text,observed_analysis::bytea,observed_policy::bytea,observed_fence::text,observed_lease_count::text,observed_source_count::text,result_transaction::text,result_attempt::text,result_fence::text,result_owner_grant::text,result_expiry_ms::text,result_accesses::bytea,result_impact::bytea,receipt_digest::bytea"
            }
            LEASES => {
                "actor_digest::bytea,idempotency_key::text,transaction_id::text,event_count::text,grant_active::boolean,last_fence::text,last_event::text"
            }
            SOURCES => {
                "actor_digest::bytea,idempotency_key::text,source_attempt_id::text,event_count::text,event_sequence::text,state::text"
            }
            _ => return Err(AuthorityFailure::Protocol),
        };
        if let Some(values) = expected.values().next()
            && selection(columns, values) != select
        {
            return Err(AuthorityFailure::Protocol);
        }
        for row in loader.rows(tx, table, select, "actor_digest,idempotency_key")? {
            let key = (
                digest(&row, 0)?,
                row.get::<_, String>(1),
                (table != PARENT).then(|| row.get::<_, String>(2)),
            );
            let values = expected.remove(&key).ok_or(AuthorityFailure::Integrity)?;
            if values.len() != columns.len()
                || values.iter().enumerate().any(|(i, v)| !v.matches(&row, i))
            {
                return Err(AuthorityFailure::Integrity);
            }
        }
        if !expected.is_empty() {
            return Err(AuthorityFailure::Integrity);
        }
    }
    Ok(())
}
pub(super) fn publish(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    for (key, record) in &after.base.mutations {
        if before.base.mutations.contains_key(key) {
            continue;
        }
        if let Ok(MutationValue::SourceRevalidated(receipt)) = &record.outcome {
            put(
                tx,
                scope,
                receipt.digest(),
                SourceRevalidationReceipt::FORMAT,
                &receipt.canonical_bytes(),
            )?;
            put(
                tx,
                scope,
                receipt.accesses.digest(),
                "celetixo/access-set/v1",
                &receipt.accesses.canonical_bytes(),
            )?;
        }
    }
    for (table, columns) in [(PARENT, MAIN), (LEASES, HELD), (SOURCES, WORK)] {
        let old = projection(before, table)?;
        for (key, values) in projection(after, table)? {
            if old.contains_key(&key) {
                continue;
            }
            let casts = values
                .iter()
                .enumerate()
                .map(|(i, v)| format!("${}::{}", i + 4, v.cast()))
                .collect::<Vec<_>>()
                .join(",");
            let args = values.iter().map(Value::argument).collect::<Vec<_>>();
            one(execute(
                tx,
                scope,
                &format!(
                    "INSERT INTO celetixo.{table} (organization_id,workspace_id,repository_id,{}) VALUES ($1::text,$2::text,$3::text,{casts})",
                    columns.join(",")
                ),
                &args,
            )?)?;
        }
    }
    Ok(())
}
