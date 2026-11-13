//! Immutable original mutation replies. SQL projects checked Engine records;
//! historical replies never resolve a capability or execute a mutation.
use super::*;
use celetixo_engine::engine::{
    AdministrativeResource, MutationActor, MutationKey, MutationRequest, MutationValue,
    RepositoryMutation,
};
use celetixo_engine::ids::{IdempotencyKey, TransactionId};
use celetixo_engine::persistence::{
    ImpactAnalysisRecord, MutationOutcomeRecord, restore_mutation_outcome, restore_mutation_request,
};
use native::{Loader, SqlResult, optional_digest, optional_number};
use postgres::types::ToSql;

fn kind(request: &MutationRequest) -> &'static str {
    match request {
        MutationRequest::Administrative(request) => request.kind(),
        MutationRequest::Repository(request) => match request.as_ref() {
            RepositoryMutation::InstallPolicy { .. } => "InstallPolicy",
            RepositoryMutation::RegisterRepository { .. } => "RegisterRepository",
            RepositoryMutation::InstallAnalysis { .. } => "InstallAnalysis",
            RepositoryMutation::Register { .. } => "Register",
            RepositoryMutation::UpdateModel { .. } => "UpdateModel",
        },
        MutationRequest::SubmitPatch { .. } => "SubmitPatch",
        MutationRequest::RecordEvidence { .. } => "RecordEvidence",
        MutationRequest::Revalidate { .. } => "Revalidate",
        MutationRequest::Propose { .. } => "Propose",
        MutationRequest::Analyze { .. } => "Analyze",
        MutationRequest::Reserve { .. } => "Reserve",
        MutationRequest::Renew { .. } => "Renew",
        MutationRequest::Execute { .. } => "Execute",
        MutationRequest::Abort { .. } => "Abort",
    }
}

pub(super) fn load(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut NativeRecords,
) -> SqlResult<()> {
    for row in loader.rows(tx, "engine_mutations",
        "actor_kind,grant_kind,actor_principal_id::text,actor_grant_id::text,idempotency_key::text,request_kind,request_digest::bytea,request_payload_digest::bytea,transaction_id::text,recorded_at_ms::text,resolved_transaction_id::text,resolved_attempt::text,state_before,lease_events_before::text,outcome_kind,outcome_payload_digest::bytea,event_sequence::text,result_fence::text,result_expiry_ms::text,result_acquisition_attempt::text,result_kind,result_state,result_attempt::text,result_access_digest::bytea,result_base_snapshot_digest::bytea,result_candidate_snapshot_digest::bytea,result_patch_digest::bytea,result_analysis_digest::bytea,result_analysis_payload_digest::bytea,result_evidence_digest::bytea,result_repository_snapshot_digest::bytea,result_repository_model_digest::bytea,result_repository_model_generation::text,result_policy_digest::bytea,result_policy_model_generation::text,actor_digest::bytea,admin_counter_before::text,admin_target_active_before,administrative_target_id::text,result_resource_kind,result_resource_id::text,result_grant_kind,result_grant_id::text,result_store_registration_id::text",
        "actor_kind,actor_digest,idempotency_key")? {
        let actor_kind = row.get::<_, String>(0);
        let actor = if actor_kind == "Session" && row.get::<_, Option<String>>(1).as_deref() == Some("session") {
            MutationActor::Session {
                principal: PrincipalId::new(row.get::<_, Option<String>>(2).ok_or(AuthorityFailure::Integrity)?).map_err(|_| AuthorityFailure::Integrity)?,
                grant: optional_number(&row, 3)?.ok_or(AuthorityFailure::Integrity)?,
            }
        } else if actor_kind == "EmbeddingAdministrator" && row.get::<_, Option<String>>(1).is_none() && row.get::<_, Option<String>>(2).is_none() && optional_number(&row, 3)?.is_none() {
            MutationActor::EmbeddingAdministrator
        } else {return Err(AuthorityFailure::UnsupportedProfile)};
        if actor.digest() != digest(&row,35)? {return Err(AuthorityFailure::Integrity);}
        let key = MutationKey {
            actor,
            key: IdempotencyKey::new(row.get::<_, String>(4)).map_err(|_| AuthorityFailure::Integrity)?,
        };
        let request_id = digest(&row, 6)?;
        if request_id != digest(&row, 7)? {
            return Err(AuthorityFailure::Integrity);
        }
        let bytes = loader.payload(tx, request_id, MutationRequest::FORMAT)?;
        let request = restore_mutation_request(loader.scope, &key.actor, request_id, &bytes, loader.budget.decode_limits()).map_err(restore)?;
        if row.get::<_, String>(5) != kind(&request)
            || row.get::<_, Option<String>>(8).as_deref() != request.transaction().map(TransactionId::as_str)
        {
            return Err(AuthorityFailure::Integrity);
        }
        let outcome_id = digest(&row, 15)?;
        let bytes = loader.payload(tx, outcome_id, MutationOutcomeRecord::FORMAT)?;
        let outcome = restore_mutation_outcome(loader.scope, &key, &request, outcome_id, &bytes, loader.budget.decode_limits()).map_err(restore)?;
        let record = outcome.record();
        if optional_number(&row,36)? != record.administration.as_ref().map(|context| context.counter_before)
            || row.get::<_,Option<bool>>(37) != record.administration.as_ref().and_then(|context| context.target_active_before)
            || optional_number(&row,38)? != administrative_target(&record.request) {return Err(AuthorityFailure::Integrity);}
        let target = record.resolved_attempt.and_then(|_| request.transaction().map(TransactionId::as_str));
        if number(&row, 9)? != record.recorded_at_ms
            || row.get::<_, Option<String>>(10).as_deref() != target
            || optional_number(&row, 11)? != record.resolved_attempt
            || row.get::<_, Option<String>>(12).as_deref() != record.state_before.map(native::state_name)
            || number(&row, 13)? != record.lease_events_before
        {
            return Err(AuthorityFailure::Integrity);
        }
        let (outcome_kind, sequence, fence, expiry, acquisition) = match record.outcome {
            Ok(MutationValue::Allocation {..} | MutationValue::Revocation {..}) => ("Accepted", None, None, None, None),
            Ok(MutationValue::Policy {..})=>("Accepted",None,None,None,None),
            Ok(MutationValue::Repository {..})=>("Accepted",None,None,None,None),
            Ok(MutationValue::Lease { expires_at_ms, fence, acquisition_attempt }) =>
                ("Accepted", Some(record.lease_events_before), Some(fence), Some(expires_at_ms), Some(acquisition_attempt)),
            Err(_) => ("Rejected", None, None, None, None),
            Ok(MutationValue::State {..})=>("Accepted",None,None,None,None),
            Ok(MutationValue::Analysis {..})=>("Accepted",None,None,None,None),
            Ok(MutationValue::Bindings {..}|MutationValue::Evidence {..})=>("Accepted",None,None,None,None),
        };
        let (result_kind,result_state,result_attempt)=match record.outcome {
            Ok(MutationValue::Allocation {..})=>(Some("Allocation"),None,None),
            Ok(MutationValue::Revocation {..})=>(Some("Revocation"),None,None),
            Ok(MutationValue::Policy {..})=>(Some("Policy"),None,None),
            Ok(MutationValue::Repository {..})=>(Some("Repository"),None,None),
            Ok(MutationValue::Lease {..})=>(Some("Lease"),None,None),
            Ok(MutationValue::State {state,attempt})=>(Some("State"),Some(native::state_name(state)),Some(attempt)),
            Ok(MutationValue::Analysis {attempt,..})=>(Some("Analysis"),Some("Analyzed"),Some(attempt)),
            Ok(MutationValue::Bindings {ref bindings})=>(Some("Bindings"),Some("Validating"),Some(bindings.attempt)),
            Ok(MutationValue::Evidence {..})=>(Some("Evidence"),Some("Validating"),record.resolved_attempt),
            Err(_)=>(None,None,None),
        };
        if row.get::<_, String>(14) != outcome_kind
            || optional_number(&row, 16)? != sequence
            || optional_number(&row, 17)? != fence
            || optional_number(&row, 18)? != expiry
            || optional_number(&row, 19)? != acquisition
            || row.get::<_,Option<String>>(20).as_deref()!=result_kind
            || row.get::<_,Option<String>>(21).as_deref()!=result_state
            || optional_number(&row,22)?!=result_attempt
        {
            return Err(AuthorityFailure::Integrity);
        }
        let expected_access=if let Ok(MutationValue::Analysis {accesses,..})=&record.outcome {Some(accesses.digest())} else {None};
        if optional_digest(&row,23)?!=expected_access {return Err(AuthorityFailure::Integrity);}
        if let Some(id)=expected_access {
            let bytes=loader.payload(tx,id,"celetixo/access-set/v1")?;
            let Ok(MutationValue::Analysis {accesses,..})=&record.outcome else {return Err(AuthorityFailure::Integrity)};
            if bytes!=accesses.canonical_bytes() {return Err(AuthorityFailure::Integrity);}
        }
        let binding=binding_columns(records,record)?;
        let expected_binding=binding.map(|values|values.into_iter().map(Some).collect::<Vec<_>>()).unwrap_or_else(||vec![None;5]);
        for (i,expected) in expected_binding.into_iter().enumerate() {
            if optional_digest(&row,24+i)?!=expected {return Err(AuthorityFailure::Integrity);}
        }
        let expected_evidence=if let Ok(MutationValue::Evidence {evidence})=record.outcome {Some(evidence.0)} else {None};
        if optional_digest(&row,29)?!=expected_evidence {return Err(AuthorityFailure::Integrity);}
        let repository = repository_result(records, record)?;
        if optional_digest(&row,30)? != repository.as_ref().map(|record| record.snapshot().0)
            || optional_digest(&row,31)? != repository.as_ref().map(|record| record.digest())
            || optional_number(&row,32)? != repository.as_ref().map(|record| record.model().generation)
        { return Err(AuthorityFailure::Integrity); }
        if let Some(record) = repository {
            let bytes = loader.payload(tx, record.digest(), celetixo_engine::persistence::EngineeringModelRecord::FORMAT)?;
            if bytes != record.canonical_bytes() { return Err(AuthorityFailure::Integrity); }
        }
        let policy = if let Ok(MutationValue::Policy {policy, model_generation}) = record.outcome {Some((policy, model_generation))} else {None};
        if optional_digest(&row,33)? != policy.map(|(id, _)| id)
            || optional_number(&row,34)? != policy.map(|(_, generation)| generation) { return Err(AuthorityFailure::Integrity); }
        let resource = resource_result(record);
        if row.get::<_,Option<String>>(39).as_deref() != resource.map(|(kind,_)| kind.name())
            || optional_number(&row,40)? != resource.map(|(_,id)| id)
            || row.get::<_,Option<String>>(41).as_deref() != resource.and_then(|(kind,_)| grant_kind(kind))
            || optional_number(&row,42)? != resource.and_then(|(kind,id)| grant_kind(kind).map(|_|id))
            || optional_number(&row,43)? != resource.and_then(|(kind,id)| (kind==AdministrativeResource::ArtifactStore).then_some(id)) {return Err(AuthorityFailure::Integrity);}
        let size = outcome.recovery_size().map_err(restore)?;
        loader.budget.entries(size.entries)?;
        loader.budget.value(size.bytes)?;
        if records.mutations.insert(key, record.clone()).is_some() {
            return Err(AuthorityFailure::Integrity);
        }
    }
    Ok(())
}

pub(super) fn publish(
    tx: &mut Transaction<'_>,
    before: &NativeRecords,
    after: &NativeRecords,
) -> SqlResult<()> {
    if before.scope != after.scope
        || before
            .mutations
            .iter()
            .any(|(key, value)| after.mutations.get(key) != Some(value))
    {
        return Err(AuthorityFailure::Protocol);
    }
    let scope = &after.scope;
    let a = snapshots::scope_parameters(scope);
    for (key, record) in &after.mutations {
        if before.mutations.contains_key(key) {
            continue;
        }
        let (actor_kind, actor_grant_kind, principal, grant) = match &key.actor {
            MutationActor::Session { principal, grant } => (
                "Session",
                Some("session"),
                Some(principal.as_str()),
                Some(grant.to_string()),
            ),
            MutationActor::EmbeddingAdministrator => ("EmbeddingAdministrator", None, None, None),
            _ => return Err(AuthorityFailure::UnsupportedProfile),
        };
        let request_id = record.request.digest(scope, &key.actor);
        semantic::put_payload(
            tx,
            scope,
            request_id,
            MutationRequest::FORMAT,
            &record.request.canonical_bytes(scope, &key.actor),
        )
        .map_err(storage)?;
        let outcome = MutationOutcomeRecord::new(scope.clone(), key.clone(), record.clone())
            .map_err(restore)?;
        semantic::put_payload(
            tx,
            scope,
            outcome.digest(),
            MutationOutcomeRecord::FORMAT,
            &outcome.canonical_bytes(),
        )
        .map_err(storage)?;
        let outcome_id = outcome.digest();
        let resolved_target = record
            .resolved_attempt
            .and_then(|_| record.request.transaction().map(TransactionId::as_str));
        let attempt = record.resolved_attempt.map(|n| n.to_string());
        let state = record.state_before.map(native::state_name);
        let (outcome_kind, sequence, fence, expiry, acquisition) = match record.outcome {
            Ok(MutationValue::Allocation { .. } | MutationValue::Revocation { .. }) => {
                ("Accepted", None, None, None, None)
            }
            Ok(MutationValue::Policy { .. }) => ("Accepted", None, None, None, None),
            Ok(MutationValue::Repository { .. }) => ("Accepted", None, None, None, None),
            Ok(MutationValue::Lease {
                expires_at_ms,
                fence,
                acquisition_attempt,
            }) => (
                "Accepted",
                Some(record.lease_events_before.to_string()),
                Some(fence.to_string()),
                Some(expires_at_ms.to_string()),
                Some(acquisition_attempt.to_string()),
            ),
            Err(_) => ("Rejected", None, None, None, None),
            Ok(MutationValue::State { .. }) => ("Accepted", None, None, None, None),
            Ok(MutationValue::Analysis { .. }) => ("Accepted", None, None, None, None),
            Ok(MutationValue::Bindings { .. } | MutationValue::Evidence { .. }) => {
                ("Accepted", None, None, None, None)
            }
        };
        let (result_kind, result_state, result_attempt) = match record.outcome {
            Ok(MutationValue::Allocation { .. }) => (Some("Allocation"), None, None),
            Ok(MutationValue::Revocation { .. }) => (Some("Revocation"), None, None),
            Ok(MutationValue::Policy { .. }) => (Some("Policy"), None, None),
            Ok(MutationValue::Repository { .. }) => (Some("Repository"), None, None),
            Ok(MutationValue::Lease { .. }) => (Some("Lease"), None, None),
            Ok(MutationValue::State { state, attempt }) => (
                Some("State"),
                Some(native::state_name(state)),
                Some(attempt.to_string()),
            ),
            Ok(MutationValue::Analysis { attempt, .. }) => (
                Some("Analysis"),
                Some("Analyzed"),
                Some(attempt.to_string()),
            ),
            Ok(MutationValue::Bindings { ref bindings }) => (
                Some("Bindings"),
                Some("Validating"),
                Some(bindings.attempt.to_string()),
            ),
            Ok(MutationValue::Evidence { .. }) => (
                Some("Evidence"),
                Some("Validating"),
                record.resolved_attempt.map(|n| n.to_string()),
            ),
            Err(_) => (None, None, None),
        };
        let access_id = if let Ok(MutationValue::Analysis { accesses, .. }) = &record.outcome {
            semantic::put_payload(
                tx,
                scope,
                accesses.digest(),
                "celetixo/access-set/v1",
                &accesses.canonical_bytes(),
            )
            .map_err(storage)?;
            Some(accesses.digest().as_bytes().to_vec())
        } else {
            None
        };
        let binding = binding_columns(after, record)?;
        let projected = binding
            .map(|values| {
                values
                    .into_iter()
                    .map(|id| Some(id.as_bytes().to_vec()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![None; 5]);
        let evidence_id = if let Ok(MutationValue::Evidence { evidence }) = record.outcome {
            Some(evidence.0.as_bytes().to_vec())
        } else {
            None
        };
        let repository = repository_result(after, record)?;
        if let Some(record) = &repository {
            semantic::put_payload(
                tx,
                scope,
                record.digest(),
                celetixo_engine::persistence::EngineeringModelRecord::FORMAT,
                &record.canonical_bytes(),
            )
            .map_err(storage)?;
        }
        let repository_snapshot = repository
            .as_ref()
            .map(|record| record.snapshot().0.as_bytes().to_vec());
        let repository_model = repository
            .as_ref()
            .map(|record| record.digest().as_bytes().to_vec());
        let repository_generation = repository
            .as_ref()
            .map(|record| record.model().generation.to_string());
        let policy = if let Ok(MutationValue::Policy {
            policy,
            model_generation,
        }) = record.outcome
        {
            Some((policy, model_generation))
        } else {
            None
        };
        let policy_id = policy.map(|(id, _)| id.as_bytes().to_vec());
        let policy_generation = policy.map(|(_, generation)| generation.to_string());
        let actor_digest = key.actor.digest();
        let admin_counter = record
            .administration
            .as_ref()
            .map(|context| context.counter_before.to_string());
        let admin_active = record
            .administration
            .as_ref()
            .and_then(|context| context.target_active_before);
        let admin_target = administrative_target(&record.request).map(|id| id.to_string());
        let resource = resource_result(record);
        let resource_kind = resource.map(|(kind, _)| kind.name());
        let resource_id = resource.map(|(_, id)| id.to_string());
        let resource_grant_kind = resource.and_then(|(kind, _)| grant_kind(kind));
        let resource_grant_id =
            resource.and_then(|(kind, id)| grant_kind(kind).map(|_| id.to_string()));
        let resource_store_id = resource.and_then(|(kind, id)| {
            (kind == AdministrativeResource::ArtifactStore).then(|| id.to_string())
        });
        let args: &[&(dyn ToSql + Sync)] = &[
            &a[0],
            &a[1],
            &a[2],
            &principal,
            &grant,
            &key.key.as_str(),
            &kind(&record.request),
            &&request_id.as_bytes()[..],
            &record.request.transaction().map(TransactionId::as_str),
            &record.recorded_at_ms.to_string(),
            &resolved_target,
            &attempt,
            &state,
            &record.lease_events_before.to_string(),
            &outcome_kind,
            &&outcome_id.as_bytes()[..],
            &sequence,
            &fence,
            &expiry,
            &acquisition,
            &result_kind,
            &result_state,
            &result_attempt,
            &access_id,
            &projected[0],
            &projected[1],
            &projected[2],
            &projected[3],
            &projected[4],
            &evidence_id,
            &repository_snapshot,
            &repository_model,
            &repository_generation,
            &policy_id,
            &policy_generation,
            &&actor_digest.as_bytes()[..],
            &actor_kind,
            &actor_grant_kind,
            &admin_counter,
            &admin_active,
            &admin_target,
            &resource_kind,
            &resource_id,
            &resource_grant_kind,
            &resource_grant_id,
            &resource_store_id,
        ];
        let changed = tx.execute(
            "INSERT INTO celetixo.engine_mutations (organization_id,workspace_id,repository_id,actor_kind,grant_kind,actor_principal_id,actor_grant_id,idempotency_key,request_kind,request_digest,request_payload_digest,transaction_id,recorded_at_ms,resolved_transaction_id,resolved_attempt,state_before,lease_events_before,outcome_kind,outcome_payload_digest,event_sequence,result_fence,result_expiry_ms,result_acquisition_attempt,result_kind,result_state,result_attempt,result_access_digest,result_base_snapshot_digest,result_candidate_snapshot_digest,result_patch_digest,result_analysis_digest,result_analysis_payload_digest,result_evidence_digest,result_repository_snapshot_digest,result_repository_model_digest,result_repository_model_generation,result_policy_digest,result_policy_model_generation,actor_digest,admin_counter_before,admin_target_active_before,administrative_target_id,result_resource_kind,result_resource_id,result_grant_kind,result_grant_id,result_store_registration_id) VALUES ($1::text,$2::text,$3::text,$37,$38,$4::text,$5::text::numeric,$6::text,$7,$8::bytea,$8::bytea,$9::text,$10::text::numeric,$11::text,$12::text::numeric,$13,$14::text::numeric,$15,$16::bytea,$17::text::numeric,$18::text::numeric,$19::text::numeric,$20::text::numeric,$21,$22,$23::text::numeric,$24::bytea,$25::bytea,$26::bytea,$27::bytea,$28::bytea,$29::bytea,$30::bytea,$31::bytea,$32::bytea,$33::text::numeric,$34::bytea,$35::text::numeric,$36::bytea,$39::text::numeric,$40,$41::text::numeric,$42,$43::text::numeric,$44,$45::text::numeric,$46::text::numeric)", args).map_err(database)?;
        if changed != 1 {
            return Err(AuthorityFailure::Contention);
        }
    }
    Ok(())
}

fn repository_result(
    records: &NativeRecords,
    record: &celetixo_engine::engine::MutationRecord,
) -> SqlResult<Option<celetixo_engine::persistence::EngineeringModelRecord>> {
    let Ok(MutationValue::Repository { snapshot, model }) = &record.outcome else {
        return Ok(None);
    };
    let base = records
        .repository
        .as_ref()
        .and_then(|repo| repo.registered.get(snapshot))
        .ok_or(AuthorityFailure::Integrity)?;
    celetixo_engine::persistence::EngineeringModelRecord::new(base, *model.clone())
        .map(Some)
        .map_err(|_| AuthorityFailure::Integrity)
}

fn binding_columns(
    records: &NativeRecords,
    record: &celetixo_engine::engine::MutationRecord,
) -> SqlResult<Option<[Digest; 5]>> {
    let Ok(MutationValue::Bindings { bindings }) = &record.outcome else {
        return Ok(None);
    };
    let attempt = records
        .transactions
        .get(&bindings.transaction)
        .and_then(|tx| {
            usize::try_from(bindings.attempt)
                .ok()
                .and_then(|n| tx.attempts.get(n))
        })
        .ok_or(AuthorityFailure::Integrity)?;
    let base = records
        .repository
        .as_ref()
        .and_then(|repo| repo.registered.get(&bindings.base_snapshot))
        .ok_or(AuthorityFailure::Integrity)?;
    let analysis = ImpactAnalysisRecord::new(
        base,
        attempt
            .analysis
            .clone()
            .ok_or(AuthorityFailure::Integrity)?,
    )
    .map_err(restore)?;
    Ok(Some([
        bindings.base_snapshot.0,
        bindings.candidate_snapshot.0,
        bindings.patch.0,
        bindings.analysis,
        analysis.digest(),
    ]))
}

fn administrative_target(request: &MutationRequest) -> Option<u64> {
    use celetixo_engine::engine::AdministrativeMutation as A;
    let MutationRequest::Administrative(request) = request else {
        return None;
    };
    match request.as_ref() {
        A::RevokeSession { grant, .. }
        | A::RevokeExecutor { grant, .. }
        | A::RevokeCleanup { grant, .. } => Some(*grant),
        A::RevokeArtifactStore { registration } => Some(*registration),
        _ => None,
    }
}
fn resource_result(
    record: &celetixo_engine::engine::MutationRecord,
) -> Option<(AdministrativeResource, u64)> {
    match record.outcome {
        Ok(
            MutationValue::Allocation { resource, id } | MutationValue::Revocation { resource, id },
        ) => Some((resource, id)),
        _ => None,
    }
}
fn grant_kind(resource: AdministrativeResource) -> Option<&'static str> {
    match resource {
        AdministrativeResource::Session => Some("session"),
        AdministrativeResource::Executor => Some("executor"),
        AdministrativeResource::Cleanup => Some("cleanup"),
        AdministrativeResource::ArtifactStore => None,
    }
}
