//! Atomic, bounded notifications of checked Engine publications. Broker I/O
//! never runs in this transaction and these facts grant no Engine capability.
use super::*;
use celetixo_engine::engine::authority::{SourceAttemptMetadata, SourceRecords, TransactionRecord};
use celetixo_engine::engine::{MutationKey, MutationRecord, MutationRequest, SourceAttemptState};
use celetixo_engine::persistence::{CommitOutcomeRecord, MutationOutcomeRecord};
use celetixo_engine::transaction::TransactionState;
use celetixo_protocol::{
    celetixo::platform::v1::*, decode_lifecycle_event, validate_lifecycle_event,
};
use native::{Loader, SqlResult};
use prost::Message;

pub(crate) const FRAME_FORMAT: &str = "celetixo/lifecycle-frame/v1";
const FACT_FORMAT: &str = "celetixo/lifecycle-fact/v1";
const EVENT_TYPE: &str = "LifecycleEvent";

pub(crate) fn aggregate_name(kind: AggregateKind) -> &'static str {
    match kind {
        AggregateKind::Repository => "Repository",
        AggregateKind::Transaction => "Transaction",
        AggregateKind::SourceAttempt => "SourceAttempt",
        AggregateKind::Mutation => "Mutation",
        AggregateKind::Authority => "Authority",
        AggregateKind::Unspecified => "Unspecified",
    }
}

fn envelope(scope: &Scope, generation: u64) -> LifecycleEvent {
    LifecycleEvent {
        contract_version: 1,
        scope: Some(ScopeSelector {
            organization_id: scope.organization.as_str().into(),
            workspace_id: scope.workspace.as_str().into(),
            repository_id: scope.repository.as_str().into(),
        }),
        aggregate_sequence: generation,
        trace_id: format!("authority:{generation}"),
        ..Default::default()
    }
}

fn fact_bytes(event: &LifecycleEvent) -> SqlResult<Vec<u8>> {
    match LifecycleEventKind::try_from(event.event_kind) {
        Ok(LifecycleEventKind::TransactionStateChanged) => {
            event.transaction.as_ref().map(Message::encode_to_vec)
        }
        Ok(LifecycleEventKind::SourceAttemptStateChanged) => {
            event.source_attempt.as_ref().map(Message::encode_to_vec)
        }
        Ok(LifecycleEventKind::EngineMutationRecorded) => {
            event.mutation.as_ref().map(Message::encode_to_vec)
        }
        Ok(LifecycleEventKind::RepositoryAuthorityChanged) => {
            event.authority.as_ref().map(Message::encode_to_vec)
        }
        _ => None,
    }
    .ok_or(AuthorityFailure::Integrity)
}

fn generated_id(event: &LifecycleEvent) -> Digest {
    let mut identity = event.clone();
    identity.event_id.clear();
    Digest::of("celetixo/lifecycle-event-id/v1", &identity.encode_to_vec())
}

fn insert(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    generation: u64,
    mut event: LifecycleEvent,
) -> SqlResult<()> {
    let a = snapshots::scope_parameters(scope);
    let commit = event.commit.is_some();
    if !commit {
        let bytes = fact_bytes(&event)?;
        let id = Digest::of(FACT_FORMAT, &bytes);
        semantic::put_payload(tx, scope, id, FACT_FORMAT, &bytes).map_err(storage)?;
        event.authority_payload_digest = id.as_bytes().to_vec();
        event.event_id = generated_id(&event).as_bytes().to_vec();
        let aggregate = aggregate_name(
            AggregateKind::try_from(event.aggregate_kind)
                .map_err(|_| AuthorityFailure::Integrity)?,
        );
        tx.execute("INSERT INTO celetixo.outbox (organization_id,workspace_id,repository_id,event_id,aggregate_kind,aggregate_id,aggregate_sequence,event_type,schema_version,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5,$6::text,$7::text::numeric,$8::text,1,$9::bytea) ON CONFLICT (organization_id,workspace_id,repository_id,event_id) DO NOTHING", &[&a[0],&a[1],&a[2],&event.event_id,&aggregate,&event.aggregate_id,&event.aggregate_sequence.to_string(),&EVENT_TYPE,&event.authority_payload_digest]).map_err(database)?;
        tx.execute("INSERT INTO celetixo.outbox_delivery (organization_id,workspace_id,repository_id,event_id) VALUES ($1::text,$2::text,$3::text,$4::bytea) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2],&event.event_id]).map_err(database)?;
    }
    validate_lifecycle_event(&event).map_err(|_| AuthorityFailure::Integrity)?;
    let bytes = event.encode_to_vec();
    let digest = Digest::of(FRAME_FORMAT, &bytes);
    semantic::put_payload(tx, scope, digest, FRAME_FORMAT, &bytes).map_err(storage)?;
    tx.execute("INSERT INTO celetixo.lifecycle_frames (organization_id,workspace_id,repository_id,event_id,frame_digest,publication_generation) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::bytea,$6::text::numeric) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2],&event.event_id,&&digest.as_bytes()[..],&generation.to_string()]).map_err(database)?;
    let row = tx.query_one(&format!("SELECT frame_digest::bytea FROM celetixo.lifecycle_frames WHERE {FILTER} AND event_id=$4::bytea"), &[&a[0],&a[1],&a[2],&event.event_id]).map_err(database)?;
    if super::digest(&row, 0)? != digest {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}

fn transaction_fact(record: &TransactionRecord) -> SqlResult<TransactionStateChanged> {
    let attempt = record.attempts.last().ok_or(AuthorityFailure::Integrity)?;
    let state = match record.state {
        TransactionState::Proposed => TransactionLifecycleState::Proposed,
        TransactionState::Analyzed => TransactionLifecycleState::Analyzed,
        TransactionState::Reserved => TransactionLifecycleState::Reserved,
        TransactionState::Executing => TransactionLifecycleState::Executing,
        TransactionState::Validating => TransactionLifecycleState::Validating,
        TransactionState::RevalidationRequired => TransactionLifecycleState::RevalidationRequired,
        TransactionState::Committed => TransactionLifecycleState::Committed,
        TransactionState::Aborted => TransactionLifecycleState::Aborted,
    };
    Ok(TransactionStateChanged {
        state: state as i32,
        attempt: attempt.number,
        base_snapshot_digest: attempt.base.0.as_bytes().to_vec(),
        lease_fence: record.lease.as_ref().map_or(0, |lease| lease.fence),
    })
}

fn source_fact(record: &SourceAttemptMetadata) -> SourceAttemptStateChanged {
    let state = match record.state {
        SourceAttemptState::Proposed => SourceLifecycleState::Proposed,
        SourceAttemptState::Admitted => SourceLifecycleState::Admitted,
        SourceAttemptState::Patching => SourceLifecycleState::Patching,
        SourceAttemptState::PatchApplied => SourceLifecycleState::PatchApplied,
        SourceAttemptState::Verifying => SourceLifecycleState::Verifying,
        SourceAttemptState::EvidenceSealed => SourceLifecycleState::EvidenceSealed,
        SourceAttemptState::Rejected => SourceLifecycleState::Rejected,
        SourceAttemptState::PatchConflict => SourceLifecycleState::PatchConflict,
        SourceAttemptState::ExecutionFailed => SourceLifecycleState::ExecutionFailed,
        SourceAttemptState::TimedOut => SourceLifecycleState::TimedOut,
        SourceAttemptState::Cancelled => SourceLifecycleState::Cancelled,
        SourceAttemptState::EvidenceIncomplete => SourceLifecycleState::EvidenceIncomplete,
        SourceAttemptState::RevalidationRequired => SourceLifecycleState::RevalidationRequired,
    };
    let binding = record.admission.binding();
    SourceAttemptStateChanged {
        transaction_id: binding.transaction.as_str().into(),
        transaction_attempt: binding.transaction_attempt,
        state: state as i32,
        event_count: record.events.len() as u64,
        admission_fence: record.admission_fence,
    }
}

fn mutation_fact(
    scope: &Scope,
    key: &MutationKey,
    record: &MutationRecord,
) -> SqlResult<EngineMutationRecorded> {
    let kind = match record.request {
        MutationRequest::SourceCommit(_) => EngineMutationKind::SourceCommit,
        MutationRequest::SourceRevalidate(_) => EngineMutationKind::SourceRevalidate,
        MutationRequest::SourceExecutor(_) => EngineMutationKind::SourceExecutor,
        MutationRequest::Administrative(_) => EngineMutationKind::Administrative,
        MutationRequest::Repository(_) => EngineMutationKind::Repository,
        MutationRequest::SubmitPatch { .. } => EngineMutationKind::SubmitPatch,
        MutationRequest::RecordEvidence { .. } => EngineMutationKind::RecordEvidence,
        MutationRequest::Revalidate { .. } => EngineMutationKind::Revalidate,
        MutationRequest::Propose { .. } => EngineMutationKind::Propose,
        MutationRequest::Analyze { .. } => EngineMutationKind::Analyze,
        MutationRequest::Reserve { .. } => EngineMutationKind::Reserve,
        MutationRequest::Renew { .. } => EngineMutationKind::Renew,
        MutationRequest::Execute { .. } => EngineMutationKind::Execute,
        MutationRequest::Abort { .. } => EngineMutationKind::Abort,
    };
    let outcome =
        MutationOutcomeRecord::new(scope.clone(), key.clone(), record.clone()).map_err(restore)?;
    Ok(EngineMutationRecorded {
        kind: kind as i32,
        request_digest: record.request.digest(scope, &key.actor).as_bytes().to_vec(),
        outcome_digest: outcome.digest().as_bytes().to_vec(),
        transaction_id: record
            .request
            .transaction()
            .map_or_else(String::new, |id| id.as_str().into()),
    })
}

fn mutation_id(key: &MutationKey) -> String {
    let mut identity = key.actor.digest().as_bytes().to_vec();
    identity.extend_from_slice(key.key.as_str().as_bytes());
    Digest::of("celetixo/mutation-aggregate/v1", &identity).to_string()
}

fn has_aggregate(tx: &mut Transaction<'_>, scope: &Scope, kind: &str, id: &str) -> SqlResult<bool> {
    let a = snapshots::scope_parameters(scope);
    Ok(tx.query_one(&format!("SELECT EXISTS (SELECT 1 FROM celetixo.outbox WHERE {FILTER} AND aggregate_kind=$4 AND aggregate_id=$5::text)"), &[&a[0],&a[1],&a[2],&kind,&id]).map_err(database)?.get(0))
}

fn revision(records: &NativeRecords) -> SqlResult<(u64, Vec<u8>)> {
    match &records.repository {
        None => Ok((0, Vec::new())),
        Some(repo) => Ok((
            repo.registered
                .get(&repo.current)
                .ok_or(AuthorityFailure::Integrity)?
                .revision()
                .0,
            repo.current.0.as_bytes().to_vec(),
        )),
    }
}

// A retained pointer to a scoped authority publication, not a grant. Its bytes
// are length-prefixed selectors, generation, clock and optional snapshot digest.
fn publication_bytes(scope: &Scope, generation: u64, clock: u64, snapshot: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in snapshots::scope_parameters(scope) {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes.extend_from_slice(&generation.to_be_bytes());
    bytes.extend_from_slice(&clock.to_be_bytes());
    bytes.extend_from_slice(snapshot);
    bytes
}
const PUBLICATION_FORMAT: &str = "celetixo/event-authority-publication/v1";

pub(super) fn publish_base(
    tx: &mut Transaction<'_>,
    generation: u64,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let scope = &after.scope;
    for (id, record) in &after.transactions {
        if before.transactions.get(id) == Some(record)
            && has_aggregate(tx, scope, "Transaction", id.as_str())?
        {
            continue;
        }
        let mut event = envelope(scope, generation);
        event.aggregate_kind = AggregateKind::Transaction as i32;
        event.aggregate_id = id.as_str().into();
        event.event_kind = LifecycleEventKind::TransactionStateChanged as i32;
        event.transaction = Some(transaction_fact(record)?);
        insert(tx, scope, generation, event)?;
    }
    for (key, record) in &after.mutations {
        if before.mutations.contains_key(key)
            && has_aggregate(tx, scope, "Mutation", &mutation_id(key))?
        {
            continue;
        }
        let mut event = envelope(scope, 0);
        event.aggregate_kind = AggregateKind::Mutation as i32;
        event.aggregate_id = mutation_id(key);
        event.trace_id = format!("mutation:{}", event.aggregate_id);
        event.event_kind = LifecycleEventKind::EngineMutationRecorded as i32;
        event.mutation = Some(mutation_fact(scope, key, record)?);
        insert(tx, scope, generation, event)?;
    }
    let (revision, snapshot) = revision(after)?;
    let bytes = publication_bytes(scope, generation, after.clock_ms, &snapshot);
    let metadata = Digest::of(PUBLICATION_FORMAT, &bytes);
    semantic::put_payload(tx, scope, metadata, PUBLICATION_FORMAT, &bytes).map_err(storage)?;
    let mut event = envelope(scope, generation);
    event.aggregate_kind = AggregateKind::Authority as i32;
    event.aggregate_id = scope.repository.as_str().into();
    event.event_kind = LifecycleEventKind::RepositoryAuthorityChanged as i32;
    event.authority = Some(RepositoryAuthorityChanged {
        revision,
        snapshot_digest: snapshot,
        metadata_digest: metadata.as_bytes().to_vec(),
    });
    insert(tx, scope, generation, event)?;
    if let Some(repo) = &after.repository {
        for change in &repo.history {
            let receipt = &change.receipt;
            let outcome = CommitOutcomeRecord::new(
                scope.clone(),
                receipt.transaction.clone(),
                receipt.attempt,
                Ok(receipt.clone()),
            )
            .map_err(restore)?;
            let event_id = Digest::of(
                "celetixo/native-commit-event/v1",
                &outcome.canonical_bytes(),
            );
            let mut event = envelope(scope, receipt.revision.0);
            event.trace_id = format!("commit:{event_id}");
            event.event_id = event_id.as_bytes().to_vec();
            event.aggregate_kind = AggregateKind::Repository as i32;
            event.aggregate_id = scope.repository.as_str().into();
            event.event_kind = LifecycleEventKind::CommitRecorded as i32;
            event.authority_payload_digest = outcome.digest().as_bytes().to_vec();
            event.commit = Some(CommitRecorded {
                transaction_id: receipt.transaction.as_str().into(),
                revision: receipt.revision.0,
                snapshot_digest: receipt.snapshot.0.as_bytes().to_vec(),
                patch_digest: receipt.patch.0.as_bytes().to_vec(),
                profile: CommitProfile::Native as i32,
            });
            insert(tx, scope, generation, event)?;
        }
    }
    Ok(())
}

pub(super) fn publish_source(
    tx: &mut Transaction<'_>,
    generation: u64,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    publish_base(tx, generation, &before.base, &after.base)?;
    let scope = &after.base.scope;
    for (id, record) in &after.attempts {
        if before.attempts.get(id) == Some(record)
            && has_aggregate(tx, scope, "SourceAttempt", id.as_str())?
        {
            continue;
        }
        let mut event = envelope(scope, generation);
        event.aggregate_kind = AggregateKind::SourceAttempt as i32;
        event.aggregate_id = id.as_str().into();
        event.event_kind = LifecycleEventKind::SourceAttemptStateChanged as i32;
        event.source_attempt = Some(source_fact(record));
        insert(tx, scope, generation, event)?;
    }
    for change in &after.history {
        let r = &change.receipt;
        let event_id = Digest::of("celetixo/source-commit-event/v1", &r.canonical_bytes());
        let mut event = envelope(scope, r.revision.0);
        event.trace_id = format!("commit:{event_id}");
        event.event_id = event_id.as_bytes().to_vec();
        event.aggregate_kind = AggregateKind::Repository as i32;
        event.aggregate_id = scope.repository.as_str().into();
        event.event_kind = LifecycleEventKind::CommitRecorded as i32;
        event.authority_payload_digest = r.digest().as_bytes().to_vec();
        event.commit = Some(CommitRecorded {
            transaction_id: r.transaction.as_str().into(),
            revision: r.revision.0,
            snapshot_digest: r.publication_snapshot.0.as_bytes().to_vec(),
            patch_digest: r.patch.0.as_bytes().to_vec(),
            profile: CommitProfile::Source as i32,
        });
        insert(tx, scope, generation, event)?;
    }
    Ok(())
}

pub(super) fn load(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &NativeRecords,
    source: Option<&SourceRecords>,
) -> SqlResult<()> {
    let scope = loader.scope;
    let generations = loader.rows(
        tx,
        "repositories",
        "state_generation::text",
        "repository_id",
    )?;
    let generation = number(generations.first().ok_or(AuthorityFailure::Integrity)?, 0)?;
    let origins = loader.rows(
        tx,
        "event_stream_origins",
        "state_generation::text",
        "repository_id",
    )?;
    let origin = origins
        .first()
        .map(|row| number(row, 0))
        .transpose()?
        .unwrap_or(0);
    if origin > generation {
        return Err(AuthorityFailure::Integrity);
    }
    let mut frames = BTreeMap::new();
    for row in loader.rows(
        tx,
        "lifecycle_frames",
        "event_id::bytea,frame_digest::bytea,publication_generation::text",
        "event_id",
    )? {
        let published = number(&row, 2)?;
        if published == 0 || published > generation {
            return Err(AuthorityFailure::Integrity);
        }
        frames.insert(digest(&row, 0)?, (digest(&row, 1)?, published));
    }
    let mut publications = BTreeMap::new();
    for row in loader.rows(
        tx,
        "authority_publications",
        "state_generation::text,observed_clock_ms::text,snapshot_digest::bytea",
        "state_generation",
    )? {
        publications.insert(
            number(&row, 0)?,
            (number(&row, 1)?, native::optional_digest(&row, 2)?),
        );
    }
    let mut ids = BTreeSet::new();
    let mut authority = BTreeSet::new();
    let mut transactions = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut mutations = BTreeMap::new();
    let mut checked_frames = BTreeMap::new();
    for row in loader.rows(tx,"outbox","event_id::bytea,aggregate_kind,aggregate_id::text,aggregate_sequence::text,event_type::text,schema_version,payload_digest::bytea","aggregate_kind,aggregate_id,aggregate_sequence")? {
        let id = digest(&row,0)?;
        ids.insert(id);
        let kind = row.get::<_,String>(1);
        let aggregate = row.get::<_,String>(2);
        let sequence = number(&row,3)?;
        let event_type = row.get::<_,String>(4);
        let payload = digest(&row,6)?;
        let Some((frame,published)) = frames.remove(&id) else {
            // Exact legacy commit associations are separately checked by each
            // profile's history loader. No unframed new/lifecycle event is valid.
            if origin>0 && kind=="Repository" && event_type!=EVENT_TYPE && generation==origin { continue; }
            return Err(AuthorityFailure::Integrity);
        };
        if !publications.contains_key(&published) { return Err(AuthorityFailure::Integrity); }
        let bytes = loader.payload(tx,frame,FRAME_FORMAT)?;
        let decoded = decode_lifecycle_event(&bytes).map_err(|_|AuthorityFailure::Integrity)?;
        let event = decoded.event();
        if decoded.canonical_bytes()!=bytes || event.scope!=envelope(scope,0).scope
            || event.event_id!=id.as_bytes() || event.aggregate_id!=aggregate || event.aggregate_sequence!=sequence
            || aggregate_name(AggregateKind::try_from(event.aggregate_kind).map_err(|_|AuthorityFailure::Integrity)?)!=kind
            || row.get::<_,i32>(5)!=1 || event.authority_payload_digest!=payload.as_bytes() {
            return Err(AuthorityFailure::Integrity);
        }
        checked_frames.insert(id,(frame,event.clone()));
        if let Some(fact) = &event.commit {
            if event_type=="NativeCommitRecorded" && source.is_none() {
                let receipt = &records.repository.as_ref().and_then(|repo|repo.history.iter().find(|c|c.receipt.revision.0==sequence)).ok_or(AuthorityFailure::Integrity)?.receipt;
                if fact.transaction_id!=receipt.transaction.as_str() || fact.snapshot_digest!=receipt.snapshot.0.as_bytes()
                    || fact.patch_digest!=receipt.patch.0.as_bytes() || fact.profile!=CommitProfile::Native as i32 { return Err(AuthorityFailure::Integrity); }
            } else if event_type=="SourceCommitRecorded" {
                let receipt = &source.and_then(|r|r.history.iter().find(|c|c.receipt.revision.0==sequence)).ok_or(AuthorityFailure::Integrity)?.receipt;
                if fact.transaction_id!=receipt.transaction.as_str() || fact.snapshot_digest!=receipt.publication_snapshot.0.as_bytes()
                    || fact.patch_digest!=receipt.patch.0.as_bytes() || fact.profile!=CommitProfile::Source as i32 { return Err(AuthorityFailure::Integrity); }
            } else { return Err(AuthorityFailure::Integrity); }
            continue;
        }
        let fact = fact_bytes(event)?;
        if event_type!=EVENT_TYPE || generated_id(event)!=id || Digest::of(FACT_FORMAT,&fact)!=payload
            || loader.payload(tx,payload,FACT_FORMAT)?!=fact
            || (event.mutation.is_none() && sequence!=published) { return Err(AuthorityFailure::Integrity); }
        if let Some(fact) = &event.authority {
            let (clock,snapshot) = publications.get(&sequence).ok_or(AuthorityFailure::Integrity)?;
            let snapshot = snapshot.map(|id|id.as_bytes().to_vec()).unwrap_or_default();
            let bytes = publication_bytes(scope,sequence,*clock,&snapshot);
            let metadata = Digest::of(PUBLICATION_FORMAT,&bytes);
            if fact.snapshot_digest!=snapshot || fact.metadata_digest!=metadata.as_bytes()
                || loader.payload(tx,metadata,PUBLICATION_FORMAT)?!=bytes { return Err(AuthorityFailure::Integrity); }
            let expected_revision = if let Some(id)=publications.get(&sequence).and_then(|(_,id)|*id) {
                loader.snapshot(tx,SnapshotId(id))?.revision().0
            } else {0};
            if fact.revision!=expected_revision { return Err(AuthorityFailure::Integrity); }
            authority.insert(sequence);
        } else if let Some(fact) = &event.transaction {
            let record = records.transactions.iter().find(|(id,_)|id.as_str()==aggregate).map(|(_,record)|record).ok_or(AuthorityFailure::Integrity)?;
            let attempt = record.attempts.iter().find(|attempt|attempt.number==fact.attempt).ok_or(AuthorityFailure::Integrity)?;
            if fact.base_snapshot_digest!=attempt.base.0.as_bytes() { return Err(AuthorityFailure::Integrity); }
            transactions.insert(aggregate,(sequence,fact.clone()));
        } else if let Some(fact) = &event.source_attempt {
            let record = source.and_then(|r|r.attempts.iter().find(|(id,_)|id.as_str()==aggregate).map(|(_,r)|r)).ok_or(AuthorityFailure::Integrity)?;
            let prefix = usize::try_from(fact.event_count).ok().and_then(|count|count.checked_sub(1)).and_then(|index|record.events.get(index)).ok_or(AuthorityFailure::Integrity)?;
            let expected = source_fact(record);
            let mut historical = record.clone();
            historical.state = prefix.to;
            if fact.transaction_id!=expected.transaction_id || fact.transaction_attempt!=expected.transaction_attempt
                || fact.admission_fence!=expected.admission_fence || fact.state!=source_fact(&historical).state { return Err(AuthorityFailure::Integrity); }
            sources.insert(aggregate,(sequence,fact.clone()));
        } else if let Some(fact) = &event.mutation
            && mutations.insert(aggregate,fact.clone()).is_some() { return Err(AuthorityFailure::Integrity); }
    }
    let delivery = loader
        .rows(tx, "outbox_delivery", "event_id::bytea", "event_id")?
        .iter()
        .map(|row| digest(row, 0))
        .collect::<SqlResult<BTreeSet<_>>>()?;
    if !frames.is_empty() || delivery != ids {
        return Err(AuthorityFailure::Integrity);
    }
    if authority
        != publications
            .keys()
            .copied()
            .filter(|value| *value > origin)
            .collect()
    {
        return Err(AuthorityFailure::Integrity);
    }
    let initialized = generation > origin;
    let mut transaction_generations = BTreeMap::new();
    for row in loader.rows(
        tx,
        "transactions",
        "transaction_id::text,generation::text",
        "transaction_id",
    )? {
        transaction_generations.insert(row.get::<_, String>(0), number(&row, 1)?);
    }
    for (id, record) in &records.transactions {
        match transactions.remove(id.as_str()) {
            Some((sequence, fact))
                if fact == transaction_fact(record)?
                    && sequence
                        >= *transaction_generations
                            .get(id.as_str())
                            .ok_or(AuthorityFailure::Integrity)? => {}
            None if !initialized => {}
            _ => return Err(AuthorityFailure::Integrity),
        }
    }
    if let Some(source) = source {
        for (id, record) in &source.attempts {
            match sources.remove(id.as_str()) {
                Some((_, fact)) if fact == source_fact(record) => {}
                None if !initialized => {}
                _ => return Err(AuthorityFailure::Integrity),
            }
        }
    }
    for (key, record) in &records.mutations {
        match mutations.remove(&mutation_id(key)) {
            Some(fact) if fact == mutation_fact(scope, key, record)? => {}
            None if !initialized => {}
            _ => return Err(AuthorityFailure::Integrity),
        }
    }
    if !transactions.is_empty() || !sources.is_empty() || !mutations.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    let mut projections: BTreeMap<(String, String, String), (u64, Digest, Digest)> =
        BTreeMap::new();
    for row in loader.rows(tx,"inbox","consumer_id::text,event_id::bytea,event_payload_digest::bytea,result_payload_digest::bytea","consumer_id,event_id")? {
        let id = digest(&row,1)?;
        let (frame,event) = checked_frames.get(&id).ok_or(AuthorityFailure::Integrity)?;
        if digest(&row,2)?!=*frame || digest(&row,3)?!=*frame { return Err(AuthorityFailure::Integrity); }
        let kind = aggregate_name(AggregateKind::try_from(event.aggregate_kind).map_err(|_|AuthorityFailure::Integrity)?).to_string();
        let key = (row.get::<_,String>(0),kind,event.aggregate_id.clone());
        let expected = (event.aggregate_sequence,id,*frame);
        match projections.get(&key) {
            Some(previous) if previous.0>=event.aggregate_sequence => {},
            _ => { projections.insert(key,expected); },
        }
    }
    for row in loader.rows(tx,"event_projections","consumer_id::text,aggregate_kind,aggregate_id::text,aggregate_sequence::text,event_id::bytea,frame_digest::bytea","consumer_id,aggregate_kind,aggregate_id")? {
        let key = (row.get::<_,String>(0),row.get::<_,String>(1),row.get::<_,String>(2));
        if projections.remove(&key)!=Some((number(&row,3)?,digest(&row,4)?,digest(&row,5)?)) { return Err(AuthorityFailure::Integrity); }
    }
    if !projections.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    for row in loader.rows(
        tx,
        "event_quarantine",
        "consumer_id::text,message_digest::bytea,reason,message_bytes",
        "consumer_id,message_digest",
    )? {
        let bytes: Vec<u8> = row.get(3);
        if Digest::of("celetixo/quarantined-event/v1", &bytes) != digest(&row, 1)?
            || !matches!(
                row.get::<_, String>(2).as_str(),
                "Schema" | "Scope" | "Authority" | "Substitution" | "RedeliveryLimit"
            )
        {
            return Err(AuthorityFailure::Integrity);
        }
    }
    Ok(())
}
