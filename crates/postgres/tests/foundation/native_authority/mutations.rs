//! Original reply recovery against independent PostgreSQL connections.
use super::*;

#[path = "mutations/native_effects.rs"]
mod native_effects;
#[path = "mutations/repository.rs"]
mod repository;
#[path = "mutations/upgrade.rs"]
mod upgrade;

fn reserve(transaction: &str, ttl_ms: u64) -> MutationRequest {
    MutationRequest::Reserve {
        transaction: id(transaction),
        ttl_ms,
    }
}
fn renew(transaction: &str, ttl_ms: u64) -> MutationRequest {
    MutationRequest::Renew {
        transaction: id(transaction),
        ttl_ms,
    }
}
fn prepare(h: &Harness, transaction: &str) {
    h.engine
        .propose(
            &h.worker,
            h.scope(),
            id(transaction),
            h.base.id(),
            h.intent("x"),
        )
        .unwrap();
    h.engine
        .analyze(&h.worker, h.scope(), &id(transaction))
        .unwrap();
}
fn fresh_worker(scope: &Scope, stage: AuthorityStage) -> (Engine, Session) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let armed = Arc::new(AtomicBool::new(false));
    let active = armed.clone();
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |point| {
        if active.load(Ordering::SeqCst) && point == stage {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(scope, 1, |_, _| Ok(()))
        .unwrap();
    armed.store(true, Ordering::SeqCst);
    (engine, worker)
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn a_rejected_mutation_survives_before_repository_registration() {
    let selected = scope("mutation-before-registration");
    authorize_registration(&mut owner(), &selected);
    let backend = Arc::new(sql_authority());
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let worker = admin
        .grant(
            PrincipalId::new("owner").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let original = engine
        .mutate(
            &worker,
            &selected,
            key("before-register"),
            reserve("later", 0),
        )
        .unwrap();
    assert_eq!(original.outcome, Err(EngineError::NotFound));
    let before = backend.inspect_registration(&selected).unwrap();
    assert!(before.records.repository.is_none());
    assert_eq!(before.records.mutations.len(), 1);
    let (engine, admin) = Engine::with_authority_backend(Arc::new(sql_authority()));
    let worker = admin
        .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
        .unwrap();
    engine
        .register(&worker, snapshot(selected.clone(), 0), native_model(1))
        .unwrap();
    let result = engine
        .mutate(
            &worker,
            &selected,
            key("before-register"),
            reserve("later", 0),
        )
        .unwrap();
    assert!(result.replayed);
    assert_eq!(result.outcome, original.outcome);
    assert_eq!(result.recorded_at_ms, original.recorded_at_ms);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn published_v4_upgrade_is_atomic_and_preserves_snapshot_data() {
    let (_admin, mut client, _database, expected) = fresh_prior_database("mutation_upgrade", 4);
    for fault in [MigrationStage::AfterDdl, MigrationStage::BeforeCommit] {
        assert_eq!(
            migrate_with_checkpoint(&mut client, |stage| {
                if stage == fault {
                    Err(StoreError::MigrationInterrupted)
                } else {
                    Ok(())
                }
            }),
            Err(StoreError::MigrationInterrupted)
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM celetixo.schema_migrations", &[])
                .unwrap()
                .get::<_, i64>(0),
            4
        );
        assert!(
            client
                .query_one("SELECT to_regclass('celetixo.engine_mutations')::text", &[])
                .unwrap()
                .get::<_, Option<String>>(0)
                .is_none()
        );
    }
    assert_eq!(migrate(&mut client).unwrap().schema_version, 7);
    validate_schema(&mut client).unwrap();
    assert!(!migrate(&mut client).unwrap().newly_applied);
    let selected = expected.scope();
    let stored=client.query_one("SELECT snapshot_digest::bytea FROM celetixo.snapshots WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&selected.organization.as_str(),&selected.workspace.as_str(),&selected.repository.as_str()]).unwrap();
    assert_eq!(stored.get::<_, Vec<u8>>(0), expected.id().0.as_bytes());
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn independent_processes_recover_one_reply_for_one_lease_effect() {
    const CHILD: &str = "CELETIXO_PG_MUTATION_CHILD";
    if let Some(directory) = std::env::var_os(CHILD) {
        let directory = PathBuf::from(directory);
        let tag = std::env::var("CELETIXO_PG_MUTATION_TAG").unwrap();
        let selected = Scope {
            organization: OrganizationId::new(std::env::var("CELETIXO_PG_MUTATION_ORG").unwrap())
                .unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repository").unwrap(),
        };
        let (engine, admin) = Engine::with_authority_backend(Arc::new(sql_authority()));
        let worker = admin
            .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
            .unwrap();
        std::fs::write(directory.join(format!("{tag}.ready")), b"ready").unwrap();
        wait_file(&directory.join("go"));
        let reply = engine
            .mutate(&worker, &selected, key("acquire"), reserve("a", 120_000))
            .unwrap();
        assert!(reply.outcome.is_ok());
        std::fs::write(
            directory.join(format!("{tag}.result")),
            if reply.replayed {
                b"replayed".as_slice()
            } else {
                b"original".as_slice()
            },
        )
        .unwrap();
        return;
    }
    let mut h = Harness::new("mutation-process-race");
    prepare(&h, "a");
    // Retain the barrier/result evidence; no fixture data is removed.
    let directory =
        std::env::temp_dir().join(format!("celetixo-mutation-race-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let mut children:Vec<_>=["first","second"].map(|tag| {
        Command::new(std::env::current_exe().unwrap()).args(["--exact","native_authority::mutations::independent_processes_recover_one_reply_for_one_lease_effect","--ignored","--nocapture"])
            .env(CHILD,&directory).env("CELETIXO_PG_MUTATION_TAG",tag).env("CELETIXO_PG_MUTATION_ORG",h.scope().organization.as_str()).spawn().unwrap()
    }).into();
    for tag in ["first", "second"] {
        wait_file(&directory.join(format!("{tag}.ready")));
    }
    std::fs::write(directory.join("go"), b"go").unwrap();
    let statuses: Vec<_> = children
        .iter_mut()
        .map(|child| child.wait().unwrap())
        .collect();
    assert!(statuses.iter().all(std::process::ExitStatus::success));
    let mut outcomes = ["first", "second"]
        .map(|tag| std::fs::read_to_string(directory.join(format!("{tag}.result"))).unwrap());
    outcomes.sort();
    assert_eq!(outcomes, ["original", "replayed"]);
    h.reopen();
    let records = h.backend.inspect_registration(h.scope()).unwrap().records;
    assert_eq!(records.mutations.len(), 1);
    assert_eq!(records.reservation_fence, 1);
    assert_eq!(records.transactions[&id("a")].lease_history.len(), 1);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn mutation_rows_enforce_immutable_scope_actor_and_effect_bindings() {
    let h = Harness::new("mutation-row-guards");
    prepare(&h, "a");
    h.engine
        .mutate(&h.worker, h.scope(), key("acquire"), reserve("a", 120_000))
        .unwrap();
    let a = [
        h.scope().organization.as_str(),
        h.scope().workspace.as_str(),
        h.scope().repository.as_str(),
    ];
    let filter = "organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text";
    let mut owner = owner();
    for sql in [
        format!(
            "UPDATE celetixo.engine_mutations SET result_expiry_ms=result_expiry_ms+1 WHERE {filter}"
        ),
        format!("DELETE FROM celetixo.engine_mutations WHERE {filter}"),
    ] {
        let mut tx = owner.transaction().unwrap();
        let error = tx.execute(&sql, &[&a[0], &a[1], &a[2]]).unwrap_err();
        assert_eq!(error.code().unwrap().code(), "23514");
        tx.rollback().unwrap();
    }
    let columns = "organization_id,workspace_id,repository_id,actor_kind,grant_kind,actor_principal_id,actor_grant_id,actor_digest,idempotency_key,request_kind,request_digest,request_payload_digest,transaction_id,recorded_at_ms,resolved_transaction_id,resolved_attempt,state_before,lease_events_before,outcome_kind,outcome_payload_digest,event_sequence,result_fence,result_expiry_ms,result_acquisition_attempt,result_kind,result_state,result_attempt";
    for (variant, expected) in [(0, "23505"), (1, "23514"), (2, "23503")] {
        let select = columns
            .split(',')
            .map(|column| match (variant, column) {
                (_, "idempotency_key") => "'forged-key'::text",
                (1, "transaction_id") => "'another-transaction'::text",
                (2, "actor_principal_id") => "'verifier'::text",
                (2, "outcome_kind") => "'Rejected'::text",
                (
                    2,
                    "event_sequence"
                    | "result_fence"
                    | "result_expiry_ms"
                    | "result_acquisition_attempt"
                    | "result_kind"
                    | "result_state"
                    | "result_attempt",
                ) => "NULL",
                _ => column,
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut tx = owner.transaction().unwrap();
        let error=tx.execute(&format!("INSERT INTO celetixo.engine_mutations ({columns}) SELECT {select} FROM celetixo.engine_mutations WHERE {filter}"),&[&a[0],&a[1],&a[2]]).unwrap_err();
        assert_eq!(error.code().unwrap().code(), expected, "variant {variant}");
        tx.rollback().unwrap();
    }
    let mut runtime = connection("runtime", None).connect().unwrap();
    for sql in [
        format!(
            "UPDATE celetixo.engine_mutations SET recorded_at_ms=recorded_at_ms WHERE {filter}"
        ),
        format!("DELETE FROM celetixo.engine_mutations WHERE {filter}"),
    ] {
        let mut tx = runtime.transaction().unwrap();
        assert_eq!(
            tx.execute(&sql, &[&a[0], &a[1], &a[2]])
                .unwrap_err()
                .code()
                .unwrap()
                .code(),
            "42501"
        );
        tx.rollback().unwrap();
    }
    let records = h.backend.inspect_registration(h.scope()).unwrap().records;
    assert_eq!(records.mutations.len(), 1);
    assert_eq!(records.transactions[&id("a")].lease_history.len(), 1);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn original_lease_replies_and_rejections_survive_cold_reopen() {
    let mut h = Harness::new("mutation-reopen");
    let missing = h
        .engine
        .mutate(&h.worker, h.scope(), key("missing"), reserve("later", 0))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    prepare(&h, "later");
    h.reopen();
    let replay = h
        .engine
        .mutate(&h.worker, h.scope(), key("missing"), reserve("later", 0))
        .unwrap();
    assert_eq!(replay.outcome, missing.outcome);
    assert_eq!(replay.recorded_at_ms, missing.recorded_at_ms);
    assert!(replay.replayed);
    let invalid = h
        .engine
        .mutate(&h.worker, h.scope(), key("invalid"), reserve("later", 0))
        .unwrap();
    assert_eq!(invalid.outcome, Err(EngineError::InvalidTime));
    let first = h
        .engine
        .mutate(
            &h.worker,
            h.scope(),
            key("reserve"),
            reserve("later", 120_000),
        )
        .unwrap();
    assert!(first.outcome.is_ok());
    let renewed = h
        .engine
        .mutate(&h.worker, h.scope(), key("renew"), renew("later", 180_000))
        .unwrap();
    assert!(renewed.outcome.is_ok());
    h.reopen();
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    for (name, request, expected) in [
        ("reserve", reserve("later", 120_000), first),
        ("renew", renew("later", 180_000), renewed),
        ("invalid", reserve("later", 0), invalid),
    ] {
        let actual = h
            .engine
            .mutate(&h.worker, h.scope(), key(name), request)
            .unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
        assert_eq!(actual.recorded_at_ms, expected.recorded_at_ms);
        assert_eq!(
            h.engine
                .mutation_outcome(&h.worker, h.scope(), &key(name))
                .unwrap(),
            Some(actual)
        );
    }
    let after = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(before.records.transactions, after.records.transactions);
    assert_eq!(before.records.mutations, after.records.mutations);
    assert_eq!(
        after.records.transactions[&id("later")].lease_history.len(),
        2
    );
    assert_eq!(
        h.engine.mutate(
            &h.worker,
            h.scope(),
            key("reserve"),
            reserve("later", 120_001)
        ),
        Err(EngineError::IdempotencyConflict)
    );
    assert_eq!(
        h.engine.mutate(
            &h.worker,
            h.scope(),
            key("reserve"),
            renew("later", 120_000)
        ),
        Err(EngineError::IdempotencyConflict)
    );
    h.admin.revoke(&h.worker).unwrap();
    assert_eq!(
        h.engine.mutate(
            &h.worker,
            h.scope(),
            key("reserve"),
            reserve("later", 120_000)
        ),
        Err(EngineError::Unauthorized)
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn execution_and_cancellation_recover_original_results_after_abort() {
    let mut h = Harness::new("mutation-state-reopen");
    prepare(&h, "a");
    h.engine
        .mutate(&h.worker, h.scope(), key("acquire"), reserve("a", 120_000))
        .unwrap();
    let request = MutationRequest::Execute {
        transaction: id("a"),
    };
    let execute = h
        .engine
        .mutate(&h.worker, h.scope(), key("execute"), request.clone())
        .unwrap();
    assert_eq!(
        execute.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Executing,
            attempt: 0
        })
    );
    let cancel_request = MutationRequest::Abort {
        transaction: id("a"),
        reason: "cancel λ\0with provenance".into(),
    };
    let cancelled = h
        .engine
        .mutate(&h.worker, h.scope(), key("cancel"), cancel_request.clone())
        .unwrap();
    assert_eq!(
        cancelled.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Aborted,
            attempt: 0
        })
    );
    h.reopen();
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    for (key_name, request, expected) in [
        ("execute", request, execute),
        ("cancel", cancel_request.clone(), cancelled),
    ] {
        let reply = h
            .engine
            .mutate(&h.worker, h.scope(), key(key_name), request)
            .unwrap();
        assert!(reply.replayed);
        assert_eq!(reply.outcome, expected.outcome);
        assert_eq!(reply.recorded_at_ms, expected.recorded_at_ms);
    }
    assert!(
        h.engine
            .mutate(
                &h.worker,
                h.scope(),
                key("cancel-second-key"),
                cancel_request
            )
            .unwrap()
            .outcome
            .is_ok()
    );
    assert_eq!(
        h.engine.execute(&h.worker, h.scope(), &id("a")),
        Err(EngineError::InvalidLease)
    );
    let after = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(before.records.transactions, after.records.transactions);
    assert_eq!(after.records.mutations.len(), 4);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn analysis_replies_survive_model_replacement_and_an_ambiguous_commit() {
    let mut h = Harness::new("mutation-analysis-reopen");
    let analyze = |name: &str| MutationRequest::Analyze {
        transaction: id(name),
    };
    let missing = h
        .engine
        .mutate(&h.worker, h.scope(), key("missing"), analyze("a"))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    h.engine
        .propose(&h.worker, h.scope(), id("a"), h.base.id(), h.intent("x"))
        .unwrap();
    let (engine, worker) = fresh_worker(h.scope(), AuthorityStage::AfterCommit);
    assert_eq!(
        engine.mutate(&worker, h.scope(), key("analysis"), analyze("a")),
        Err(authority_error(AuthorityFailure::CommitIndeterminate))
    );
    h.reopen();
    let original = h
        .engine
        .mutate(&h.worker, h.scope(), key("analysis"), analyze("a"))
        .unwrap();
    assert!(original.replayed);
    assert!(
        matches!(&original.outcome,Ok(MutationValue::Analysis {attempt:0,accesses}) if accesses.writes==[e("x")].into())
    );
    h.engine
        .propose(&h.worker, h.scope(), id("b"), h.base.id(), h.intent("y"))
        .unwrap();
    h.engine
        .update_model(&h.worker, h.scope(), native_model(2))
        .unwrap();
    let changed = h
        .engine
        .mutate(&h.worker, h.scope(), key("model-changed"), analyze("b"))
        .unwrap();
    assert_eq!(changed.outcome, Err(EngineError::ModelChanged));
    h.engine
        .abort(
            &h.worker,
            h.scope(),
            &id("a"),
            "terminal after analysis".into(),
        )
        .unwrap();
    h.reopen();
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    for (name, request, expected) in [
        ("missing", analyze("a"), missing),
        ("analysis", analyze("a"), original),
        ("model-changed", analyze("b"), changed),
    ] {
        let actual = h
            .engine
            .mutate(&h.worker, h.scope(), key(name), request)
            .unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
        assert_eq!(actual.recorded_at_ms, expected.recorded_at_ms);
    }
    let after = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(before.records.transactions, after.records.transactions);
    assert_eq!(before.records.mutations, after.records.mutations);
    let row=owner().query_one("SELECT result_kind,result_state,result_attempt::text,result_access_digest IS NOT NULL FROM celetixo.engine_mutations WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND request_kind='Analyze' AND outcome_kind='Accepted'",&[&h.scope().organization.as_str(),&h.scope().workspace.as_str(),&h.scope().repository.as_str()]).unwrap();
    assert_eq!(row.get::<_, String>(0), "Analysis");
    assert_eq!(row.get::<_, String>(1), "Analyzed");
    assert_eq!(row.get::<_, String>(2), "0");
    assert!(row.get::<_, bool>(3));
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn proposal_originals_survive_registration_and_a_lost_admission_reply() {
    let digest = |name: &str| Digest::of("proposal-sql-test/v1", name.as_bytes());
    let selected = scope("mutation-proposal-recovery");
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
    let check = CheckSpec::new(
        CheckId::new("check").unwrap(),
        vec!["reference-not-executed".into()],
        "reference/v1".into(),
        digest("environment"),
        digest("configuration"),
    )
    .unwrap();
    let intent = TransactionIntent {
        objective: "original proposal".into(),
        accesses: AccessSet {
            writes: [e("x")].into(),
            ..Default::default()
        },
        invariants: vec![],
        required_checks: vec![check],
        depends_on: Default::default(),
    };
    let request = MutationRequest::Propose {
        transaction: id("work"),
        base: base.id(),
        intent: Box::new(intent.clone()),
    };
    let missing = engine
        .mutate(&worker, &selected, key("missing"), request.clone())
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    let mut invalid = intent.clone();
    invalid.required_checks.clear();
    let invalid_request = MutationRequest::Propose {
        transaction: id("work"),
        base: SnapshotId(digest("unknown-snapshot")),
        intent: Box::new(invalid),
    };
    let invalid = engine
        .mutate(&worker, &selected, key("invalid"), invalid_request.clone())
        .unwrap();
    assert!(matches!(
        invalid.outcome,
        Err(EngineError::InvalidIntent(_))
    ));
    engine.register(&worker, base, native_model(1)).unwrap();
    let (uncertain, capability) = fresh_worker(&selected, AuthorityStage::AfterCommit);
    assert_eq!(
        uncertain.mutate(&capability, &selected, key("proposal"), request.clone()),
        Err(authority_error(AuthorityFailure::CommitIndeterminate))
    );
    let accepted = engine
        .mutate(&worker, &selected, key("proposal"), request.clone())
        .unwrap();
    assert!(accepted.replayed);
    assert_eq!(
        accepted.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Proposed,
            attempt: 0
        })
    );
    engine
        .mutate(
            &worker,
            &selected,
            key("analysis"),
            MutationRequest::Analyze {
                transaction: id("work"),
            },
        )
        .unwrap();
    let existing = engine
        .mutate(&worker, &selected, key("existing"), request.clone())
        .unwrap();
    assert_eq!(
        existing.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Analyzed,
            attempt: 0
        })
    );
    engine
        .abort(&worker, &selected, &id("work"), "finished".into())
        .unwrap();
    let before = backend.inspect_registration(&selected).unwrap();
    let new_backend = Arc::new(sql_authority());
    let (reopened, admin) = Engine::with_authority_backend(new_backend.clone());
    let worker = admin
        .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
        .unwrap();
    for (name, request, expected) in [
        ("missing", request.clone(), missing),
        ("invalid", invalid_request, invalid),
        ("proposal", request.clone(), accepted),
        ("existing", request, existing),
    ] {
        let reply = reopened
            .mutate(&worker, &selected, key(name), request)
            .unwrap();
        assert!(reply.replayed);
        assert_eq!(reply.outcome, expected.outcome);
        assert_eq!(reply.recorded_at_ms, expected.recorded_at_ms);
    }
    let after = new_backend.inspect_registration(&selected).unwrap();
    assert_eq!(before.records.transactions, after.records.transactions);
    assert_eq!(before.records.mutations, after.records.mutations);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn mutation_faults_roll_back_or_reconcile_without_a_second_effect() {
    let mut h = Harness::new("mutation-faults");
    prepare(&h, "a");
    for stage in [AuthorityStage::StateLoaded, AuthorityStage::BeforeCommit] {
        let (engine, worker) = fresh_worker(h.scope(), stage);
        let before = h.backend.inspect_registration(h.scope()).unwrap();
        assert_eq!(
            engine.mutate(&worker, h.scope(), key("acquire"), reserve("a", 120_000)),
            Err(authority_error(AuthorityFailure::Unavailable))
        );
        assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    }
    let (engine, worker) = fresh_worker(h.scope(), AuthorityStage::AfterCommit);
    assert_eq!(
        engine.mutate(&worker, h.scope(), key("acquire"), reserve("a", 120_000)),
        Err(authority_error(AuthorityFailure::CommitIndeterminate))
    );
    h.reopen();
    let reply = h
        .engine
        .mutate(&h.worker, h.scope(), key("acquire"), reserve("a", 120_000))
        .unwrap();
    assert!(reply.replayed);
    assert!(reply.outcome.is_ok());
    let records = h.backend.inspect_registration(h.scope()).unwrap().records;
    assert_eq!(records.mutations.len(), 1);
    assert_eq!(records.transactions[&id("a")].lease_history.len(), 1);
    assert_eq!(records.reservation_fence, 1);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn delayed_renewal_cannot_publish_after_original_lease_expiry() {
    let h = Harness::new("mutation-renew-expiry");
    prepare(&h, "a");
    let first = h
        .engine
        .mutate(&h.worker, h.scope(), key("acquire"), reserve("a", 6_000))
        .unwrap();
    let Ok(MutationValue::Lease { expires_at_ms, .. }) = first.outcome else {
        panic!("lease result");
    };
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            let now = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(
                expires_at_ms.saturating_sub(now) + 20,
            ));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.mutate(&worker, h.scope(), key("renew"), renew("a", 120_000)),
        Err(authority_error(AuthorityFailure::Contention))
    );
    assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    let retry = h
        .engine
        .mutate(&h.worker, h.scope(), key("acquire"), reserve("a", 6_000))
        .unwrap();
    assert!(retry.replayed);
    assert_eq!(
        h.engine.execute(&h.worker, h.scope(), &id("a")),
        Err(EngineError::InvalidLease)
    );
    let reply = h
        .engine
        .mutate(&h.worker, h.scope(), key("renew"), renew("a", 120_000))
        .unwrap();
    assert_eq!(reply.outcome, Err(EngineError::InvalidLease));
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn delayed_execution_does_not_publish_success_after_lease_expiry() {
    let h = Harness::new("mutation-execute-expiry");
    prepare(&h, "a");
    let acquired = h
        .engine
        .mutate(&h.worker, h.scope(), key("acquire"), reserve("a", 6_000))
        .unwrap();
    let Ok(MutationValue::Lease { expires_at_ms, .. }) = acquired.outcome else {
        panic!("lease result")
    };
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            let now = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(
                expires_at_ms.saturating_sub(now) + 20,
            ));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let request = MutationRequest::Execute {
        transaction: id("a"),
    };
    assert_eq!(
        engine.mutate(&worker, h.scope(), key("execute"), request.clone()),
        Err(authority_error(AuthorityFailure::Contention))
    );
    assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    let reply = h
        .engine
        .mutate(&h.worker, h.scope(), key("execute"), request)
        .unwrap();
    assert!(!reply.replayed);
    assert_eq!(reply.outcome, Err(EngineError::InvalidLease));
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn duplicate_lease_mutations_use_one_atomic_reply_across_connections() {
    let h = Harness::new("mutation-concurrent");
    prepare(&h, "a");
    let barrier = Arc::new(Barrier::new(3));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let scope = h.scope().clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let (engine, admin) = Engine::with_authority_backend(Arc::new(sql_authority()));
                let worker = admin
                    .authenticate_existing_session(&scope, 1, |_, _| Ok(()))
                    .unwrap();
                barrier.wait();
                engine
                    .mutate(&worker, &scope, key("acquire"), reserve("a", 120_000))
                    .unwrap()
            })
        })
        .collect();
    barrier.wait();
    let replies: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(replies.iter().filter(|r| !r.replayed).count(), 1);
    assert_eq!(replies[0].outcome, replies[1].outcome);
    assert_eq!(replies[0].recorded_at_ms, replies[1].recorded_at_ms);
    let records = h.backend.inspect_registration(h.scope()).unwrap().records;
    assert_eq!(records.mutations.len(), 1);
    assert_eq!(records.transactions[&id("a")].lease_history.len(), 1);
}
