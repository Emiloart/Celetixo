use super::*;
use celetixo_engine::engine::{EngineError, SourceAttemptState, SourceRejectionView};
use celetixo_engine::source_evidence::{SOURCE_EVIDENCE_FORMAT, SourceEvidenceBinding};

pub(in crate::authority) fn load(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    limits: NativeRecordLimits,
) -> SqlResult<SourceRecords> {
    let mut loader = Loader::new(scope, limits);
    let mut records = SourceRecords::empty(scope.clone());
    records.base = native::load_source_base(tx, &mut loader)?;
    definitions::catalogs(tx, &mut loader, &mut records)?;
    definitions::grants(tx, &mut loader, &mut records)?;
    // Admission reconstruction validates the registered revision chain. Load
    // source publication history before replaying any historical admission.
    super::publication::load(tx, &mut loader, &mut records)?;
    operations(tx, &mut loader, &mut records)?;
    attempts(tx, &mut loader, &mut records)?;
    rejections(tx, &mut loader, &mut records)?;
    evidence(tx, &mut loader, &mut records)?;
    unprepared_commits(tx, &mut loader, &mut records)?;
    super::revalidation::load(tx, &mut loader, &records)?;
    records.check_recovery_limits(limits).map_err(restore)?;
    artifacts(tx, &mut loader, &records)?;
    let generations = loader.rows(
        tx,
        "repositories",
        "state_generation::text",
        "repository_id",
    )?;
    if number(generations.first().ok_or(AuthorityFailure::Integrity)?, 0)? == 0
        && records != SourceRecords::empty(scope.clone())
    {
        return Err(AuthorityFailure::Integrity);
    }
    // This constructor recomputes Engine semantics, patch/plan/evidence bindings,
    // events, grants and lease history. Structural SQL loading cannot admit state.
    AuthorityState::from_source_records(records.clone(), limits).map_err(restore)?;
    super::super::events::load(tx, &mut loader, &records.base, Some(&records))?;
    Ok(records)
}

fn operations(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    let mut contexts = BTreeMap::new();
    for row in loader.rows(tx,"source_operation_contexts","idempotency_key::text,transaction_id::text,owner_grant_id::text,recorded_at_ms::text,fence_generation::text,lease_event_count::text,observed_analysis::bytea,observed_policy::bytea,observed_model_generation::text,operation_kind,owner_grant_kind","idempotency_key")? {
        if row.get::<_,String>(9)!="SourceAdmission"||row.get::<_,String>(10)!="session" {return Err(AuthorityFailure::Integrity);}
        let key=IdempotencyKey::new(row.get::<_,String>(0)).map_err(|_|AuthorityFailure::Integrity)?;
        contexts.insert(key,(transaction(&row,1)?,number(&row,2)?,number(&row,3)?,optional_number(&row,4)?,number(&row,5)?,optional_digest(&row,6)?,optional_digest(&row,7)?,number(&row,8)?));
    }
    for row in loader.rows(tx,"operations","operation_kind,idempotency_key::text,request_digest::bytea,transaction_id::text,attempt_number::text,outcome_kind,outcome_payload_digest::bytea,request_payload_digest::bytea","operation_kind,idempotency_key")? {
        if row.get::<_,String>(0)!="SourceAdmission" {return Err(AuthorityFailure::UnsupportedProfile);}
        let key=IdempotencyKey::new(row.get::<_,String>(1)).map_err(|_|AuthorityFailure::Integrity)?;
        let transaction=transaction(&row,3)?;let target_attempt=number(&row,4)?;
        let payload=optional_digest(&row,7)?.ok_or(AuthorityFailure::Integrity)?;
        let bytes=loader.payload(tx,payload,SourceRequestRecord::FORMAT)?;
        let request=restore_source_request(loader.scope,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        if request.request_digest()!=digest(&row,2)?||request.patch().binding().transaction!=transaction {return Err(AuthorityFailure::Integrity);}
        let outcome_id=digest(&row,6)?;
        let bytes=loader.payload(tx,outcome_id,SourceOutcomeRecord::FORMAT)?;
        let outcome=restore_source_outcome(&SourceOutcomeContext {scope:loader.scope.clone(),transaction:transaction.clone(),target_attempt,key:key.clone(),request:request.request_digest()},outcome_id,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        if row.get::<_,String>(5)!=if outcome.outcome().is_ok(){"Accepted"}else{"Rejected"} {return Err(AuthorityFailure::Integrity);}
        let (observed_tx,owner_grant,recorded_at_ms,fence,lease_event_count,observed_analysis,observed_policy,observed_model_generation)=contexts.remove(&key).ok_or(AuthorityFailure::Integrity)?;
        if observed_tx!=transaction {return Err(AuthorityFailure::Integrity);}
        records.operations.insert(key,SourceOperationRecord {request,target_attempt,owner_grant,recorded_at_ms,fence,lease_event_count,observed_analysis,observed_policy,observed_model_generation,outcome:outcome.outcome().clone()});
    }
    if !contexts.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}

fn attempts(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    let mut contexts = BTreeMap::new();
    for row in loader.rows(tx,"source_admission_contexts","source_attempt_id::text,transaction_id::text,request_payload_digest::bytea,admission_fence::text,lease_event_count::text","source_attempt_id")? {
        contexts.insert(source_attempt(&row,0)?,(transaction(&row,1)?,digest(&row,2)?,number(&row,3)?,number(&row,4)?));
    }
    for row in loader.rows(tx,"source_attempts","source_attempt_id::text,transaction_id::text,transaction_attempt::text,admission_digest::bytea,admission_payload_digest::bytea,patch_digest::bytea,patch_payload_digest::bytea,application_payload_digest::bytea,owner_grant_id::text,executor_grant_id::text,state,owner_grant_kind,executor_grant_kind","source_attempt_id")? {
        let id=source_attempt(&row,0)?;let transaction=transaction(&row,1)?;let attempt=number(&row,2)?;
        let (context_tx,request_id,admission_fence,admission_lease_event_count)=contexts.remove(&id).ok_or(AuthorityFailure::Integrity)?;
        if context_tx!=transaction||row.get::<_,String>(11)!="session"||row.get::<_,String>(12)!="executor" {return Err(AuthorityFailure::Integrity);}
        let bytes=loader.payload(tx,request_id,SourceRequestRecord::FORMAT)?;
        let request=restore_source_request(loader.scope,request_id,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        let binding=request.patch().binding();let admission=SourceAdmissionId(digest(&row,3)?);
        if binding.source_attempt!=id||binding.transaction!=transaction||binding.transaction_attempt!=attempt||admission.0!=digest(&row,4)?||request.patch().id().0!=digest(&row,5)?||digest(&row,5)?!=digest(&row,6)? {return Err(AuthorityFailure::Integrity);}
        let patch=loader.payload(tx,request.patch().id().0,"celetixo/source-patch/v1")?;
        if patch!=request.patch().canonical_bytes(){return Err(AuthorityFailure::Integrity);}
        let owner_grant=number(&row,8)?;
        let policy=records.attempt_contexts.get(&(transaction.clone(),attempt)).and_then(|c|c.policy).ok_or(AuthorityFailure::Integrity)?;
        let bytes=loader.payload(tx,admission.0,SourceAdmissionRecord::FORMAT)?;
        let admission=records.restore_admission(SourceAdmissionRecordInput {request:&request,owner_grant,policy,expected:admission,bytes:&bytes,decode_limits:loader.budget.decode_limits()},loader.budget.limits).map_err(restore)?;
        let plan=loader.payload(tx,admission.plan().id().0,VerificationPlanRecord::FORMAT)?;
        if plan!=admission.plan().canonical_bytes(){return Err(AuthorityFailure::Integrity);}
        let report=digest(&row,7)?;
        if report!=admission.application_report(){return Err(AuthorityFailure::Integrity);}
        // Replay uses the Engine-checked admission; compare the independently
        // persisted report preimage without promoting its candidate to a base.
        let base=records.base.repository.as_ref().and_then(|r|r.registered.get(&binding.base_snapshot)).ok_or(AuthorityFailure::Integrity)?;
        let application=request.patch().apply(base).map_err(|_|AuthorityFailure::Integrity)?;
        let report_bytes=loader.payload(tx,report,"celetixo/source-application-report/v1")?;
        if report_bytes!=application.report().canonical_bytes(){return Err(AuthorityFailure::Integrity);}
        records.attempts.insert(id,SourceAttemptMetadata {admission,patch:request.patch().clone(),options:request.options().clone(),admission_fence,admission_lease_event_count,state:source_state(&row.get::<_,String>(10))?,owner_grant,executor_grant:optional_number(&row,9)?,evidence:None,events:vec![]});
    }
    if !contexts.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    for row in loader.rows(tx,"source_attempt_events","source_attempt_id::text,event_sequence::text,at_ms::text,from_state,to_state,payload_digest::bytea","source_attempt_id,event_sequence")? {
        let id=source_attempt(&row,0)?;let sequence=number(&row,1)?;let payload=digest(&row,5)?;
        let bytes=loader.payload(tx,payload,SourceEventKind::Attempt.format())?;
        let event=restore_source_event(SourceEventKind::Attempt,loader.scope,&id,sequence,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        if event.event().at_ms!=number(&row,2)?||event.from_label()!=row.get::<_,String>(3)||event.to_label()!=row.get::<_,String>(4){return Err(AuthorityFailure::Integrity);}
        records.attempts.get_mut(&id).ok_or(AuthorityFailure::Integrity)?.events.push(event.event().clone());
    }
    let mut terminal = BTreeMap::new();
    for row in loader.rows(
        tx,
        "source_attempt_outcomes",
        "source_attempt_id::text,terminal_state,payload_digest::bytea",
        "source_attempt_id",
    )? {
        terminal.insert(
            source_attempt(&row, 0)?,
            (source_state(&row.get::<_, String>(1))?, digest(&row, 2)?),
        );
    }
    for (id, a) in &records.attempts {
        if a.state.is_terminal() {
            let last = a.events.last().ok_or(AuthorityFailure::Integrity)?;
            let fact = SourceEventRecord::new(
                SourceEventKind::Attempt,
                loader.scope.clone(),
                id.clone(),
                last.clone(),
            )
            .map_err(restore)?;
            if terminal.remove(id) != Some((a.state, fact.digest())) {
                return Err(AuthorityFailure::Integrity);
            }
        }
    }
    if !terminal.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}

fn rejections(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    let mut contexts = BTreeMap::new();
    for row in loader.rows(tx,"source_rejection_contexts","source_attempt_id::text,transaction_id::text,resolved_attempt::text,operation_kind,idempotency_key::text","source_attempt_id")? {
        if row.get::<_,Option<String>>(3).as_deref()!=Some("SourceAdmission"){return Err(AuthorityFailure::UnsupportedProfile);}
        let key=IdempotencyKey::new(row.get::<_,Option<String>>(4).ok_or(AuthorityFailure::Integrity)?).map_err(|_|AuthorityFailure::Integrity)?;
        contexts.insert(source_attempt(&row,0)?,(transaction(&row,1)?,number(&row,2)?,key));
    }
    for row in loader.rows(tx,"source_rejections","source_attempt_id::text,transaction_id::text,transaction_attempt::text,request_digest::bytea,rejection_payload_digest::bytea","source_attempt_id")? {
        let id=source_attempt(&row,0)?;let transaction=transaction(&row,1)?;let submitted=number(&row,2)?;
        let (resolved_tx,resolved,key)=contexts.remove(&id).ok_or(AuthorityFailure::Integrity)?;
        let o=records.operations.get(&key).ok_or(AuthorityFailure::Integrity)?;
        if resolved_tx!=transaction||o.target_attempt!=resolved||o.request.patch().binding().transaction!=transaction||o.request.patch().binding().transaction_attempt!=submitted||o.request.patch().binding().source_attempt!=id||o.request.request_digest()!=digest(&row,3)? {return Err(AuthorityFailure::Integrity);}
        let outcome=SourceOutcomeRecord::new(SourceOutcomeContext {scope:loader.scope.clone(),transaction:transaction.clone(),target_attempt:resolved,key,request:o.request.request_digest()},o.outcome.clone()).map_err(restore)?;
        let payload=digest(&row,4)?;
        if payload!=outcome.digest()||loader.payload(tx,payload,SourceOutcomeRecord::FORMAT)?!=outcome.canonical_bytes(){return Err(AuthorityFailure::Integrity);}
        let error=o.outcome.clone().err().ok_or(AuthorityFailure::Integrity)?;
        let owner=records.base.grants.get(&o.owner_grant).ok_or(AuthorityFailure::Integrity)?.principal.clone();
        records.rejections.insert(id.clone(),SourceRejectionView {scope:loader.scope.clone(),transaction,transaction_attempt:submitted,source_attempt:id,owner,request:o.request.request_digest(),error,rejected_at_ms:o.recorded_at_ms,events:vec![]});
    }
    if !contexts.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    for row in loader.rows(tx,"source_rejection_events","source_attempt_id::text,event_sequence::text,at_ms::text,from_state,to_state,payload_digest::bytea","source_attempt_id,event_sequence")? {
        let id=source_attempt(&row,0)?;let sequence=number(&row,1)?;let payload=digest(&row,5)?;
        let bytes=loader.payload(tx,payload,SourceEventKind::Rejection.format())?;
        let event=restore_source_event(SourceEventKind::Rejection,loader.scope,&id,sequence,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        if event.event().at_ms!=number(&row,2)?||event.from_label()!=row.get::<_,String>(3)||event.to_label()!=row.get::<_,String>(4){return Err(AuthorityFailure::Integrity);}
        records.rejections.get_mut(&id).ok_or(AuthorityFailure::Integrity)?.events.push(event.event().clone());
    }
    Ok(())
}

fn evidence(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    let mut receipts = BTreeMap::new();
    for row in loader.rows(tx,"evidence_receipts","evidence_digest::bytea,source_attempt_id::text,registration_id::text,assessment,envelope_object_digest::bytea,receipt_payload_digest::bytea,recorded_at_ms::text,envelope_reference_digest::bytea","evidence_digest")? {
        receipts.insert(SourceEvidenceId(digest(&row,0)?),(source_attempt(&row,1)?,number(&row,2)?,row.get::<_,String>(3),digest(&row,4)?,digest(&row,5)?,number(&row,6)?,optional_digest(&row,7)?.ok_or(AuthorityFailure::Integrity)?));
    }
    for row in loader.rows(tx,"evidence","evidence_digest::bytea,evidence_kind,transaction_id::text,attempt_number::text,evidence_payload_digest::bytea,verifier_grant_kind,verifier_grant_id::text","evidence_digest")? {
        let id=SourceEvidenceId(digest(&row,0)?);
        if row.get::<_,String>(1)!="SourceV2"||row.get::<_,String>(5)!="executor"||digest(&row,4)?!=id.0 {return Err(AuthorityFailure::UnsupportedProfile);}
        let (attempt_id,registration,assessment,envelope,payload,at,envelope_ref)=receipts.remove(&id).ok_or(AuthorityFailure::Integrity)?;
        let attempt=records.attempts.get(&attempt_id).ok_or(AuthorityFailure::Integrity)?;
        let executor=number(&row,6)?;
        let principal=&records.executors.get(&executor).ok_or(AuthorityFailure::Integrity)?.principal;
        let b=attempt.admission.binding();
        if b.transaction!=transaction(&row,2)?||b.transaction_attempt!=number(&row,3)?||attempt.executor_grant!=Some(executor){return Err(AuthorityFailure::Integrity);}
        let bytes=loader.payload(tx,id.0,SOURCE_EVIDENCE_FORMAT)?;
        let evidence=restore_source_evidence(&SourceEvidenceBinding::for_admission(&attempt.admission,principal.clone()),id,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        let bytes=loader.payload(tx,payload,SourceEvidenceReceiptRecord::FORMAT)?;
        let receipt=restore_source_evidence_receipt(loader.scope,registration,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        let expected_assessment=match receipt.receipt().assessment {celetixo_engine::source_evidence::SourceEvidenceAssessment::Passed=>"Passed",celetixo_engine::source_evidence::SourceEvidenceAssessment::Failed {..}=>"Failed",celetixo_engine::source_evidence::SourceEvidenceAssessment::Incomplete {..}=>"Incomplete"};
        if receipt.receipt().evidence!=id||receipt.receipt().admission!=attempt.admission.id()||receipt.receipt().recorded_at_ms!=at||receipt.receipt().envelope.content!=envelope||receipt.envelope().digest()!=envelope_ref||assessment!=expected_assessment {return Err(AuthorityFailure::Integrity);}
        records.attempts.get_mut(&attempt_id).ok_or(AuthorityFailure::Integrity)?.evidence=Some(id);
        records.evidence.insert(id,SourceEvidenceMetadata {evidence,receipt:receipt.receipt().clone(),store_registration:registration});
    }
    if !receipts.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}
fn unprepared_commits(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    for row in loader.rows(tx,"source_unprepared_commits","idempotency_key::text,transaction_id::text,attempt_number::text,outcome_payload_digest::bytea","idempotency_key")? {
        let key=IdempotencyKey::new(row.get::<_,String>(0)).map_err(|_|AuthorityFailure::Integrity)?;
        let transaction=transaction(&row,1)?;let attempt=number(&row,2)?;let payload=digest(&row,3)?;
        let bytes=loader.payload(tx,payload,CommitOutcomeRecord::FORMAT)?;
        let outcome=restore_commit_outcome(loader.scope,&transaction,attempt,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        if outcome.outcome()!=&Err(EngineError::SourceMutationUnsupported){return Err(AuthorityFailure::Integrity);}
        records.unprepared_commits.insert(key,SourceUnpreparedCommitRecord {transaction,attempt,outcome:outcome.outcome().clone()});
    }
    Ok(())
}
fn artifacts(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &SourceRecords,
) -> SqlResult<()> {
    let mut contents = BTreeMap::new();
    let mut locations = BTreeSet::new();
    let mut references = BTreeMap::new();
    let mut associations = BTreeMap::new();
    for (evidence_id, e) in &records.evidence {
        let mut objects = vec![("Envelope", None, e.receipt.envelope.clone())];
        objects.extend(
            e.evidence
                .provenance()
                .values()
                .cloned()
                .map(|r| ("Provenance", None, r)),
        );
        for (check, record) in e.evidence.checks() {
            objects.extend(
                record
                    .input()
                    .artifacts
                    .values()
                    .cloned()
                    .map(|r| ("Check", Some(check.clone()), r)),
            );
        }
        loader.budget.entries(objects.len())?;
        for (sequence, (role, check, reference)) in objects.into_iter().enumerate() {
            let value = SourceArtifactReferenceRecord::new(
                loader.scope.clone(),
                e.store_registration,
                reference,
            )
            .map_err(restore)?;
            loader.budget.value(value.canonical_bytes().len())?;
            let object = value.reference();
            if contents
                .insert(object.content, object.bytes)
                .is_some_and(|old| old != object.bytes)
            {
                return Err(AuthorityFailure::Integrity);
            }
            locations.insert((e.store_registration, object.content));
            associations.insert(
                (*evidence_id, sequence as u64),
                (
                    role.to_owned(),
                    check,
                    object.name.clone(),
                    e.store_registration,
                    value.digest(),
                ),
            );
            let key = (e.store_registration, value.digest());
            if references
                .insert(key, value.clone())
                .is_some_and(|old| old != value)
            {
                return Err(AuthorityFailure::Integrity);
            }
        }
    }
    for row in loader.rows(
        tx,
        "artifact_contents",
        "object_digest::bytea,byte_length::text",
        "object_digest",
    )? {
        if contents.remove(&digest(&row, 0)?) != Some(number(&row, 1)?) {
            return Err(AuthorityFailure::Integrity);
        }
    }
    for row in loader.rows(
        tx,
        "artifact_locations",
        "registration_id::text,object_digest::bytea,locator_kind,locator_payload_digest::bytea",
        "registration_id,object_digest",
    )? {
        if row.get::<_, String>(2) != "ContentAddressed"
            || optional_digest(&row, 3)?.is_some()
            || !locations.remove(&(number(&row, 0)?, digest(&row, 1)?))
        {
            return Err(AuthorityFailure::Integrity);
        }
    }
    for row in loader.rows(tx,"artifact_references","registration_id::text,reference_digest::bytea,object_digest::bytea,reference_name,media_type,retention_digest::bytea,payload_digest::bytea","registration_id,reference_digest")? {
        let registration=number(&row,0)?;let id=digest(&row,1)?;
        let reference=references.remove(&(registration,id)).ok_or(AuthorityFailure::Integrity)?;
        let expected=reference.reference();
        if digest(&row,2)?!=expected.content||row.get::<_,String>(3)!=expected.name||row.get::<_,String>(4)!=expected.media_type||digest(&row,5)?!=expected.retention_policy||digest(&row,6)?!=id {return Err(AuthorityFailure::Integrity);}
        let bytes=loader.payload(tx,id,SourceArtifactReferenceRecord::FORMAT)?;
        if restore_source_artifact_reference(loader.scope,registration,id,&bytes,loader.budget.decode_limits()).map_err(restore)?!=reference {return Err(AuthorityFailure::Integrity);}
    }
    for row in loader.rows(tx,"source_evidence_objects","evidence_digest::bytea,object_sequence::text,object_role,check_id::text,reference_name,registration_id::text,reference_digest::bytea,evidence_kind","evidence_digest,object_sequence")? {
        let check=row.get::<_,Option<String>>(3).map(|s|celetixo_engine::ids::CheckId::new(s).map_err(|_|AuthorityFailure::Integrity)).transpose()?;
        let association=(row.get::<_,String>(2),check,row.get::<_,String>(4),number(&row,5)?,digest(&row,6)?);
        if row.get::<_,String>(7)!="SourceV2"||associations.remove(&(SourceEvidenceId(digest(&row,0)?),number(&row,1)?))!=Some(association){return Err(AuthorityFailure::Integrity);}
    }
    if !contents.is_empty()
        || !locations.is_empty()
        || !references.is_empty()
        || !associations.is_empty()
    {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}
fn source_state(value: &str) -> SqlResult<SourceAttemptState> {
    use SourceAttemptState::*;
    Ok(match value {
        "Proposed" => Proposed,
        "Admitted" => Admitted,
        "Patching" => Patching,
        "PatchApplied" => PatchApplied,
        "Verifying" => Verifying,
        "EvidenceSealed" => EvidenceSealed,
        "Rejected" => Rejected,
        "PatchConflict" => PatchConflict,
        "ExecutionFailed" => ExecutionFailed,
        "TimedOut" => TimedOut,
        "Cancelled" => Cancelled,
        "EvidenceIncomplete" => EvidenceIncomplete,
        "RevalidationRequired" => RevalidationRequired,
        _ => return Err(AuthorityFailure::Integrity),
    })
}
