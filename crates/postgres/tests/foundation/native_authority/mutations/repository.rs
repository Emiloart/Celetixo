//! Repository command results persist without fabricated transaction rows.
use super::*;

fn register(base: &Snapshot) -> MutationRequest {
    MutationRequest::Repository(Box::new(RepositoryMutation::Register {
        snapshot: Box::new(base.clone()),
        model: Box::new(native_model(1)),
    }))
}
fn update(generation: u64) -> MutationRequest {
    MutationRequest::Repository(Box::new(RepositoryMutation::UpdateModel {
        model: Box::new(native_model(generation)),
    }))
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn repository_original_replies_survive_registration_model_replacement_and_lost_responses() {
    let selected = scope("mutation-repository-original");
    authorize_registration(&mut owner(), &selected);
    let backend = Arc::new(sql_authority());
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let worker = admin
        .grant(
            PrincipalId::new("worker").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = snapshot(selected.clone(), 0);
    let missing = engine
        .mutate(&worker, &selected, key("missing"), update(2))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    let invalid = engine
        .mutate(&worker, &selected, key("invalid"), update(0))
        .unwrap();
    assert_eq!(invalid.outcome, Err(EngineError::IncompleteAnalysis));
    for (operation, input) in [("register", register(&base)), ("model", update(2))] {
        for stage in [AuthorityStage::StateLoaded, AuthorityStage::BeforeCommit] {
            let (fault, actor) = fresh_worker(&selected, stage);
            let before = backend.inspect_registration(&selected).unwrap();
            assert_eq!(
                fault.mutate(&actor, &selected, key(operation), input.clone()),
                Err(authority_error(AuthorityFailure::Unavailable))
            );
            assert_eq!(backend.inspect_registration(&selected).unwrap(), before);
        }
        let (fault, actor) = fresh_worker(&selected, AuthorityStage::AfterCommit);
        assert_eq!(
            fault.mutate(&actor, &selected, key(operation), input.clone()),
            Err(authority_error(AuthorityFailure::CommitIndeterminate))
        );
        let reply = engine
            .mutate(&worker, &selected, key(operation), input)
            .unwrap();
        assert!(reply.replayed);
        assert!(reply.outcome.is_ok(), "{reply:?}");
    }
    let originals = backend.inspect_registration(&selected).unwrap();
    engine
        .update_model(&worker, &selected, native_model(3))
        .unwrap();
    let (engine, admin) = Engine::with_authority_backend(Arc::new(sql_authority()));
    let worker = admin
        .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
        .unwrap();
    for (key, record) in &originals.records.mutations {
        assert!(record.request.transaction().is_none());
        assert!(record.resolved_attempt.is_none());
        let replay = engine
            .mutate(&worker, &selected, key.key.clone(), record.request.clone())
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, record.outcome);
        assert_eq!(replay.recorded_at_ms, record.recorded_at_ms);
    }
    assert_eq!(
        engine.mutate(&worker, &selected, key("register"), update(4)),
        Err(EngineError::IdempotencyConflict)
    );
    let current = backend.inspect_registration(&selected).unwrap();
    assert_eq!(
        current
            .records
            .repository
            .as_ref()
            .unwrap()
            .model
            .generation,
        3
    );
    assert_eq!(current.records.mutations, originals.records.mutations);
    assert!(current.records.transactions.is_empty());
    let mut client = owner();
    let rows = client.query("SELECT transaction_id::text,resolved_transaction_id::text,resolved_attempt::text,state_before,lease_events_before::text FROM celetixo.engine_mutations WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&selected.organization.as_str(), &selected.workspace.as_str(), &selected.repository.as_str()]).unwrap();
    assert_eq!(rows.len(), 4);
    for row in rows {
        for column in 0..4 {
            assert!(row.get::<_, Option<String>>(column).is_none());
        }
        assert_eq!(row.get::<_, String>(4), "0");
    }
    admin.revoke(&worker).unwrap();
    assert_eq!(
        engine.mutate(&worker, &selected, key("register"), register(&base)),
        Err(EngineError::Unauthorized)
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn repository_reply_rows_bind_model_payloads_snapshot_and_optional_target() {
    let selected = scope("mutation-repository-row-guards");
    authorize_registration(&mut owner(), &selected);
    let (engine, admin) = Engine::with_authority_backend(Arc::new(sql_authority()));
    let worker = admin
        .grant(
            PrincipalId::new("worker").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    engine
        .mutate(
            &worker,
            &selected,
            key("register"),
            register(&snapshot(selected.clone(), 0)),
        )
        .unwrap();
    let columns = "organization_id,workspace_id,repository_id,actor_kind,grant_kind,actor_principal_id,actor_grant_id,actor_digest,idempotency_key,request_kind,request_digest,request_payload_digest,transaction_id,recorded_at_ms,resolved_transaction_id,resolved_attempt,state_before,lease_events_before,outcome_kind,outcome_payload_digest,event_sequence,result_fence,result_expiry_ms,result_acquisition_attempt,result_kind,result_state,result_attempt,result_access_digest,result_base_snapshot_digest,result_candidate_snapshot_digest,result_patch_digest,result_analysis_digest,result_analysis_payload_digest,result_evidence_digest,result_repository_snapshot_digest,result_repository_model_digest,result_repository_model_generation";
    for (variant, expected) in [(0, "23505"), (1, "23514"), (2, "23514")] {
        let select = columns
            .split(',')
            .map(|column| match (variant, column) {
                (_, "idempotency_key") => "'forged-repository-result'::text",
                (1, "transaction_id") => "'invented-transaction'::text",
                (2, "result_repository_model_digest") => "NULL",
                _ => column,
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut client = owner();
        let mut tx = client.transaction().unwrap();
        let error = tx.execute(&format!("INSERT INTO celetixo.engine_mutations ({columns}) SELECT {select} FROM celetixo.engine_mutations WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"), &[&selected.organization.as_str(), &selected.workspace.as_str(), &selected.repository.as_str()]).unwrap_err();
        assert_eq!(error.code().unwrap().code(), expected, "variant {variant}");
        tx.rollback().unwrap();
    }
}
