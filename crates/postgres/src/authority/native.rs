//! Normalized native lifecycle storage. These projections preserve checked
//! Engine facts; they do not decide semantic admission or serialize a Database.
use super::*;
use celetixo_engine::access::AccessSet;
use celetixo_engine::engine::authority::{
    AdmittedEvidenceRecord, AttemptRecord, CommitOperationRecord, CommittedRecord, LeaseRecord,
    TransactionRecord,
};
use celetixo_engine::engine::{AccessProvenance, CommitReceipt, LeaseEventKind};
use celetixo_engine::ids::{
    CheckId, ElementId, EvidenceId, IdempotencyKey, PatchId, TransactionId,
};
use celetixo_engine::persistence::*;
use celetixo_engine::snapshot::Snapshot;
use celetixo_engine::transaction::TransactionState;
use postgres::types::ToSql;

mod load;
mod publish;
pub(super) use load::{load, load_source_base};
pub(super) use publish::{publish, publish_source_base};

/// Recheck time and the original acquisition after publication/readback while
/// retaining the repository lock. Committed state has already released its
/// lease, so authorization must come from the captured pre-transition state.
/// This is the final database validation point, not a WAL-flush time promise.
pub(super) fn final_commit_gate(
    tx: &mut Transaction<'_>,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let now = database_clock(tx)?;
    if now < after.clock_ms {
        return Err(AuthorityFailure::ClockRegression);
    }
    final_lease_gate(now, before, after)?;
    let first = before.repository.as_ref().map_or(0, |r| r.history.len());
    if let Some(repo) = &after.repository {
        for change in repo.history.iter().skip(first) {
            let original = before
                .transactions
                .get(&change.receipt.transaction)
                .ok_or(AuthorityFailure::Integrity)?;
            let lease = original.lease.as_ref().ok_or(AuthorityFailure::Integrity)?;
            let grant = before
                .grants
                .get(&lease.owner_grant)
                .ok_or(AuthorityFailure::Integrity)?;
            if lease.expires_at <= now
                || !grant.active
                || grant.principal != original.owner
                || !grant.permissions.contains(&Permission::Commit)
            {
                return Err(AuthorityFailure::Contention);
            }
            let committed = after
                .transactions
                .get(&change.receipt.transaction)
                .ok_or(AuthorityFailure::Integrity)?;
            if !committed.lease_history.iter().skip(original.lease_history.len()).any(|event|event.fence==lease.fence && event.authorization_grant==lease.owner_grant && matches!(&event.kind,LeaseEventKind::Released { reason } if reason=="committed")) {
                return Err(AuthorityFailure::Integrity);
            }
        }
    }
    Ok(())
}

/// Historical results carry no current authority. Newly published acquisitions
/// and renewals require a live grant and unexpired authority at this final
/// database-time check. In particular, a delayed renewal cannot revive a lease
/// that expired while the callback was running.
pub(super) fn final_lease_gate(
    now: u64,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    for (id, transaction) in &after.transactions {
        let previous_transaction = before.transactions.get(id);
        // Check actual native attempt effects for both entry points. Exact
        // journal replay changes neither a patch nor an attempt and grants no
        // renewed authority. Evidence admission does not require an owner lease.
        let native_effect = transaction.attempts.last().filter(|attempt| {
            attempt.patch.is_some()
                && previous_transaction
                    .and_then(|tx| tx.attempts.iter().find(|old| old.number == attempt.number))
                    .is_none_or(|old| {
                        old.patch != attempt.patch || old.candidate != attempt.candidate
                    })
        });
        let permission = if native_effect.is_some() {
            Some(
                if previous_transaction
                    .is_some_and(|old| old.attempts.len() < transaction.attempts.len())
                {
                    Permission::Analyze
                } else {
                    Permission::Execute
                },
            )
        } else if transaction.state == TransactionState::Executing
            && previous_transaction.is_none_or(|t| t.state != TransactionState::Executing)
        {
            Some(Permission::Execute)
        } else {
            None
        };
        if let Some(permission) = permission {
            let lease = transaction
                .lease
                .as_ref()
                .ok_or(AuthorityFailure::Integrity)?;
            let grant = after
                .grants
                .get(&lease.owner_grant)
                .ok_or(AuthorityFailure::Integrity)?;
            if lease.expires_at <= now
                || !grant.active
                || grant.principal != transaction.owner
                || !grant.permissions.contains(&permission)
            {
                return Err(AuthorityFailure::Contention);
            }
        }
        let previous = before
            .transactions
            .get(id)
            .map_or(0, |t| t.lease_history.len());
        for event in transaction.lease_history.iter().skip(previous) {
            let expiry = match event.kind {
                LeaseEventKind::Acquired { expires_at } => expires_at,
                LeaseEventKind::Renewed {
                    previous_expiry,
                    expires_at,
                } => {
                    if previous_expiry <= now {
                        return Err(AuthorityFailure::Contention);
                    }
                    expires_at
                }
                _ => continue,
            };
            let grant = after
                .grants
                .get(&event.authorization_grant)
                .ok_or(AuthorityFailure::Integrity)?;
            if expiry <= now
                || !grant.active
                || grant.principal != transaction.owner
                || !grant.permissions.contains(&Permission::Reserve)
            {
                return Err(AuthorityFailure::Contention);
            }
        }
    }
    Ok(())
}

pub(super) type SqlResult<T> = Result<T, AuthorityFailure>;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LoadProfile {
    Native,
    Source,
}
const ACCESS_FORMAT: &str = "celetixo/access-set/v1";
const PATCH_FORMAT: &str = "celetixo/patch/v1";
const SNAPSHOT_FORMAT: &str = "celetixo/snapshot/v1";
const EVENT_TYPE: &str = "NativeCommitRecorded";

pub(super) struct Budget {
    pub(super) limits: NativeRecordLimits,
    remaining_entries: usize,
    remaining_bytes: usize,
}
impl Budget {
    fn new(limits: NativeRecordLimits) -> Self {
        Self {
            limits,
            remaining_entries: limits.entries,
            remaining_bytes: limits.total_value_bytes,
        }
    }
    pub(super) fn entries(&mut self, count: usize) -> SqlResult<()> {
        self.remaining_entries = self
            .remaining_entries
            .checked_sub(count)
            .ok_or(AuthorityFailure::Integrity)?;
        Ok(())
    }
    pub(super) fn value(&mut self, count: usize) -> SqlResult<()> {
        if count > self.limits.value_bytes {
            return Err(AuthorityFailure::Integrity);
        }
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(count)
            .ok_or(AuthorityFailure::Integrity)?;
        Ok(())
    }
    fn metadata(&mut self, entries: usize, bytes: usize, largest_row: usize) -> SqlResult<()> {
        self.entries(entries)?;
        if largest_row > self.limits.value_bytes {
            return Err(AuthorityFailure::Integrity);
        }
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(bytes)
            .ok_or(AuthorityFailure::Integrity)?;
        Ok(())
    }
    pub(super) fn decode_limits(&self) -> DomainDecodeLimits {
        DomainDecodeLimits {
            encoded_bytes: self.limits.value_bytes,
            field_bytes: self.limits.value_bytes,
            collection_entries: self.remaining_entries,
        }
    }
    fn access(&mut self, access: &AccessSet) -> SqlResult<()> {
        self.entries(
            access.reads.len()
                + access.writes.len()
                + access.invariant_scopes.len()
                + access.predicates.len(),
        )
    }
}

pub(super) struct Loader<'a> {
    pub(super) scope: &'a Scope,
    pub(super) budget: Budget,
    snapshots: BTreeMap<SnapshotId, Arc<Snapshot>>,
}
impl<'a> Loader<'a> {
    pub(super) fn new(scope: &'a Scope, limits: NativeRecordLimits) -> Self {
        Self {
            scope,
            budget: Budget::new(limits),
            snapshots: BTreeMap::new(),
        }
    }
    /// The SQL LIMIT is applied before materializing metadata. Every result set
    /// consumes one common budget, including auxiliary rows absent from records.
    pub(super) fn rows(
        &mut self,
        tx: &mut Transaction<'_>,
        table: &str,
        columns: &str,
        order: &str,
    ) -> SqlResult<Vec<Row>> {
        let a = snapshots::scope_parameters(self.scope);
        let bound = i64::try_from(self.budget.remaining_entries)
            .map_err(|_| AuthorityFailure::Integrity)?;
        // Qualify underlying columns: output aliases such as revision::text
        // otherwise make ORDER BY revision lexical (1,10,2), not numeric.
        let order = order
            .split(',')
            .map(|column| format!("fact.{column}"))
            .collect::<Vec<_>>()
            .join(",");
        // Projections are trusted simple SQL expressions without embedded
        // commas. Measure uncompressed projected values on the server before
        // fetching a Vec<Row>, for both inline and TOASTed values.
        let row_cost = columns
            .split(',')
            .map(|column| format!("COALESCE(octet_length(({column})::text),0)::bigint+16"))
            .collect::<Vec<_>>()
            .join("+");
        let measured = tx.query_one(&format!(
            "SELECT count(*)::text,COALESCE(sum(row_cost),0)::text,COALESCE(max(row_cost),0)::text FROM \
             (SELECT ({row_cost}) AS row_cost FROM celetixo.{table} AS fact WHERE {FILTER} ORDER BY {order} LIMIT $4::bigint+1) AS bounded"
        ), &[&a[0],&a[1],&a[2],&bound]).map_err(database)?;
        let count =
            usize::try_from(number(&measured, 0)?).map_err(|_| AuthorityFailure::Integrity)?;
        self.budget.metadata(
            count,
            usize::try_from(number(&measured, 1)?).map_err(|_| AuthorityFailure::Integrity)?,
            usize::try_from(number(&measured, 2)?).map_err(|_| AuthorityFailure::Integrity)?,
        )?;
        let rows=tx.query(&format!("SELECT {columns} FROM celetixo.{table} AS fact WHERE {FILTER} ORDER BY {order} LIMIT $4::bigint+1"), &[&a[0],&a[1],&a[2],&bound]).map_err(database)?;
        // All authoritative writers retain the repository lock across both
        // queries. A differing row set is never silently accepted.
        if rows.len() != count {
            return Err(AuthorityFailure::Integrity);
        }
        Ok(rows)
    }
    pub(super) fn payload(
        &mut self,
        tx: &mut Transaction<'_>,
        id: Digest,
        format: &str,
    ) -> SqlResult<Vec<u8>> {
        let a = snapshots::scope_parameters(self.scope);
        let row=tx.query_opt(&format!("SELECT payload_format::text,payload_version,octet_length(payload_bytes) FROM celetixo.domain_payloads WHERE {FILTER} AND payload_digest=$4::bytea"), &[&a[0],&a[1],&a[2],&&id.as_bytes()[..]]).map_err(database)?.ok_or(AuthorityFailure::Integrity)?;
        if row.get::<_, String>(0) != format || row.get::<_, i32>(1) != 1 {
            return Err(AuthorityFailure::Integrity);
        }
        self.budget.value(
            usize::try_from(row.get::<_, i32>(2)).map_err(|_| AuthorityFailure::Integrity)?,
        )?;
        let row=tx.query_one(&format!("SELECT payload_bytes FROM celetixo.domain_payloads WHERE {FILTER} AND payload_digest=$4::bytea"), &[&a[0],&a[1],&a[2],&&id.as_bytes()[..]]).map_err(database)?;
        let bytes: Vec<u8> = row.get(0);
        if Digest::of(format, &bytes) != id {
            return Err(AuthorityFailure::Integrity);
        }
        Ok(bytes)
    }
    pub(super) fn snapshot(
        &mut self,
        tx: &mut Transaction<'_>,
        id: SnapshotId,
    ) -> SqlResult<Arc<Snapshot>> {
        if let Some(snapshot) = self.snapshots.get(&id) {
            return Ok(snapshot.clone());
        }
        let a = snapshots::scope_parameters(self.scope);
        let row=tx.query_opt(&format!("SELECT revision::text,manifest_digest::bytea,payload_digest::bytea FROM celetixo.snapshots WHERE {FILTER} AND snapshot_digest=$4::bytea"), &[&a[0],&a[1],&a[2],&&id.0.as_bytes()[..]]).map_err(database)?.ok_or(AuthorityFailure::Integrity)?;
        if digest(&row, 2)? != id.0 {
            return Err(AuthorityFailure::Integrity);
        }
        let bytes = self.payload(tx, id.0, SNAPSHOT_FORMAT)?;
        let snapshot =
            restore_snapshot(id, &bytes, self.budget.decode_limits()).map_err(restore)?;
        if snapshot.scope() != self.scope
            || snapshot.revision().0 != number(&row, 0)?
            || snapshot.manifest().digest() != digest(&row, 1)?
        {
            return Err(AuthorityFailure::Integrity);
        }
        self.budget.entries(
            1 + snapshot.state().len()
                + snapshot.manifest().inputs.len()
                + snapshot.manifest().environment.len()
                + snapshot.manifest().inclusion_policy.captured_layers.len()
                + snapshot.manifest().inclusion_policy.excluded_paths.len(),
        )?;
        let snapshot = Arc::new(snapshot);
        self.snapshots.insert(id, snapshot.clone());
        Ok(snapshot)
    }
    pub(super) fn owned_snapshot(
        &mut self,
        tx: &mut Transaction<'_>,
        id: SnapshotId,
    ) -> SqlResult<Snapshot> {
        let snapshot = self.snapshot(tx, id)?;
        // Charge an owned copy before cloning cached content into normalized
        // records. A repeated candidate cannot amplify allocations unboundedly.
        self.budget.value(snapshot.canonical_bytes().len())?;
        self.budget.entries(snapshot.state().len())?;
        Ok((*snapshot).clone())
    }
    fn model(
        &mut self,
        tx: &mut Transaction<'_>,
        snapshot: &Snapshot,
        generation: u64,
        id: Digest,
    ) -> SqlResult<celetixo_engine::model::EngineeringModel> {
        let bytes = self.payload(tx, id, EngineeringModelRecord::FORMAT)?;
        let record = restore_model_record(
            snapshot,
            generation,
            id,
            &bytes,
            self.budget.decode_limits(),
        )
        .map_err(restore)?;
        let model = record.model();
        self.budget.entries(
            model.dependencies.len()
                + model
                    .dependencies
                    .values()
                    .map(BTreeSet::len)
                    .sum::<usize>(),
        )?;
        if let celetixo_engine::access::Completeness::Incomplete { reasons } = &model.completeness {
            self.budget.entries(reasons.len())?;
        }
        Ok(model.clone())
    }
}

pub(super) fn optional_digest(row: &Row, column: usize) -> SqlResult<Option<Digest>> {
    row.get::<_, Option<Vec<u8>>>(column)
        .map(|bytes| {
            bytes
                .try_into()
                .map(Digest::from_bytes)
                .map_err(|_| AuthorityFailure::Integrity)
        })
        .transpose()
}
pub(super) fn optional_number(row: &Row, column: usize) -> SqlResult<Option<u64>> {
    row.get::<_, Option<String>>(column)
        .map(|text| {
            let value = text
                .parse::<u64>()
                .map_err(|_| AuthorityFailure::Integrity)?;
            if value.to_string() != text {
                return Err(AuthorityFailure::Integrity);
            }
            Ok(value)
        })
        .transpose()
}
fn transaction_id(row: &Row, column: usize) -> SqlResult<TransactionId> {
    TransactionId::new(row.get::<_, String>(column)).map_err(|_| AuthorityFailure::Integrity)
}
pub(super) fn state_name(state: TransactionState) -> &'static str {
    match state {
        TransactionState::Proposed => "Proposed",
        TransactionState::Analyzed => "Analyzed",
        TransactionState::Reserved => "Reserved",
        TransactionState::Executing => "Executing",
        TransactionState::Validating => "Validating",
        TransactionState::RevalidationRequired => "RevalidationRequired",
        TransactionState::Committed => "Committed",
        TransactionState::Aborted => "Aborted",
    }
}
fn state(text: &str) -> SqlResult<TransactionState> {
    [
        TransactionState::Proposed,
        TransactionState::Analyzed,
        TransactionState::Reserved,
        TransactionState::Executing,
        TransactionState::Validating,
        TransactionState::RevalidationRequired,
        TransactionState::Committed,
        TransactionState::Aborted,
    ]
    .into_iter()
    .find(|value| state_name(*value) == text)
    .ok_or(AuthorityFailure::UnsupportedProfile)
}
fn event_name(kind: &LeaseEventKind) -> &'static str {
    match kind {
        LeaseEventKind::Acquired { .. } => "Acquired",
        LeaseEventKind::Renewed { .. } => "Renewed",
        LeaseEventKind::Expired => "Expired",
        LeaseEventKind::Revoked => "Revoked",
        LeaseEventKind::Released { .. } => "Released",
    }
}
fn provenance(tx: &TransactionRecord, attempt: &AttemptRecord) -> SqlResult<AccessProvenance> {
    let analysis = attempt
        .analysis
        .as_ref()
        .ok_or(AuthorityFailure::Integrity)?;
    Ok(AccessProvenance {
        explicit: tx.intent.accesses.clone(),
        derived: analysis.derived().clone(),
        invariant_scopes: attempt
            .effective
            .as_ref()
            .ok_or(AuthorityFailure::Integrity)?
            .invariant_scopes
            .clone(),
        analysis: analysis.stamp().clone(),
        completeness: analysis.completeness().clone(),
    })
}
struct PayloadFact {
    kind: &'static str,
    id: Digest,
    format: &'static str,
    bytes: Vec<u8>,
}
fn access_facts(
    scope: &Scope,
    id: &TransactionId,
    tx: &TransactionRecord,
    attempt: &AttemptRecord,
    base: &Snapshot,
) -> SqlResult<Vec<PayloadFact>> {
    if scope != base.scope() {
        return Err(AuthorityFailure::Integrity);
    }
    let explicit = &tx.intent.accesses;
    let mut facts = vec![PayloadFact {
        kind: "explicit",
        id: explicit.digest(),
        format: ACCESS_FORMAT,
        bytes: explicit.canonical_bytes(),
    }];
    if let Some(analysis) = &attempt.analysis {
        let effective = attempt
            .effective
            .as_ref()
            .ok_or(AuthorityFailure::Integrity)?;
        for (kind, access) in [
            ("derived", analysis.derived().clone()),
            ("effective", effective.clone()),
            (
                "invariant",
                AccessSet {
                    invariant_scopes: effective.invariant_scopes.clone(),
                    ..Default::default()
                },
            ),
            (
                "predicate",
                AccessSet {
                    predicates: effective.predicates.clone(),
                    ..Default::default()
                },
            ),
        ] {
            facts.push(PayloadFact {
                kind,
                id: access.digest(),
                format: ACCESS_FORMAT,
                bytes: access.canonical_bytes(),
            });
        }
        let record =
            AccessProvenanceRecord::new(base, id.clone(), attempt.number, provenance(tx, attempt)?)
                .map_err(restore)?;
        facts.push(PayloadFact {
            kind: "analysis",
            id: record.digest(),
            format: AccessProvenanceRecord::FORMAT,
            bytes: record.canonical_bytes(),
        });
    } else if attempt.effective.is_some() {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(facts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReservationFact {
    transaction: TransactionId,
    attempt: u64,
    owner_grant: u64,
    acquired_at: u64,
    expires_at: u64,
    state: &'static str,
    writes: BTreeSet<ElementId>,
}
fn reservations(records: &NativeRecords) -> SqlResult<BTreeMap<u64, ReservationFact>> {
    let mut result = BTreeMap::new();
    for (id, tx) in &records.transactions {
        for event in &tx.lease_history {
            match &event.kind {
                LeaseEventKind::Acquired { expires_at } => {
                    if result
                        .insert(
                            event.fence,
                            ReservationFact {
                                transaction: id.clone(),
                                attempt: event.acquisition_attempt,
                                owner_grant: event.authorization_grant,
                                acquired_at: event.at,
                                expires_at: *expires_at,
                                state: "Active",
                                writes: event.scopes.clone(),
                            },
                        )
                        .is_some()
                    {
                        return Err(AuthorityFailure::Integrity);
                    }
                }
                kind => {
                    let row = result
                        .get_mut(&event.fence)
                        .ok_or(AuthorityFailure::Integrity)?;
                    match kind {
                        LeaseEventKind::Renewed { expires_at, .. } => row.expires_at = *expires_at,
                        LeaseEventKind::Expired => row.state = "Expired",
                        LeaseEventKind::Revoked => row.state = "Revoked",
                        LeaseEventKind::Released { .. } => row.state = "Released",
                        LeaseEventKind::Acquired { .. } => return Err(AuthorityFailure::Integrity),
                    }
                }
            }
        }
    }
    Ok(result)
}
fn claims(records: &NativeRecords) -> SqlResult<BTreeMap<ElementId, u64>> {
    let mut result = BTreeMap::new();
    for tx in records.transactions.values() {
        if let Some(lease) = &tx.lease
            && lease.expires_at > records.clock_ms
            && records
                .grants
                .get(&lease.owner_grant)
                .is_some_and(|grant| grant.active)
        {
            for element in &lease.writes {
                if result.insert(element.clone(), lease.fence).is_some() {
                    return Err(AuthorityFailure::Integrity);
                }
            }
        }
    }
    Ok(result)
}
fn accepted(scope: &Scope, receipt: &CommitReceipt) -> SqlResult<CommitOutcomeRecord> {
    CommitOutcomeRecord::new(
        scope.clone(),
        receipt.transaction.clone(),
        receipt.attempt,
        Ok(receipt.clone()),
    )
    .map_err(restore)
}
fn event_id(outcome: &CommitOutcomeRecord) -> Digest {
    Digest::of(
        "celetixo/native-commit-event/v1",
        &outcome.canonical_bytes(),
    )
}
fn reason(
    scope: &Scope,
    id: &TransactionId,
    tx: &TransactionRecord,
) -> SqlResult<Option<TransactionReasonRecord>> {
    tx.last_reason
        .as_ref()
        .map(|text| {
            TransactionReasonRecord::new(
                scope.clone(),
                id.clone(),
                tx.attempts
                    .last()
                    .ok_or(AuthorityFailure::Integrity)?
                    .number,
                text.clone(),
            )
            .map_err(restore)
        })
        .transpose()
}
