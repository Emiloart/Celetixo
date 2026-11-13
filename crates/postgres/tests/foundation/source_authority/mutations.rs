//! Shared mutation replies restore only after the entire source graph is checked.
use super::*;

#[path = "mutations/administrative.rs"]
mod administrative;
#[path = "mutations/policy.rs"]
mod policy;

fn analysis_for(base: &Snapshot, label: &str) -> RepositoryAnalysis {
    RepositoryAnalysis::new(
        base,
        RepositoryAnalysisInput {
            index_generation: d(&format!("index-{label}")),
            analyzer_version: "complete-synthetic/v1".into(),
            configuration: d(label),
            elements: [element()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element(), [check_id()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap()
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_repository_requests_recover_original_registration_and_analysis_heads() {
    use celetixo_engine::repository::RepositoryAnalysisError;
    use std::sync::atomic::{AtomicBool, Ordering};
    let template = Harness::new("source-repository-template");
    let selected = scope("source-repository-original");
    authorize_source(&mut owner(), &selected);
    let backend = Arc::new(authority());
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner_session = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = Snapshot::new(
        selected.clone(),
        Revision(0),
        template.base.manifest().clone(),
        BTreeMap::new(),
    )
    .unwrap();
    let original = analysis_for(&base, "one");
    let register = MutationRequest::Repository(Box::new(RepositoryMutation::RegisterRepository {
        snapshot: Box::new(base.clone()),
        analysis: Box::new(original.clone()),
    }));
    let only_register = admin
        .grant(
            PrincipalId::new("register-only").unwrap(),
            selected.clone(),
            [Permission::Read, Permission::Register].into(),
        )
        .unwrap();
    assert_eq!(
        engine.mutate(&only_register, &selected, key("register"), register.clone()),
        Err(EngineError::Unauthorized)
    );
    assert!(
        backend
            .inspect_source(&selected)
            .unwrap()
            .records
            .base
            .mutations
            .is_empty()
    );
    let registered = engine
        .mutate(&owner_session, &selected, key("register"), register.clone())
        .unwrap();
    assert!(
        matches!(&registered.outcome, Ok(MutationValue::Repository {model, ..}) if model.generation == 1)
    );
    let next = analysis_for(&base, "two");
    let stale = MutationRequest::Repository(Box::new(RepositoryMutation::InstallAnalysis {
        expected: d("wrong-head"),
        analysis: Box::new(next.clone()),
    }));
    let rejected = engine
        .mutate(&owner_session, &selected, key("stale"), stale.clone())
        .unwrap();
    assert_eq!(
        rejected.outcome,
        Err(EngineError::RepositoryAnalysis(
            RepositoryAnalysisError::StaleGeneration
        ))
    );
    let install = MutationRequest::Repository(Box::new(RepositoryMutation::InstallAnalysis {
        expected: original.digest(),
        analysis: Box::new(next.clone()),
    }));
    let armed = Arc::new(AtomicBool::new(false));
    let active = armed.clone();
    let fault = Arc::new(authority().with_checkpoint(Arc::new(move |stage| {
        if active.load(Ordering::SeqCst) && stage == celetixo_postgres::AuthorityStage::AfterCommit
        {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (fault_engine, fault_admin) = Engine::with_authority_backend(fault);
    let fault_owner = fault_admin
        .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
        .unwrap();
    armed.store(true, Ordering::SeqCst);
    assert_eq!(
        fault_engine.mutate(&fault_owner, &selected, key("install"), install.clone()),
        Err(EngineError::Authority(
            AuthorityFailure::CommitIndeterminate
        ))
    );
    let installed = engine
        .mutate(&owner_session, &selected, key("install"), install.clone())
        .unwrap();
    assert!(installed.replayed);
    assert!(
        matches!(&installed.outcome, Ok(MutationValue::Repository {model, ..}) if model.generation == 2)
    );
    let unchanged = engine
        .mutate(
            &owner_session,
            &selected,
            key("already-current"),
            install.clone(),
        )
        .unwrap();
    assert_eq!(unchanged.outcome, installed.outcome);
    let last = analysis_for(&base, "three");
    engine
        .install_repository_analysis(&owner_session, &selected, next.digest(), last.clone())
        .unwrap();
    let (engine, admin) = Engine::with_authority_backend(Arc::new(authority()));
    let owner_session = admin
        .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
        .unwrap();
    for (operation, request, original_reply) in [
        ("register", register, registered),
        ("stale", stale, rejected),
        ("install", install, installed),
    ] {
        let replay = engine
            .mutate(&owner_session, &selected, key(operation), request)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, original_reply.outcome);
        assert_eq!(replay.recorded_at_ms, original_reply.recorded_at_ms);
    }
    let state = backend.inspect_source(&selected).unwrap().records;
    assert_eq!(state.analysis_head, Some(last.digest()));
    assert_eq!(state.base.repository.as_ref().unwrap().model.generation, 3);
    assert_eq!(state.analyses.len(), 3);
    assert!(state.base.transactions.is_empty());
    assert_eq!(state.base.mutations.len(), 4);
}

fn request(renew: bool, ttl_ms: u64) -> MutationRequest {
    if renew {
        MutationRequest::Renew {
            transaction: transaction_id(),
            ttl_ms,
        }
    } else {
        MutationRequest::Reserve {
            transaction: transaction_id(),
            ttl_ms,
        }
    }
}
fn key(name: &str) -> IdempotencyKey {
    IdempotencyKey::new(name).unwrap()
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_patch_and_revalidation_rejections_never_substitute_for_source_operations() {
    use celetixo_engine::patch::{Change, Patch};
    let mut h = Harness::new("source-native-rejection-replay");
    let requests = [
        MutationRequest::SubmitPatch {
            transaction: transaction_id(),
            patch: Patch::new(
                h.base.id(),
                [(
                    ElementId::new("reference").unwrap(),
                    Change {
                        expected: None,
                        value: Some(1),
                    },
                )]
                .into(),
            )
            .unwrap(),
        },
        MutationRequest::Revalidate {
            transaction: transaction_id(),
        },
    ];
    let replies: Vec<_> = requests
        .iter()
        .enumerate()
        .map(|(i, request)| {
            let reply = h
                .engine
                .mutate(
                    &h.owner,
                    h.scope(),
                    key(&format!("native-{i}")),
                    request.clone(),
                )
                .unwrap();
            assert_eq!(reply.outcome, Err(EngineError::SourceMutationUnsupported));
            reply
        })
        .collect();
    h.propose();
    h.engine
        .abort(
            &h.owner,
            h.scope(),
            &transaction_id(),
            "no source work launched".into(),
        )
        .unwrap();
    h.reopen();
    let before = h.authority.inspect_source(h.scope()).unwrap();
    for (i, (request, original)) in requests.into_iter().zip(replies).enumerate() {
        let replay = h
            .engine
            .mutate(&h.owner, h.scope(), key(&format!("native-{i}")), request)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, original.outcome);
        assert_eq!(replay.recorded_at_ms, original.recorded_at_ms);
    }
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(before.records.base.mutations, after.records.base.mutations);
    assert_eq!(
        before.records.base.transactions,
        after.records.base.transactions
    );
    assert_eq!(
        before.records.base.repository,
        after.records.base.repository
    );
    assert!(after.records.attempts.is_empty());
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_lease_original_results_survive_full_graph_reopen() {
    let mut h = Harness::new("source-mutation-reopen");
    let missing = h
        .engine
        .mutate(&h.owner, h.scope(), key("missing"), request(false, 0))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    h.propose();
    h.engine
        .analyze(&h.owner, h.scope(), &transaction_id())
        .unwrap();
    let acquired = h
        .engine
        .mutate(&h.owner, h.scope(), key("acquire"), request(false, 120_000))
        .unwrap();
    let renewed = h
        .engine
        .mutate(&h.owner, h.scope(), key("renew"), request(true, 180_000))
        .unwrap();
    assert!(acquired.outcome.is_ok());
    assert!(renewed.outcome.is_ok());
    h.reopen();
    let before = h.authority.inspect_source(h.scope()).unwrap();
    for (name, request, expected) in [
        ("missing", request(false, 0), missing),
        ("acquire", request(false, 120_000), acquired),
        ("renew", request(true, 180_000), renewed),
    ] {
        let actual = h
            .engine
            .mutate(&h.owner, h.scope(), key(name), request)
            .unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
        assert_eq!(actual.recorded_at_ms, expected.recorded_at_ms);
    }
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(before.records.base.mutations, after.records.base.mutations);
    assert_eq!(
        before.records.base.transactions,
        after.records.base.transactions
    );
    assert_eq!(after.records.base.mutations.len(), 3);
    assert_eq!(
        after.records.base.transactions[&transaction_id()]
            .lease_history
            .len(),
        2
    );
    assert_eq!(
        h.engine
            .mutate(&h.owner, h.scope(), key("acquire"), request(false, 120_001)),
        Err(EngineError::IdempotencyConflict)
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_cancellation_keeps_execution_reply_and_closes_admitted_work() {
    let mut h = Harness::new("source-mutation-cancel");
    h.propose();
    h.engine
        .analyze(&h.owner, h.scope(), &transaction_id())
        .unwrap();
    h.engine
        .mutate(&h.owner, h.scope(), key("acquire"), request(false, 120_000))
        .unwrap();
    let execute_request = MutationRequest::Execute {
        transaction: transaction_id(),
    };
    let executed = h
        .engine
        .mutate(&h.owner, h.scope(), key("execute"), execute_request.clone())
        .unwrap();
    let patch = h.patch("attempt-before-cancel");
    let admission = h
        .engine
        .admit_source_attempt(&h.owner, h.scope(), patch, key("admission"))
        .unwrap();
    let cancel_request = MutationRequest::Abort {
        transaction: transaction_id(),
        reason: "source cancellation λ\0exact".into(),
    };
    let cancelled = h
        .engine
        .mutate(&h.owner, h.scope(), key("cancel"), cancel_request.clone())
        .unwrap();
    assert!(cancelled.outcome.is_ok());
    h.reopen();
    for (name, request, expected) in [
        ("execute", execute_request, executed),
        ("cancel", cancel_request, cancelled),
    ] {
        let actual = h
            .engine
            .mutate(&h.owner, h.scope(), key(name), request)
            .unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
    }
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(after.records.base.mutations.len(), 3);
    assert_eq!(
        after.records.base.transactions[&transaction_id()].state,
        TransactionState::Aborted
    );
    assert_eq!(
        after.records.attempts[&admission.binding().source_attempt].state,
        SourceAttemptState::Cancelled
    );
    assert!(
        h.engine
            .claim_source_attempt(&h.executor, &admission)
            .is_err()
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_analysis_returns_the_original_effective_accesses_after_abort_and_reopen() {
    let mut h = Harness::new("source-analysis-reopen");
    h.propose();
    let request = MutationRequest::Analyze {
        transaction: transaction_id(),
    };
    let original = h
        .engine
        .mutate(&h.owner, h.scope(), key("analysis"), request.clone())
        .unwrap();
    let Ok(MutationValue::Analysis { attempt, accesses }) = &original.outcome else {
        panic!("source analysis result")
    };
    assert_eq!(*attempt, 0);
    assert!(accesses.writes.contains(&element()));
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("acquire"),
            MutationRequest::Reserve {
                transaction: transaction_id(),
                ttl_ms: 120_000,
            },
        )
        .unwrap();
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("execute"),
            MutationRequest::Execute {
                transaction: transaction_id(),
            },
        )
        .unwrap();
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("cancel"),
            MutationRequest::Abort {
                transaction: transaction_id(),
                reason: "source analysis retained".into(),
            },
        )
        .unwrap();
    h.reopen();
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let actual = h
        .engine
        .mutate(&h.owner, h.scope(), key("analysis"), request)
        .unwrap();
    assert!(actual.replayed);
    assert_eq!(actual.outcome, original.outcome);
    assert_eq!(actual.recorded_at_ms, original.recorded_at_ms);
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(
        before.records.base.transactions,
        after.records.base.transactions
    );
    assert_eq!(before.records.base.mutations, after.records.base.mutations);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_proposal_replays_its_original_state_after_source_execution_and_cancellation() {
    let mut h = Harness::new("source-proposal-recovery");
    let intent = TransactionIntent {
        objective: "keyed source proposal".into(),
        accesses: AccessSet {
            writes: [element()].into(),
            ..Default::default()
        },
        invariants: vec![],
        required_checks: vec![h.policy.checks()[&check_id()].input().check.clone()],
        depends_on: Default::default(),
    };
    let request = MutationRequest::Propose {
        transaction: transaction_id(),
        base: h.base.id(),
        intent: Box::new(intent),
    };
    let original = h
        .engine
        .mutate(&h.owner, h.scope(), key("proposal"), request.clone())
        .unwrap();
    assert_eq!(
        original.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Proposed,
            attempt: 0
        })
    );
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("analysis"),
            MutationRequest::Analyze {
                transaction: transaction_id(),
            },
        )
        .unwrap();
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("reserve"),
            MutationRequest::Reserve {
                transaction: transaction_id(),
                ttl_ms: 120_000,
            },
        )
        .unwrap();
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("execute"),
            MutationRequest::Execute {
                transaction: transaction_id(),
            },
        )
        .unwrap();
    h.engine
        .mutate(
            &h.owner,
            h.scope(),
            key("cancel"),
            MutationRequest::Abort {
                transaction: transaction_id(),
                reason: "source proposal finished".into(),
            },
        )
        .unwrap();
    h.reopen();
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let actual = h
        .engine
        .mutate(&h.owner, h.scope(), key("proposal"), request)
        .unwrap();
    assert!(actual.replayed);
    assert_eq!(actual.outcome, original.outcome);
    assert_eq!(actual.recorded_at_ms, original.recorded_at_ms);
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(
        before.records.base.transactions,
        after.records.base.transactions
    );
    assert_eq!(before.records.base.mutations, after.records.base.mutations);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_lease_reply_survives_an_ambiguous_database_commit() {
    let mut h = Harness::new("source-mutation-uncertain");
    h.propose();
    h.engine
        .analyze(&h.owner, h.scope(), &transaction_id())
        .unwrap();
    let authority = Arc::new(super::super::authority().with_checkpoint(Arc::new(|stage| {
        if stage == AuthorityStage::AfterCommit {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(authority);
    let owner = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.mutate(&owner, h.scope(), key("acquire"), request(false, 120_000)),
        Err(EngineError::Authority(
            AuthorityFailure::CommitIndeterminate
        ))
    );
    h.reopen();
    let actual = h
        .engine
        .mutate(&h.owner, h.scope(), key("acquire"), request(false, 120_000))
        .unwrap();
    assert!(actual.replayed);
    assert!(actual.outcome.is_ok());
    let records = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(records.base.mutations.len(), 1);
    assert_eq!(
        records.base.transactions[&transaction_id()]
            .lease_history
            .len(),
        1
    );
}
