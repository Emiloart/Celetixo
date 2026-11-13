//! Original executor replies through a normalized atomic fault backend. This
//! establishes Engine semantics, not PostgreSQL durability or launch isolation.
use super::*;
use celetixo_engine::engine::authority::*;
use celetixo_engine::persistence::{
    MutationOutcomeRecord, restore_mutation_outcome, restore_mutation_request,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

#[path = "mutation_layout.rs"]
mod layout;

struct Backend {
    records: Mutex<SourceRecords>,
    fault: AtomicU8,
    now: AtomicU64,
}
impl AuthorityBackend for Backend {
    fn access(
        &self,
        context: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure> {
        let mut stored = self.records.lock().unwrap();
        if stored.base.scope != *context.scope() {
            return Err(AuthorityFailure::ScopeDenied);
        }
        let update = matches!(
            context.mode(),
            AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
        );
        let fault = if update {
            self.fault.swap(0, Ordering::SeqCst)
        } else {
            0
        };
        if fault == 1 {
            return Err(AuthorityFailure::Unavailable);
        }
        let mut state = AuthorityState::from_source_records(stored.clone(), Default::default())
            .map_err(|_| AuthorityFailure::Integrity)?;
        if update {
            state.advance_authoritative_clock(self.now.load(Ordering::SeqCst))
        } else {
            state.observe_authoritative_clock(self.now.load(Ordering::SeqCst))
        }
        .map_err(|_| AuthorityFailure::ClockRegression)?;
        evaluate(&mut state)?;
        let after = state
            .source_records(Default::default())
            .map_err(|_| AuthorityFailure::Integrity)?;
        AuthorityState::from_source_records(after.clone(), Default::default())
            .map_err(|_| AuthorityFailure::Integrity)?;
        if fault == 2 {
            return Err(AuthorityFailure::Unavailable);
        }
        if update {
            *stored = after;
        }
        if fault == 3 {
            return Err(AuthorityFailure::CommitIndeterminate);
        }
        Ok(())
    }
}
fn key(value: &str) -> IdempotencyKey {
    IdempotencyKey::new(value).unwrap()
}
fn open(backend: Arc<Backend>) -> (Engine, Administrator, Session, SourceExecutorCapability) {
    let (engine, admin) = Engine::with_authority_backend(backend);
    let owner = admin
        .authenticate_existing_session(&scope(), 1, |_, _| Ok(()))
        .unwrap();
    let executor = admin
        .authenticate_existing_source_executor(&scope(), 1, |_, _| Ok(()))
        .unwrap();
    (engine, admin, owner, executor)
}
fn fixture() -> (Harness, SourceAdmission, Arc<Backend>) {
    let mut h = Harness::new(false);
    let tx = h.execute("executor-mutation");
    let admission = h.admit(&tx, "source-original");
    let records = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    let backend = Arc::new(Backend {
        now: AtomicU64::new(records.base.clock_ms),
        records: Mutex::new(records),
        fault: AtomicU8::new(0),
    });
    let (engine, admin, owner, executor) = open(backend.clone());
    h.engine = engine;
    h.admin = admin;
    h.owner = owner;
    h.executor = executor;
    (h, admission, backend)
}
fn call(
    h: &Harness,
    admission: &SourceAdmission,
    name: &str,
    action: SourceExecutorAction,
) -> MutationResponse {
    h.engine
        .mutate_source_executor(
            &h.executor,
            &scope(),
            key(name),
            SourceExecutorMutation::new(admission, action),
        )
        .unwrap()
}
fn finish() -> SourceExecutorAction {
    SourceExecutorAction::Finish {
        outcome: SourceAttemptState::ExecutionFailed,
        reason: "original worker failure".into(),
    }
}
fn records(backend: &Backend) -> SourceRecords {
    backend.records.lock().unwrap().clone()
}

#[test]
fn original_executor_results_and_rejections_survive_terminal_state_and_fresh_embedding() {
    let (h, a, backend) = fixture();
    let rejected = call(&h, &a, "too-early", SourceExecutorAction::BeginVerification);
    assert_eq!(
        rejected.outcome,
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
    let mut originals = vec![(
        "too-early",
        SourceExecutorAction::BeginVerification,
        rejected,
    )];
    for (name, action) in [
        ("claim", SourceExecutorAction::Claim),
        ("claimed-again", SourceExecutorAction::Claim),
        ("patch", SourceExecutorAction::PatchApplied),
        ("verify", SourceExecutorAction::BeginVerification),
        ("finish", finish()),
    ] {
        let reply = call(&h, &a, name, action.clone());
        if name == "claimed-again" {
            assert_eq!(
                reply.outcome,
                Err(SourceAttemptError::AlreadyClaimed.into())
            );
        } else {
            assert!(matches!(reply.outcome, Ok(MutationValue::SourceAttempt(_))));
        }
        originals.push((name, action, reply));
    }
    let before = records(&backend);
    let (engine, _, _, executor) = open(backend.clone());
    for (name, action, expected) in originals {
        let reply = engine
            .mutate_source_executor(
                &executor,
                &scope(),
                key(name),
                SourceExecutorMutation::new(&a, action),
            )
            .unwrap();
        assert!(reply.replayed);
        assert_eq!(reply.outcome, expected.outcome);
        assert_eq!(reply.recorded_at_ms, expected.recorded_at_ms);
        assert_eq!(
            engine
                .source_executor_mutation_outcome(&executor, &scope(), &key(name))
                .unwrap()
                .unwrap()
                .outcome,
            expected.outcome
        );
    }
    assert_eq!(records(&backend), before);
    assert_eq!(before.attempts[&a.binding().source_attempt].events.len(), 5);
    assert_eq!(
        engine.validate_source_attempt(&executor, &a),
        Err(SourceAttemptError::InvalidState.into())
    );
    let mut changed = SourceExecutorMutation::new(&a, finish());
    if let SourceExecutorAction::Finish { reason, .. } = &mut changed.action {
        *reason = "replacement failure".into();
    }
    assert_eq!(
        engine.mutate_source_executor(&executor, &scope(), key("finish"), changed),
        Err(EngineError::IdempotencyConflict)
    );
    assert_eq!(records(&backend), before);
}

#[test]
fn executor_faults_roll_back_or_reconcile_each_transition_without_a_second_event() {
    let (h, a, backend) = fixture();
    for (name, action) in [
        ("claim", SourceExecutorAction::Claim),
        ("patch", SourceExecutorAction::PatchApplied),
        ("verify", SourceExecutorAction::BeginVerification),
        ("finish", finish()),
    ] {
        let request = SourceExecutorMutation::new(&a, action);
        for fault in [1, 2] {
            let before = records(&backend);
            backend.fault.store(fault, Ordering::SeqCst);
            assert_eq!(
                h.engine
                    .mutate_source_executor(&h.executor, &scope(), key(name), request.clone()),
                Err(EngineError::Authority(AuthorityFailure::Unavailable))
            );
            assert_eq!(records(&backend), before);
        }
        backend.fault.store(3, Ordering::SeqCst);
        assert_eq!(
            h.engine
                .mutate_source_executor(&h.executor, &scope(), key(name), request.clone()),
            Err(EngineError::Authority(
                AuthorityFailure::CommitIndeterminate
            ))
        );
        let before = records(&backend);
        let (engine, _, _, executor) = open(backend.clone());
        let reply = engine
            .mutate_source_executor(&executor, &scope(), key(name), request)
            .unwrap();
        assert!(reply.replayed && reply.outcome.is_ok());
        assert_eq!(records(&backend), before);
    }
    let state = records(&backend);
    assert_eq!(state.base.mutations.len(), 4);
    assert_eq!(state.attempts[&a.binding().source_attempt].events.len(), 5);
}

#[test]
fn missing_source_selector_remains_missing_in_its_original_reply_after_later_admission() {
    let (h, a, backend) = fixture();
    let mut missing = SourceExecutorMutation::new(&a, SourceExecutorAction::Claim);
    missing.binding.source_attempt = SourceAttemptId::new("future").unwrap();
    let reply = h
        .engine
        .mutate_source_executor(&h.executor, &scope(), key("missing"), missing.clone())
        .unwrap();
    assert_eq!(reply.outcome, Err(EngineError::NotFound));
    call(&h, &a, "claim", SourceExecutorAction::Claim);
    call(
        &h,
        &a,
        "cancel",
        SourceExecutorAction::Finish {
            outcome: SourceAttemptState::Cancelled,
            reason: "release original source attempt".into(),
        },
    );
    let next = h.admit(&a.binding().transaction, "future");
    let before = records(&backend);
    let (engine, _, _, executor) = open(backend.clone());
    let replay = engine
        .mutate_source_executor(&executor, &scope(), key("missing"), missing)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, reply.outcome);
    assert_eq!(records(&backend), before);
    assert!(
        before.attempts[&next.binding().source_attempt]
            .executor_grant
            .is_none()
    );
    assert!(
        engine
            .mutate_source_executor(
                &executor,
                &scope(),
                key("new-claim"),
                SourceExecutorMutation::new(&next, SourceExecutorAction::Claim)
            )
            .unwrap()
            .outcome
            .is_ok()
    );
}

#[test]
fn invalid_finish_expiry_reconciliation_and_revocation_keep_original_history() {
    let (h, a, backend) = fixture();
    let invalid = SourceExecutorAction::Finish {
        outcome: SourceAttemptState::Verifying,
        reason: "invalid nonterminal finish".into(),
    };
    assert_eq!(
        call(&h, &a, "invalid", invalid.clone()).outcome,
        Err(SourceAttemptError::InvalidReason.into())
    );
    backend.now.store(a.expires_at_ms() + 1, Ordering::SeqCst);
    assert_eq!(
        call(&h, &a, "expired", SourceExecutorAction::Claim).outcome,
        Err(SourceAttemptError::InvalidState.into())
    );
    assert_eq!(
        records(&backend).attempts[&a.binding().source_attempt].state,
        SourceAttemptState::TimedOut
    );
    let after = records(&backend);
    assert!(call(&h, &a, "invalid", invalid).replayed);
    assert_eq!(records(&backend), after);
    h.admin.revoke_source_executor(&h.executor).unwrap();
    let before = records(&backend);
    assert_eq!(
        h.engine.mutate_source_executor(
            &h.executor,
            &scope(),
            key("expired"),
            SourceExecutorMutation::new(&a, SourceExecutorAction::Claim)
        ),
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
    assert_eq!(
        h.engine
            .source_executor_mutation_outcome(&h.executor, &scope(), &key("expired")),
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
    assert_eq!(
        h.engine.mutate(
            &h.owner,
            &scope(),
            key("self-executor"),
            MutationRequest::SourceExecutor(Box::new(SourceExecutorMutation::new(
                &a,
                SourceExecutorAction::Claim
            )))
        ),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(records(&backend), before);
}

#[test]
fn executor_original_keys_bind_current_grant_scope_store_and_expiry() {
    let (h, a, backend) = fixture();
    let metadata = records(&backend).executors[&1].clone();
    // Keep the native lease beyond executor expiry so this test distinguishes
    // denied executor access from the separate native lease-expiry mechanism.
    h.engine
        .renew(
            &h.owner,
            &scope(),
            &a.binding().transaction,
            metadata.expires_at_ms + 1,
        )
        .unwrap();
    let original = call(&h, &a, "claim", SourceExecutorAction::Claim);
    assert!(original.outcome.is_ok());
    let second = h
        .admin
        .grant_source_executor(
            h.executor.principal().clone(),
            scope(),
            metadata.backend.clone(),
            metadata.tiers.clone(),
            metadata.resource_ceiling,
            metadata.expires_at_ms,
        )
        .unwrap();
    assert!(
        h.engine
            .source_executor_mutation_outcome(&second, &scope(), &key("claim"))
            .unwrap()
            .is_none()
    );
    let reply = h
        .engine
        .mutate_source_executor(
            &second,
            &scope(),
            key("claim"),
            SourceExecutorMutation::new(&a, SourceExecutorAction::Claim),
        )
        .unwrap();
    assert!(!reply.replayed);
    assert_eq!(
        reply.outcome,
        Err(SourceAttemptError::AlreadyClaimed.into())
    );
    let before = records(&backend);
    assert_eq!(before.base.mutations.len(), 2);
    let (cold, _, _, _) = open(backend.clone());
    assert_eq!(
        cold.source_executor_mutation_outcome(&h.executor, &scope(), &key("claim")),
        Err(EngineError::Unauthorized)
    );
    let foreign = Scope {
        organization: OrganizationId::new("foreign").unwrap(),
        ..scope()
    };
    assert_eq!(
        h.engine
            .source_executor_mutation_outcome(&h.executor, &foreign, &key("claim")),
        Err(EngineError::Authority(AuthorityFailure::ScopeDenied))
    );
    assert_eq!(records(&backend), before);
    backend.now.store(metadata.expires_at_ms, Ordering::SeqCst);
    assert_eq!(
        h.engine
            .source_executor_mutation_outcome(&h.executor, &scope(), &key("claim")),
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
    assert_eq!(
        h.engine.mutate_source_executor(
            &h.executor,
            &scope(),
            key("claim"),
            SourceExecutorMutation::new(&a, SourceExecutorAction::Claim)
        ),
        Err(SourceAttemptError::UntrustedExecutor.into())
    );
    // The independent backend clock advances during the denied update. Every
    // other normalized fact, including the original replies and events, stays exact.
    let mut expected = before;
    expected.base.clock_ms = metadata.expires_at_ms;
    assert_eq!(records(&backend), expected);
}

#[test]
fn source_journal_codec_and_normalized_recovery_reject_substituted_context_and_events() {
    let (h, a, backend) = fixture();
    for (name, action) in [
        ("claim", SourceExecutorAction::Claim),
        ("patch", SourceExecutorAction::PatchApplied),
        ("verify", SourceExecutorAction::BeginVerification),
        ("finish", finish()),
    ] {
        call(&h, &a, name, action);
    }
    let state = records(&backend);
    for (key, record) in &state.base.mutations {
        let input = record.request.canonical_bytes(&scope(), &key.actor);
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &key.actor,
                record.request.digest(&scope(), &key.actor),
                &input,
                Default::default()
            )
            .unwrap(),
            record.request
        );
        let encoded = MutationOutcomeRecord::new(scope(), key.clone(), record.clone()).unwrap();
        let bytes = encoded.canonical_bytes();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                key,
                &record.request,
                encoded.digest(),
                &bytes,
                Default::default()
            )
            .unwrap(),
            encoded
        );
        for end in 0..input.len() {
            let prefix = &input[..end];
            assert!(
                restore_mutation_request(
                    &scope(),
                    &key.actor,
                    Digest::of(MutationRequest::FORMAT, prefix),
                    prefix,
                    Default::default()
                )
                .is_err()
            );
        }
        for end in 0..bytes.len() {
            let prefix = &bytes[..end];
            assert!(
                restore_mutation_outcome(
                    &scope(),
                    key,
                    &record.request,
                    Digest::of(MutationOutcomeRecord::FORMAT, prefix),
                    prefix,
                    Default::default()
                )
                .is_err()
            );
        }
    }
    for variant in 0..6 {
        let mut forged = state.clone();
        let key = MutationKey {
            actor: MutationActor::SourceExecutor {
                principal: h.executor.principal().clone(),
                grant: 1,
            },
            key: key("claim"),
        };
        let record = forged.base.mutations.get_mut(&key).unwrap();
        match variant {
            0 => record.source_execution.as_mut().unwrap().events_before += 1,
            1 => record.source_execution.as_mut().unwrap().executor_before = Some(1),
            2 => {
                record.source_execution.as_mut().unwrap().admission =
                    Some(SourceAdmissionId(d("substitution")))
            }
            3 => {
                let Ok(MutationValue::SourceAttempt(value)) = &mut record.outcome else {
                    panic!()
                };
                value.events[0].reason = "substitution".into();
            }
            4 => {
                let Ok(MutationValue::SourceAttempt(value)) = &mut record.outcome else {
                    panic!()
                };
                value.binding.producer = "substitution".into();
            }
            5 => {
                let Ok(MutationValue::SourceAttempt(value)) = &mut record.outcome else {
                    panic!()
                };
                value.executor = Some(PrincipalId::new("substitution").unwrap());
            }
            _ => unreachable!(),
        }
        assert!(
            AuthorityState::from_source_records(forged, Default::default()).is_err(),
            "accepted forged variant {variant}"
        );
    }
}
