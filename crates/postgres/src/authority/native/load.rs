use super::*;

pub(in crate::authority) fn load(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    limits: NativeRecordLimits,
) -> SqlResult<NativeRecords> {
    let mut loader = Loader::new(scope, limits);
    load_base(tx, &mut loader, LoadProfile::Native)
}

/// Structural shared rows enter only the complete source reconstruction path;
/// they are never passed through the native-only AuthorityState constructor.
pub(in crate::authority) fn load_source_base(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
) -> SqlResult<NativeRecords> {
    load_base(tx, loader, LoadProfile::Source)
}
fn load_base(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    profile: LoadProfile,
) -> SqlResult<NativeRecords> {
    let scope = loader.scope;
    let a = snapshots::scope_parameters(scope);
    let unsupported = if profile == LoadProfile::Native {
        &[
            "policies",
            "policy_heads",
            "artifact_store_registrations",
            "source_attempts",
            "source_attempt_events",
            "source_attempt_outcomes",
            "source_rejections",
            "artifact_objects",
            "evidence_receipts",
            "inbox",
            "source_analyses",
            "source_policy_catalog",
            "source_check_definitions",
            "source_policy_dependencies",
            "source_attempt_contexts",
            "source_attempt_policies",
            "source_admission_contexts",
            "source_operation_contexts",
            "source_rejection_contexts",
            "source_rejection_events",
            "source_unprepared_commits",
            "source_executor_definitions",
            "source_executor_tiers",
            "source_cleanup_definitions",
            "artifact_contents",
            "artifact_locations",
            "artifact_references",
            "source_evidence_objects",
        ][..]
    } else {
        &[
            "artifact_objects",
            "inbox",
            "committed_changes",
            "outbox",
            "outbox_delivery",
            "reference_evidence_admissions",
            "reference_evidence_signers",
            "attempt_checks",
        ][..]
    };
    for table in unsupported {
        let row = tx
            .query_one(
                &format!("SELECT EXISTS(SELECT 1 FROM celetixo.{table} WHERE {FILTER})"),
                &[&a[0], &a[1], &a[2]],
            )
            .map_err(database)?;
        if row.get::<_, bool>(0) {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
    }
    let generations = loader.rows(
        tx,
        "repositories",
        "state_generation::text",
        "repository_id",
    )?;
    let generation = number(generations.first().ok_or(AuthorityFailure::Integrity)?, 0)?;
    let mut records = NativeRecords::empty(scope.clone());
    let counters=loader.rows(tx,"engine_counters","last_clock_ms::text,next_session_grant::text,next_executor_grant::text,next_cleanup_grant::text,next_artifact_store::text","repository_id")?;
    if let Some(row) = counters.first() {
        records.clock_ms = number(row, 0)?;
        records.session_counter = number(row, 1)?;
        if profile == LoadProfile::Native && (2..5).any(|i| row.get::<_, String>(i) != "0") {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
    }
    for row in loader.rows(tx,"grants","grant_id::text,principal_id::text,active,grant_kind,expires_at_ms::text,payload_digest::bytea,generation::text","grant_kind,grant_id")? {
        if profile == LoadProfile::Source && row.get::<_,String>(3)!="session" { continue; }
        if row.get::<_,String>(3)!="session" || optional_number(&row,4)?.is_some() || optional_digest(&row,5)?.is_some() {return Err(AuthorityFailure::UnsupportedProfile);}
        valid_generation(number(&row,6)?,generation)?;
        records.grants.insert(number(&row,0)?,SessionGrantRecord { principal:PrincipalId::new(row.get::<_,String>(1)).map_err(|_|AuthorityFailure::Integrity)?,permissions:BTreeSet::new(),active:row.get(2) });
    }
    for row in loader.rows(
        tx,
        "grant_permissions",
        "grant_id::text,permission,grant_kind",
        "grant_kind,grant_id,permission",
    )? {
        if row.get::<_, String>(2) != "session" {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
        records
            .grants
            .get_mut(&number(&row, 0)?)
            .ok_or(AuthorityFailure::Integrity)?
            .permissions
            .insert(permission(&row.get::<_, String>(1))?);
    }
    for row in loader.rows(
        tx,
        "reservation_fences",
        "generation::text",
        "repository_id",
    )? {
        records.reservation_fence = number(&row, 0)?;
    }
    let heads = loader.rows(
        tx,
        "repository_heads",
        "snapshot_digest::bytea,revision::text,head_generation::text",
        "repository_id",
    )?;
    let models=loader.rows(tx,"engine_model_heads","initial_snapshot_digest::bytea,snapshot_digest::bytea,model_generation::text,model_payload_digest::bytea,model_kind,runtime_profile,analysis_payload_digest::bytea,index_generation::bytea","repository_id")?;
    match (heads.first(), models.first()) {
        (None, None) => {}
        (Some(head), Some(model)) => {
            valid_generation(number(head, 2)?, generation)?;
            let publication=tx.query_opt(&format!("SELECT snapshot_digest::bytea FROM celetixo.authority_publications WHERE {FILTER} AND state_generation=$4::text::numeric"), &[&a[0],&a[1],&a[2],&number(head,2)?.to_string()]).map_err(database)?.ok_or(AuthorityFailure::Integrity)?;
            loader.budget.entries(1)?;
            if optional_digest(&publication, 0)? != Some(digest(head, 0)?) {
                return Err(AuthorityFailure::Integrity);
            }

            let initial = SnapshotId(digest(model, 0)?);
            let current = SnapshotId(digest(head, 0)?);
            let shape_matches = match profile {
                LoadProfile::Native => {
                    model.get::<_, String>(4) == "Reference"
                        && model.get::<_, Option<String>>(5).as_deref() == Some(PROFILE)
                        && optional_digest(model, 6)?.is_none()
                        && optional_digest(model, 7)?.is_none()
                }
                LoadProfile::Source => {
                    model.get::<_, String>(4) == "Source"
                        && model.get::<_, Option<String>>(5).as_deref()
                            == Some("source-execution-v1")
                        && optional_digest(model, 6)?.is_some()
                        && optional_digest(model, 7)?.is_some()
                }
            };
            if digest(model, 1)? != current.0 || !shape_matches {
                return Err(AuthorityFailure::UnsupportedProfile);
            }
            let current_snapshot = loader.snapshot(tx, current)?;
            if current_snapshot.revision().0 != number(head, 1)? {
                return Err(AuthorityFailure::Integrity);
            }
            let model =
                loader.model(tx, &current_snapshot, number(model, 2)?, digest(model, 3)?)?;
            let initial_snapshot = loader.owned_snapshot(tx, initial)?;
            records.repository = Some(RepositoryRecord {
                initial,
                current,
                registered: BTreeMap::from([(initial, initial_snapshot)]),
                model,
                history: vec![],
            });
        }
        _ => return Err(AuthorityFailure::UnsupportedProfile),
    }
    let mut original_bases = BTreeMap::new();
    let mut current_attempts = BTreeMap::new();
    let mut current_fences = BTreeMap::new();
    for row in loader.rows(tx,"transactions","transaction_id::text,owner_principal_id::text,owner_grant_id::text,state,current_attempt::text,original_snapshot_digest::bytea,intent_payload_digest::bytea,current_fence::text,last_reason,last_reason_payload_digest::bytea,owner_grant_kind,generation::text","transaction_id")? {
        valid_generation(number(&row,11)?,generation)?;
        let id=transaction_id(&row,0)?;let current=number(&row,4)?;let original=SnapshotId(digest(&row,5)?);
        if row.get::<_,Option<String>>(8).is_some() || row.get::<_,String>(10)!="session" {return Err(AuthorityFailure::UnsupportedProfile);}
        let base=loader.snapshot(tx,original)?;
        let intent_id=digest(&row,6)?;
        let bytes=loader.payload(tx,intent_id,NativeIntentRecord::FORMAT)?;
        let intent=restore_native_intent(&base,intent_id,&bytes,loader.budget.decode_limits()).map_err(restore)?.intent().clone();
        loader.budget.access(&intent.accesses)?;
        loader.budget.entries(intent.invariants.len()+intent.required_checks.len()+intent.depends_on.len()+intent.required_checks.iter().map(|check|check.command().len()).sum::<usize>())?;
        let last_reason=if let Some(reason)=optional_digest(&row,9)? {
            let bytes=loader.payload(tx,reason,TransactionReasonRecord::FORMAT)?;
            Some(restore_transaction_reason(scope,&id,current,reason,&bytes,loader.budget.decode_limits()).map_err(restore)?.reason().to_owned())
        } else {None};
        original_bases.insert(id.clone(),original);current_attempts.insert(id.clone(),current);current_fences.insert(id.clone(),optional_number(&row,7)?);
        records.transactions.insert(id,TransactionRecord {owner:PrincipalId::new(row.get::<_,String>(1)).map_err(|_|AuthorityFailure::Integrity)?,proposal_grant:number(&row,2)?,intent,state:state(&row.get::<_,String>(3))?,attempts:vec![],lease:None,lease_history:vec![],last_reason});
    }
    for row in loader.rows(tx,"attempts","transaction_id::text,attempt_number::text,base_snapshot_digest::bytea,candidate_snapshot_digest::bytea,model_payload_digest::bytea,analysis_payload_digest::bytea,patch_digest::bytea,patch_payload_digest::bytea,model_generation::text","transaction_id,attempt_number")? {
        let id=transaction_id(&row,0)?;let number=number(&row,1)?;
        let base_id=SnapshotId(digest(&row,2)?);let base=loader.snapshot(tx,base_id)?;
        let generation=optional_number(&row,8)?.ok_or(AuthorityFailure::UnsupportedProfile)?;
        let model=loader.model(tx,&base,generation,digest(&row,4)?)?;
        let analysis=if let Some(analysis)=optional_digest(&row,5)? {
            let bytes=loader.payload(tx,analysis,ImpactAnalysisRecord::FORMAT)?;
            let value=restore_impact_record(&base,generation,analysis,&bytes,loader.budget.decode_limits()).map_err(restore)?.analysis().clone();
            loader.budget.access(value.derived())?;Some(value)
        } else {None};
        let patch=match (optional_digest(&row,6)?,optional_digest(&row,7)?) {
            (None,None)=>None,
            (Some(id),Some(payload)) if id==payload=> {
                let bytes=loader.payload(tx,payload,PATCH_FORMAT)?;
                let patch=restore_reference_patch(&base,PatchId(id),&bytes,loader.budget.decode_limits()).map_err(restore)?;
                loader.budget.entries(patch.changes().len())?;Some(patch)
            },
            _=>return Err(AuthorityFailure::Integrity),
        };
        let candidate=optional_digest(&row,3)?.map(|digest|loader.owned_snapshot(tx,SnapshotId(digest))).transpose()?;
        let transaction=records.transactions.get_mut(&id).ok_or(AuthorityFailure::Integrity)?;
        if number!=transaction.attempts.len() as u64 {return Err(AuthorityFailure::Integrity);}
        transaction.attempts.push(AttemptRecord {number,base:base_id,model,effective:None,analysis,patch,candidate,evidence:BTreeMap::new()});
    }
    for (id, transaction) in &records.transactions {
        if transaction.attempts.first().map(|a| a.base) != original_bases.get(id).copied()
            || transaction.attempts.last().map(|a| a.number) != current_attempts.get(id).copied()
        {
            return Err(AuthorityFailure::Integrity);
        }
    }
    let access_rows = loader.rows(
        tx,
        "access_provenance",
        "transaction_id::text,attempt_number::text,provenance_kind,payload_digest::bytea",
        "transaction_id,attempt_number,provenance_kind",
    )?;
    let mut accesses = BTreeMap::new();
    for row in access_rows {
        let id = transaction_id(&row, 0)?;
        let number = number(&row, 1)?;
        let kind = row.get::<_, String>(2);
        let payload = digest(&row, 3)?;
        let transaction = records
            .transactions
            .get_mut(&id)
            .ok_or(AuthorityFailure::Integrity)?;
        let attempt = transaction
            .attempts
            .get_mut(usize::try_from(number).map_err(|_| AuthorityFailure::Integrity)?)
            .ok_or(AuthorityFailure::Integrity)?;
        if kind == "effective" {
            let bytes = loader.payload(tx, payload, ACCESS_FORMAT)?;
            let effective = restore_access_set(payload, &bytes, loader.budget.decode_limits())
                .map_err(restore)?;
            loader.budget.access(&effective)?;
            attempt.effective = Some(effective);
        }
        if accesses.insert((id, number, kind), payload).is_some() {
            return Err(AuthorityFailure::Integrity);
        }
    }
    for (id, transaction) in &records.transactions {
        for attempt in &transaction.attempts {
            let base = loader.snapshot(tx, attempt.base)?;
            for fact in access_facts(scope, id, transaction, attempt, &base)? {
                if accesses.remove(&(id.clone(), attempt.number, fact.kind.into())) != Some(fact.id)
                {
                    return Err(AuthorityFailure::Integrity);
                }
                if loader.payload(tx, fact.id, fact.format)? != fact.bytes {
                    return Err(AuthorityFailure::Integrity);
                }
            }
        }
    }
    if !accesses.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    if profile == LoadProfile::Native {
        load_evidence(tx, loader, &mut records)?;
    }
    load_leases(tx, loader, &mut records, &current_fences)?;
    if profile == LoadProfile::Native {
        load_commits(tx, loader, &mut records)?;
        load_operations(tx, loader, &mut records)?;
    }
    load_terminal_and_outbox(tx, loader, &records)?;
    if counters.is_empty()
        && (!records.grants.is_empty()
            || records.repository.is_some()
            || !records.transactions.is_empty())
    {
        return Err(AuthorityFailure::Integrity);
    }
    load_publication(tx, loader, generation, &records, profile)?;
    Ok(records)
}

fn load_evidence(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut NativeRecords,
) -> SqlResult<()> {
    let mut origins = BTreeMap::new();
    for row in loader.rows(tx,"evidence","evidence_digest::bytea,evidence_kind,transaction_id::text,attempt_number::text,evidence_payload_digest::bytea,verifier_grant_kind,verifier_grant_id::text","evidence_digest")? {
        if row.get::<_,String>(1)!="Reference" || row.get::<_,String>(5)!="session" {return Err(AuthorityFailure::UnsupportedProfile);}
        let id=EvidenceId(digest(&row,0)?);let transaction=transaction_id(&row,2)?;let attempt=number(&row,3)?;
        let attempt_record=records.transactions.get(&transaction).and_then(|tx|tx.attempts.get(usize::try_from(attempt).ok()?)).ok_or(AuthorityFailure::Integrity)?;
        let base=loader.snapshot(tx,attempt_record.base)?;
        let payload=digest(&row,4)?;let bytes=loader.payload(tx,payload,VerificationEvidenceRecord::FORMAT)?;
        let evidence=restore_verification_record(&base,id,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?.evidence().clone();
        if evidence.bindings().transaction!=transaction || evidence.bindings().attempt!=attempt {return Err(AuthorityFailure::Integrity);}
        loader.budget.entries(evidence.check().command().len()+evidence.artifacts().len()+evidence.diagnostics().len())?;
        let original=number(&row,6)?;
        let grant=records.grants.get(&original).ok_or(AuthorityFailure::Integrity)?;
        if grant.principal!=evidence.provenance().verifier || !grant.permissions.contains(&Permission::Verify) {return Err(AuthorityFailure::Integrity);}
        origins.insert(id,original);
        records.evidence.insert(id,AdmittedEvidenceRecord {evidence,verifier_grant:original});
    }
    let mut admissions = BTreeSet::new();
    for row in loader.rows(tx,"reference_evidence_admissions","evidence_digest::bytea,verifier_grant_id::text,admitted_at_ms::text,evidence_kind,verifier_grant_kind","evidence_digest,verifier_grant_id")? {
        if row.get::<_,String>(3)!="Reference" || row.get::<_,String>(4)!="session" {return Err(AuthorityFailure::UnsupportedProfile);}
        let id=EvidenceId(digest(&row,0)?);let grant_id=number(&row,1)?;
        let evidence=&records.evidence.get(&id).ok_or(AuthorityFailure::Integrity)?.evidence;
        let grant=records.grants.get(&grant_id).ok_or(AuthorityFailure::Integrity)?;
        let admitted=number(&row,2)?;
        if admitted>records.clock_ms || admitted<evidence.provenance().finished_at || grant.principal!=evidence.provenance().verifier || !grant.permissions.contains(&Permission::Verify) {return Err(AuthorityFailure::Integrity);}
        admissions.insert((id,grant_id));
    }
    let mut signers = BTreeSet::new();
    for row in loader.rows(
        tx,
        "reference_evidence_signers",
        "evidence_digest::bytea,verifier_grant_id::text",
        "evidence_digest",
    )? {
        let id = EvidenceId(digest(&row, 0)?);
        let grant = number(&row, 1)?;
        if !admissions.contains(&(id, grant)) {
            return Err(AuthorityFailure::Integrity);
        }
        records
            .evidence
            .get_mut(&id)
            .ok_or(AuthorityFailure::Integrity)?
            .verifier_grant = grant;
        signers.insert(id);
    }
    if records
        .evidence
        .keys()
        .any(|id| !signers.contains(id) || !admissions.contains(&(*id, origins[id])))
    {
        return Err(AuthorityFailure::Integrity);
    }
    for row in loader.rows(
        tx,
        "attempt_checks",
        "transaction_id::text,attempt_number::text,check_id::text,evidence_digest::bytea",
        "transaction_id,attempt_number,check_id",
    )? {
        let id = transaction_id(&row, 0)?;
        let number = number(&row, 1)?;
        let check =
            CheckId::new(row.get::<_, String>(2)).map_err(|_| AuthorityFailure::Integrity)?;
        let evidence = EvidenceId(digest(&row, 3)?);
        records
            .transactions
            .get_mut(&id)
            .and_then(|tx| tx.attempts.get_mut(usize::try_from(number).ok()?))
            .ok_or(AuthorityFailure::Integrity)?
            .evidence
            .insert(check, evidence);
    }
    Ok(())
}

fn load_leases(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut NativeRecords,
    current_fences: &BTreeMap<TransactionId, Option<u64>>,
) -> SqlResult<()> {
    let mut rows = BTreeMap::new();
    for row in loader.rows(tx,"reservations","fence_generation::text,transaction_id::text,attempt_number::text,owner_grant_kind,owner_grant_id::text,acquired_at_ms::text,expires_at_ms::text,state","fence_generation")? {
        if row.get::<_,String>(3)!="session" {return Err(AuthorityFailure::UnsupportedProfile);}
        let fence=number(&row,0)?;
        rows.insert(fence,(transaction_id(&row,1)?,number(&row,2)?,number(&row,4)?,number(&row,5)?,number(&row,6)?,row.get::<_,String>(7)));
    }
    let mut events = BTreeMap::new();
    for row in loader.rows(
        tx,
        "reservation_events",
        "fence_generation::text,event_sequence::text,at_ms::text,event_kind,payload_digest::bytea",
        "fence_generation,event_sequence",
    )? {
        let fence = number(&row, 0)?;
        let sequence = number(&row, 1)?;
        let (id, ..) = rows.get(&fence).ok_or(AuthorityFailure::Integrity)?;
        let payload = digest(&row, 4)?;
        let bytes = loader.payload(tx, payload, LeaseEventRecord::FORMAT)?;
        let event = restore_lease_event(
            loader.scope,
            id,
            sequence,
            payload,
            &bytes,
            loader.budget.decode_limits(),
        )
        .map_err(restore)?
        .event()
        .clone();
        loader.budget.entries(event.scopes.len())?;
        if event.fence != fence
            || event.at != number(&row, 2)?
            || event_name(&event.kind) != row.get::<_, String>(3)
        {
            return Err(AuthorityFailure::Integrity);
        }
        if events.insert((id.clone(), sequence), event).is_some() {
            return Err(AuthorityFailure::Integrity);
        }
    }
    for ((id, sequence), event) in events {
        let history = &mut records
            .transactions
            .get_mut(&id)
            .ok_or(AuthorityFailure::Integrity)?
            .lease_history;
        if sequence != history.len() as u64 {
            return Err(AuthorityFailure::Integrity);
        }
        history.push(event);
    }
    let expected = reservations(records)?;
    if rows.len() != expected.len() {
        return Err(AuthorityFailure::Integrity);
    }
    for (fence, fact) in &expected {
        let row = rows.get(fence).ok_or(AuthorityFailure::Integrity)?;
        if row.0 != fact.transaction
            || row.1 != fact.attempt
            || row.2 != fact.owner_grant
            || row.3 != fact.acquired_at
            || row.4 != fact.expires_at
            || row.5 != fact.state
        {
            return Err(AuthorityFailure::Integrity);
        }
    }
    for (id, fence) in current_fences {
        if let Some(fence) = fence {
            let row = expected.get(fence).ok_or(AuthorityFailure::Integrity)?;
            if row.transaction != *id {
                return Err(AuthorityFailure::Integrity);
            }
            records
                .transactions
                .get_mut(id)
                .ok_or(AuthorityFailure::Integrity)?
                .lease = Some(LeaseRecord {
                fence: *fence,
                owner_grant: row.owner_grant,
                acquired_at: row.acquired_at,
                acquired_attempt: row.attempt,
                expires_at: row.expires_at,
                writes: row.writes.clone(),
            });
        }
    }
    let mut actual_claims = BTreeMap::new();
    for row in loader.rows(
        tx,
        "reservation_claims",
        "element_id::text,fence_generation::text",
        "element_id",
    )? {
        actual_claims.insert(
            ElementId::new(row.get::<_, String>(0)).map_err(|_| AuthorityFailure::Integrity)?,
            number(&row, 1)?,
        );
    }
    if actual_claims != claims(records)? {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}

fn load_commits(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut NativeRecords,
) -> SqlResult<()> {
    for row in loader.rows(tx,"committed_changes","revision::text,transaction_id::text,attempt_number::text,snapshot_digest::bytea,patch_digest::bytea,provenance_payload_digest::bytea,receipt_payload_digest::bytea","revision")? {
        let id=transaction_id(&row,1)?;let number=number(&row,2)?;
        let attempt=records.transactions.get(&id).and_then(|tx|tx.attempts.get(usize::try_from(number).ok()?)).ok_or(AuthorityFailure::Integrity)?;
        let base=loader.snapshot(tx,attempt.base)?;
        let payload=digest(&row,5)?;let bytes=loader.payload(tx,payload,AccessProvenanceRecord::FORMAT)?;
        let provenance=restore_access_provenance(&base,&id,number,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?.provenance().clone();
        let payload=digest(&row,6)?;let bytes=loader.payload(tx,payload,CommitOutcomeRecord::FORMAT)?;
        let receipt=restore_commit_outcome(loader.scope,&id,number,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?.outcome().clone().map_err(|_|AuthorityFailure::Integrity)?;
        if receipt.revision.0!=super::number(&row,0)? || receipt.snapshot.0!=digest(&row,3)? || receipt.patch.0!=digest(&row,4)? {return Err(AuthorityFailure::Integrity);}
        let snapshot=loader.owned_snapshot(tx,receipt.snapshot)?;
        let repo=records.repository.as_mut().ok_or(AuthorityFailure::Integrity)?;
        if repo.registered.insert(snapshot.id(),snapshot).is_some() {return Err(AuthorityFailure::Integrity);}
        repo.history.push(CommittedRecord {receipt,accesses:attempt.effective.clone().ok_or(AuthorityFailure::Integrity)?,provenance});
    }
    Ok(())
}

fn load_operations(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut NativeRecords,
) -> SqlResult<()> {
    for row in loader.rows(tx,"operations","operation_kind,idempotency_key::text,request_digest::bytea,transaction_id::text,attempt_number::text,outcome_kind,outcome_payload_digest::bytea,request_payload_digest::bytea","operation_kind,idempotency_key")? {
        if row.get::<_,String>(0)!="Commit" {return Err(AuthorityFailure::UnsupportedProfile);}
        let key=IdempotencyKey::new(row.get::<_,String>(1)).map_err(|_|AuthorityFailure::Integrity)?;
        let id=transaction_id(&row,3)?;let attempt=number(&row,4)?;
        let principal=&records.transactions.get(&id).ok_or(AuthorityFailure::Integrity)?.owner;
        let expected=digest(&row,2)?;
        if optional_digest(&row,7)?.ok_or(AuthorityFailure::UnsupportedProfile)?!=expected {return Err(AuthorityFailure::Integrity);}
        let bytes=loader.payload(tx,expected,celetixo_engine::engine::CommitRequest::FORMAT)?;
        let request=restore_commit_request(loader.scope,principal,key.clone(),expected,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        loader.budget.entries(request.evidence.len())?;
        let payload=digest(&row,6)?;let bytes=loader.payload(tx,payload,CommitOutcomeRecord::FORMAT)?;
        let outcome=restore_commit_outcome(loader.scope,&id,attempt,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?.outcome().clone();
        if request.transaction!=id || request.attempt!=attempt || row.get::<_,String>(5)!=if outcome.is_ok(){"Accepted"}else{"Rejected"} {return Err(AuthorityFailure::Integrity);}
        records.operations.insert(key,CommitOperationRecord {request,outcome});
    }
    Ok(())
}

fn load_terminal_and_outbox(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &NativeRecords,
) -> SqlResult<()> {
    let mut terminal = BTreeMap::new();
    for row in loader.rows(
        tx,
        "transaction_outcomes",
        "transaction_id::text,attempt_number::text,terminal_state,outcome_payload_digest::bytea",
        "transaction_id",
    )? {
        terminal.insert(
            transaction_id(&row, 0)?,
            (number(&row, 1)?, row.get::<_, String>(2), digest(&row, 3)?),
        );
    }
    let history = records
        .repository
        .as_ref()
        .map(|r| r.history.as_slice())
        .unwrap_or_default();
    for (id, transaction) in &records.transactions {
        let fact = match transaction.state {
            TransactionState::Aborted => {
                let reason =
                    reason(loader.scope, id, transaction)?.ok_or(AuthorityFailure::Integrity)?;
                Some((
                    reason.digest(),
                    TransactionReasonRecord::FORMAT,
                    reason.canonical_bytes(),
                ))
            }
            TransactionState::Committed => {
                let receipt = &history
                    .iter()
                    .find(|change| change.receipt.transaction == *id)
                    .ok_or(AuthorityFailure::Integrity)?
                    .receipt;
                let accepted = accepted(loader.scope, receipt)?;
                Some((
                    accepted.digest(),
                    CommitOutcomeRecord::FORMAT,
                    accepted.canonical_bytes(),
                ))
            }
            _ => None,
        };
        if let Some((id_digest, format, bytes)) = fact {
            let row = terminal.remove(id).ok_or(AuthorityFailure::Integrity)?;
            if row.0
                != transaction
                    .attempts
                    .last()
                    .ok_or(AuthorityFailure::Integrity)?
                    .number
                || row.1 != state_name(transaction.state)
                || row.2 != id_digest
                || loader.payload(tx, id_digest, format)? != bytes
            {
                return Err(AuthorityFailure::Integrity);
            }
        }
    }
    if !terminal.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    let mut events = BTreeMap::new();
    for row in loader.rows(tx,"outbox","event_id::bytea,aggregate_kind,aggregate_id::text,aggregate_sequence::text,event_type::text,schema_version,payload_digest::bytea","aggregate_kind,aggregate_id,aggregate_sequence")? {
        if row.get::<_,String>(1)!="Repository" || row.get::<_,String>(2)!=loader.scope.repository.as_str() || row.get::<_,String>(4)!=EVENT_TYPE || row.get::<_,i32>(5)!=1 {return Err(AuthorityFailure::UnsupportedProfile);}
        events.insert(number(&row,3)?,(digest(&row,0)?,digest(&row,6)?));
    }
    let mut expected_events = BTreeSet::new();
    for change in history {
        let outcome = accepted(loader.scope, &change.receipt)?;
        let event = event_id(&outcome);
        if events.remove(&change.receipt.revision.0) != Some((event, outcome.digest()))
            || loader.payload(tx, outcome.digest(), CommitOutcomeRecord::FORMAT)?
                != outcome.canonical_bytes()
        {
            return Err(AuthorityFailure::Integrity);
        }
        expected_events.insert(event);
    }
    if !events.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    let mut delivery = BTreeSet::new();
    for row in loader.rows(tx, "outbox_delivery", "event_id::bytea", "event_id")? {
        delivery.insert(digest(&row, 0)?);
    }
    if delivery != expected_events {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(())
}

fn valid_generation(value: u64, generation: u64) -> SqlResult<()> {
    if value == 0 || value > generation {
        Err(AuthorityFailure::Integrity)
    } else {
        Ok(())
    }
}

// Publication audit is append-only. Check the latest link, rather than silently
// accepting counters/head state detached from the last atomic publication. Past
// login identities are attribution, not current authentication credentials.
fn load_publication(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    generation: u64,
    records: &NativeRecords,
    profile: LoadProfile,
) -> SqlResult<()> {
    let a = snapshots::scope_parameters(loader.scope);
    let row=tx.query_opt(&format!("SELECT state_generation::text,observed_clock_ms::text,actor_kind,requested_principal_id::text,requested_grant_id::text,requested_permission,snapshot_digest::bytea FROM celetixo.authority_publications AS fact WHERE {FILTER} ORDER BY fact.state_generation DESC LIMIT 1"), &[&a[0],&a[1],&a[2]]).map_err(database)?;
    let Some(row) = row else {
        return if generation == 0 && records == &NativeRecords::empty(loader.scope.clone()) {
            Ok(())
        } else {
            Err(AuthorityFailure::Integrity)
        };
    };
    loader.budget.entries(1)?;
    if number(&row, 0)? != generation
        || number(&row, 1)? != records.clock_ms
        || optional_digest(&row, 6)? != records.repository.as_ref().map(|repo| repo.current.0)
    {
        return Err(AuthorityFailure::Integrity);
    }
    match row.get::<_, String>(2).as_str() {
        "EmbeddingAdministrator" => {
            if row.get::<_, Option<String>>(3).is_some()
                || optional_number(&row, 4)?.is_some()
                || row.get::<_, Option<String>>(5).is_some()
            {
                return Err(AuthorityFailure::Integrity);
            }
        }
        "Session" => {
            // Requested actor is attribution, including rejected requests; it
            // does not establish that the callback authorized this operation.
            PrincipalId::new(
                row.get::<_, Option<String>>(3)
                    .ok_or(AuthorityFailure::Integrity)?,
            )
            .map_err(|_| AuthorityFailure::Integrity)?;
            if optional_number(&row, 4)?.is_none_or(|id| id == 0) {
                return Err(AuthorityFailure::Integrity);
            }
            permission(
                &row.get::<_, Option<String>>(5)
                    .ok_or(AuthorityFailure::Integrity)?,
            )?;
        }
        "SourceExecutor" | "SourceCleanup" if profile == LoadProfile::Source => {
            PrincipalId::new(
                row.get::<_, Option<String>>(3)
                    .ok_or(AuthorityFailure::Integrity)?,
            )
            .map_err(|_| AuthorityFailure::Integrity)?;
            if optional_number(&row, 4)?.is_none_or(|id| id == 0)
                || row.get::<_, Option<String>>(5).is_some()
            {
                return Err(AuthorityFailure::Integrity);
            }
        }
        _ => return Err(AuthorityFailure::Integrity),
    }
    Ok(())
}
