//! Original executor outcomes on actual independent PostgreSQL connections.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn admitted(label: &str) -> (Harness, SourceAdmission) {
    let h = Harness::new(label);
    h.prepare();
    let admission = h
        .engine
        .admit_source_attempt(&h.owner, h.scope(), h.patch("attempt"), key("admission"))
        .unwrap();
    (h, admission)
}
fn finish() -> SourceExecutorAction {
    SourceExecutorAction::Finish {
        outcome: SourceAttemptState::ExecutionFailed,
        reason: "original synthetic executor failure".into(),
    }
}
fn invoke(
    h: &Harness,
    a: &SourceAdmission,
    name: &str,
    action: SourceExecutorAction,
) -> MutationResponse {
    h.engine
        .mutate_source_executor(
            &h.executor,
            h.scope(),
            key(name),
            SourceExecutorMutation::new(a, action),
        )
        .unwrap()
}
fn fresh(scope: &Scope) -> (Engine, SourceExecutorCapability) {
    let (engine, admin) = Engine::with_authority_backend(Arc::new(authority()));
    let executor = admin
        .authenticate_existing_source_executor(scope, 1, |_, _| Ok(()))
        .unwrap();
    (engine, executor)
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn executor_original_replies_and_rejections_survive_terminal_state_and_fresh_connections() {
    let (h, a) = admitted("executor-originals");
    let mut replies = Vec::new();
    for (name, action) in [
        ("too-early", SourceExecutorAction::BeginVerification),
        ("claim", SourceExecutorAction::Claim),
        ("claimed-again", SourceExecutorAction::Claim),
        ("patch", SourceExecutorAction::PatchApplied),
        ("verify", SourceExecutorAction::BeginVerification),
        ("finish", finish()),
    ] {
        let reply = invoke(&h, &a, name, action.clone());
        if name == "too-early" {
            assert_eq!(
                reply.outcome,
                Err(SourceAttemptError::UntrustedExecutor.into())
            );
        } else if name == "claimed-again" {
            assert_eq!(
                reply.outcome,
                Err(SourceAttemptError::AlreadyClaimed.into())
            );
        } else {
            assert!(matches!(
                &reply.outcome,
                Ok(MutationValue::SourceAttempt(_))
            ));
        }
        replies.push((name, action, reply));
    }
    let before = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(before.base.mutations.len(), 6);
    assert_eq!(before.attempts[&a.binding().source_attempt].events.len(), 5);
    let (engine, executor) = fresh(h.scope());
    for (name, action, original) in replies {
        let replay = engine
            .mutate_source_executor(
                &executor,
                h.scope(),
                key(name),
                SourceExecutorMutation::new(&a, action),
            )
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, original.outcome);
        assert_eq!(replay.recorded_at_ms, original.recorded_at_ms);
        assert_eq!(
            engine
                .source_executor_mutation_outcome(&executor, h.scope(), &key(name))
                .unwrap()
                .unwrap()
                .outcome,
            original.outcome
        );
    }
    let after = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(after.base.mutations, before.base.mutations);
    assert_eq!(after.attempts, before.attempts);
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "source_authority::lifecycle::mutations::executor::executor_replay_child",
        ])
        .env(
            "CELETIXO_SOURCE_EXECUTOR_REPLAY_ORGANIZATION",
            h.scope().organization.as_str(),
        )
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(
        engine.validate_source_attempt(&executor, &a),
        Err(SourceAttemptError::InvalidState.into())
    );
    assert_eq!(
        engine.mutate_source_executor(
            &executor,
            h.scope(),
            key("claim"),
            SourceExecutorMutation::new(&a, finish())
        ),
        Err(EngineError::IdempotencyConflict)
    );
    h.admin.revoke_source_executor(&h.executor).unwrap();
    assert_eq!(
        engine.source_executor_mutation_outcome(&executor, h.scope(), &key("claim")),
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
    assert_eq!(
        engine.mutate_source_executor(
            &executor,
            h.scope(),
            key("claim"),
            SourceExecutorMutation::new(&a, SourceExecutorAction::Claim)
        ),
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
}

#[test]
#[ignore = "invoked by the original-outcome parent in a fresh process"]
fn executor_replay_child() {
    let Ok(organization) = std::env::var("CELETIXO_SOURCE_EXECUTOR_REPLAY_ORGANIZATION") else {
        return;
    };
    let scope = Scope {
        organization: OrganizationId::new(organization).unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    };
    let backend = authority();
    let before = backend.inspect_source(&scope).unwrap().records;
    let a = before.attempts.values().next().unwrap().admission.clone();
    let (engine, executor) = fresh(&scope);
    let reply = engine
        .mutate_source_executor(
            &executor,
            &scope,
            key("claim"),
            SourceExecutorMutation::new(&a, SourceExecutorAction::Claim),
        )
        .unwrap();
    assert!(reply.replayed);
    assert!(
        matches!(reply.outcome, Ok(MutationValue::SourceAttempt(value)) if value.state == SourceAttemptState::Patching && value.events.len() == 2)
    );
    assert_eq!(
        engine.validate_source_attempt(&executor, &a),
        Err(SourceAttemptError::InvalidState.into())
    );
    let after = backend.inspect_source(&scope).unwrap().records;
    assert_eq!(after.base.mutations, before.base.mutations);
    assert_eq!(after.attempts, before.attempts);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn executor_sql_constraints_reject_partial_context_results_and_duplicate_events() {
    let (h, a) = admitted("executor-row-guards");
    assert_eq!(
        invoke(&h, &a, "too-early", SourceExecutorAction::BeginVerification).outcome,
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
    assert!(
        invoke(&h, &a, "claim", SourceExecutorAction::Claim)
            .outcome
            .is_ok()
    );
    let mut client = owner();
    let columns: Vec<String> = client.query("SELECT attname::text FROM pg_attribute WHERE attrelid='celetixo.engine_mutations'::regclass AND attnum>0 AND NOT attisdropped ORDER BY attnum", &[]).unwrap().iter().map(|row|row.get(0)).collect();
    for (operation, replacements, expected) in [
        ("claim", vec![], "23505"),
        ("claim", vec![("raw_source_attempt_id", "NULL")], "23514"),
        (
            "claim",
            vec![("raw_source_admission_digest", "NULL")],
            "23514",
        ),
        ("claim", vec![("source_events_before", "NULL")], "23514"),
        ("claim", vec![("source_events_after", "NULL")], "23514"),
        ("claim", vec![("source_admission_digest", "NULL")], "23514"),
        (
            "claim",
            vec![("source_effect_event_sequence", "NULL")],
            "23514",
        ),
        ("claim", vec![("result_source_state", "NULL")], "23514"),
        (
            "claim",
            vec![("result_source_event_sequence", "NULL")],
            "23514",
        ),
        (
            "claim",
            vec![("result_source_executor_grant_id", "NULL")],
            "23514",
        ),
        ("claim", vec![("source_executor_before", "1")], "23514"),
        (
            "too-early",
            vec![("source_executor_before", "999")],
            "23503",
        ),
        (
            "too-early",
            vec![("source_state_before", "'Verifying'")],
            "23503",
        ),
        (
            "too-early",
            vec![("source_admission_digest", "decode(repeat('00',32),'hex')")],
            "23503",
        ),
    ] {
        let select = columns
            .iter()
            .map(|column| {
                if column == "idempotency_key" {
                    "'constraint-probe'::text"
                } else {
                    replacements
                        .iter()
                        .find(|(name, _)| *name == column.as_str())
                        .map_or(column.as_str(), |(_, value)| *value)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut tx = client.transaction().unwrap();
        let error=tx.execute(&format!("INSERT INTO celetixo.engine_mutations ({}) SELECT {select} FROM celetixo.engine_mutations WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND idempotency_key=$4::text",columns.join(",")),&[&h.scope().organization.as_str(),&h.scope().workspace.as_str(),&h.scope().repository.as_str(),&operation]).unwrap_err();
        assert_eq!(
            error.code().unwrap().code(),
            expected,
            "{operation}: {replacements:?}"
        );
        tx.rollback().unwrap();
    }
    let before = h.authority.inspect_source(h.scope()).unwrap().records;
    let foreign = Scope {
        organization: OrganizationId::new("unauthorized-executor-organization").unwrap(),
        ..h.scope().clone()
    };
    assert!(
        h.engine
            .source_executor_mutation_outcome(&h.executor, &foreign, &key("claim"))
            .is_err()
    );
    assert!(
        h.engine
            .mutate_source_executor(
                &h.executor,
                &foreign,
                key("claim"),
                SourceExecutorMutation::new(&a, SourceExecutorAction::Claim)
            )
            .is_err()
    );
    let after = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(after.attempts, before.attempts);
    assert_eq!(after.base.mutations, before.base.mutations);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn executor_final_database_clock_rolls_back_the_transition_and_original_key() {
    let h = Harness::new("executor-final-clock");
    let (_, a, _) = h.ready_with_ttl("attempt", 9_000);
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let expires = before.records.base.transactions[&transaction_id()]
        .lease
        .as_ref()
        .unwrap()
        .expires_at;
    let backend = Arc::new(authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            assert!(now() < expires, "transition must start with a live lease");
            let remaining = expires.saturating_add(2).saturating_sub(now());
            assert!(remaining <= 9_002, "expiry checkpoint must stay bounded");
            std::thread::sleep(std::time::Duration::from_millis(remaining));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let executor = admin
        .authenticate_existing_source_executor(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.mutate_source_executor(
            &executor,
            h.scope(),
            key("finish"),
            SourceExecutorMutation::new(&a, finish())
        ),
        Err(EngineError::Authority(AuthorityFailure::Contention))
    );
    assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), before);
    assert!(
        h.engine
            .source_executor_mutation_outcome(&h.executor, h.scope(), &key("finish"))
            .unwrap()
            .is_none()
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn executor_each_transition_rolls_back_and_recovers_lost_commit_replies() {
    let (h, a) = admitted("executor-faults");
    for (name, action) in [
        ("claim", SourceExecutorAction::Claim),
        ("patch", SourceExecutorAction::PatchApplied),
        ("verify", SourceExecutorAction::BeginVerification),
        ("finish", finish()),
    ] {
        for target in [
            AuthorityStage::StateLoaded,
            AuthorityStage::BeforeCommit,
            AuthorityStage::AfterCommit,
        ] {
            let armed = Arc::new(AtomicBool::new(false));
            let active = armed.clone();
            let backend = Arc::new(authority().with_checkpoint(Arc::new(move |stage| {
                if active.load(Ordering::SeqCst) && stage == target {
                    Err(AuthorityFailure::Unavailable)
                } else {
                    Ok(())
                }
            })));
            let (engine, admin) = Engine::with_authority_backend(backend);
            let executor = admin
                .authenticate_existing_source_executor(h.scope(), 1, |_, _| Ok(()))
                .unwrap();
            let before = h.authority.inspect_source(h.scope()).unwrap().records;
            armed.store(true, Ordering::SeqCst);
            let result = engine.mutate_source_executor(
                &executor,
                h.scope(),
                key(name),
                SourceExecutorMutation::new(&a, action.clone()),
            );
            assert_eq!(
                result,
                Err(EngineError::Authority(
                    if target == AuthorityStage::AfterCommit {
                        AuthorityFailure::CommitIndeterminate
                    } else {
                        AuthorityFailure::Unavailable
                    }
                ))
            );
            let after = h.authority.inspect_source(h.scope()).unwrap().records;
            if target == AuthorityStage::AfterCommit {
                assert_eq!(after.base.mutations.len(), before.base.mutations.len() + 1);
                assert_eq!(
                    after.attempts[&a.binding().source_attempt].events.len(),
                    before.attempts[&a.binding().source_attempt].events.len() + 1
                );
            } else {
                assert_eq!(after.base.mutations, before.base.mutations);
                assert_eq!(after.attempts, before.attempts);
            }
        }
        let (engine, executor) = fresh(h.scope());
        let replay = engine
            .mutate_source_executor(
                &executor,
                h.scope(),
                key(name),
                SourceExecutorMutation::new(&a, action),
            )
            .unwrap();
        assert!(replay.replayed && replay.outcome.is_ok());
    }
    let after = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(after.base.mutations.len(), 4);
    assert_eq!(after.attempts[&a.binding().source_attempt].events.len(), 5);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn executor_concurrent_original_claims_publish_one_event_and_one_original_result() {
    let (h, a) = admitted("executor-race");
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let (engine, executor) = fresh(h.scope());
            let barrier = barrier.clone();
            let scope = h.scope().clone();
            let a = a.clone();
            std::thread::spawn(move || {
                barrier.wait();
                engine
                    .mutate_source_executor(
                        &executor,
                        &scope,
                        key("claim"),
                        SourceExecutorMutation::new(&a, SourceExecutorAction::Claim),
                    )
                    .unwrap()
            })
        })
        .collect();
    let replies: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(replies.iter().filter(|r| !r.replayed).count(), 1);
    assert!(
        replies
            .iter()
            .all(|r| r.outcome == replies[0].outcome
                && r.recorded_at_ms == replies[0].recorded_at_ms)
    );
    let before = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(before.base.mutations.len(), 1);
    assert_eq!(before.attempts[&a.binding().source_attempt].events.len(), 2);
    assert_eq!(
        invoke(&h, &a, "second-key", SourceExecutorAction::Claim).outcome,
        Err(SourceAttemptError::AlreadyClaimed.into())
    );
    let after = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(after.attempts, before.attempts);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn executor_original_missing_selector_and_invalid_finish_remain_original_after_admission() {
    let (h, a) = admitted("executor-missing");
    let mut mismatched = SourceExecutorMutation::new(&a, SourceExecutorAction::Claim);
    mismatched.binding.transaction = TransactionId::new("missing-raw-transaction").unwrap();
    let mismatch = h
        .engine
        .mutate_source_executor(&h.executor, h.scope(), key("mismatch"), mismatched.clone())
        .unwrap();
    assert_eq!(
        mismatch.outcome,
        Err(SourceAttemptError::BindingMismatch.into())
    );
    let mut missing = SourceExecutorMutation::new(&a, SourceExecutorAction::Claim);
    missing.binding.source_attempt = SourceAttemptId::new("later").unwrap();
    let original = h
        .engine
        .mutate_source_executor(&h.executor, h.scope(), key("missing"), missing.clone())
        .unwrap();
    assert_eq!(original.outcome, Err(EngineError::NotFound));
    let invalid = SourceExecutorMutation::new(
        &a,
        SourceExecutorAction::Finish {
            outcome: SourceAttemptState::Patching,
            reason: "not terminal".into(),
        },
    );
    let invalid_reply = h
        .engine
        .mutate_source_executor(&h.executor, h.scope(), key("invalid"), invalid.clone())
        .unwrap();
    assert_eq!(
        invalid_reply.outcome,
        Err(SourceAttemptError::InvalidReason.into())
    );
    invoke(&h, &a, "claim", SourceExecutorAction::Claim);
    invoke(
        &h,
        &a,
        "cancel",
        SourceExecutorAction::Finish {
            outcome: SourceAttemptState::Cancelled,
            reason: "release source attempt".into(),
        },
    );
    let next = h
        .engine
        .admit_source_attempt(
            &h.owner,
            h.scope(),
            h.patch("later"),
            key("later-admission"),
        )
        .unwrap();
    let (engine, executor) = fresh(h.scope());
    let replay = engine
        .mutate_source_executor(&executor, h.scope(), key("mismatch"), mismatched)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, mismatch.outcome);
    let reply = engine
        .mutate_source_executor(&executor, h.scope(), key("missing"), missing)
        .unwrap();
    assert!(reply.replayed);
    assert_eq!(reply.outcome, original.outcome);
    let reply = engine
        .mutate_source_executor(&executor, h.scope(), key("invalid"), invalid)
        .unwrap();
    assert!(reply.replayed);
    assert_eq!(reply.outcome, invalid_reply.outcome);
    assert!(
        h.authority
            .inspect_source(h.scope())
            .unwrap()
            .records
            .attempts[&next.binding().source_attempt]
            .executor_grant
            .is_none()
    );
}
