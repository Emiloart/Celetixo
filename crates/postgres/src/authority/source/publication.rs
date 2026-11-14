//! SQL projections of Engine source publication decisions; no admission logic.
use super::*;
use celetixo_engine::engine::{
    MutationActor, MutationKey, MutationRecord, MutationRequest, MutationValue, SourceCommitReceipt,
};
use postgres::types::ToSql;

mod history;

const COLUMNS: &[&str] = &[
    "actor_digest",
    "idempotency_key",
    "request_digest",
    "analysis_principal",
    "analysis_grant",
    "observed_head",
    "observed_model_generation",
    "observed_analysis",
    "observed_policy",
    "source_attempt_id",
    "admission_digest",
    "source_state",
    "source_event_count",
    "source_event_sequence",
    "evidence_digest",
    "executor_grant",
    "executor_active",
    "store_registration",
    "store_active",
    "artifact_evidence",
    "artifact_registration",
    "artifact_provenance",
    "artifact_observed_at_ms",
    "receipt_digest",
];

/// Only SQL representation lives here. Every value is compared with the
/// independently decoded, Engine-checked original journal record on load.
enum Value {
    Text(Option<String>),
    Number(Option<String>),
    Bytes(Option<Vec<u8>>),
    Bool(Option<bool>),
}
impl Value {
    fn number(value: Option<u64>) -> Self {
        Self::Number(value.map(|v| v.to_string()))
    }
    fn digest(value: Option<Digest>) -> Self {
        Self::Bytes(value.map(|v| v.as_bytes().to_vec()))
    }
    fn cast(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Number(_) => "text::numeric",
            Self::Bytes(_) => "bytea",
            Self::Bool(_) => "boolean",
        }
    }
    fn argument(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(v) | Self::Number(v) => v,
            Self::Bytes(v) => v,
            Self::Bool(v) => v,
        }
    }
    fn matches(&self, row: &Row, column: usize) -> bool {
        match self {
            Self::Text(v) | Self::Number(v) => *v == row.get::<_, Option<String>>(column),
            Self::Bytes(v) => *v == row.get::<_, Option<Vec<u8>>>(column),
            Self::Bool(v) => *v == row.get::<_, Option<bool>>(column),
        }
    }
}
fn columns(scope: &Scope, key: &MutationKey, record: &MutationRecord) -> SqlResult<Vec<Value>> {
    let MutationRequest::SourceCommit(_) = &record.request else {
        return Err(AuthorityFailure::Integrity);
    };
    let context = record
        .source_commit
        .as_ref()
        .ok_or(AuthorityFailure::Integrity)?;
    let source = context.source.as_ref();
    let artifacts = context.artifacts.as_ref();
    Ok(vec![
        Value::digest(Some(key.actor.digest())),
        Value::Text(Some(key.key.as_str().into())),
        Value::digest(Some(record.request.digest(scope, &key.actor))),
        Value::Text(Some(context.analysis_principal.as_str().into())),
        Value::number(Some(context.analysis_grant)),
        Value::digest(context.head.map(|id| id.0)),
        Value::number(context.model_generation),
        Value::digest(context.analysis),
        Value::digest(context.policy),
        Value::Text(source.map(|s| s.source_attempt.as_str().into())),
        Value::digest(source.map(|s| s.admission.0)),
        Value::Text(source.map(|s| s.state.name().into())),
        Value::number(source.map(|s| s.event_count)),
        Value::number(source.and_then(|s| s.event_count.checked_sub(1))),
        Value::digest(source.and_then(|s| s.evidence.map(|id| id.0))),
        Value::number(source.and_then(|s| s.executor_grant)),
        Value::Bool(source.and_then(|s| s.executor_active)),
        Value::number(source.and_then(|s| s.store_registration)),
        Value::Bool(source.and_then(|s| s.store_active)),
        Value::digest(artifacts.map(|a| a.evidence.0)),
        Value::number(artifacts.map(|a| a.registration)),
        Value::digest(artifacts.map(|a| a.provenance)),
        Value::number(artifacts.map(|a| a.observed_at_ms)),
        Value::digest(match &record.outcome {
            Ok(MutationValue::SourceCommitted(r)) => Some(r.digest()),
            _ => None,
        }),
    ])
}

pub(super) fn load(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    let mut expected = BTreeMap::new();
    for (key, record) in &records.base.mutations {
        if matches!(record.request, MutationRequest::SourceCommit(_)) {
            expected.insert(
                (key.actor.digest(), key.key.as_str().to_string()),
                columns(loader.scope, key, record)?,
            );
        }
    }
    // The shape is fixed independently of whether this repository has any rows.
    let kinds = [
        "bytea", "text", "bytea", "text", "text", "bytea", "text", "bytea", "bytea", "text",
        "bytea", "text", "text", "text", "bytea", "text", "boolean", "text", "boolean", "bytea",
        "text", "bytea", "text", "bytea",
    ];
    let select = COLUMNS
        .iter()
        .zip(kinds)
        .map(|(name, kind)| format!("{name}::{kind}"))
        .collect::<Vec<_>>()
        .join(",");
    for row in loader.rows(
        tx,
        "source_commit_mutations",
        &select,
        "actor_digest,idempotency_key",
    )? {
        let values = expected
            .remove(&(digest(&row, 0)?, row.get::<_, String>(1)))
            .ok_or(AuthorityFailure::Integrity)?;
        if values.len() != COLUMNS.len()
            || values.iter().enumerate().any(|(i, v)| !v.matches(&row, i))
        {
            return Err(AuthorityFailure::Integrity);
        }
    }
    if !expected.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    history::load(tx, loader, records)
}
pub(super) fn publish(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    for (key, record) in &after.base.mutations {
        if before.base.mutations.contains_key(key)
            || !matches!(record.request, MutationRequest::SourceCommit(_))
        {
            continue;
        }
        let values = columns(scope, key, record)?;
        if let Ok(MutationValue::SourceCommitted(receipt)) = &record.outcome {
            put(
                tx,
                scope,
                receipt.digest(),
                SourceCommitReceipt::FORMAT,
                &receipt.canonical_bytes(),
            )?;
        }
        let casts = values
            .iter()
            .enumerate()
            .map(|(i, value)| format!("${}::{}", i + 4, value.cast()))
            .collect::<Vec<_>>()
            .join(",");
        let args = values.iter().map(Value::argument).collect::<Vec<_>>();
        one(execute(
            tx,
            scope,
            &format!(
                "INSERT INTO celetixo.source_commit_mutations (organization_id,workspace_id,repository_id,{}) VALUES ($1::text,$2::text,$3::text,{casts})",
                COLUMNS.join(",")
            ),
            &args,
        )?)?;
    }
    history::publish(tx, before, after)
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
