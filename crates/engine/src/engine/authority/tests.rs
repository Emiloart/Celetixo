use super::*;
use crate::access::{AccessSet, Completeness};
use crate::evidence::{Artifact, CheckResult, CheckSpec, ExecutionProvenance};
use crate::patch::Change;
use crate::snapshot::{InclusionPolicy, SnapshotManifest};

fn d(s: &str) -> Digest {
    Digest::of("authority-reconstruction-test/v1", s.as_bytes())
}
fn e(s: &str) -> ElementId {
    ElementId::new(s).unwrap()
}
fn id(s: &str) -> TransactionId {
    TransactionId::new(s).unwrap()
}
fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::new(s).unwrap()
}
struct Harness {
    store: InMemoryStore,
    admin: Administrator,
    engine: Engine,
    worker: Session,
    verifier: Session,
    base: Snapshot,
    check: CheckSpec,
}
impl Harness {
    fn intent(&self, write: &str) -> TransactionIntent {
        TransactionIntent {
            objective: "normalized reopen semantics".into(),
            accesses: AccessSet {
                writes: BTreeSet::from([e(write)]),
                ..Default::default()
            },
            invariants: vec![],
            required_checks: vec![self.check.clone()],
            depends_on: BTreeSet::new(),
        }
    }
    fn new() -> Self {
        let scope = Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("ws").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        };
        let base = Snapshot::new(
            scope.clone(),
            Revision(0),
            SnapshotManifest {
                base_commit: "fixture".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "authority-reconstruction-test/v1".into(),
                inputs: vec![],
                environment: BTreeMap::new(),
            },
            BTreeMap::from([(e("a"), 1), (e("b"), 2)]),
        )
        .unwrap();
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store.clone());
        let worker = admin
            .grant(
                PrincipalId::new("worker").unwrap(),
                scope.clone(),
                Permission::ALL
                    .into_iter()
                    .filter(|p| *p != Permission::Verify)
                    .collect(),
            )
            .unwrap();
        let verifier = admin
            .grant(
                PrincipalId::new("verifier").unwrap(),
                scope,
                BTreeSet::from([Permission::Verify]),
            )
            .unwrap();
        engine
            .register(
                &worker,
                base.clone(),
                EngineeringModel {
                    generation: 1,
                    analyzer: "finite-reference".into(),
                    version: "v1".into(),
                    configuration: d("model"),
                    dependencies: BTreeMap::new(),
                    completeness: Completeness::Complete,
                },
            )
            .unwrap();
        let check = CheckSpec::new(
            CheckId::new("check").unwrap(),
            vec!["reference-only-not-executed".into()],
            "reference/v1".into(),
            d("env"),
            d("config"),
        )
        .unwrap();
        Self {
            store,
            admin,
            engine,
            worker,
            verifier,
            base,
            check,
        }
    }
    fn scope(&self) -> &Scope {
        self.base.scope()
    }
    fn ready(&self, name: &str, write: &str, value: i64) {
        self.engine
            .propose(
                &self.worker,
                self.scope(),
                id(name),
                self.base.id(),
                TransactionIntent {
                    objective: "normalized reopen semantics".into(),
                    accesses: AccessSet {
                        writes: BTreeSet::from([e(write)]),
                        ..Default::default()
                    },
                    invariants: vec![],
                    required_checks: vec![self.check.clone()],
                    depends_on: BTreeSet::new(),
                },
            )
            .unwrap();
        self.engine
            .analyze(&self.worker, self.scope(), &id(name))
            .unwrap();
        self.engine
            .reserve(&self.worker, self.scope(), &id(name), 100)
            .unwrap();
        self.engine
            .execute(&self.worker, self.scope(), &id(name))
            .unwrap();
        self.engine
            .submit_patch(
                &self.worker,
                self.scope(),
                &id(name),
                Patch::new(
                    self.base.id(),
                    BTreeMap::from([(
                        e(write),
                        Change {
                            expected: self.base.state().get(&e(write)).copied(),
                            value: Some(value),
                        },
                    )]),
                )
                .unwrap(),
            )
            .unwrap();
    }
    fn verify(&self, name: &str) {
        verify(
            &self.engine,
            &self.verifier,
            &self.worker,
            self.scope(),
            name,
            &self.check,
        );
    }
    fn request(&self, name: &str, operation: &str) -> CommitRequest {
        self.engine
            .prepare_commit_request(&self.worker, self.scope(), &id(name), key(operation))
            .unwrap()
    }
    fn commit(&self, request: CommitRequest) -> Result<CommitReceipt, EngineError> {
        self.engine
            .commit_prepared(&self.worker, self.scope(), request)
    }
    fn records(&self) -> NativeRecords {
        records::export(&self.store.lock().unwrap(), self.scope()).unwrap()
    }
}

#[test]
fn all_native_phases_and_binary_cancellation_reason_reopen_without_sanitizing_history() {
    let h = Harness::new();
    let roundtrip = || {
        let data = h.records();
        assert_eq!(
            restored(data.clone())
                .native_records(NativeRecordLimits::default())
                .unwrap(),
            data
        );
    };
    roundtrip();
    h.engine
        .propose(
            &h.worker,
            h.scope(),
            id("phases"),
            h.base.id(),
            h.intent("a"),
        )
        .unwrap();
    roundtrip();
    h.engine
        .analyze(&h.worker, h.scope(), &id("phases"))
        .unwrap();
    roundtrip();
    h.engine
        .reserve(&h.worker, h.scope(), &id("phases"), 100)
        .unwrap();
    roundtrip();
    h.engine
        .renew(&h.worker, h.scope(), &id("phases"), 200)
        .unwrap();
    roundtrip();
    h.engine
        .execute(&h.worker, h.scope(), &id("phases"))
        .unwrap();
    roundtrip();
    h.engine
        .submit_patch(
            &h.worker,
            h.scope(),
            &id("phases"),
            Patch::new(
                h.base.id(),
                BTreeMap::from([(
                    e("a"),
                    Change {
                        expected: Some(1),
                        value: Some(10),
                    },
                )]),
            )
            .unwrap(),
        )
        .unwrap();
    roundtrip();
    h.verify("phases");
    roundtrip();
    let reason = "stop\0preserve bytes";
    h.engine
        .abort(&h.worker, h.scope(), &id("phases"), reason.into())
        .unwrap();
    roundtrip();
    h.admin.advance_time(1000).unwrap();
    roundtrip();
    assert_eq!(
        h.records().transactions[&id("phases")]
            .last_reason
            .as_deref(),
        Some(reason)
    );
}

#[test]
fn incomplete_model_remains_incomplete_after_native_reopen() {
    let h = Harness::new();
    h.engine
        .update_model(
            &h.worker,
            h.scope(),
            EngineeringModel {
                generation: 2,
                analyzer: "finite-reference".into(),
                version: "v1".into(),
                configuration: d("incomplete"),
                dependencies: BTreeMap::new(),
                completeness: Completeness::Incomplete {
                    reasons: vec!["unresolved".into()],
                },
            },
        )
        .unwrap();
    h.engine
        .propose(
            &h.worker,
            h.scope(),
            id("incomplete"),
            h.base.id(),
            h.intent("a"),
        )
        .unwrap();
    let state = restored(h.records());
    let (store, admin) = InMemoryStore::new();
    *store.lock().unwrap() = state.db;
    let engine = Engine::new(store);
    let session = admin
        .authenticate_existing_session(h.scope(), h.worker.token, |principal, scope| {
            assert_eq!(principal, h.worker.principal());
            assert_eq!(scope, h.scope());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        engine.analyze(&session, h.scope(), &id("incomplete")),
        Err(EngineError::IncompleteAnalysis)
    );
    assert_eq!(
        engine
            .transaction(&session, h.scope(), &id("incomplete"))
            .unwrap()
            .state,
        TransactionState::Proposed
    );
    assert_eq!(engine.current(&session, h.scope()).unwrap(), h.base);
}
fn verify(
    engine: &Engine,
    verifier: &Session,
    reader: &Session,
    scope: &Scope,
    name: &str,
    check: &CheckSpec,
) {
    let evidence = VerificationEvidence::new(
        engine.bindings(reader, scope, &id(name)).unwrap(),
        check.clone(),
        CheckResult::Passed { exit_code: 0 },
        ExecutionProvenance {
            verifier: verifier.principal().clone(),
            runner: "reference".into(),
            run_id: name.into(),
            started_at: 0,
            finished_at: 0,
        },
        vec![],
        vec![Artifact::new("stdout", "application/octet-stream", b"ok\0\xff".to_vec()).unwrap()],
    )
    .unwrap();
    engine
        .record_evidence(verifier, scope, &id(name), evidence)
        .unwrap();
}
fn restored(records: NativeRecords) -> AuthorityState {
    AuthorityState::from_native_records(records, NativeRecordLimits::default()).unwrap()
}
fn rejects(records: NativeRecords) {
    assert!(AuthorityState::from_native_records(records, NativeRecordLimits::default()).is_err());
}

#[test]
fn reopened_state_retains_original_requests_attempts_receipts_and_requires_fresh_authentication() {
    let h = Harness::new();
    h.ready("a", "a", 10);
    h.verify("a");
    h.ready("b", "b", 20);
    h.verify("b");
    let first = h.request("a", "a");
    let original = h.request("b", "b:0");
    assert_eq!(
        h.engine.commit_prepared_with_fault(
            &h.worker,
            h.scope(),
            first.clone(),
            CommitFault::AfterPublication
        ),
        Err(EngineError::ResponseLost)
    );
    let rejection = h.commit(original.clone());
    assert!(matches!(
        rejection,
        Err(EngineError::RevalidationRequired { .. })
    ));
    h.engine.revalidate(&h.worker, h.scope(), &id("b")).unwrap();
    let before = h.records();
    let state = restored(before.clone());
    assert_eq!(
        state.native_records(NativeRecordLimits::default()).unwrap(),
        before
    );
    assert_eq!(before.transactions[&id("b")].attempts.len(), 2);
    assert_eq!(
        before.transactions[&id("b")]
            .lease
            .as_ref()
            .unwrap()
            .acquired_attempt,
        0
    );
    let (store, admin) = InMemoryStore::new();
    *store.lock().unwrap() = state.db;
    let engine = Engine::new(store);
    assert_eq!(
        engine.current(&h.worker, h.scope()),
        Err(EngineError::Unauthorized)
    );
    assert!(matches!(
        admin.authenticate_existing_session(h.scope(), h.worker.token, |_, _| Err(
            EngineError::Unauthorized
        )),
        Err(EngineError::Unauthorized)
    ));
    let mut authenticated = false;
    let worker = admin
        .authenticate_existing_session(h.scope(), h.worker.token, |principal, scope| {
            assert_eq!(principal, h.worker.principal());
            assert_eq!(scope, h.scope());
            authenticated = true;
            Ok(())
        })
        .unwrap();
    assert!(authenticated);
    let verifier = admin
        .authenticate_existing_session(h.scope(), h.verifier.token, |principal, scope| {
            assert_eq!(principal, h.verifier.principal());
            assert_eq!(scope, h.scope());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        engine.commit_prepared(&worker, h.scope(), original),
        rejection
    );
    let a = engine.commit_prepared(&worker, h.scope(), first).unwrap();
    assert_eq!(a.revision, Revision(1));
    verify(&engine, &verifier, &worker, h.scope(), "b", &h.check);
    let request = engine
        .prepare_commit_request(&worker, h.scope(), &id("b"), key("b:1"))
        .unwrap();
    assert_eq!(
        engine
            .commit_prepared(&worker, h.scope(), request)
            .unwrap()
            .revision,
        Revision(2)
    );
    assert_eq!(
        engine.current(&worker, h.scope()).unwrap().state(),
        &BTreeMap::from([(e("a"), 10), (e("b"), 20)])
    );
    admin.revoke(&verifier).unwrap();
    admin.revoke(&worker).unwrap();
    let data = records::export(&engine.store.lock().unwrap(), h.scope()).unwrap();
    // Revoked grants remain historical facts; they cannot authenticate again.
    let reopened = restored(data.clone());
    assert_eq!(
        reopened
            .native_records(NativeRecordLimits::default())
            .unwrap(),
        data
    );
    assert!(matches!(
        admin.authenticate_existing_session(h.scope(), h.worker.token, |_, _| panic!(
            "inactive grant must reject before authentication"
        )),
        Err(EngineError::Unauthorized)
    ));
}

#[test]
fn evidence_replacement_does_not_reconstruct_an_old_rejected_request_from_current_selection() {
    let h = Harness::new();
    h.ready("a", "a", 10);
    let old = h.request("a", "sticky");
    assert_eq!(
        h.commit(old.clone()),
        Err(EngineError::MissingEvidence(h.check.id().clone()))
    );
    h.verify("a");
    let data = h.records();
    assert!(data.operations[&key("sticky")].request.evidence.is_empty());
    assert!(!data.transactions[&id("a")].attempts[0].evidence.is_empty());
    let state = restored(data.clone());
    assert_eq!(
        state.native_records(NativeRecordLimits::default()).unwrap(),
        data
    );
    let (store, _) = InMemoryStore::new();
    *store.lock().unwrap() = state.db;
    let tx = store.lock().unwrap().operations[&(h.scope().clone(), key("sticky"))].clone();
    assert_eq!(
        tx.outcome,
        Err(EngineError::MissingEvidence(h.check.id().clone()))
    );
    assert_eq!(tx.request, Some(old));
}

#[test]
fn authentication_runs_outside_state_callbacks_and_rechecks_revocation_after_success() {
    let h = Harness::new();
    let admin = h.admin.clone();
    let session = h.worker.clone();
    assert!(matches!(
        h.admin
            .authenticate_existing_session(h.scope(), h.worker.token, |_, _| {
                admin.revoke(&session)?;
                Ok(())
            }),
        Err(EngineError::Unauthorized)
    ));
    assert_eq!(
        h.engine.current(&h.worker, h.scope()),
        Err(EngineError::Unauthorized)
    );
}

#[test]
fn checked_reconstruction_rejects_private_bases_model_substitution_and_forged_candidates() {
    let h = Harness::new();
    h.ready("a", "a", 10);
    h.verify("a");
    let original = h.records();
    let mut bad = original.clone();
    let candidate = bad.transactions[&id("a")].attempts[0]
        .candidate
        .clone()
        .unwrap();
    bad.repository
        .as_mut()
        .unwrap()
        .registered
        .insert(candidate.id(), candidate);
    rejects(bad);
    let mut bad = original.clone();
    bad.transactions.get_mut(&id("a")).unwrap().attempts[0]
        .model
        .dependencies
        .insert(e("a"), BTreeSet::from([e("b")]));
    rejects(bad);
    let mut bad = original.clone();
    bad.transactions.get_mut(&id("a")).unwrap().attempts[0]
        .effective
        .as_mut()
        .unwrap()
        .writes
        .insert(e("undeclared"));
    rejects(bad);
    let mut bad = original.clone();
    let candidate = h
        .base
        .successor(Revision(1), BTreeMap::from([(e("a"), 999), (e("b"), 2)]))
        .unwrap();
    bad.transactions.get_mut(&id("a")).unwrap().attempts[0].candidate = Some(candidate);
    rejects(bad);
    let mut bad = original.clone();
    bad.transactions
        .get_mut(&id("a"))
        .unwrap()
        .lease
        .as_mut()
        .unwrap()
        .owner_grant = h.verifier.token;
    rejects(bad);
    let mut bad = original.clone();
    bad.transactions.get_mut(&id("a")).unwrap().lease_history[0].authorization_grant =
        h.verifier.token;
    rejects(bad);
    let mut bad = original.clone();
    bad.scope.repository = RepositoryId::new("foreign").unwrap();
    rejects(bad);
    let mut bad = original;
    bad.evidence.clear();
    rejects(bad);
}

#[test]
fn terminal_history_missing_evidence_or_receipt_and_cyclic_dependencies_cannot_reopen() {
    let h = Harness::new();
    h.ready("a", "a", 10);
    h.verify("a");
    h.commit(h.request("a", "a")).unwrap();
    let original = h.records();
    restored(original.clone());
    let mut bad = original.clone();
    bad.operations.clear();
    rejects(bad);
    let mut bad = original.clone();
    bad.transactions.get_mut(&id("a")).unwrap().attempts[0]
        .evidence
        .clear();
    rejects(bad);
    let mut bad = original.clone();
    bad.operations
        .get_mut(&key("a"))
        .unwrap()
        .request
        .evidence
        .clear();
    rejects(bad);
    let mut bad = original.clone();
    bad.repository.as_mut().unwrap().history[0]
        .accesses
        .writes
        .clear();
    rejects(bad);
    let mut bad = original.clone();
    bad.repository.as_mut().unwrap().history[0].receipt.revision = Revision(2);
    rejects(bad);
    let mut bad = original.clone();
    bad.transactions
        .get_mut(&id("a"))
        .unwrap()
        .intent
        .depends_on
        .insert(id("a"));
    rejects(bad);
    let mut bad = original.clone();
    bad.transactions.get_mut(&id("a")).unwrap().lease_history[1].at = 100;
    bad.clock_ms = 100;
    rejects(bad);
    let mut bad = original;
    let revoked = LeaseEvent {
        kind: LeaseEventKind::Revoked,
        ..bad.transactions[&id("a")].lease_history[0].clone()
    };
    bad.transactions
        .get_mut(&id("a"))
        .unwrap()
        .lease_history
        .insert(1, revoked);
    rejects(bad);
}

#[test]
fn scoped_restore_budgets_and_backward_clock_fail_closed_and_expiry_is_retained() {
    let h = Harness::new();
    h.ready("a", "a", 10);
    let data = h.records();
    for limits in [
        NativeRecordLimits {
            entries: 1,
            ..Default::default()
        },
        NativeRecordLimits {
            value_bytes: 1,
            ..Default::default()
        },
        NativeRecordLimits {
            total_value_bytes: 1,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            AuthorityState::from_native_records(data.clone(), limits),
            Err(RestoreError::Limit)
        ));
    }
    let mut state = restored(data);
    state.advance_authoritative_clock(100).unwrap();
    assert_eq!(state.clock_ms(), 100);
    assert_eq!(
        state.advance_authoritative_clock(99),
        Err(EngineError::InvalidTime)
    );
    assert_eq!(state.clock_ms(), 100);
    let data = state.native_records(NativeRecordLimits::default()).unwrap();
    assert!(matches!(
        data.transactions[&id("a")]
            .lease_history
            .last()
            .unwrap()
            .kind,
        LeaseEventKind::Expired
    ));
    restored(data);
}

#[derive(Clone, Copy)]
enum BackendBehavior {
    Normal,
    Skip,
    Twice,
    WrongScope,
    PublicationFailure,
}
struct FixtureBackend {
    records: Mutex<NativeRecords>,
    behavior: Mutex<BackendBehavior>,
    contexts: Mutex<Vec<AuthorityAccess>>,
}
impl AuthorityBackend for FixtureBackend {
    fn access(
        &self,
        context: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure> {
        self.contexts.lock().unwrap().push(context.clone());
        let mut records = self.records.lock().unwrap();
        let behavior = *self.behavior.lock().unwrap();
        if matches!(behavior, BackendBehavior::Skip) {
            return Ok(());
        }
        let mut state = restored(records.clone());
        if matches!(behavior, BackendBehavior::WrongScope) {
            state.scope.repository = RepositoryId::new("wrong").unwrap();
            let _ = evaluate(&mut state); // Deliberately broken provider ignores callback failure.
            return Ok(());
        }
        evaluate(&mut state)?;
        if matches!(behavior, BackendBehavior::Twice) {
            let _ = evaluate(&mut state);
        }
        if matches!(behavior, BackendBehavior::PublicationFailure) {
            return Err(AuthorityFailure::CommitIndeterminate);
        }
        if matches!(
            context.mode(),
            AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
        ) {
            let next = state
                .native_records(NativeRecordLimits::default())
                .map_err(|_| AuthorityFailure::Integrity)?;
            // Independently reopen the exported facts before the test adapter publishes.
            AuthorityState::from_native_records(next.clone(), NativeRecordLimits::default())
                .map_err(|_| AuthorityFailure::Integrity)?;
            *records = next;
        }
        Ok(())
    }
}
fn backend(h: &Harness) -> Arc<FixtureBackend> {
    Arc::new(FixtureBackend {
        records: Mutex::new(h.records()),
        behavior: Mutex::new(BackendBehavior::Normal),
        contexts: Mutex::new(vec![]),
    })
}
#[test]
fn scoped_backend_rejects_missing_repeated_misbound_or_unconfirmed_evaluation() {
    let h = Harness::new();
    let backend = backend(&h);
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let worker = admin
        .authenticate_existing_session(h.scope(), h.worker.token, |principal, scope| {
            assert_eq!(principal, h.worker.principal());
            assert_eq!(scope, h.scope());
            Ok(())
        })
        .unwrap();
    assert_eq!(engine.current(&worker, h.scope()).unwrap(), h.base);
    // The prior process's private capability never authenticates this embedding.
    assert_eq!(
        engine.current(&h.worker, h.scope()),
        Err(EngineError::Unauthorized)
    );
    for (behavior, expected) in [
        (BackendBehavior::Skip, AuthorityFailure::Protocol),
        (BackendBehavior::Twice, AuthorityFailure::Protocol),
        (BackendBehavior::WrongScope, AuthorityFailure::Integrity),
        (
            BackendBehavior::PublicationFailure,
            AuthorityFailure::CommitIndeterminate,
        ),
    ] {
        *backend.behavior.lock().unwrap() = behavior;
        assert_eq!(
            engine.current(&worker, h.scope()),
            Err(EngineError::Authority(expected))
        );
    }
    *backend.behavior.lock().unwrap() = BackendBehavior::Normal;
    assert_eq!(
        admin.advance_time(1),
        Err(EngineError::Authority(AuthorityFailure::UnsupportedProfile))
    );
    let contexts = backend.contexts.lock().unwrap();
    assert!(contexts.iter().all(|c| c.scope() == h.scope()));
    assert_eq!(contexts[0].mode(), AuthorityMode::AdministrativeRead);
    assert_eq!(contexts[0].actor(), &AuthorityActor::EmbeddingAdministrator);
    assert!(contexts.iter().any(|c| c.actor()
        == &AuthorityActor::Session {
            principal: worker.principal.clone(),
            grant: worker.token,
            permission: Permission::Read
        }));
}
#[test]
fn backend_runs_existing_commit_gate_and_publishes_rejections_before_acknowledging() {
    let h = Harness::new();
    h.ready("commit", "a", 7);
    let backend = backend(&h);
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let worker = admin
        .authenticate_existing_session(h.scope(), h.worker.token, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.commit(&worker, h.scope(), &id("commit"), key("legacy")),
        Err(EngineError::CommitRequestRequired)
    );
    let request = engine
        .prepare_commit_request(&worker, h.scope(), &id("commit"), key("rejected"))
        .unwrap();
    assert_eq!(
        engine.commit_prepared(&worker, h.scope(), request.clone()),
        Err(EngineError::MissingEvidence(h.check.id().clone()))
    );
    assert!(
        backend
            .records
            .lock()
            .unwrap()
            .operations
            .contains_key(&request.key)
    );
    // A later independently authenticated verifier can supply evidence, but the
    // original rejected operation remains rejected with its original exact body.
    let verifier = admin
        .authenticate_existing_session(h.scope(), h.verifier.token, |_, _| Ok(()))
        .unwrap();
    verify(&engine, &verifier, &worker, h.scope(), "commit", &h.check);
    assert_eq!(
        engine.commit_prepared(&worker, h.scope(), request),
        Err(EngineError::MissingEvidence(h.check.id().clone()))
    );
    let accepted = engine
        .prepare_commit_request(&worker, h.scope(), &id("commit"), key("accepted"))
        .unwrap();
    let receipt = engine
        .commit_prepared(&worker, h.scope(), accepted.clone())
        .unwrap();
    assert_eq!(receipt.revision, Revision(1));
    assert_eq!(
        engine
            .commit_prepared(&worker, h.scope(), accepted)
            .unwrap(),
        receipt
    );
    assert_eq!(
        backend
            .records
            .lock()
            .unwrap()
            .repository
            .as_ref()
            .unwrap()
            .history
            .len(),
        1
    );
}
#[test]
fn read_only_clock_observation_does_not_invent_expiry_history() {
    let h = Harness::new();
    h.ready("observe", "a", 9);
    let mut state = restored(h.records());
    let before = state
        .db
        .transactions
        .values()
        .next()
        .unwrap()
        .lease_history
        .clone();
    state.observe_authoritative_clock(101).unwrap();
    assert_eq!(
        state.db.transactions.values().next().unwrap().lease_history,
        before
    );
    assert_eq!(
        state.observe_authoritative_clock(100),
        Err(EngineError::InvalidTime)
    );
    // A write starts from the stored floor, and records expiry exactly once.
    let mut update = restored(h.records());
    update.advance_authoritative_clock(101).unwrap();
    update.advance_authoritative_clock(102).unwrap();
    assert_eq!(
        update
            .db
            .transactions
            .values()
            .next()
            .unwrap()
            .lease_history
            .len(),
        before.len() + 1
    );
}
