use super::*;

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
fn exactly_one(changed: u64) -> SqlResult<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(AuthorityFailure::Contention)
    }
}
fn preserve(before: &NativeRecords, after: &NativeRecords) -> SqlResult<()> {
    if before.scope != after.scope
        || before.clock_ms > after.clock_ms
        || before.session_counter > after.session_counter
        || before.reservation_fence > after.reservation_fence
    {
        return Err(AuthorityFailure::Protocol);
    }
    if let Some(old) = &before.repository {
        let new = after
            .repository
            .as_ref()
            .ok_or(AuthorityFailure::Protocol)?;
        if new.initial != old.initial
            || !new.history.starts_with(&old.history)
            || old
                .registered
                .iter()
                .any(|(id, value)| new.registered.get(id) != Some(value))
        {
            return Err(AuthorityFailure::Protocol);
        }
    }
    for (id, old) in &before.transactions {
        let new = after
            .transactions
            .get(id)
            .ok_or(AuthorityFailure::Protocol)?;
        if old.owner != new.owner
            || old.proposal_grant != new.proposal_grant
            || old.intent != new.intent
            || new.attempts.len() < old.attempts.len()
            || !new.lease_history.starts_with(&old.lease_history)
        {
            return Err(AuthorityFailure::Protocol);
        }
        if matches!(
            old.state,
            TransactionState::Committed | TransactionState::Aborted
        ) && old != new
        {
            return Err(AuthorityFailure::Protocol);
        }
        for (i, previous) in old.attempts.iter().enumerate() {
            let current = &new.attempts[i];
            if previous.number != current.number
                || previous.base != current.base
                || previous.model != current.model
                || previous
                    .effective
                    .as_ref()
                    .is_some_and(|v| current.effective.as_ref() != Some(v))
                || previous
                    .analysis
                    .as_ref()
                    .is_some_and(|v| current.analysis.as_ref() != Some(v))
                || previous
                    .patch
                    .as_ref()
                    .is_some_and(|v| current.patch.as_ref() != Some(v))
                || previous
                    .candidate
                    .as_ref()
                    .is_some_and(|v| current.candidate.as_ref() != Some(v))
                || (i + 1 < new.attempts.len() && previous.evidence != current.evidence)
            {
                return Err(AuthorityFailure::Protocol);
            }
        }
    }
    if before.evidence.iter().any(|(id, old)| {
        after
            .evidence
            .get(id)
            .is_none_or(|new| new.evidence != old.evidence)
    }) || before
        .operations
        .iter()
        .any(|(key, value)| after.operations.get(key) != Some(value))
    {
        return Err(AuthorityFailure::Protocol);
    }
    Ok(())
}

pub(in crate::authority) fn publish(
    tx: &mut Transaction<'_>,
    generation: u64,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    publish_base(tx, generation, before, after, LoadProfile::Native)?;
    super::super::mutations::publish(tx, before, after)
}
pub(in crate::authority) fn publish_source_base(
    tx: &mut Transaction<'_>,
    generation: u64,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    if !before.evidence.is_empty()
        || !after.evidence.is_empty()
        || !before.operations.is_empty()
        || !after.operations.is_empty()
        || after
            .repository
            .as_ref()
            .is_some_and(|repo| !repo.history.is_empty())
    {
        return Err(AuthorityFailure::Protocol);
    }
    publish_base(tx, generation, before, after, LoadProfile::Source)?;
    Ok(())
}
fn publish_base(
    tx: &mut Transaction<'_>,
    generation: u64,
    before: &NativeRecords,
    after: &NativeRecords,
    profile: LoadProfile,
) -> SqlResult<()> {
    preserve(before, after)?;
    let scope = &after.scope;
    let Some(repo) = &after.repository else {
        if !after.transactions.is_empty()
            || !after.evidence.is_empty()
            || !after.operations.is_empty()
            || after.reservation_fence != 0
        {
            return Err(AuthorityFailure::Integrity);
        }
        return Ok(());
    };
    for (id, snapshot) in &repo.registered {
        if before
            .repository
            .as_ref()
            .and_then(|r| r.registered.get(id))
            != Some(snapshot)
        {
            snapshots::persist(tx, snapshot).map_err(storage)?;
        }
    }
    for (id, transaction) in &after.transactions {
        let previous = before.transactions.get(id);
        if previous == Some(transaction) {
            continue;
        }
        let original = repo
            .registered
            .get(
                &transaction
                    .attempts
                    .first()
                    .ok_or(AuthorityFailure::Integrity)?
                    .base,
            )
            .ok_or(AuthorityFailure::Integrity)?;
        let intent =
            NativeIntentRecord::new(original, transaction.intent.clone()).map_err(restore)?;
        put(
            tx,
            scope,
            intent.digest(),
            NativeIntentRecord::FORMAT,
            &intent.canonical_bytes(),
        )?;
        let current = transaction
            .attempts
            .last()
            .ok_or(AuthorityFailure::Integrity)?
            .number;
        let why = reason(scope, id, transaction)?;
        if let Some(reason) = &why {
            put(
                tx,
                scope,
                reason.digest(),
                TransactionReasonRecord::FORMAT,
                &reason.canonical_bytes(),
            )?;
        }
        let reason_digest = why.as_ref().map(|r| r.digest().as_bytes().to_vec());
        let fence = transaction
            .lease
            .as_ref()
            .map(|lease| lease.fence.to_string());
        if previous.is_some() {
            exactly_one(execute(
                tx,
                scope,
                &format!(
                    "UPDATE celetixo.transactions SET state=$5,current_attempt=$6::text::numeric,generation=$7::text::numeric,current_fence=$8::text::numeric,last_reason=NULL,last_reason_payload_digest=$9::bytea WHERE {FILTER} AND transaction_id=$4::text"
                ),
                &[
                    &id.as_str(),
                    &state_name(transaction.state),
                    &current.to_string(),
                    &generation.to_string(),
                    &fence,
                    &reason_digest,
                ],
            )?)?;
        } else {
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.transactions (organization_id,workspace_id,repository_id,transaction_id,owner_principal_id,owner_grant_id,state,current_attempt,original_snapshot_digest,intent_payload_digest,generation,current_fence,last_reason_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text,$6::text::numeric,$7,$8::text::numeric,$9::bytea,$10::bytea,$11::text::numeric,$12::text::numeric,$13::bytea)",
                &[
                    &id.as_str(),
                    &transaction.owner.as_str(),
                    &transaction.proposal_grant.to_string(),
                    &state_name(transaction.state),
                    &current.to_string(),
                    &&original.id().0.as_bytes()[..],
                    &&intent.digest().as_bytes()[..],
                    &generation.to_string(),
                    &fence,
                    &reason_digest,
                ],
            )?;
        }
        for attempt in &transaction.attempts {
            if previous.and_then(|tx| tx.attempts.get(attempt.number as usize)) == Some(attempt) {
                continue;
            }
            let base = repo
                .registered
                .get(&attempt.base)
                .ok_or(AuthorityFailure::Integrity)?;
            let model = EngineeringModelRecord::new(base, attempt.model.clone())
                .map_err(|_| AuthorityFailure::Integrity)?;
            put(
                tx,
                scope,
                model.digest(),
                EngineeringModelRecord::FORMAT,
                &model.canonical_bytes(),
            )?;
            let analysis = attempt
                .analysis
                .as_ref()
                .map(|analysis| ImpactAnalysisRecord::new(base, analysis.clone()).map_err(restore))
                .transpose()?;
            if let Some(record) = &analysis {
                put(
                    tx,
                    scope,
                    record.digest(),
                    ImpactAnalysisRecord::FORMAT,
                    &record.canonical_bytes(),
                )?;
            }
            if let Some(patch) = &attempt.patch {
                put(
                    tx,
                    scope,
                    patch.id().0,
                    PATCH_FORMAT,
                    &patch.canonical_bytes(),
                )?;
            }
            if let Some(snapshot) = &attempt.candidate {
                snapshots::persist(tx, snapshot).map_err(storage)?;
            }
            let candidate = attempt
                .candidate
                .as_ref()
                .map(|snapshot| snapshot.id().0.as_bytes().to_vec());
            let analysis = analysis
                .as_ref()
                .map(|record| record.digest().as_bytes().to_vec());
            let patch = attempt
                .patch
                .as_ref()
                .map(|patch| patch.id().0.as_bytes().to_vec());
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.attempts (organization_id,workspace_id,repository_id,transaction_id,attempt_number,base_snapshot_digest,candidate_snapshot_digest,model_payload_digest,analysis_payload_digest,patch_digest,patch_payload_digest,model_generation) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6::bytea,$7::bytea,$8::bytea,$9::bytea,$10::bytea,$10::bytea,$11::text::numeric) ON CONFLICT (organization_id,workspace_id,repository_id,transaction_id,attempt_number) DO UPDATE SET candidate_snapshot_digest=EXCLUDED.candidate_snapshot_digest,analysis_payload_digest=EXCLUDED.analysis_payload_digest,patch_digest=EXCLUDED.patch_digest,patch_payload_digest=EXCLUDED.patch_payload_digest",
                &[
                    &id.as_str(),
                    &attempt.number.to_string(),
                    &&base.id().0.as_bytes()[..],
                    &candidate,
                    &&model.digest().as_bytes()[..],
                    &analysis,
                    &patch,
                    &attempt.model.generation.to_string(),
                ],
            )?;
            for fact in access_facts(scope, id, transaction, attempt, base)? {
                put(tx, scope, fact.id, fact.format, &fact.bytes)?;
                execute(
                    tx,
                    scope,
                    "INSERT INTO celetixo.access_provenance (organization_id,workspace_id,repository_id,transaction_id,attempt_number,provenance_kind,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6,$7::bytea) ON CONFLICT DO NOTHING",
                    &[
                        &id.as_str(),
                        &attempt.number.to_string(),
                        &fact.kind,
                        &&fact.id.as_bytes()[..],
                    ],
                )?;
            }
        }
    }
    if profile == LoadProfile::Native {
        publish_evidence(tx, before, after)?;
    }
    publish_leases(tx, before, after)?;
    if profile == LoadProfile::Native {
        publish_history(tx, before, after)?;
        publish_operations(tx, before, after)?;
    }
    publish_terminal(tx, before, after)?;
    if profile == LoadProfile::Native {
        publish_heads(tx, generation, before, after)?;
    }
    Ok(())
}

fn publish_evidence(
    tx: &mut Transaction<'_>,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let scope = &after.scope;
    let repo = after
        .repository
        .as_ref()
        .ok_or(AuthorityFailure::Integrity)?;
    for (id, record) in &after.evidence {
        if before.evidence.get(id) == Some(record) {
            continue;
        }
        let evidence = &record.evidence;
        let base = repo
            .registered
            .get(&evidence.bindings().base_snapshot)
            .ok_or(AuthorityFailure::Integrity)?;
        let envelope = VerificationEvidenceRecord::new(base, evidence.clone()).map_err(restore)?;
        put(
            tx,
            scope,
            envelope.digest(),
            VerificationEvidenceRecord::FORMAT,
            &envelope.canonical_bytes(),
        )?;
        // The immutable evidence issuer remains its FIRST grant, independent
        // of a later fresh admission by another grant for the same principal.
        let original = before
            .evidence
            .get(id)
            .map_or(record.verifier_grant, |e| e.verifier_grant);
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.evidence (organization_id,workspace_id,repository_id,evidence_digest,evidence_kind,transaction_id,attempt_number,evidence_payload_digest,verifier_grant_kind,verifier_grant_id) VALUES ($1::text,$2::text,$3::text,$4::bytea,'Reference',$5::text,$6::text::numeric,$7::bytea,'session',$8::text::numeric) ON CONFLICT DO NOTHING",
            &[
                &&id.0.as_bytes()[..],
                &evidence.bindings().transaction.as_str(),
                &evidence.bindings().attempt.to_string(),
                &&envelope.digest().as_bytes()[..],
                &original.to_string(),
            ],
        )?;
        // Existing admissions retain their first recorded timestamp. Switching
        // current selection never rewrites or manufactures historical admissions.
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.reference_evidence_admissions (organization_id,workspace_id,repository_id,evidence_digest,verifier_grant_id,admitted_at_ms) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text::numeric,$6::text::numeric) ON CONFLICT DO NOTHING",
            &[
                &&id.0.as_bytes()[..],
                &record.verifier_grant.to_string(),
                &after.clock_ms.to_string(),
            ],
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.reference_evidence_signers (organization_id,workspace_id,repository_id,evidence_digest,verifier_grant_id) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text::numeric) ON CONFLICT (organization_id,workspace_id,repository_id,evidence_digest) DO UPDATE SET verifier_grant_id=EXCLUDED.verifier_grant_id",
            &[&&id.0.as_bytes()[..], &record.verifier_grant.to_string()],
        )?;
    }
    for (id, transaction) in &after.transactions {
        for attempt in &transaction.attempts {
            let previous = before
                .transactions
                .get(id)
                .and_then(|tx| tx.attempts.get(attempt.number as usize));
            for (check, evidence) in &attempt.evidence {
                if previous.and_then(|a| a.evidence.get(check)) == Some(evidence) {
                    continue;
                }
                execute(
                    tx,
                    scope,
                    "INSERT INTO celetixo.attempt_checks (organization_id,workspace_id,repository_id,transaction_id,attempt_number,check_id,evidence_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6::text,$7::bytea) ON CONFLICT (organization_id,workspace_id,repository_id,transaction_id,attempt_number,check_id) DO UPDATE SET evidence_digest=EXCLUDED.evidence_digest",
                    &[
                        &id.as_str(),
                        &attempt.number.to_string(),
                        &check.as_str(),
                        &&evidence.0.as_bytes()[..],
                    ],
                )?;
            }
            if previous.is_some_and(|a| {
                a.evidence
                    .keys()
                    .any(|check| !attempt.evidence.contains_key(check))
            }) {
                return Err(AuthorityFailure::Protocol);
            }
        }
    }
    Ok(())
}

fn publish_leases(
    tx: &mut Transaction<'_>,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let scope = &after.scope;
    if before.reservation_fence != after.reservation_fence {
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.reservation_fences (organization_id,workspace_id,repository_id,generation) VALUES ($1::text,$2::text,$3::text,$4::text::numeric) ON CONFLICT (organization_id,workspace_id,repository_id) DO UPDATE SET generation=EXCLUDED.generation",
            &[&after.reservation_fence.to_string()],
        )?;
    }
    let old = reservations(before)?;
    for (fence, fact) in reservations(after)? {
        if old.get(&fence) == Some(&fact) {
            continue;
        }
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.reservations (organization_id,workspace_id,repository_id,fence_generation,transaction_id,attempt_number,owner_grant_id,acquired_at_ms,expires_at_ms,state) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text,$6::text::numeric,$7::text::numeric,$8::text::numeric,$9::text::numeric,$10) ON CONFLICT (organization_id,workspace_id,repository_id,fence_generation) DO UPDATE SET expires_at_ms=EXCLUDED.expires_at_ms,state=EXCLUDED.state",
            &[
                &fence.to_string(),
                &fact.transaction.as_str(),
                &fact.attempt.to_string(),
                &fact.owner_grant.to_string(),
                &fact.acquired_at.to_string(),
                &fact.expires_at.to_string(),
                &fact.state,
            ],
        )?;
    }
    for (id, transaction) in &after.transactions {
        let first = before
            .transactions
            .get(id)
            .map_or(0, |tx| tx.lease_history.len());
        for (sequence, event) in transaction.lease_history.iter().enumerate().skip(first) {
            let record =
                LeaseEventRecord::new(scope.clone(), id.clone(), sequence as u64, event.clone())
                    .map_err(restore)?;
            put(
                tx,
                scope,
                record.digest(),
                LeaseEventRecord::FORMAT,
                &record.canonical_bytes(),
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.reservation_events (organization_id,workspace_id,repository_id,fence_generation,event_sequence,at_ms,event_kind,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text::numeric,$6::text::numeric,$7,$8::bytea)",
                &[
                    &event.fence.to_string(),
                    &(sequence as u64).to_string(),
                    &event.at.to_string(),
                    &event_name(&event.kind),
                    &&record.digest().as_bytes()[..],
                ],
            )?;
        }
    }
    let old = claims(before)?;
    let new = claims(after)?;
    for (element, fence) in &old {
        if new.get(element) != Some(fence) {
            exactly_one(execute(
                tx,
                scope,
                &format!(
                    "DELETE FROM celetixo.reservation_claims WHERE {FILTER} AND element_id=$4::text AND fence_generation=$5::text::numeric"
                ),
                &[&element.as_str(), &fence.to_string()],
            )?)?;
        }
    }
    for (element, fence) in &new {
        if old.get(element) != Some(fence) {
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.reservation_claims (organization_id,workspace_id,repository_id,element_id,fence_generation) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric)",
                &[&element.as_str(), &fence.to_string()],
            )?;
        }
    }
    Ok(())
}

fn publish_history(
    tx: &mut Transaction<'_>,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let scope = &after.scope;
    let repo = after
        .repository
        .as_ref()
        .ok_or(AuthorityFailure::Integrity)?;
    let first = before.repository.as_ref().map_or(0, |r| r.history.len());
    for change in repo.history.iter().skip(first) {
        let receipt = &change.receipt;
        let base = repo
            .registered
            .get(&change.provenance.analysis.snapshot)
            .ok_or(AuthorityFailure::Integrity)?;
        let provenance = AccessProvenanceRecord::new(
            base,
            receipt.transaction.clone(),
            receipt.attempt,
            change.provenance.clone(),
        )
        .map_err(restore)?;
        let outcome = accepted(scope, receipt)?;
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
            outcome.digest(),
            CommitOutcomeRecord::FORMAT,
            &outcome.canonical_bytes(),
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.committed_changes (organization_id,workspace_id,repository_id,revision,transaction_id,attempt_number,snapshot_digest,patch_digest,provenance_payload_digest,receipt_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text,$6::text::numeric,$7::bytea,$8::bytea,$9::bytea,$10::bytea)",
            &[
                &receipt.revision.0.to_string(),
                &receipt.transaction.as_str(),
                &receipt.attempt.to_string(),
                &&receipt.snapshot.0.as_bytes()[..],
                &&receipt.patch.0.as_bytes()[..],
                &&provenance.digest().as_bytes()[..],
                &&outcome.digest().as_bytes()[..],
            ],
        )?;
        let event = event_id(&outcome);
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.outbox (organization_id,workspace_id,repository_id,event_id,aggregate_kind,aggregate_id,aggregate_sequence,event_type,schema_version,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,'Repository',$5::text,$6::text::numeric,$7::text,1,$8::bytea)",
            &[
                &&event.as_bytes()[..],
                &scope.repository.as_str(),
                &receipt.revision.0.to_string(),
                &EVENT_TYPE,
                &&outcome.digest().as_bytes()[..],
            ],
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.outbox_delivery (organization_id,workspace_id,repository_id,event_id) VALUES ($1::text,$2::text,$3::text,$4::bytea)",
            &[&&event.as_bytes()[..]],
        )?;
    }
    Ok(())
}

fn publish_operations(
    tx: &mut Transaction<'_>,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let scope = &after.scope;
    for (key, operation) in &after.operations {
        if before.operations.contains_key(key) {
            continue;
        }
        let request = &operation.request;
        let principal = &after
            .transactions
            .get(&request.transaction)
            .ok_or(AuthorityFailure::Integrity)?
            .owner;
        let request_digest = request.digest(scope, principal);
        put(
            tx,
            scope,
            request_digest,
            celetixo_engine::engine::CommitRequest::FORMAT,
            &request.canonical_bytes(scope, principal),
        )?;
        let outcome = CommitOutcomeRecord::new(
            scope.clone(),
            request.transaction.clone(),
            request.attempt,
            operation.outcome.clone(),
        )
        .map_err(restore)?;
        put(
            tx,
            scope,
            outcome.digest(),
            CommitOutcomeRecord::FORMAT,
            &outcome.canonical_bytes(),
        )?;
        let kind = if operation.outcome.is_ok() {
            "Accepted"
        } else {
            "Rejected"
        };
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.operations (organization_id,workspace_id,repository_id,operation_kind,idempotency_key,request_digest,transaction_id,attempt_number,outcome_kind,outcome_payload_digest,request_payload_digest) VALUES ($1::text,$2::text,$3::text,'Commit',$4::text,$5::bytea,$6::text,$7::text::numeric,$8,$9::bytea,$5::bytea)",
            &[
                &key.as_str(),
                &&request_digest.as_bytes()[..],
                &request.transaction.as_str(),
                &request.attempt.to_string(),
                &kind,
                &&outcome.digest().as_bytes()[..],
            ],
        )?;
    }
    Ok(())
}

fn publish_terminal(
    tx: &mut Transaction<'_>,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let scope = &after.scope;
    let repo = after
        .repository
        .as_ref()
        .ok_or(AuthorityFailure::Integrity)?;
    for (id, transaction) in &after.transactions {
        if before.transactions.get(id).is_some_and(|old| {
            matches!(
                old.state,
                TransactionState::Committed | TransactionState::Aborted
            )
        }) {
            continue;
        }
        let payload = match transaction.state {
            TransactionState::Aborted => reason(scope, id, transaction)?
                .ok_or(AuthorityFailure::Integrity)?
                .digest(),
            TransactionState::Committed => accepted(
                scope,
                &repo
                    .history
                    .iter()
                    .find(|change| change.receipt.transaction == *id)
                    .ok_or(AuthorityFailure::Integrity)?
                    .receipt,
            )?
            .digest(),
            _ => continue,
        };
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.transaction_outcomes (organization_id,workspace_id,repository_id,transaction_id,attempt_number,terminal_state,outcome_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6,$7::bytea)",
            &[
                &id.as_str(),
                &transaction
                    .attempts
                    .last()
                    .ok_or(AuthorityFailure::Integrity)?
                    .number
                    .to_string(),
                &state_name(transaction.state),
                &&payload.as_bytes()[..],
            ],
        )?;
    }
    Ok(())
}

fn publish_heads(
    tx: &mut Transaction<'_>,
    generation: u64,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    let scope = &after.scope;
    let repo = after
        .repository
        .as_ref()
        .ok_or(AuthorityFailure::Integrity)?;
    let snapshot = repo
        .registered
        .get(&repo.current)
        .ok_or(AuthorityFailure::Integrity)?;
    let model = EngineeringModelRecord::new(snapshot, repo.model.clone())
        .map_err(|_| AuthorityFailure::Integrity)?;
    if before.repository.as_ref() != Some(repo) {
        put(
            tx,
            scope,
            model.digest(),
            EngineeringModelRecord::FORMAT,
            &model.canonical_bytes(),
        )?;
    }
    if let Some(old) = &before.repository {
        let old_snapshot = old
            .registered
            .get(&old.current)
            .ok_or(AuthorityFailure::Integrity)?;
        if old.current != repo.current {
            if repo.history.len() != old.history.len() + 1
                || snapshot.revision().0
                    != old_snapshot
                        .revision()
                        .0
                        .checked_add(1)
                        .ok_or(AuthorityFailure::CounterExhausted)?
            {
                return Err(AuthorityFailure::Protocol);
            }
            exactly_one(execute(
                tx,
                scope,
                &format!(
                    "UPDATE celetixo.repository_heads SET revision=$6::text::numeric,snapshot_digest=$7::bytea,head_generation=$8::text::numeric WHERE {FILTER} AND revision=$4::text::numeric AND snapshot_digest=$5::bytea"
                ),
                &[
                    &old_snapshot.revision().0.to_string(),
                    &&old.current.0.as_bytes()[..],
                    &snapshot.revision().0.to_string(),
                    &&repo.current.0.as_bytes()[..],
                    &generation.to_string(),
                ],
            )?)?;
        }
        if old.current != repo.current || old.model != repo.model {
            let previous = EngineeringModelRecord::new(old_snapshot, old.model.clone())
                .map_err(|_| AuthorityFailure::Integrity)?;
            exactly_one(execute(
                tx,
                scope,
                &format!(
                    "UPDATE celetixo.engine_model_heads SET snapshot_digest=$7::bytea,model_generation=$8::text::numeric,model_payload_digest=$9::bytea WHERE {FILTER} AND snapshot_digest=$4::bytea AND model_generation=$5::text::numeric AND model_payload_digest=$6::bytea AND runtime_profile=$10"
                ),
                &[
                    &&old.current.0.as_bytes()[..],
                    &old.model.generation.to_string(),
                    &&previous.digest().as_bytes()[..],
                    &&repo.current.0.as_bytes()[..],
                    &repo.model.generation.to_string(),
                    &&model.digest().as_bytes()[..],
                    &PROFILE,
                ],
            )?)?;
        }
    } else {
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.repository_heads (organization_id,workspace_id,repository_id,revision,snapshot_digest,head_generation) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::bytea,$6::text::numeric)",
            &[
                &snapshot.revision().0.to_string(),
                &&repo.current.0.as_bytes()[..],
                &generation.to_string(),
            ],
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.engine_model_heads (organization_id,workspace_id,repository_id,initial_snapshot_digest,snapshot_digest,model_generation,model_payload_digest,model_kind,runtime_profile) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::bytea,$6::text::numeric,$7::bytea,'Reference',$8)",
            &[
                &&repo.initial.0.as_bytes()[..],
                &&repo.current.0.as_bytes()[..],
                &repo.model.generation.to_string(),
                &&model.digest().as_bytes()[..],
                &PROFILE,
            ],
        )?;
    }
    Ok(())
}
