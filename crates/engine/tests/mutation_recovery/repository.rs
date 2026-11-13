//! Original repository commands have no transaction and cannot reinstall a head.
use super::*;

fn update(generation: u64) -> MutationRequest {
    let mut model = model();
    model.generation = generation;
    MutationRequest::Repository(Box::new(RepositoryMutation::UpdateModel {
        model: Box::new(model),
    }))
}
fn register(base: &Snapshot) -> MutationRequest {
    MutationRequest::Repository(Box::new(RepositoryMutation::Register {
        snapshot: Box::new(base.clone()),
        model: Box::new(model()),
    }))
}

#[test]
fn original_registration_and_model_replies_survive_new_heads_and_cold_reopen() {
    let selected = scope("repository-original-replies");
    let backend = Arc::new(FaultBackend {
        records: Mutex::new(NativeRecords::empty(selected.clone())),
        fault: AtomicU8::new(0),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("owner").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = snapshot(selected.clone());
    let missing = engine
        .mutate(&owner, &selected, key("missing"), update(2))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    let invalid = engine
        .mutate(&owner, &selected, key("invalid"), update(0))
        .unwrap();
    assert_eq!(invalid.outcome, Err(EngineError::IncompleteAnalysis));
    let registered = engine
        .mutate(&owner, &selected, key("register"), register(&base))
        .unwrap();
    let replacement = engine
        .mutate(&owner, &selected, key("model"), update(2))
        .unwrap();
    assert!(
        matches!(&registered.outcome, Ok(MutationValue::Repository {model, ..}) if model.generation == 1)
    );
    assert!(
        matches!(&replacement.outcome, Ok(MutationValue::Repository {model, ..}) if model.generation == 2)
    );
    let mut third = model();
    third.generation = 3;
    engine.update_model(&owner, &selected, third).unwrap();
    let original = backend.records.lock().unwrap().clone();
    assert!(original.transactions.is_empty());
    for record in original.mutations.values() {
        assert!(record.request.transaction().is_none());
        assert!(record.resolved_attempt.is_none());
        assert!(record.state_before.is_none());
    }
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
        .unwrap();
    for (operation, input, expected) in [
        ("missing", update(2), missing),
        ("invalid", update(0), invalid),
        ("register", register(&base), registered),
        ("model", update(2), replacement),
    ] {
        let replay = engine
            .mutate(&owner, &selected, key(operation), input)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, expected.outcome);
        assert_eq!(replay.recorded_at_ms, expected.recorded_at_ms);
    }
    assert_eq!(
        backend
            .records
            .lock()
            .unwrap()
            .repository
            .as_ref()
            .unwrap()
            .model
            .generation,
        3
    );
    assert_eq!(
        engine.mutate(&owner, &selected, key("register"), update(4)),
        Err(EngineError::IdempotencyConflict)
    );
    assert_eq!(
        engine.mutate(&owner, &selected, key("register"), reserve("work", 10)),
        Err(EngineError::IdempotencyConflict)
    );
    let mut forged = original.clone();
    let record = forged
        .mutations
        .values_mut()
        .find(|record| matches!(record.outcome, Ok(MutationValue::Repository { .. })))
        .unwrap();
    record.resolved_attempt = Some(0);
    assert!(AuthorityState::from_native_records(forged, Default::default()).is_err());
    let mut duplicate = original;
    let (actor, record) = duplicate
        .mutations
        .iter()
        .find(|(_, record)| matches!(record.outcome, Ok(MutationValue::Repository { .. })))
        .map(|(key, record)| (key.actor.clone(), record.clone()))
        .unwrap();
    duplicate.mutations.insert(
        MutationKey {
            actor,
            key: key("forged-model-effect"),
        },
        record,
    );
    assert!(AuthorityState::from_native_records(duplicate, Default::default()).is_err());
    admin.revoke(&owner).unwrap();
    assert_eq!(
        engine.mutate(&owner, &selected, key("register"), register(&base)),
        Err(EngineError::Unauthorized)
    );
}

#[test]
fn repository_authorization_and_failure_do_not_consume_a_key_or_publish_a_head() {
    let selected = scope("repository-original-rollback");
    let backend = Arc::new(FaultBackend {
        records: Mutex::new(NativeRecords::empty(selected.clone())),
        fault: AtomicU8::new(0),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("owner").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let before = backend.records.lock().unwrap().clone();
    let foreign = snapshot(scope("foreign"));
    assert_eq!(
        engine.mutate(&owner, &selected, key("register"), register(&foreign)),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(*backend.records.lock().unwrap(), before);
    let base = snapshot(selected.clone());
    for failure in [1, 2] {
        backend.fault.store(failure, Ordering::SeqCst);
        assert_eq!(
            engine.mutate(&owner, &selected, key("register"), register(&base)),
            Err(EngineError::Authority(AuthorityFailure::Unavailable))
        );
        assert_eq!(*backend.records.lock().unwrap(), before);
    }
    backend.fault.store(0, Ordering::SeqCst);
    assert!(
        !engine
            .mutate(&owner, &selected, key("register"), register(&base))
            .unwrap()
            .replayed
    );
    let read_only = admin
        .grant(
            PrincipalId::new("reader").unwrap(),
            selected.clone(),
            [Permission::Read].into(),
        )
        .unwrap();
    assert_eq!(
        engine.mutate(&read_only, &selected, key("model"), update(2)),
        Err(EngineError::Unauthorized)
    );
    assert!(
        engine
            .mutation_outcome(&read_only, &selected, &key("model"))
            .unwrap()
            .is_none()
    );
}
