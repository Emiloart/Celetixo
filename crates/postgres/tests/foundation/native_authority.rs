//! Actual SQL lifecycle/failure tests. Reference checks never execute commands.
use super::*;
use celetixo_engine::access::{AccessSet, Predicate};
use celetixo_engine::engine::authority::AuthorityFailure;
use celetixo_engine::engine::*;
use celetixo_engine::evidence::*;
use celetixo_engine::ids::{IdempotencyKey, PrincipalId, TransactionId};
use celetixo_engine::patch::{Change, Patch};
use celetixo_engine::transaction::{TransactionIntent, TransactionState};
use std::time::{Duration, Instant};

#[path = "native_authority/mutations.rs"]
mod mutations;

fn e(name: &str) -> ElementId {
    ElementId::new(name).unwrap()
}
fn id(name: &str) -> TransactionId {
    TransactionId::new(name).unwrap()
}
fn key(name: &str) -> IdempotencyKey {
    IdempotencyKey::new(name).unwrap()
}

struct Harness {
    backend: Arc<PostgresAuthority>,
    engine: Engine,
    admin: Administrator,
    worker: Session,
    verifier: Session,
    base: Snapshot,
    check: CheckSpec,
}
impl Harness {
    fn new(label: &str) -> Self {
        let selected = scope(label);
        authorize_registration(&mut owner(), &selected);
        let backend = Arc::new(sql_authority());
        let (engine, admin) = Engine::with_authority_backend(backend.clone());
        let worker = admin
            .grant(
                PrincipalId::new("worker").unwrap(),
                selected.clone(),
                Permission::ALL
                    .into_iter()
                    .filter(|p| *p != Permission::Verify)
                    .collect(),
            )
            .unwrap();
        let verifier = admin
            .grant(
                PrincipalId::new("verifier").unwrap(),
                selected.clone(),
                BTreeSet::from([Permission::Verify]),
            )
            .unwrap();
        let empty = snapshot(selected, 0);
        let base = Snapshot::new(
            empty.scope().clone(),
            Revision(0),
            empty.manifest().clone(),
            BTreeMap::from([(e("x"), 1), (e("y"), 2)]),
        )
        .unwrap();
        engine
            .register(&worker, base.clone(), native_model(1))
            .unwrap();
        let check = CheckSpec::new(
            CheckId::new("verify").unwrap(),
            vec!["not-executed-reference-check".into()],
            "reference/v1".into(),
            Digest::of("native-sql-test/v1", b"environment"),
            Digest::of("native-sql-test/v1", b"check"),
        )
        .unwrap();
        Self {
            backend,
            engine,
            admin,
            worker,
            verifier,
            base,
            check,
        }
    }
    fn scope(&self) -> &Scope {
        self.base.scope()
    }
    fn reopen(&mut self) {
        let before = self.backend.inspect_registration(self.scope()).unwrap();
        let backend = Arc::new(sql_authority());
        let (engine, admin) = Engine::with_authority_backend(backend.clone());
        let worker = admin
            .authenticate_existing_session(self.scope(), 1, |principal, _| {
                assert_eq!(principal, self.worker.principal());
                Ok(())
            })
            .unwrap();
        let verifier = admin
            .authenticate_existing_session(self.scope(), 2, |principal, _| {
                assert_eq!(principal, self.verifier.principal());
                Ok(())
            })
            .unwrap();
        assert_eq!(backend.inspect_registration(self.scope()).unwrap(), before);
        self.backend = backend;
        self.engine = engine;
        self.admin = admin;
        self.worker = worker;
        self.verifier = verifier;
    }
    fn intent(&self, write: &str) -> TransactionIntent {
        TransactionIntent {
            objective: "native lifecycle λ\0with exact provenance".into(),
            accesses: AccessSet {
                writes: BTreeSet::from([e(write)]),
                predicates: BTreeSet::from([Predicate::Prefix("future:".into())]),
                ..Default::default()
            },
            invariants: vec![],
            required_checks: vec![self.check.clone()],
            depends_on: BTreeSet::new(),
        }
    }
    fn ready(&mut self, name: &str, write: &str, value: i64, verify: bool) {
        self.ready_with_ttl(name, write, value, verify, 120_000);
    }
    fn ready_with_ttl(&mut self, name: &str, write: &str, value: i64, verify: bool, ttl: u64) {
        self.engine
            .propose(
                &self.worker,
                self.scope(),
                id(name),
                self.base.id(),
                self.intent(write),
            )
            .unwrap();
        self.reopen();
        self.engine
            .analyze(&self.worker, self.scope(), &id(name))
            .unwrap();
        self.reopen();
        self.engine
            .reserve(&self.worker, self.scope(), &id(name), ttl)
            .unwrap();
        self.reopen();
        self.engine
            .execute(&self.worker, self.scope(), &id(name))
            .unwrap();
        self.reopen();
        let patch = Patch::new(
            self.base.id(),
            BTreeMap::from([(
                e(write),
                Change {
                    expected: self.base.state().get(&e(write)).copied(),
                    value: Some(value),
                },
            )]),
        )
        .unwrap();
        self.engine
            .submit_patch(&self.worker, self.scope(), &id(name), patch)
            .unwrap();
        self.reopen();
        if verify {
            self.verify(name);
            self.reopen();
        }
    }
    fn evidence(&self, name: &str, run: &str) -> VerificationEvidence {
        self.evidence_result(name, run, CheckResult::Passed { exit_code: 0 })
    }
    fn evidence_result(&self, name: &str, run: &str, result: CheckResult) -> VerificationEvidence {
        VerificationEvidence::new(
            self.engine
                .bindings(&self.worker, self.scope(), &id(name))
                .unwrap(),
            self.check.clone(),
            result,
            ExecutionProvenance {
                verifier: self.verifier.principal().clone(),
                runner: "reference/v1".into(),
                run_id: run.into(),
                started_at: 0,
                finished_at: 0,
            },
            vec!["diagnostic\0λ".into()],
            vec![
                Artifact::new("stdout", "application/octet-stream", b"ok\0\xff".to_vec()).unwrap(),
            ],
        )
        .unwrap()
    }
    fn verify(&self, name: &str) {
        self.engine
            .record_evidence(
                &self.verifier,
                self.scope(),
                &id(name),
                self.evidence(name, name),
            )
            .unwrap();
    }
    // Each operation still uses a fresh SQL connection. Phase-by-phase private
    // Session invalidation is covered by ready/reopen above.
    fn ready_at(
        &self,
        name: &str,
        base: &Snapshot,
        write: &str,
        value: i64,
        intent: TransactionIntent,
    ) {
        self.engine
            .propose(&self.worker, self.scope(), id(name), base.id(), intent)
            .unwrap();
        self.engine
            .analyze(&self.worker, self.scope(), &id(name))
            .unwrap();
        self.engine
            .reserve(&self.worker, self.scope(), &id(name), 600_000)
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
                    base.id(),
                    BTreeMap::from([(
                        e(write),
                        Change {
                            expected: base.state().get(&e(write)).copied(),
                            value: Some(value),
                        },
                    )]),
                )
                .unwrap(),
            )
            .unwrap();
        self.verify(name);
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
    fn counts(&self) -> Vec<i64> {
        let a = [
            self.scope().organization.as_str(),
            self.scope().workspace.as_str(),
            self.scope().repository.as_str(),
        ];
        let mut owner = owner();
        ["committed_changes","transaction_outcomes","operations","outbox","outbox_delivery","reservation_claims"].into_iter().map(|table|owner.query_one(&format!("SELECT count(*) FROM celetixo.{table} WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"),&[&a[0],&a[1],&a[2]]).unwrap().get(0)).collect()
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_lifecycle_reopens_and_commits_receipt_operation_terminal_and_outbox_atomically() {
    let mut h = Harness::new("native-lifecycle");
    h.ready("a", "x", 10, true);
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(
        before.records.transactions[&id("a")].state,
        TransactionState::Validating
    );
    assert_eq!(h.counts(), vec![0, 0, 0, 0, 0, 1]);
    let request = h.request("a", "commit:a");
    assert_eq!(
        h.engine
            .commit(&h.worker, h.scope(), &id("a"), key("legacy")),
        Err(EngineError::CommitRequestRequired)
    );
    let receipt = h.commit(request.clone()).unwrap();
    h.reopen();
    assert_eq!(h.commit(request.clone()), Ok(receipt.clone()));
    assert_eq!(h.counts(), vec![1, 1, 1, 1, 1, 0]);
    assert_eq!(
        h.engine.current(&h.worker, h.scope()).unwrap().state()[&e("x")],
        10
    );
    let after = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(after.records.operations[&request.key].request, request);
    assert_eq!(
        after.records.repository.as_ref().unwrap().history[0].receipt,
        receipt
    );
    assert!(matches!(
        after.records.transactions[&id("a")]
            .lease_history
            .last()
            .unwrap()
            .kind,
        LeaseEventKind::Released { .. }
    ));
    // Delivery progress belongs to the dispatcher; native writes never reset it.
    let s = h.scope();
    owner().execute("UPDATE celetixo.outbox_delivery SET publication_attempts=7,claim_generation=2,published_at=clock_timestamp() WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&s.organization.as_str(),&s.workspace.as_str(),&s.repository.as_str()]).unwrap();
    assert_eq!(h.commit(request), Ok(receipt));
    let row=owner().query_one("SELECT publication_attempts::text,claim_generation::text,published_at IS NOT NULL FROM celetixo.outbox_delivery WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&s.organization.as_str(),&s.workspace.as_str(),&s.repository.as_str()]).unwrap();
    assert_eq!(row.get::<_, String>(0), "7");
    assert_eq!(row.get::<_, String>(1), "2");
    assert!(row.get::<_, bool>(2));
    h.admin.revoke(&h.worker).unwrap();
    assert_eq!(
        h.commit(
            h.backend
                .inspect_registration(h.scope())
                .unwrap()
                .records
                .operations[&key("commit:a")]
                .request
                .clone()
        ),
        Err(EngineError::Unauthorized)
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_original_requests_survive_evidence_replacement_and_revalidation() {
    let mut h = Harness::new("native-revalidation");
    h.ready("a", "x", 10, false);
    h.ready("b", "y", 20, true);
    let rejected = h.request("a", "missing");
    let missing = h.commit(rejected.clone()).unwrap_err();
    assert!(matches!(missing, EngineError::MissingEvidence(_)));
    h.reopen();
    h.engine
        .record_evidence(
            &h.verifier,
            h.scope(),
            &id("a"),
            h.evidence_result("a", "failed", CheckResult::Failed { exit_code: 1 }),
        )
        .unwrap();
    let failed = h.request("a", "failed-check");
    assert_eq!(
        h.commit(failed.clone()),
        Err(EngineError::EvidenceNotPassed(h.check.id().clone()))
    );
    h.reopen();
    h.verify("a");
    h.reopen();
    assert_eq!(
        h.commit(failed.clone()),
        Err(EngineError::EvidenceNotPassed(h.check.id().clone()))
    );
    assert_eq!(h.commit(rejected.clone()), Err(missing));
    let stale = h.request("a", "stale");
    h.commit(h.request("b", "advance")).unwrap();
    h.reopen();
    let stale_result = h.commit(stale.clone());
    assert!(matches!(
        stale_result,
        Err(EngineError::RevalidationRequired { .. })
    ));
    h.reopen();
    let old_lease = h
        .engine
        .lease_history(&h.worker, h.scope(), &id("a"))
        .unwrap()[0]
        .clone();
    h.engine.revalidate(&h.worker, h.scope(), &id("a")).unwrap();
    h.reopen();
    assert_eq!(
        h.engine
            .attempts(&h.worker, h.scope(), &id("a"))
            .unwrap()
            .len(),
        2
    );
    let view = h.backend.inspect_registration(h.scope()).unwrap();
    let lease = view.records.transactions[&id("a")].lease.as_ref().unwrap();
    assert_eq!(lease.acquired_attempt, 0);
    assert_eq!(lease.fence, old_lease.fence);
    assert_eq!(h.commit(stale), stale_result);
    assert_eq!(
        h.commit(rejected.clone()),
        Err(EngineError::MissingEvidence(h.check.id().clone()))
    );
    h.verify("a");
    h.reopen();
    let accepted = h.request("a", "fresh");
    let receipt = h.commit(accepted.clone()).unwrap();
    h.reopen();
    assert_eq!(receipt.revision, Revision(2));
    assert_eq!(receipt.attempt, 1);
    assert_eq!(h.commit(accepted.clone()), Ok(receipt));
    let mut substituted = accepted;
    substituted.key = rejected.key;
    assert_eq!(h.commit(substituted), Err(EngineError::IdempotencyConflict));
    assert_eq!(
        h.commit(failed),
        Err(EngineError::EvidenceNotPassed(h.check.id().clone()))
    );
    assert_eq!(h.counts(), vec![2, 2, 5, 2, 2, 0]);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_repeated_admission_preserves_original_signer_timestamp_and_binary_abort() {
    let mut h = Harness::new("native-admission");
    h.ready("a", "x", 10, true);
    let evidence = h.evidence("a", "a");
    let s = h.scope().clone();
    let admission = || {
        owner().query_one("SELECT admitted_at_ms::text FROM celetixo.reference_evidence_admissions WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND verifier_grant_id=2",&[&s.organization.as_str(),&s.workspace.as_str(),&s.repository.as_str()]).unwrap().get::<_,String>(0)
    };
    let first = admission();
    let second = h
        .admin
        .grant(
            h.verifier.principal().clone(),
            s.clone(),
            BTreeSet::from([Permission::Verify]),
        )
        .unwrap();
    h.engine
        .record_evidence(&second, &s, &id("a"), evidence.clone())
        .unwrap();
    h.engine
        .record_evidence(&h.verifier, &s, &id("a"), evidence)
        .unwrap();
    h.reopen();
    assert_eq!(admission(), first);
    let row=owner().query_one("SELECT verifier_grant_id::text,(SELECT count(*) FROM celetixo.reference_evidence_admissions WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text) FROM celetixo.evidence WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&s.organization.as_str(),&s.workspace.as_str(),&s.repository.as_str()]).unwrap();
    assert_eq!(row.get::<_, String>(0), "2");
    assert_eq!(row.get::<_, i64>(1), 2);
    h.engine
        .abort(&h.worker, &s, &id("a"), "cancel\0λ".into())
        .unwrap();
    h.reopen();
    let view = h.backend.inspect_registration(&s).unwrap();
    assert_eq!(
        view.records.transactions[&id("a")].last_reason.as_deref(),
        Some("cancel\0λ")
    );
    assert_eq!(h.counts(), vec![0, 1, 0, 0, 0, 0]);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_sql_faults_roll_back_or_reconcile_exactly_once() {
    let mut h = Harness::new("native-lifecycle-faults");
    h.ready("a", "x", 10, true);
    let request = h.request("a", "uncertain");
    for fault in [
        CommitFault::BeforeValidation,
        CommitFault::DuringValidation,
        CommitFault::BeforePublication,
    ] {
        let before = h.backend.inspect_registration(h.scope()).unwrap();
        assert_eq!(
            h.engine
                .commit_prepared_with_fault(&h.worker, h.scope(), request.clone(), fault),
            Err(EngineError::InjectedFailure(fault))
        );
        let after = h.backend.inspect_registration(h.scope()).unwrap();
        assert_eq!(after.records.transactions, before.records.transactions);
        assert_eq!(after.records.operations, before.records.operations);
        assert_eq!(h.counts(), vec![0, 0, 0, 0, 0, 1]);
    }
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(|stage| {
        if stage == AuthorityStage::BeforeCommit {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(
        engine.commit_prepared(&worker, h.scope(), request.clone()),
        Err(authority_error(AuthorityFailure::Unavailable))
    );
    assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(|stage| {
        if stage == AuthorityStage::AfterCommit {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.commit_prepared(&worker, h.scope(), request.clone()),
        Err(authority_error(AuthorityFailure::CommitIndeterminate))
    );
    h.reopen();
    let receipt = h.commit(request.clone()).unwrap();
    assert_eq!(h.commit(request), Ok(receipt));
    assert_eq!(h.counts(), vec![1, 1, 1, 1, 1, 0]);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_expiry_read_observation_final_gate_and_revocation_fail_closed() {
    let mut h = Harness::new("native-expiry");
    h.ready_with_ttl("a", "x", 10, true, 30_000);
    h.engine
        .renew(&h.worker, h.scope(), &id("a"), 1_500)
        .unwrap();
    let request = h.request("a", "expiry");
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    let expiry = before.records.transactions[&id("a")]
        .lease
        .as_ref()
        .unwrap()
        .expires_at;
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            let now = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(expiry.saturating_sub(now) + 20));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    // Wait outside the SQL transaction, leaving a short live interval for
    // validation. The trusted hook then crosses expiry without exceeding the
    // configured 10-second idle-transaction deadline.
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(
        expiry.saturating_sub(now).saturating_sub(4_000),
    ));
    assert_eq!(
        engine.commit_prepared(&worker, h.scope(), request.clone()),
        Err(authority_error(AuthorityFailure::Contention))
    );
    assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    assert_eq!(h.counts(), vec![0, 0, 0, 0, 0, 1]);
    assert_eq!(
        h.engine
            .prepare_commit_request(&h.worker, h.scope(), &id("a"), key("expired-read")),
        Err(EngineError::InvalidLease)
    );
    assert_eq!(
        h.backend.inspect_registration(h.scope()).unwrap(),
        before,
        "read expiry must not persist an event or clock"
    );
    assert_eq!(h.commit(request.clone()), Err(EngineError::InvalidLease));
    h.reopen();
    let once = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(
        once.records.transactions[&id("a")]
            .lease_history
            .iter()
            .filter(|e| matches!(e.kind, LeaseEventKind::Expired))
            .count(),
        1
    );
    assert_eq!(h.commit(request), Err(EngineError::InvalidLease));
    assert_eq!(
        h.backend
            .inspect_registration(h.scope())
            .unwrap()
            .records
            .transactions,
        once.records.transactions
    );
    h.engine
        .abort(&h.worker, h.scope(), &id("a"), "expired".into())
        .unwrap();
    h.reopen();
    h.ready("b", "y", 20, true);
    let revoked = h.request("b", "revoked");
    h.admin.revoke(&h.worker).unwrap();
    assert_eq!(h.commit(revoked), Err(EngineError::Unauthorized));
    assert_eq!(h.counts(), vec![0, 1, 1, 0, 0, 0]);
}

fn wait_file(path: &std::path::Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "process barrier timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_independent_processes_race_and_recover_without_duplicate_revision() {
    const CHILD: &str = "CELETIXO_PG_NATIVE_CHILD";
    if let Some(directory) = std::env::var_os(CHILD) {
        let directory = PathBuf::from(directory);
        let name = std::env::var("CELETIXO_PG_NATIVE_TX").unwrap();
        let tag = std::env::var("CELETIXO_PG_NATIVE_TAG").unwrap();
        let selected = Scope {
            organization: OrganizationId::new(std::env::var("CELETIXO_PG_NATIVE_ORG").unwrap())
                .unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repository").unwrap(),
        };
        let (engine, admin) = Engine::with_authority_backend(Arc::new(sql_authority()));
        let worker = admin
            .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
            .unwrap();
        let request = engine
            .prepare_commit_request(&worker, &selected, &id(&name), key(&name))
            .unwrap();
        std::fs::write(directory.join(format!("{tag}.ready")), b"ready").unwrap();
        wait_file(&directory.join("go"));
        let result = engine.commit_prepared(&worker, &selected, request.clone());
        let text = match result {
            Ok(_) => "accepted",
            Err(EngineError::RevalidationRequired { .. }) => "revalidation",
            other => panic!("unexpected process outcome: {other:?}"),
        };
        assert_eq!(engine.commit_prepared(&worker, &selected, request), result);
        std::fs::write(directory.join(format!("{tag}.result")), text).unwrap();
        return;
    }
    for variant in 0..3 {
        let mut h = Harness::new(&format!("native-process-{variant}"));
        h.ready_at("a", &h.base, "x", 10, h.intent("x"));
        if variant != 2 {
            let mut intent = h.intent("y");
            if variant == 1 {
                intent.accesses.reads.insert(e("x"));
            }
            h.ready_at("b", &h.base, "y", 20, intent);
        }
        let directory = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let names = if variant == 2 { ["a", "a"] } else { ["a", "b"] };
        let tags = ["first", "second"];
        let mut children = vec![];
        for (name, tag) in names.into_iter().zip(tags) {
            children.push(Command::new(&executable).args(["--exact","native_authority::native_independent_processes_race_and_recover_without_duplicate_revision","--ignored","--nocapture"]).env(CHILD,directory.path()).env("CELETIXO_PG_NATIVE_TX",name).env("CELETIXO_PG_NATIVE_TAG",tag).env("CELETIXO_PG_NATIVE_ORG",h.scope().organization.as_str()).spawn().unwrap());
        }
        for tag in tags {
            wait_file(&directory.path().join(format!("{tag}.ready")));
        }
        std::fs::write(directory.path().join("go"), b"go").unwrap();
        // Always reap both processes, including a failed contender.
        let statuses = children
            .iter_mut()
            .map(|child| child.wait().unwrap())
            .collect::<Vec<_>>();
        assert!(statuses.iter().all(std::process::ExitStatus::success));
        let raw = tags.map(|tag| {
            std::fs::read_to_string(directory.path().join(format!("{tag}.result"))).unwrap()
        });
        let mut outcomes = raw.clone();
        outcomes.sort();
        assert_eq!(
            outcomes,
            if variant == 2 {
                ["accepted", "accepted"]
            } else {
                ["accepted", "revalidation"]
            }
        );
        h.reopen();
        assert_eq!(
            h.engine.current(&h.worker, h.scope()).unwrap().revision(),
            Revision(1)
        );
        if variant == 2 {
            assert_eq!(h.counts(), vec![1, 1, 1, 1, 1, 0]);
            continue;
        }
        assert_eq!(h.counts(), vec![1, 1, 2, 1, 1, 1]);
        let loser = if raw[0] == "revalidation" { "a" } else { "b" };
        let revalidated = h.engine.revalidate(&h.worker, h.scope(), &id(loser));
        if variant == 1 {
            assert!(matches!(revalidated, Err(EngineError::Conflict(_))));
            h.engine
                .abort(
                    &h.worker,
                    h.scope(),
                    &id(loser),
                    "conflicting process history".into(),
                )
                .unwrap();
            assert_eq!(h.counts(), vec![1, 2, 2, 1, 1, 0]);
        } else {
            revalidated.unwrap();
            h.verify(loser);
            h.commit(h.request(loser, "revalidated")).unwrap();
            h.reopen();
            assert_eq!(
                h.engine.current(&h.worker, h.scope()).unwrap().revision(),
                Revision(2)
            );
            assert_eq!(h.counts(), vec![2, 2, 3, 2, 2, 0]);
        }
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_numeric_attempt_revision_and_event_order_survives_double_digits() {
    let mut h = Harness::new("native-ordinal");
    h.ready_at("long", &h.base, "x", 10, h.intent("x"));
    let first = h
        .engine
        .lease_history(&h.worker, h.scope(), &id("long"))
        .unwrap()[0]
        .clone();
    for number in 1..=11 {
        let current = h.engine.current(&h.worker, h.scope()).unwrap();
        let name = format!("advance:{number}");
        let write = format!("extra:{number}");
        h.ready_at(&name, &current, &write, number, h.intent(&write));
        h.commit(h.request(&name, &name)).unwrap();
        let request = h.request("long", &format!("stale:{number}"));
        assert!(matches!(
            h.commit(request),
            Err(EngineError::RevalidationRequired { .. })
        ));
        h.engine
            .revalidate(&h.worker, h.scope(), &id("long"))
            .unwrap();
        h.verify("long");
    }
    h.reopen();
    let record = h.backend.inspect_registration(h.scope()).unwrap();
    let tx = &record.records.transactions[&id("long")];
    assert_eq!(
        tx.attempts
            .iter()
            .map(|attempt| attempt.number)
            .collect::<Vec<_>>(),
        (0..=11).collect::<Vec<_>>()
    );
    assert_eq!(tx.lease.as_ref().unwrap().acquired_attempt, 0);
    assert_eq!(tx.lease.as_ref().unwrap().fence, first.fence);
    assert_eq!(
        record
            .records
            .repository
            .as_ref()
            .unwrap()
            .history
            .iter()
            .map(|c| c.receipt.revision.0)
            .collect::<Vec<_>>(),
        (1..=11).collect::<Vec<_>>()
    );
    for _ in 0..11 {
        h.engine
            .renew(&h.worker, h.scope(), &id("long"), 600_000)
            .unwrap();
    }
    assert_eq!(
        h.engine
            .lease_history(&h.worker, h.scope(), &id("long"))
            .unwrap()
            .len(),
        12
    );
    h.commit(h.request("long", "final")).unwrap();
    h.reopen();
    assert_eq!(h.counts(), vec![12, 12, 23, 12, 12, 0]);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_generated_pair_histories_match_independent_bitmask_oracle() {
    // Independent integer masks decide expectations; no production conflict or
    // analyzer routine is used as the oracle. The larger Engine oracle retains
    // graph generation and shrinking; this SQL suite checks durable projection.
    let mut saw_compatible = false;
    let mut saw_conflict = false;
    for seed in 0_u32..16 {
        let mut h = Harness::new(&format!("native-oracle-{seed}"));
        let mut a = h.intent("x");
        let mut b = h.intent("y");
        a.accesses.reads.insert(e("shared-read"));
        b.accesses.reads.insert(e("shared-read"));
        let a_read = if seed & 1 != 0 { 2 } else { 0 };
        let b_read = if seed & 2 != 0 { 1 } else { 0 };
        let b_scope = if seed & 4 != 0 { 1 } else { 0 };
        // The shared read alone is compatible. Symmetric write/read still
        // conflicts when a reads y, which b writes.
        if a_read != 0 {
            a.accesses.reads.insert(e("y"));
        }
        if b_read != 0 {
            b.accesses.reads.insert(e("x"));
        }
        if b_scope != 0 {
            b.accesses.invariant_scopes.insert(e("x"));
        }
        let predicate = seed & 8 != 0;
        if predicate {
            b.accesses.predicates.insert(Predicate::Prefix("x".into()));
        }
        let conflict = (a_read & 2 != 0) || (b_read & 1 != 0) || (b_scope & 1 != 0) || predicate;
        h.ready_at("a", &h.base, "x", 10, a);
        h.ready_at("b", &h.base, "y", 20, b);
        h.commit(h.request("a", "first")).unwrap();
        assert!(
            matches!(
                h.commit(h.request("b", "stale")),
                Err(EngineError::RevalidationRequired { .. })
            ),
            "seed {seed}"
        );
        let revalidated = h.engine.revalidate(&h.worker, h.scope(), &id("b"));
        if conflict {
            saw_conflict = true;
            assert!(
                matches!(revalidated, Err(EngineError::Conflict(_))),
                "seed {seed}: {revalidated:?}"
            );
            h.engine
                .abort(&h.worker, h.scope(), &id("b"), "oracle conflict".into())
                .unwrap();
        } else {
            saw_compatible = true;
            revalidated.unwrap();
            h.verify("b");
            h.commit(h.request("b", "second")).unwrap();
        }
        h.reopen();
        let current = h.engine.current(&h.worker, h.scope()).unwrap();
        assert_eq!(
            current.revision().0,
            if conflict { 1 } else { 2 },
            "seed {seed}"
        );
        assert_eq!(current.state()[&e("x")], 10);
        assert_eq!(current.state()[&e("y")], if conflict { 2 } else { 20 });
    }
    assert!(saw_compatible && saw_conflict);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_auxiliary_corruption_and_unsupported_state_cannot_be_silently_repaired() {
    for variant in 0..5 {
        let h = Harness::new(&format!("native-corruption-{variant}"));
        h.ready_at("a", &h.base, "x", 10, h.intent("x"));
        let s = h.scope();
        let a = [
            s.organization.as_str(),
            s.workspace.as_str(),
            s.repository.as_str(),
        ];
        let statement = match variant {
            0 => {
                "UPDATE celetixo.repository_heads SET head_generation=18446744073709551615 WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"
            }
            1 => {
                "UPDATE celetixo.engine_counters SET last_clock_ms=0 WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"
            }
            2 => {
                "DELETE FROM celetixo.reservation_claims WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"
            }
            3 => {
                "DELETE FROM celetixo.reference_evidence_signers WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"
            }
            _ => {
                "UPDATE celetixo.engine_counters SET next_executor_grant=1 WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"
            }
        };
        owner().execute(statement, &[&a[0], &a[1], &a[2]]).unwrap();
        let expected = if variant == 4 {
            AuthorityFailure::UnsupportedProfile
        } else {
            AuthorityFailure::Integrity
        };
        assert_eq!(
            h.backend.inspect_registration(s),
            Err(expected),
            "variant {variant}"
        );
        let generation=owner().query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&a[0],&a[1],&a[2]]).unwrap().get::<_,String>(0);
        assert_eq!(
            h.engine
                .abort(&h.worker, s, &id("a"), "cannot repair".into()),
            Err(authority_error(expected))
        );
        assert_eq!(owner().query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&a[0],&a[1],&a[2]]).unwrap().get::<_,String>(0),generation);
    }
}

pub(super) fn prepare_database_restart() -> (Scope, celetixo_postgres::RegistrationView) {
    let h = Harness::new("native-full-restart");
    for (name, write) in [("committed", "x"), ("pending", "y"), ("aborted", "z")] {
        h.ready_at(name, &h.base, write, 10, h.intent(write));
    }
    h.engine
        .abort(
            &h.worker,
            h.scope(),
            &id("aborted"),
            "restart\0abort".into(),
        )
        .unwrap();
    h.commit(h.request("committed", "restart-operation"))
        .unwrap();
    let reply = h
        .engine
        .mutate(
            &h.worker,
            h.scope(),
            key("restart-renew"),
            MutationRequest::Renew {
                transaction: id("pending"),
                ttl_ms: 300_000,
            },
        )
        .unwrap();
    assert!(reply.outcome.is_ok());
    let rejected = h
        .engine
        .mutate(
            &h.worker,
            h.scope(),
            key("restart-rejected"),
            MutationRequest::Reserve {
                transaction: id("committed"),
                ttl_ms: 0,
            },
        )
        .unwrap();
    assert!(matches!(
        rejected.outcome,
        Err(EngineError::IllegalTransition { .. })
    ));
    (
        h.scope().clone(),
        h.backend.inspect_registration(h.scope()).unwrap(),
    )
}

pub(super) fn assert_database_restart(
    selected: &Scope,
    expected: &celetixo_postgres::RegistrationView,
) {
    let backend = Arc::new(sql_authority());
    assert_eq!(&backend.inspect_registration(selected).unwrap(), expected);
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(selected, 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(expected.records.mutations.len(), 2);
    for (key, record) in &expected.records.mutations {
        let actual = engine
            .mutate(&worker, selected, key.key.clone(), record.request.clone())
            .unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, record.outcome);
        assert_eq!(actual.recorded_at_ms, record.recorded_at_ms);
    }
    let operation = &expected.records.operations[&key("restart-operation")];
    assert_eq!(
        engine.commit_prepared(&worker, selected, operation.request.clone()),
        operation.outcome
    );
    assert_eq!(
        engine
            .transaction(&worker, selected, &id("pending"))
            .unwrap()
            .state,
        TransactionState::Validating
    );
    assert_eq!(
        engine
            .transaction(&worker, selected, &id("aborted"))
            .unwrap()
            .last_reason
            .as_deref(),
        Some("restart\0abort")
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_reservation_conflict_and_distinct_proposal_acquisition_grants_remain_exact() {
    let mut h = Harness::new("native-grant-attribution");
    for name in ["a", "b"] {
        h.engine
            .propose(&h.worker, h.scope(), id(name), h.base.id(), h.intent("x"))
            .unwrap();
        h.engine.analyze(&h.worker, h.scope(), &id(name)).unwrap();
    }
    let acquisition = h
        .admin
        .grant(
            h.worker.principal().clone(),
            h.scope().clone(),
            Permission::ALL
                .into_iter()
                .filter(|p| *p != Permission::Verify)
                .collect(),
        )
        .unwrap();
    h.engine
        .reserve(&acquisition, h.scope(), &id("a"), 120_000)
        .unwrap();
    assert_eq!(
        h.engine.reserve(&h.worker, h.scope(), &id("b"), 120_000),
        Err(EngineError::ReservationConflict)
    );
    assert_eq!(
        h.engine.renew(&h.worker, h.scope(), &id("a"), 120_000),
        Err(EngineError::InvalidLease)
    );
    h.engine.execute(&acquisition, h.scope(), &id("a")).unwrap();
    h.engine
        .submit_patch(
            &acquisition,
            h.scope(),
            &id("a"),
            Patch::new(
                h.base.id(),
                BTreeMap::from([(
                    e("x"),
                    Change {
                        expected: Some(1),
                        value: Some(10),
                    },
                )]),
            )
            .unwrap(),
        )
        .unwrap();
    h.verify("a");
    h.reopen();
    let view = h.backend.inspect_native(h.scope()).unwrap();
    let tx = &view.records.transactions[&id("a")];
    assert_eq!(tx.proposal_grant, 1);
    assert_eq!(tx.lease.as_ref().unwrap().owner_grant, 3);
    assert_eq!(tx.lease_history[0].authorization_grant, 3);
    let acquisition = h
        .admin
        .authenticate_existing_session(h.scope(), 3, |_, _| Ok(()))
        .unwrap();
    let request = h
        .engine
        .prepare_commit_request(&acquisition, h.scope(), &id("a"), key("attribution"))
        .unwrap();
    assert_eq!(request.authorization_grant, 3);
    h.engine
        .commit_prepared(&acquisition, h.scope(), request)
        .unwrap();
    h.engine
        .abort(&h.worker, h.scope(), &id("b"), "competing write".into())
        .unwrap();
    h.reopen();
    assert_eq!(h.counts(), vec![1, 2, 1, 1, 1, 0]);
}

#[path = "commit_transport.rs"]
mod commit_transport;

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_process_death_around_sql_commit_retains_one_unambiguous_outcome() {
    const CHILD: &str = "CELETIXO_PG_COMMIT_CRASH";
    if let Ok(mode) = std::env::var(CHILD) {
        let selected = Scope {
            organization: OrganizationId::new(std::env::var("CELETIXO_PG_NATIVE_ORG").unwrap())
                .unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repository").unwrap(),
        };
        let stage = if mode == "before" {
            AuthorityStage::BeforeCommit
        } else {
            AuthorityStage::AfterCommit
        };
        let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |observed| {
            if observed == stage {
                std::process::exit(if stage == AuthorityStage::BeforeCommit {
                    73
                } else {
                    74
                });
            }
            Ok(())
        })));
        let (engine, admin) = Engine::with_authority_backend(backend);
        let worker = admin
            .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
            .unwrap();
        let request = engine
            .prepare_commit_request(&worker, &selected, &id("a"), key("crash"))
            .unwrap();
        let _ = engine.commit_prepared(&worker, &selected, request);
        panic!("crash point was not reached");
    }
    let mut h = Harness::new("native-process-crash");
    h.ready_at("a", &h.base, "x", 10, h.intent("x"));
    let request = h.request("a", "crash");
    let before = h.backend.inspect_native(h.scope()).unwrap();
    for (mode, code) in [("before", 73), ("after", 74)] {
        let status=Command::new(std::env::current_exe().unwrap()).args(["--exact","native_authority::native_process_death_around_sql_commit_retains_one_unambiguous_outcome","--ignored","--nocapture"]).env(CHILD,mode).env("CELETIXO_PG_NATIVE_ORG",h.scope().organization.as_str()).status().unwrap();
        assert_eq!(status.code(), Some(code));
        if mode == "before" {
            assert_eq!(h.backend.inspect_native(h.scope()).unwrap(), before);
            assert_eq!(h.counts(), vec![0, 0, 0, 0, 0, 1]);
        } else {
            assert_eq!(h.counts(), vec![1, 1, 1, 1, 1, 0]);
        }
    }
    h.reopen();
    let receipt = h.commit(request.clone()).unwrap();
    assert_eq!(h.commit(request), Ok(receipt));
    assert_eq!(h.counts(), vec![1, 1, 1, 1, 1, 0]);
}
