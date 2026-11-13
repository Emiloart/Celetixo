//! Reference-state mutation recovery. A normalized memory backend fault probe
//! establishes callback/publication behavior, rather than PostgreSQL durability.
use celetixo_engine::{
    access::{AccessSet, Completeness},
    engine::{authority::*, *},
    evidence::CheckSpec,
    ids::*,
    model::EngineeringModel,
    snapshot::{InclusionPolicy, Snapshot, SnapshotManifest},
    transaction::TransactionIntent,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Barrier, Mutex,
    atomic::{AtomicU8, Ordering},
};

#[path = "mutation_recovery/native_effects.rs"]
mod native_effects;
#[path = "mutation_recovery/repository.rs"]
mod repository;

#[test]
fn administrative_revocation_replay_does_not_append_another_lease_event() {
    let selected = scope("admin-lease-revocation");
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
    engine.register(&owner, base.clone(), model()).unwrap();
    prepared(&engine, &owner, &base, "work");
    engine
        .reserve(&owner, &selected, &id("work"), 1000)
        .unwrap();
    let request = AdministrativeMutation::RevokeSession {
        grant: 1,
        principal: owner.principal().clone(),
    };
    let original = admin
        .mutate(&selected, key("revoke"), request.clone())
        .unwrap();
    assert_eq!(
        original.outcome,
        Ok(MutationValue::Revocation {
            resource: AdministrativeResource::Session,
            id: 1
        })
    );
    let records = backend.records.lock().unwrap().clone();
    assert_eq!(records.transactions[&id("work")].lease_history.len(), 2);
    assert!(matches!(
        records.transactions[&id("work")].lease_history[1].kind,
        LeaseEventKind::Revoked
    ));
    let (_, fresh) = Engine::with_authority_backend(backend.clone());
    let replay = fresh.mutate(&selected, key("revoke"), request).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, original.outcome);
    assert_eq!(*backend.records.lock().unwrap(), records);
    assert_eq!(
        engine.execute(&owner, &selected, &id("work")),
        Err(EngineError::Unauthorized)
    );
}

fn id(s: &str) -> TransactionId {
    TransactionId::new(s).unwrap()
}
fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::new(s).unwrap()
}
fn scope(s: &str) -> Scope {
    Scope {
        organization: OrganizationId::new(s).unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    }
}
fn snapshot(scope: Scope) -> Snapshot {
    Snapshot::new(
        scope,
        Revision(0),
        SnapshotManifest {
            base_commit: "reference".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "mutation-reference/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        [(ElementId::new("x").unwrap(), 0)].into(),
    )
    .unwrap()
}
fn model() -> EngineeringModel {
    EngineeringModel {
        generation: 1,
        analyzer: "finite-reference".into(),
        version: "v1".into(),
        configuration: Digest::of("mutation-reference/v1", b"configuration"),
        dependencies: BTreeMap::new(),
        completeness: Completeness::Complete,
    }
}
fn intent() -> TransactionIntent {
    TransactionIntent {
        objective: "recover the original mutation".into(),
        accesses: AccessSet {
            writes: [ElementId::new("x").unwrap()].into(),
            ..Default::default()
        },
        invariants: vec![],
        required_checks: vec![
            CheckSpec::new(
                CheckId::new("reference-check").unwrap(),
                vec!["reference-not-executed".into()],
                "reference/v1".into(),
                Digest::of("mutation-reference/v1", b"environment"),
                Digest::of("mutation-reference/v1", b"check"),
            )
            .unwrap(),
        ],
        depends_on: BTreeSet::new(),
    }
}
fn prepared(engine: &Engine, owner: &Session, base: &Snapshot, name: &str) {
    engine
        .propose(owner, base.scope(), id(name), base.id(), intent())
        .unwrap();
    engine.analyze(owner, base.scope(), &id(name)).unwrap();
}
fn reserve(name: &str, ttl_ms: u64) -> MutationRequest {
    MutationRequest::Reserve {
        transaction: id(name),
        ttl_ms,
    }
}
fn renew(name: &str, ttl_ms: u64) -> MutationRequest {
    MutationRequest::Renew {
        transaction: id(name),
        ttl_ms,
    }
}
fn value(expires_at_ms: u64, fence: u64) -> MutationValue {
    MutationValue::Lease {
        expires_at_ms,
        fence,
        acquisition_attempt: 0,
    }
}
fn execute_request(name: &str) -> MutationRequest {
    MutationRequest::Execute {
        transaction: id(name),
    }
}
fn abort_request(name: &str, reason: &str) -> MutationRequest {
    MutationRequest::Abort {
        transaction: id(name),
        reason: reason.into(),
    }
}

#[test]
fn analysis_original_accesses_and_errors_survive_model_changes_and_terminal_state() {
    let scope = scope("analysis-mutation-recovery");
    let backend = Arc::new(FaultBackend {
        records: Mutex::new(NativeRecords::empty(scope.clone())),
        fault: AtomicU8::new(0),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = snapshot(scope.clone());
    engine.register(&owner, base.clone(), model()).unwrap();
    let analyze = |name: &str| MutationRequest::Analyze {
        transaction: id(name),
    };
    let missing = engine
        .mutate(&owner, &scope, key("original-missing"), analyze("work"))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    engine
        .propose(&owner, &scope, id("work"), base.id(), intent())
        .unwrap();
    let original = engine
        .mutate(&owner, &scope, key("analysis"), analyze("work"))
        .unwrap();
    assert!(
        matches!(&original.outcome,Ok(MutationValue::Analysis {attempt:0,accesses}) if accesses.writes==[ElementId::new("x").unwrap()].into())
    );
    let illegal = engine
        .mutate(&owner, &scope, key("second-analysis"), analyze("work"))
        .unwrap();
    assert!(matches!(
        illegal.outcome,
        Err(EngineError::IllegalTransition { .. })
    ));
    engine
        .mutate(&owner, &scope, key("lease"), reserve("work", 120_000))
        .unwrap();
    engine
        .mutate(&owner, &scope, key("execute"), execute_request("work"))
        .unwrap();
    engine
        .mutate(
            &owner,
            &scope,
            key("abort"),
            abort_request("work", "finished"),
        )
        .unwrap();
    engine
        .propose(&owner, &scope, id("stale-model"), base.id(), intent())
        .unwrap();
    let mut updated = model();
    updated.generation = 2;
    engine.update_model(&owner, &scope, updated).unwrap();
    let changed = engine
        .mutate(&owner, &scope, key("changed-model"), analyze("stale-model"))
        .unwrap();
    assert_eq!(changed.outcome, Err(EngineError::ModelChanged));
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .authenticate_existing_session(&scope, 1, |_, _| Ok(()))
        .unwrap();
    let before = backend.records.lock().unwrap().clone();
    for (name, request, expected) in [
        ("original-missing", analyze("work"), missing),
        ("analysis", analyze("work"), original),
        ("second-analysis", analyze("work"), illegal),
        ("changed-model", analyze("stale-model"), changed),
    ] {
        let actual = engine.mutate(&owner, &scope, key(name), request).unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
        assert_eq!(actual.recorded_at_ms, expected.recorded_at_ms);
    }
    assert_eq!(*backend.records.lock().unwrap(), before);
    assert_eq!(
        engine.mutate(&owner, &scope, key("analysis"), analyze("stale-model")),
        Err(EngineError::IdempotencyConflict)
    );
    let mut corrupt = before.clone();
    let record = corrupt
        .mutations
        .iter_mut()
        .find(|(k, _)| k.key.as_str() == "analysis")
        .unwrap()
        .1;
    let Ok(MutationValue::Analysis { accesses, .. }) = &mut record.outcome else {
        panic!("analysis result")
    };
    accesses.writes.clear();
    assert!(AuthorityState::from_native_records(corrupt, NativeRecordLimits::default()).is_err());
    let mut duplicate = before;
    let (key, record) = duplicate
        .mutations
        .iter()
        .find(|(k, _)| k.key.as_str() == "analysis")
        .unwrap();
    let mut other = key.clone();
    other.key = IdempotencyKey::new("aliased-effect").unwrap();
    let record = record.clone();
    duplicate.mutations.insert(other, record);
    assert!(AuthorityState::from_native_records(duplicate, NativeRecordLimits::default()).is_err());
}

#[test]
fn incomplete_analysis_is_an_original_rejection_and_reserves_no_lease() {
    let h = Harness::new("analysis-incomplete");
    let scope = h.base.scope();
    let mut incomplete = model();
    incomplete.generation = 2;
    incomplete.completeness = Completeness::Incomplete {
        reasons: vec!["unresolved dependency".into()],
    };
    h.engine.update_model(&h.owner, scope, incomplete).unwrap();
    h.engine
        .propose(&h.owner, scope, id("work"), h.base.id(), intent())
        .unwrap();
    let request = MutationRequest::Analyze {
        transaction: id("work"),
    };
    let first = h
        .engine
        .mutate(&h.owner, scope, key("analysis"), request.clone())
        .unwrap();
    assert_eq!(first.outcome, Err(EngineError::IncompleteAnalysis));
    let mut complete = model();
    complete.generation = 3;
    h.engine.update_model(&h.owner, scope, complete).unwrap();
    let actual = h
        .engine
        .mutate(&h.owner, scope, key("analysis"), request.clone())
        .unwrap();
    assert!(actual.replayed);
    assert_eq!(actual.outcome, first.outcome);
    assert_eq!(
        h.engine
            .mutate(&h.owner, scope, key("fresh-analysis"), request)
            .unwrap()
            .outcome,
        Err(EngineError::ModelChanged)
    );
    assert!(
        h.engine
            .lease_history(&h.owner, scope, &id("work"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn proposal_original_inputs_and_results_survive_registration_and_later_transitions() {
    use celetixo_engine::transaction::TransactionState;
    let scope = scope("proposal-recovery");
    let backend = Arc::new(FaultBackend {
        records: Mutex::new(NativeRecords::empty(scope.clone())),
        fault: AtomicU8::new(0),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("owner").unwrap(),
            scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = snapshot(scope.clone());
    let proposal = |name: &str, body: TransactionIntent| MutationRequest::Propose {
        transaction: id(name),
        base: base.id(),
        intent: Box::new(body),
    };
    let missing = engine
        .mutate(
            &owner,
            &scope,
            key("before-registration"),
            proposal("work", intent()),
        )
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    let mut invalid = intent();
    invalid.objective = " \n".into();
    invalid.required_checks.clear();
    let rejected = engine
        .mutate(
            &owner,
            &scope,
            key("invalid"),
            proposal("work", invalid.clone()),
        )
        .unwrap();
    assert!(matches!(
        rejected.outcome,
        Err(EngineError::InvalidIntent(_))
    ));
    engine.register(&owner, base.clone(), model()).unwrap();
    let accepted = engine
        .mutate(&owner, &scope, key("proposal"), proposal("work", intent()))
        .unwrap();
    assert_eq!(
        accepted.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Proposed,
            attempt: 0
        })
    );
    engine
        .mutate(
            &owner,
            &scope,
            key("analysis"),
            MutationRequest::Analyze {
                transaction: id("work"),
            },
        )
        .unwrap();
    let existing = engine
        .mutate(&owner, &scope, key("existing"), proposal("work", intent()))
        .unwrap();
    assert_eq!(
        existing.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Analyzed,
            attempt: 0
        })
    );
    engine
        .abort(
            &owner,
            &scope,
            &id("work"),
            "finished after original admission".into(),
        )
        .unwrap();
    let mut changed = intent();
    changed.objective = "a different body".into();
    let conflict = engine
        .mutate(
            &owner,
            &scope,
            key("conflict"),
            proposal("work", changed.clone()),
        )
        .unwrap();
    assert_eq!(conflict.outcome, Err(EngineError::AlreadyExists));
    let mut dependent = intent();
    dependent.depends_on.insert(id("missing-dependency"));
    let dependency = engine
        .mutate(
            &owner,
            &scope,
            key("dependency"),
            proposal("dependent", dependent.clone()),
        )
        .unwrap();
    assert_eq!(dependency.outcome, Err(EngineError::UnsatisfiedDependency));
    let before = backend.records.lock().unwrap().clone();
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .authenticate_existing_session(&scope, 1, |_, _| Ok(()))
        .unwrap();
    for (name, request, expected) in [
        ("before-registration", proposal("work", intent()), missing),
        ("invalid", proposal("work", invalid), rejected),
        ("proposal", proposal("work", intent()), accepted),
        ("existing", proposal("work", intent()), existing),
        ("conflict", proposal("work", changed), conflict),
        ("dependency", proposal("dependent", dependent), dependency),
    ] {
        let actual = engine.mutate(&owner, &scope, key(name), request).unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
        assert_eq!(actual.recorded_at_ms, expected.recorded_at_ms);
    }
    assert_eq!(*backend.records.lock().unwrap(), before);
    let mut corrupt = before;
    let record = corrupt
        .mutations
        .iter_mut()
        .find(|(k, _)| k.key.as_str() == "proposal")
        .unwrap()
        .1;
    let MutationRequest::Propose { base, .. } = &mut record.request else {
        panic!("proposal")
    };
    *base = SnapshotId(Digest::of("wrong-original/v1", b"snapshot"));
    assert!(AuthorityState::from_native_records(corrupt, NativeRecordLimits::default()).is_err());
}

#[test]
fn oversized_original_input_does_not_reserve_a_key_or_cancel_a_transaction() {
    use celetixo_engine::transaction::TransactionState;
    let h = Harness::new("mutation-size-limit");
    let scope = h.base.scope();
    prepared(&h.engine, &h.owner, &h.base, "work");
    let request = MutationRequest::Abort {
        transaction: id("work"),
        reason: "x".repeat(NativeRecordLimits::default().value_bytes + 1),
    };
    assert_eq!(
        h.engine.mutate(&h.owner, scope, key("bounded"), request),
        Err(EngineError::InvalidIntent(
            "mutation request exceeds recovery limits".into()
        ))
    );
    assert!(
        h.engine
            .mutation_outcome(&h.owner, scope, &key("bounded"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        h.engine
            .transaction(&h.owner, scope, &id("work"))
            .unwrap()
            .state,
        TransactionState::Analyzed
    );
    let accepted = h
        .engine
        .mutate(
            &h.owner,
            scope,
            key("bounded"),
            abort_request("work", "within recovery limits"),
        )
        .unwrap();
    assert!(!accepted.replayed);
    assert_eq!(
        accepted.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Aborted,
            attempt: 0
        })
    );
}

#[test]
fn execution_and_cancellation_replies_are_historical_and_do_not_restore_authority() {
    use celetixo_engine::transaction::TransactionState;
    let h = Harness::new("state-mutations");
    let scope = h.base.scope();
    prepared(&h.engine, &h.owner, &h.base, "work");
    let rejected = h
        .engine
        .mutate(&h.owner, scope, key("too-early"), execute_request("work"))
        .unwrap();
    assert_eq!(rejected.outcome, Err(EngineError::InvalidLease));
    h.engine
        .mutate(&h.owner, scope, key("lease"), reserve("work", 120_000))
        .unwrap();
    let executed = h
        .engine
        .mutate(&h.owner, scope, key("execute"), execute_request("work"))
        .unwrap();
    assert_eq!(
        executed.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Executing,
            attempt: 0
        })
    );
    let cancelled = h
        .engine
        .mutate(
            &h.owner,
            scope,
            key("abort"),
            abort_request("work", "cancel λ\0with provenance"),
        )
        .unwrap();
    assert_eq!(
        cancelled.outcome,
        Ok(MutationValue::State {
            state: TransactionState::Aborted,
            attempt: 0
        })
    );
    for (key_name, request, expected) in [
        ("too-early", execute_request("work"), rejected),
        ("execute", execute_request("work"), executed),
        (
            "abort",
            abort_request("work", "cancel λ\0with provenance"),
            cancelled,
        ),
    ] {
        let reply = h
            .engine
            .mutate(&h.owner, scope, key(key_name), request)
            .unwrap();
        assert!(reply.replayed);
        assert_eq!(reply.outcome, expected.outcome);
        assert_eq!(reply.recorded_at_ms, expected.recorded_at_ms);
    }
    assert_eq!(
        h.engine.execute(&h.owner, scope, &id("work")),
        Err(EngineError::InvalidLease)
    );
    let repeated = h
        .engine
        .mutate(
            &h.owner,
            scope,
            key("same-cancel-new-key"),
            abort_request("work", "cancel λ\0with provenance"),
        )
        .unwrap();
    assert!(!repeated.replayed);
    assert!(repeated.outcome.is_ok());
    assert_eq!(
        h.engine
            .lease_history(&h.owner, scope, &id("work"))
            .unwrap()
            .len(),
        2
    );
    assert!(matches!(
        h.engine
            .mutate(
                &h.owner,
                scope,
                key("different-reason"),
                abort_request("work", "different")
            )
            .unwrap()
            .outcome,
        Err(EngineError::IllegalTransition { .. })
    ));
}

#[test]
fn cancellation_preserves_original_bad_reason_and_checks_its_own_permission() {
    let h = Harness::new("mutation-cancel-permission");
    let scope = h.base.scope();
    let bad = h
        .engine
        .mutate(
            &h.owner,
            scope,
            key("bad-reason"),
            abort_request("future", " \n\t"),
        )
        .unwrap();
    assert_eq!(
        bad.outcome,
        Err(EngineError::InvalidIntent("abort needs reason".into()))
    );
    prepared(&h.engine, &h.owner, &h.base, "future");
    let replay = h
        .engine
        .mutate(
            &h.owner,
            scope,
            key("bad-reason"),
            abort_request("future", " \n\t"),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, bad.outcome);
    let cancellation = h
        .admin
        .grant(
            PrincipalId::new("canceller").unwrap(),
            scope.clone(),
            [Permission::Read, Permission::Cancel].into(),
        )
        .unwrap();
    assert_eq!(
        h.engine
            .mutate(&cancellation, scope, key("reserve"), reserve("future", 10)),
        Err(EngineError::Unauthorized)
    );
    assert!(
        h.engine
            .mutate(
                &cancellation,
                scope,
                key("cancel"),
                abort_request("future", "authorized cancellation")
            )
            .unwrap()
            .outcome
            .is_ok()
    );
    assert_eq!(
        h.engine.mutate(
            &cancellation,
            scope,
            key("cancel"),
            abort_request("future", "changed reason")
        ),
        Err(EngineError::IdempotencyConflict)
    );
    h.admin.revoke(&cancellation).unwrap();
    assert_eq!(
        h.engine.mutate(
            &cancellation,
            scope,
            key("cancel"),
            abort_request("future", "authorized cancellation")
        ),
        Err(EngineError::Unauthorized)
    );
}

#[test]
fn normalized_recovery_keeps_execution_and_cancellation_receipts_after_terminal_state() {
    let scope = scope("state-mutation-recovery");
    let backend = Arc::new(FaultBackend {
        records: Mutex::new(NativeRecords::empty(scope.clone())),
        fault: AtomicU8::new(0),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = snapshot(scope.clone());
    engine.register(&owner, base.clone(), model()).unwrap();
    prepared(&engine, &owner, &base, "work");
    engine
        .mutate(&owner, &scope, key("lease"), reserve("work", 120_000))
        .unwrap();
    let executed = engine
        .mutate(&owner, &scope, key("execute"), execute_request("work"))
        .unwrap();
    let cancelled = engine
        .mutate(
            &owner,
            &scope,
            key("abort"),
            abort_request("work", "exact original reason"),
        )
        .unwrap();
    let before = backend.records.lock().unwrap().clone();
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .authenticate_existing_session(&scope, 1, |_, _| Ok(()))
        .unwrap();
    for (name, request, expected) in [
        ("execute", execute_request("work"), executed),
        (
            "abort",
            abort_request("work", "exact original reason"),
            cancelled,
        ),
    ] {
        let actual = engine.mutate(&owner, &scope, key(name), request).unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, expected.outcome);
    }
    assert_eq!(*backend.records.lock().unwrap(), before);
    let mut corrupt = before;
    let record = corrupt
        .mutations
        .iter_mut()
        .find(|(key, _)| key.key.as_str() == "abort")
        .unwrap()
        .1;
    record.request = abort_request("work", "substituted historical reason");
    assert!(AuthorityState::from_native_records(corrupt, NativeRecordLimits::default()).is_err());
}
struct Harness {
    engine: Engine,
    admin: Administrator,
    owner: Session,
    base: Snapshot,
}
impl Harness {
    fn new(label: &str) -> Self {
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
        let base = snapshot(scope(label));
        let owner = admin
            .grant(
                PrincipalId::new("human").unwrap(),
                base.scope().clone(),
                Permission::ALL.into(),
            )
            .unwrap();
        engine.register(&owner, base.clone(), model()).unwrap();
        Self {
            engine,
            admin,
            owner,
            base,
        }
    }
}

#[test]
fn exact_lease_retries_preserve_the_first_expiry_and_never_restore_expired_authority() {
    let h = Harness::new("lease-retry");
    let scope = h.base.scope();
    prepared(&h.engine, &h.owner, &h.base, "work");
    let first = h
        .engine
        .mutate(&h.owner, scope, key("reserve"), reserve("work", 10))
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(first.outcome, Ok(value(10, 1)));
    h.admin.advance_time(2).unwrap();
    let replay = h
        .engine
        .mutate(&h.owner, scope, key("reserve"), reserve("work", 10))
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, first.outcome);
    assert_eq!(replay.recorded_at_ms, first.recorded_at_ms);
    assert_eq!(
        h.engine
            .lease_history(&h.owner, scope, &id("work"))
            .unwrap()
            .len(),
        1
    );
    let renewed = h
        .engine
        .mutate(&h.owner, scope, key("renew"), renew("work", 20))
        .unwrap();
    assert_eq!(renewed.outcome, Ok(value(22, 1)));
    h.admin.advance_time(5).unwrap();
    assert_eq!(
        h.engine
            .mutate(&h.owner, scope, key("renew"), renew("work", 20))
            .unwrap()
            .outcome,
        renewed.outcome
    );
    assert_eq!(
        h.engine
            .lease_history(&h.owner, scope, &id("work"))
            .unwrap()
            .len(),
        2
    );
    h.admin.advance_time(20).unwrap();
    let late = h
        .engine
        .mutate(&h.owner, scope, key("renew"), renew("work", 20))
        .unwrap();
    assert_eq!(late.outcome, renewed.outcome);
    assert_eq!(late.recorded_at_ms, renewed.recorded_at_ms);
    assert!(late.replayed);
    assert_eq!(
        h.engine.execute(&h.owner, scope, &id("work")),
        Err(EngineError::InvalidLease)
    );
    assert_eq!(
        h.engine
            .lease_history(&h.owner, scope, &id("work"))
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        h.engine
            .mutation_outcome(&h.owner, scope, &key("renew"))
            .unwrap(),
        Some(late)
    );
    assert_eq!(
        h.engine
            .mutate(&h.owner, scope, key("renew-new"), renew("work", 20))
            .unwrap()
            .outcome,
        Err(EngineError::InvalidLease)
    );
}

#[test]
fn rejected_original_bodies_survive_later_registration_and_key_substitution_rejects() {
    let h = Harness::new("original-rejection");
    let scope = h.base.scope();
    let missing = h
        .engine
        .mutate(&h.owner, scope, key("missing"), reserve("later", 10))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    prepared(&h.engine, &h.owner, &h.base, "later");
    assert_eq!(
        h.engine
            .mutate(&h.owner, scope, key("missing"), reserve("later", 10))
            .unwrap()
            .outcome,
        missing.outcome
    );
    let invalid = h
        .engine
        .mutate(&h.owner, scope, key("invalid-ttl"), reserve("later", 0))
        .unwrap();
    assert_eq!(invalid.outcome, Err(EngineError::InvalidTime));
    assert_eq!(
        h.engine
            .mutate(&h.owner, scope, key("invalid-ttl"), reserve("later", 10)),
        Err(EngineError::IdempotencyConflict)
    );
    assert!(
        h.engine
            .lease_history(&h.owner, scope, &id("later"))
            .unwrap()
            .is_empty()
    );
    let accepted = h
        .engine
        .mutate(&h.owner, scope, key("fresh"), reserve("later", 10))
        .unwrap();
    assert_eq!(accepted.outcome, Ok(value(10, 1)));
    assert_eq!(
        h.engine
            .mutate(&h.owner, scope, key("fresh"), renew("later", 10)),
        Err(EngineError::IdempotencyConflict)
    );
}

#[test]
fn actor_scoped_keys_and_fresh_authorization_prevent_poisoning_or_revocation_replay() {
    let h = Harness::new("actor-scope");
    let scope = h.base.scope();
    let other = h
        .admin
        .grant(
            PrincipalId::new("other").unwrap(),
            scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    prepared(&h.engine, &h.owner, &h.base, "work");
    let rejected = h
        .engine
        .mutate(&other, scope, key("same-key"), reserve("work", 10))
        .unwrap();
    assert_eq!(rejected.outcome, Err(EngineError::Unauthorized));
    let accepted = h
        .engine
        .mutate(&h.owner, scope, key("same-key"), reserve("work", 10))
        .unwrap();
    assert_eq!(accepted.outcome, Ok(value(10, 1)));
    assert!(!accepted.replayed);
    assert_eq!(
        h.engine
            .mutation_outcome(&other, scope, &key("same-key"))
            .unwrap()
            .unwrap()
            .outcome,
        rejected.outcome
    );
    assert_eq!(
        h.engine.mutate(
            &h.owner,
            &self::scope("foreign"),
            key("same-key"),
            reserve("work", 10)
        ),
        Err(EngineError::Unauthorized)
    );
    h.admin.revoke(&h.owner).unwrap();
    assert_eq!(
        h.engine
            .mutate(&h.owner, scope, key("same-key"), reserve("work", 10)),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(
        h.engine.mutation_outcome(&h.owner, scope, &key("same-key")),
        Err(EngineError::Unauthorized)
    );
}

#[test]
fn concurrent_duplicates_have_one_transition_and_different_requests_cannot_share_a_key() {
    for different in [false, true] {
        let h = Harness::new(if different {
            "different-race"
        } else {
            "duplicate-race"
        });
        prepared(&h.engine, &h.owner, &h.base, "work");
        let barrier = Arc::new(Barrier::new(2));
        let threads = (0..2)
            .map(|number| {
                let engine = h.engine.clone();
                let owner = h.owner.clone();
                let scope = h.base.scope().clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    engine.mutate(
                        &owner,
                        &scope,
                        key("race"),
                        reserve("work", if different && number == 1 { 20 } else { 10 }),
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .filter(|r| r.as_ref().is_ok_and(|r| !r.replayed))
                .count(),
            1
        );
        if different {
            assert_eq!(
                results
                    .iter()
                    .filter(|r| **r == Err(EngineError::IdempotencyConflict))
                    .count(),
                1
            );
        } else {
            assert_eq!(
                results
                    .iter()
                    .filter(|r| r.as_ref().is_ok_and(|r| r.replayed))
                    .count(),
                1
            );
            assert_eq!(
                results[0].as_ref().unwrap().outcome,
                results[1].as_ref().unwrap().outcome
            );
        }
        assert_eq!(
            h.engine
                .lease_history(&h.owner, h.base.scope(), &id("work"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            h.engine
                .reservation(&h.owner, h.base.scope(), &id("work"))
                .unwrap()
                .fence,
            1
        );
    }
}

struct FaultBackend {
    records: Mutex<NativeRecords>,
    fault: AtomicU8,
}
impl AuthorityBackend for FaultBackend {
    fn access(
        &self,
        context: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure> {
        if self.fault.load(Ordering::SeqCst) == 1 {
            return Err(AuthorityFailure::Unavailable);
        }
        let mut records = self.records.lock().unwrap();
        if records.scope != *context.scope() {
            return Err(AuthorityFailure::ScopeDenied);
        }
        let limits = NativeRecordLimits::default();
        let mut state = AuthorityState::from_native_records(records.clone(), limits)
            .map_err(|_| AuthorityFailure::Integrity)?;
        evaluate(&mut state)?;
        if self.fault.load(Ordering::SeqCst) == 2 {
            return Err(AuthorityFailure::Unavailable);
        }
        *records = state
            .native_records(limits)
            .map_err(|_| AuthorityFailure::UnsupportedProfile)?;
        Ok(())
    }
}

#[test]
fn backend_failure_before_or_after_evaluation_never_falls_back_or_publishes_only_the_lease() {
    let scope = scope("storage-failure");
    let backend = Arc::new(FaultBackend {
        records: Mutex::new(NativeRecords::empty(scope.clone())),
        fault: AtomicU8::new(0),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = snapshot(scope.clone());
    engine.register(&owner, base.clone(), model()).unwrap();
    prepared(&engine, &owner, &base, "work");
    for fault in [1, 2] {
        let before = backend.records.lock().unwrap().clone();
        backend.fault.store(fault, Ordering::SeqCst);
        assert_eq!(
            engine.mutate(&owner, &scope, key("fault"), reserve("work", 10)),
            Err(EngineError::Authority(AuthorityFailure::Unavailable))
        );
        backend.fault.store(0, Ordering::SeqCst);
        assert_eq!(*backend.records.lock().unwrap(), before);
        assert_eq!(
            engine
                .mutation_outcome(&owner, &scope, &key("fault"))
                .unwrap(),
            None
        );
        assert!(
            engine
                .lease_history(&owner, &scope, &id("work"))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn normalized_reopen_retains_original_results_and_rejects_corrupt_history_bindings() {
    let scope = scope("normalized-recovery");
    let backend = Arc::new(FaultBackend {
        records: Mutex::new(NativeRecords::empty(scope.clone())),
        fault: AtomicU8::new(0),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let base = snapshot(scope.clone());
    let missing = engine
        .mutate(&owner, &scope, key("missing"), reserve("later", 10))
        .unwrap();
    assert_eq!(missing.outcome, Err(EngineError::NotFound));
    engine.register(&owner, base.clone(), model()).unwrap();
    prepared(&engine, &owner, &base, "later");
    let first = engine
        .mutate(&owner, &scope, key("reserve"), reserve("later", 10))
        .unwrap();
    let second = engine
        .mutate(&owner, &scope, key("renew"), renew("later", 20))
        .unwrap();
    let before = backend.records.lock().unwrap().clone();
    assert_eq!(before.mutations.len(), 3);
    drop((engine, admin));
    let (fresh, admin) = Engine::with_authority_backend(backend.clone());
    assert_eq!(
        fresh.mutate(&owner, &scope, key("reserve"), reserve("later", 10)),
        Err(EngineError::Unauthorized)
    );
    let authenticated = admin
        .authenticate_existing_session(&scope, 1, |principal, actual| {
            assert_eq!(principal.as_str(), "human");
            assert_eq!(actual, &scope);
            Ok(())
        })
        .unwrap();
    for (name, request, original) in [
        ("missing", reserve("later", 10), missing),
        ("reserve", reserve("later", 10), first),
        ("renew", renew("later", 20), second),
    ] {
        let replay = fresh
            .mutate(&authenticated, &scope, key(name), request)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, original.outcome);
        assert_eq!(replay.recorded_at_ms, original.recorded_at_ms);
    }
    assert_eq!(*backend.records.lock().unwrap(), before);
    assert_eq!(
        fresh
            .lease_history(&authenticated, &scope, &id("later"))
            .unwrap()
            .len(),
        2
    );
    for corruption in 0..4 {
        let mut bad = before.clone();
        let mutation_key = bad
            .mutations
            .keys()
            .find(|k| k.key == key("reserve"))
            .unwrap()
            .clone();
        let record = bad.mutations.get_mut(&mutation_key).unwrap();
        match corruption {
            0 => record.outcome = Ok(value(999, 1)),
            1 => record.request = reserve("later", 999),
            2 => {
                record.state_before = Some(celetixo_engine::transaction::TransactionState::Proposed)
            }
            _ => record.lease_events_before = 1,
        }
        assert!(
            AuthorityState::from_native_records(bad, NativeRecordLimits::default()).is_err(),
            "accepted corrupt journal field {corruption}"
        );
    }
    let mut duplicate = before.clone();
    let (original_key, original) = duplicate
        .mutations
        .iter()
        .find(|(k, _)| k.key == key("reserve"))
        .unwrap();
    let mut second_key = original_key.clone();
    second_key.key = key("aliased-effect");
    let original = original.clone();
    duplicate.mutations.insert(second_key, original);
    assert!(AuthorityState::from_native_records(duplicate, NativeRecordLimits::default()).is_err());
    assert!(
        AuthorityState::from_native_records(
            before,
            NativeRecordLimits {
                entries: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
}
