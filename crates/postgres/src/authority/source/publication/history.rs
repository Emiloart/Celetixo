//! Typed committed source history, terminal outcomes and transactional outbox.
use super::*;
use celetixo_engine::transaction::TransactionState;

const EVENT_TYPE: &str = "SourceCommitRecorded";
fn event_id(receipt: &SourceCommitReceipt) -> Digest {
    Digest::of(
        "celetixo/source-commit-event/v1",
        &receipt.canonical_bytes(),
    )
}

pub(super) fn load(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    for row in loader.rows(tx, "source_committed_changes", "revision::text,transaction_id::text,attempt_number::text,snapshot_digest::bytea,verified_snapshot_digest::bytea,source_attempt_id::text,admission_digest::bytea,source_patch_digest::bytea,evidence_digest::bytea,analysis_digest::bytea,model_generation::text,committed_at_ms::text,actor_digest::bytea,idempotency_key::text,receipt_digest::bytea,accesses_digest::bytea,provenance_digest::bytea", "revision")? {
        let actor = digest(&row,12)?;
        let key = row.get::<_,String>(13);
        let (mutation_key, record) = records.base.mutations.iter().find(|(k,_)| k.actor.digest()==actor && k.key.as_str()==key).ok_or(AuthorityFailure::Integrity)?;
        let MutationActor::Session {principal,..} = &mutation_key.actor else {return Err(AuthorityFailure::Integrity)};
        let MutationRequest::SourceCommit(request) = &record.request else {return Err(AuthorityFailure::Integrity)};
        let Ok(MutationValue::SourceCommitted(expected)) = &record.outcome else {return Err(AuthorityFailure::Integrity)};
        let receipt_id=digest(&row,14)?;
        let bytes=loader.payload(tx,receipt_id,SourceCommitReceipt::FORMAT)?;
        let receipt=restore_source_commit_receipt(loader.scope,principal,request,receipt_id,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        if receipt != **expected || receipt.revision.0 != number(&row,0)? || receipt.transaction != transaction(&row,1)?
            || receipt.attempt != number(&row,2)? || receipt.publication_snapshot.0 != digest(&row,3)?
            || receipt.verified_snapshot.0 != digest(&row,4)? || receipt.source_attempt != source_attempt(&row,5)?
            || receipt.admission.0 != digest(&row,6)? || receipt.patch.0 != digest(&row,7)?
            || receipt.evidence.0 != digest(&row,8)? || receipt.analysis != digest(&row,9)?
            || receipt.model_generation != number(&row,10)? || receipt.committed_at_ms != number(&row,11)? {
            return Err(AuthorityFailure::Integrity);
        }
        let attempt=records.base.transactions.get(&receipt.transaction).and_then(|t|t.attempts.get(usize::try_from(receipt.attempt).ok()?)).ok_or(AuthorityFailure::Integrity)?;
        let accesses=attempt.effective.clone().ok_or(AuthorityFailure::Integrity)?;
        let accesses_id=digest(&row,15)?;
        if accesses_id != accesses.digest() || loader.payload(tx,accesses_id,"celetixo/access-set/v1")? != accesses.canonical_bytes() {return Err(AuthorityFailure::Integrity);}
        let base=loader.snapshot(tx,attempt.base)?;
        let provenance_id=digest(&row,16)?;
        let bytes=loader.payload(tx,provenance_id,AccessProvenanceRecord::FORMAT)?;
        let provenance=restore_access_provenance(&base,&receipt.transaction,receipt.attempt,provenance_id,&bytes,loader.budget.decode_limits()).map_err(restore)?.provenance().clone();
        let snapshot=loader.owned_snapshot(tx,receipt.publication_snapshot)?;
        if records.base.repository.as_mut().ok_or(AuthorityFailure::Integrity)?.registered.insert(snapshot.id(),snapshot.clone()).is_some_and(|old|old!=snapshot) {return Err(AuthorityFailure::Integrity);}
        records.history.push(SourceCommittedRecord {receipt,accesses,provenance});
    }
    terminal_and_outbox(tx, loader, records)
}

pub(super) fn publish(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    if !after.history.starts_with(&before.history) {
        return Err(AuthorityFailure::Protocol);
    }
    let scope = &after.base.scope;
    for change in after.history.iter().skip(before.history.len()) {
        let r = &change.receipt;
        let (key,_) = after.base.mutations.iter().find(|(_,m)|matches!(&m.outcome,Ok(MutationValue::SourceCommitted(result)) if result.as_ref()==r)).ok_or(AuthorityFailure::Integrity)?;
        let repo = after
            .base
            .repository
            .as_ref()
            .ok_or(AuthorityFailure::Integrity)?;
        let base = repo
            .registered
            .get(&change.provenance.analysis.snapshot)
            .ok_or(AuthorityFailure::Integrity)?;
        let provenance = AccessProvenanceRecord::new(
            base,
            r.transaction.clone(),
            r.attempt,
            change.provenance.clone(),
        )
        .map_err(restore)?;
        put(
            tx,
            scope,
            provenance.digest(),
            AccessProvenanceRecord::FORMAT,
            &provenance.canonical_bytes(),
        )?;
        put(
            tx,
            scope,
            change.accesses.digest(),
            "celetixo/access-set/v1",
            &change.accesses.canonical_bytes(),
        )?;
        one(execute(
            tx,
            scope,
            "INSERT INTO celetixo.source_committed_changes (organization_id,workspace_id,repository_id,revision,transaction_id,attempt_number,snapshot_digest,verified_snapshot_digest,source_attempt_id,admission_digest,source_patch_digest,evidence_digest,analysis_digest,model_generation,committed_at_ms,actor_digest,idempotency_key,receipt_digest,accesses_digest,provenance_digest) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text,$6::text::numeric,$7::bytea,$8::bytea,$9::text,$10::bytea,$11::bytea,$12::bytea,$13::bytea,$14::text::numeric,$15::text::numeric,$16::bytea,$17::text,$18::bytea,$19::bytea,$20::bytea)",
            &[
                &r.revision.0.to_string(),
                &r.transaction.as_str(),
                &r.attempt.to_string(),
                &&r.publication_snapshot.0.as_bytes()[..],
                &&r.verified_snapshot.0.as_bytes()[..],
                &r.source_attempt.as_str(),
                &&r.admission.0.as_bytes()[..],
                &&r.patch.0.as_bytes()[..],
                &&r.evidence.0.as_bytes()[..],
                &&r.analysis.as_bytes()[..],
                &r.model_generation.to_string(),
                &r.committed_at_ms.to_string(),
                &&key.actor.digest().as_bytes()[..],
                &key.key.as_str(),
                &&r.digest().as_bytes()[..],
                &&change.accesses.digest().as_bytes()[..],
                &&provenance.digest().as_bytes()[..],
            ],
        )?)?;
        one(execute(
            tx,
            scope,
            "INSERT INTO celetixo.transaction_outcomes (organization_id,workspace_id,repository_id,transaction_id,attempt_number,terminal_state,outcome_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,'Committed',$6::bytea)",
            &[
                &r.transaction.as_str(),
                &r.attempt.to_string(),
                &&r.digest().as_bytes()[..],
            ],
        )?)?;
        let event = event_id(r);
        one(execute(
            tx,
            scope,
            "INSERT INTO celetixo.outbox (organization_id,workspace_id,repository_id,event_id,aggregate_kind,aggregate_id,aggregate_sequence,event_type,schema_version,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,'Repository',$5::text,$6::text::numeric,$7::text,1,$8::bytea)",
            &[
                &&event.as_bytes()[..],
                &scope.repository.as_str(),
                &r.revision.0.to_string(),
                &EVENT_TYPE,
                &&r.digest().as_bytes()[..],
            ],
        )?)?;
        one(execute(
            tx,
            scope,
            "INSERT INTO celetixo.outbox_delivery (organization_id,workspace_id,repository_id,event_id) VALUES ($1::text,$2::text,$3::text,$4::bytea)",
            &[&&event.as_bytes()[..]],
        )?)?;
    }
    Ok(())
}

fn terminal_and_outbox(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &SourceRecords,
) -> SqlResult<()> {
    let mut expected = BTreeMap::new();
    for (id, transaction) in &records.base.transactions {
        let fact = match transaction.state {
            TransactionState::Committed => {
                let r = &records
                    .history
                    .iter()
                    .find(|c| c.receipt.transaction == *id)
                    .ok_or(AuthorityFailure::Integrity)?
                    .receipt;
                Some((
                    r.attempt,
                    "Committed",
                    r.digest(),
                    SourceCommitReceipt::FORMAT,
                    r.canonical_bytes(),
                ))
            }
            TransactionState::Aborted => {
                let reason = native::reason(loader.scope, id, transaction)?
                    .ok_or(AuthorityFailure::Integrity)?;
                Some((
                    transaction
                        .attempts
                        .last()
                        .ok_or(AuthorityFailure::Integrity)?
                        .number,
                    "Aborted",
                    reason.digest(),
                    TransactionReasonRecord::FORMAT,
                    reason.canonical_bytes(),
                ))
            }
            _ => None,
        };
        if let Some(fact) = fact {
            expected.insert(id.clone(), fact);
        }
    }
    for row in loader.rows(
        tx,
        "transaction_outcomes",
        "transaction_id::text,attempt_number::text,terminal_state,outcome_payload_digest::bytea",
        "transaction_id",
    )? {
        let (attempt, state, payload, format, bytes) = expected
            .remove(&transaction(&row, 0)?)
            .ok_or(AuthorityFailure::Integrity)?;
        if number(&row, 1)? != attempt
            || row.get::<_, String>(2) != state
            || digest(&row, 3)? != payload
            || loader.payload(tx, payload, format)? != bytes
        {
            return Err(AuthorityFailure::Integrity);
        }
    }
    if !expected.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    let mut events = records
        .history
        .iter()
        .map(|c| (event_id(&c.receipt), &c.receipt))
        .collect::<BTreeMap<_, _>>();
    let expected_ids = events.keys().copied().collect::<BTreeSet<_>>();
    for row in loader.rows(tx,"outbox","event_id::bytea,aggregate_kind,aggregate_id::text,aggregate_sequence::text,event_type::text,schema_version,payload_digest::bytea","aggregate_kind,aggregate_id,aggregate_sequence")? {
        let receipt=events.remove(&digest(&row,0)?).ok_or(AuthorityFailure::Integrity)?;
        if row.get::<_,String>(1)!="Repository" || row.get::<_,String>(2)!=loader.scope.repository.as_str() || number(&row,3)?!=receipt.revision.0 || row.get::<_,String>(4)!=EVENT_TYPE || row.get::<_,i32>(5)!=1 || digest(&row,6)?!=receipt.digest() {return Err(AuthorityFailure::Integrity);}
    }
    if !events.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    let mut delivered = BTreeSet::new();
    for row in loader.rows(tx, "outbox_delivery", "event_id::bytea", "event_id")? {
        delivered.insert(digest(&row, 0)?);
    }
    if delivered != expected_ids {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}
