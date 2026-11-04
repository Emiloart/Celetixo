use celetixo_engine::{
    access::{AccessSet, Completeness, Condition, Invariant, Predicate},
    engine::{Administrator, CommitFault, Engine, EngineError, InMemoryStore, Permission, Session},
    evidence::{
        Artifact, CheckResult, CheckSpec, EvidenceBindings, ExecutionProvenance,
        VerificationEvidence,
    },
    ids::*,
    model::EngineeringModel,
    patch::{Change, Patch},
    snapshot::{InclusionPolicy, Snapshot, SnapshotManifest},
    transaction::{TransactionIntent, TransactionState},
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Barrier};

fn element(id: &str) -> ElementId {
    ElementId::new(id).unwrap()
}
fn transaction(id: &str) -> TransactionId {
    TransactionId::new(id).unwrap()
}
fn key(id: &str) -> IdempotencyKey {
    IdempotencyKey::new(id).unwrap()
}
fn accesses(reads: &[&str], writes: &[&str]) -> AccessSet {
    AccessSet {
        reads: reads.iter().map(|s| element(s)).collect(),
        writes: writes.iter().map(|s| element(s)).collect(),
        ..Default::default()
    }
}

struct Harness {
    store: InMemoryStore,
    admin: Administrator,
    engine: Engine,
    operator: Session,
    worker: Session,
    verifier: Session,
    scope: Scope,
    base: Snapshot,
    model: EngineeringModel,
    check: CheckSpec,
}
impl Harness {
    fn new() -> Self {
        let scope = Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("work").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        };
        let (store, admin) = InMemoryStore::new();
        let operator = admin
            .grant(
                PrincipalId::new("operator").unwrap(),
                scope.clone(),
                Permission::ALL.into(),
            )
            .unwrap();
        let worker = admin
            .grant(
                PrincipalId::new("worker").unwrap(),
                scope.clone(),
                [
                    Permission::Read,
                    Permission::Propose,
                    Permission::Analyze,
                    Permission::Reserve,
                    Permission::Execute,
                    Permission::Commit,
                    Permission::Cancel,
                ]
                .into(),
            )
            .unwrap();
        let verifier = admin
            .grant(
                PrincipalId::new("verifier").unwrap(),
                scope.clone(),
                [Permission::Read, Permission::Verify].into(),
            )
            .unwrap();
        let base = Snapshot::new(
            scope.clone(),
            Revision(0),
            SnapshotManifest {
                base_commit: "git:initial".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "trusted-reference-capture/v1".into(),
                inputs: vec![],
                environment: BTreeMap::new(),
            },
            [(element("x"), 1), (element("y"), 2), (element("z"), 3)].into(),
        )
        .unwrap();
        let model = EngineeringModel {
            generation: 1,
            analyzer: "declared-reference".into(),
            version: "1".into(),
            configuration: Digest::of("config", b"1"),
            dependencies: BTreeMap::new(),
            completeness: Completeness::Complete,
        };
        let check = CheckSpec::new(
            CheckId::new("reference-invariants").unwrap(),
            vec!["reference-evaluator/v1".into()],
            "reference-state/v1".into(),
            Digest::of("environment", b"reference"),
            Digest::of("checks", b"invariants/v1"),
        )
        .unwrap();
        let engine = Engine::new(store.clone());
        engine
            .register(&operator, base.clone(), model.clone())
            .unwrap();
        Self {
            store,
            admin,
            engine,
            operator,
            worker,
            verifier,
            scope,
            base,
            model,
            check,
        }
    }
    fn intent(&self, accesses: AccessSet, invariants: Vec<Invariant>) -> TransactionIntent {
        TransactionIntent {
            objective: "test semantic transition".into(),
            accesses,
            invariants,
            required_checks: vec![self.check.clone()],
            depends_on: BTreeSet::new(),
        }
    }
    fn propose(&self, id: &str, accesses: AccessSet, invariants: Vec<Invariant>) {
        self.engine
            .propose(
                &self.worker,
                &self.scope,
                transaction(id),
                self.base.id(),
                self.intent(accesses, invariants),
            )
            .unwrap();
        self.engine
            .analyze(&self.worker, &self.scope, &transaction(id))
            .unwrap();
    }
    fn execute(&self, id: &str, edits: &[(&str, Option<i64>, Option<i64>)]) -> EvidenceBindings {
        self.engine
            .reserve(&self.worker, &self.scope, &transaction(id), 100)
            .unwrap();
        self.engine
            .execute(&self.worker, &self.scope, &transaction(id))
            .unwrap();
        let patch = Patch::new(
            self.base.id(),
            edits
                .iter()
                .map(|(id, expected, value)| {
                    (
                        element(id),
                        Change {
                            expected: *expected,
                            value: *value,
                        },
                    )
                })
                .collect(),
        )
        .unwrap();
        self.engine
            .submit_patch(&self.worker, &self.scope, &transaction(id), patch)
            .unwrap()
    }
    fn evidence(
        &self,
        bindings: EvidenceBindings,
        check: CheckSpec,
        result: CheckResult,
    ) -> VerificationEvidence {
        let candidate = self
            .engine
            .candidate(&self.verifier, &self.scope, &bindings.transaction)
            .unwrap();
        // This verifier is explicitly trusted reference code; it retains the actual state
        // it examined. Negative tests can attest a failed/incomplete execution as well.
        let transcript = format!(
            "candidate={} state={:?} result={result:?}",
            candidate.id(),
            candidate.state()
        );
        VerificationEvidence::new(
            bindings.clone(),
            check,
            result,
            ExecutionProvenance {
                verifier: self.verifier.principal().clone(),
                runner: "reference-evaluator/v1".into(),
                run_id: format!("{}:{}", bindings.transaction, bindings.attempt),
                started_at: 0,
                finished_at: 0,
            },
            vec![],
            vec![Artifact::new("report", "text/plain", transcript.into_bytes()).unwrap()],
        )
        .unwrap()
    }
    fn verify(&self, id: &str) {
        let bindings = self
            .engine
            .bindings(&self.verifier, &self.scope, &transaction(id))
            .unwrap();
        let evidence = self.evidence(
            bindings,
            self.check.clone(),
            CheckResult::Passed { exit_code: 0 },
        );
        self.engine
            .record_evidence(&self.verifier, &self.scope, &transaction(id), evidence)
            .unwrap();
    }
    fn ready(&self, id: &str, access: AccessSet, edits: &[(&str, Option<i64>, Option<i64>)]) {
        self.propose(id, access, vec![]);
        self.execute(id, edits);
        self.verify(id);
    }
    fn current(&self) -> Snapshot {
        self.engine.current(&self.worker, &self.scope).unwrap()
    }
    fn commit(
        &self,
        id: &str,
        operation: &str,
    ) -> Result<celetixo_engine::engine::CommitReceipt, EngineError> {
        self.engine
            .commit(&self.worker, &self.scope, &transaction(id), key(operation))
    }
}

#[test]
fn compatible_same_snapshot_transactions_require_fresh_attempt_evidence_and_preserve_both_writes() {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    h.ready("b", accesses(&[], &["y"]), &[("y", Some(2), Some(20))]);
    let old = h
        .engine
        .attempts(&h.worker, &h.scope, &transaction("b"))
        .unwrap();
    h.commit("a", "commit-a").unwrap();
    assert!(matches!(
        h.commit("b", "old-b"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let bindings = h
        .engine
        .revalidate(&h.worker, &h.scope, &transaction("b"))
        .unwrap();
    assert_eq!(bindings.attempt, 1);
    assert_ne!(bindings.base_snapshot, h.base.id());
    assert!(matches!(
        h.commit("b", "unverified-b"),
        Err(EngineError::MissingEvidence(_))
    ));
    h.verify("b");
    let receipt = h.commit("b", "fresh-b").unwrap();
    assert_eq!(receipt.revision, Revision(2));
    assert_eq!(h.current().state()[&element("x")], 10);
    assert_eq!(h.current().state()[&element("y")], 20);
    let attempts = h
        .engine
        .attempts(&h.worker, &h.scope, &transaction("b"))
        .unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], old[0]);
    assert_ne!(attempts[0].patch, attempts[1].patch);
    assert_ne!(attempts[0].candidate, attempts[1].candidate);
}

#[test]
fn invalidated_read_is_explained_with_both_transactions_revision_and_analysis_provenance() {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    h.ready("b", accesses(&["x"], &["y"]), &[("y", Some(2), Some(20))]);
    h.commit("a", "a").unwrap();
    assert!(matches!(
        h.commit("b", "b"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let unchanged = h.current();
    let Err(EngineError::Conflict(reports)) =
        h.engine.revalidate(&h.worker, &h.scope, &transaction("b"))
    else {
        panic!("invalid read accepted")
    };
    assert_eq!(reports[0].contender, transaction("b"));
    assert_eq!(reports[0].committed, transaction("a"));
    assert_eq!(reports[0].committed_revision, Revision(1));
    assert!(
        reports[0]
            .contender_provenance
            .explicit
            .reads
            .contains(&element("x"))
    );
    assert_eq!(
        reports[0].contender_provenance.analysis.snapshot,
        h.base.id()
    );
    assert_eq!(h.current(), unchanged);
}

#[test]
fn write_write_reservation_prevents_partial_acquisition_and_stale_writer_cannot_rebase_over_committed_value()
 {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    h.propose("b", accesses(&[], &["x", "y"]), vec![]);
    assert_eq!(
        h.engine
            .reserve(&h.worker, &h.scope, &transaction("b"), 100),
        Err(EngineError::ReservationConflict)
    );
    h.ready("c", accesses(&[], &["y"]), &[("y", Some(2), Some(20))]);
    h.commit("a", "a").unwrap();
    h.engine
        .abort(
            &h.worker,
            &h.scope,
            &transaction("c"),
            "release test lease".into(),
        )
        .unwrap();
    h.execute("b", &[("x", Some(1), Some(11))]);
    h.verify("b");
    assert!(matches!(
        h.commit("b", "b"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    assert!(matches!(
        h.engine.revalidate(&h.worker, &h.scope, &transaction("b")),
        Err(EngineError::Conflict(_))
    ));
    assert_eq!(h.current().state()[&element("x")], 10);
}

#[test]
fn predicate_phantoms_and_invariant_scope_overlap_are_conflicts() {
    for predicate in [true, false] {
        let h = Harness::new();
        let mut assumptions = accesses(&[], &["y"]);
        if predicate {
            assumptions
                .predicates
                .insert(Predicate::Prefix("new:".into()));
        } else {
            assumptions.invariant_scopes.insert(element("new:item"));
        }
        h.ready("reader", assumptions, &[("y", Some(2), Some(4))]);
        h.ready(
            "creator",
            accesses(&[], &["new:item"]),
            &[("new:item", None, Some(8))],
        );
        h.commit("creator", "creator").unwrap();
        assert!(matches!(
            h.commit("reader", "reader"),
            Err(EngineError::RevalidationRequired { .. })
        ));
        assert!(matches!(
            h.engine
                .revalidate(&h.worker, &h.scope, &transaction("reader")),
            Err(EngineError::Conflict(_))
        ));
        assert_eq!(h.current().revision(), Revision(1));
    }
}

#[test]
fn actual_invariants_reject_bad_patches_and_derive_undeclared_dependencies() {
    let h = Harness::new();
    let invariant = Invariant {
        id: InvariantId::new("budget").unwrap(),
        condition: Condition::SumAtMost {
            elements: [element("x"), element("y")].into(),
            maximum: 10,
        },
    };
    h.propose("bad", accesses(&[], &["x"]), vec![invariant]);
    h.engine
        .reserve(&h.worker, &h.scope, &transaction("bad"), 100)
        .unwrap();
    h.engine
        .execute(&h.worker, &h.scope, &transaction("bad"))
        .unwrap();
    let patch = Patch::new(
        h.base.id(),
        [(
            element("x"),
            Change {
                expected: Some(1),
                value: Some(20),
            },
        )]
        .into(),
    )
    .unwrap();
    assert!(matches!(
        h.engine
            .submit_patch(&h.worker, &h.scope, &transaction("bad"), patch),
        Err(EngineError::Invariant(_))
    ));
    assert_eq!(h.current(), h.base);
    assert_eq!(
        h.engine
            .transaction(&h.worker, &h.scope, &transaction("bad"))
            .unwrap()
            .state,
        TransactionState::Executing
    );
}

#[test]
fn missing_failed_and_incomplete_evidence_cannot_pass_the_gate() {
    for status in [
        None,
        Some(CheckResult::Failed { exit_code: 1 }),
        Some(CheckResult::Incomplete {
            reason: "interrupted".into(),
        }),
    ] {
        let h = Harness::new();
        h.propose("a", accesses(&[], &["x"]), vec![]);
        let bindings = h.execute("a", &[("x", Some(1), Some(10))]);
        if let Some(status) = status {
            let evidence = h.evidence(bindings, h.check.clone(), status);
            h.engine
                .record_evidence(&h.verifier, &h.scope, &transaction("a"), evidence)
                .unwrap();
            assert!(matches!(
                h.commit("a", "a"),
                Err(EngineError::EvidenceNotPassed(_))
            ));
        } else {
            assert!(matches!(
                h.commit("a", "a"),
                Err(EngineError::MissingEvidence(_))
            ));
        }
        assert_eq!(h.current(), h.base);
    }
}

#[test]
fn exact_evidence_gate_rejects_each_mismatch_before_it_can_authorize_a_commit() {
    for field in 0..10 {
        let h = Harness::new();
        h.propose("a", accesses(&[], &["x"]), vec![]);
        let mut binding = h.execute("a", &[("x", Some(1), Some(10))]);
        let mut check = h.check.clone();
        let fake = Digest::of("wrong", b"identity");
        match field {
            0 => binding.transaction = transaction("different"),
            1 => binding.attempt = 9,
            2 => binding.base_snapshot = SnapshotId(fake),
            3 => binding.candidate_snapshot = SnapshotId(fake),
            4 => binding.patch = PatchId(fake),
            5 => binding.analysis = fake,
            6 => {
                check = CheckSpec::new(
                    h.check.id().clone(),
                    h.check.command().to_vec(),
                    h.check.toolchain().into(),
                    fake,
                    h.check.configuration(),
                )
                .unwrap()
            }
            7 => {
                check = CheckSpec::new(
                    h.check.id().clone(),
                    h.check.command().to_vec(),
                    "other-toolchain".into(),
                    h.check.environment(),
                    h.check.configuration(),
                )
                .unwrap()
            }
            8 => {
                check = CheckSpec::new(
                    h.check.id().clone(),
                    vec!["different-command".into()],
                    h.check.toolchain().into(),
                    h.check.environment(),
                    h.check.configuration(),
                )
                .unwrap()
            }
            _ => {
                check = CheckSpec::new(
                    h.check.id().clone(),
                    h.check.command().to_vec(),
                    h.check.toolchain().into(),
                    h.check.environment(),
                    fake,
                )
                .unwrap()
            }
        }
        // Construct the record directly: a forged transaction selector must not be used
        // for the harness's candidate lookup.
        let evidence = VerificationEvidence::new(
            binding,
            check,
            CheckResult::Passed { exit_code: 0 },
            ExecutionProvenance {
                verifier: h.verifier.principal().clone(),
                runner: "reference/v1".into(),
                run_id: "forged".into(),
                started_at: 0,
                finished_at: 0,
            },
            vec![],
            vec![Artifact::new("report", "text/plain", b"captured".to_vec()).unwrap()],
        )
        .unwrap();
        assert_eq!(
            h.engine
                .record_evidence(&h.verifier, &h.scope, &transaction("a"), evidence),
            Err(EngineError::EvidenceMismatch),
            "field {field}"
        );
        assert!(matches!(
            h.commit("a", "a"),
            Err(EngineError::MissingEvidence(_))
        ));
        assert_eq!(h.current(), h.base);
    }
}

#[test]
fn scoped_capabilities_fail_closed_for_foreign_stores_scopes_and_revocation() {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    let mut foreign_scope = h.scope.clone();
    foreign_scope.organization = OrganizationId::new("other").unwrap();
    assert_eq!(
        h.engine.current(&h.worker, &foreign_scope),
        Err(EngineError::Unauthorized)
    );
    let (foreign_store, foreign_admin) = InMemoryStore::new();
    let foreign_session = foreign_admin
        .grant(
            h.worker.principal().clone(),
            h.scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    assert_eq!(
        h.engine.current(&foreign_session, &h.scope),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(
        Engine::new(foreign_store).current(&h.worker, &h.scope),
        Err(EngineError::Unauthorized)
    );
    h.admin.revoke(&h.worker).unwrap();
    assert_eq!(h.commit("a", "a"), Err(EngineError::Unauthorized));
    assert_eq!(h.engine.current(&h.operator, &h.scope).unwrap(), h.base);
}

#[test]
fn revoked_verifier_cannot_authorize_a_new_commit_with_previously_recorded_evidence() {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    h.admin.revoke(&h.verifier).unwrap();
    assert_eq!(h.commit("a", "a"), Err(EngineError::Unauthorized));
    assert_eq!(h.current(), h.base);
}

#[test]
fn workers_cannot_change_model_or_sign_their_own_evidence() {
    let h = Harness::new();
    let mut model = h.model.clone();
    model.generation += 1;
    assert_eq!(
        h.engine.update_model(&h.worker, &h.scope, model),
        Err(EngineError::Unauthorized)
    );
    h.propose("a", accesses(&[], &["x"]), vec![]);
    let binding = h.execute("a", &[("x", Some(1), Some(10))]);
    let evidence = h.evidence(
        binding,
        h.check.clone(),
        CheckResult::Passed { exit_code: 0 },
    );
    assert_eq!(
        h.engine
            .record_evidence(&h.worker, &h.scope, &transaction("a"), evidence),
        Err(EngineError::Unauthorized)
    );
    assert!(matches!(
        h.commit("a", "a"),
        Err(EngineError::MissingEvidence(_))
    ));
}

#[test]
fn incomplete_or_changed_analysis_never_becomes_safe_empty_impact() {
    let h = Harness::new();
    h.ready("old", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    let mut model = h.model.clone();
    model.generation = 2;
    model.completeness = Completeness::Incomplete {
        reasons: vec!["unsupported region".into()],
    };
    h.engine.update_model(&h.operator, &h.scope, model).unwrap();
    assert_eq!(h.commit("old", "old"), Err(EngineError::ModelChanged));
    h.engine
        .propose(
            &h.worker,
            &h.scope,
            transaction("new"),
            h.base.id(),
            h.intent(accesses(&[], &["y"]), vec![]),
        )
        .unwrap();
    assert_eq!(
        h.engine.analyze(&h.worker, &h.scope, &transaction("new")),
        Err(EngineError::IncompleteAnalysis)
    );
    assert_eq!(h.current(), h.base);
}

#[test]
fn leases_use_one_monotonic_clock_and_expired_work_cannot_renew_or_commit() {
    let h = Harness::new();
    h.ready(
        "expired",
        accesses(&[], &["x"]),
        &[("x", Some(1), Some(10))],
    );
    h.admin.advance_time(99).unwrap();
    assert_eq!(
        h.engine
            .renew(&h.worker, &h.scope, &transaction("expired"), 2)
            .unwrap(),
        101
    );
    h.admin.advance_time(2).unwrap();
    assert_eq!(
        h.engine
            .renew(&h.worker, &h.scope, &transaction("expired"), 10),
        Err(EngineError::InvalidLease)
    );
    assert_eq!(
        h.commit("expired", "expired"),
        Err(EngineError::InvalidLease)
    );
    assert_eq!(h.current(), h.base);
    // Reclaiming an abandoned scope creates new authority and never revives its owner.
    h.ready(
        "replacement",
        accesses(&[], &["x"]),
        &[("x", Some(1), Some(11))],
    );
    h.commit("replacement", "replacement").unwrap();
    assert_eq!(h.current().state()[&element("x")], 11);
    use celetixo_engine::engine::LeaseEventKind::*;
    let history = h
        .engine
        .lease_history(&h.worker, &h.scope, &transaction("expired"))
        .unwrap();
    assert!(matches!(history[0].kind, Acquired { expires_at: 100 }));
    assert!(matches!(
        history[1].kind,
        Renewed {
            previous_expiry: 100,
            expires_at: 101
        }
    ));
    assert_eq!(history[2].kind, Expired);
    assert_eq!(history[2].at, 101);
}

#[test]
fn zero_and_overflowing_lease_times_fail_without_state_changes() {
    let h = Harness::new();
    h.propose("a", accesses(&[], &["x"]), vec![]);
    assert_eq!(
        h.engine.reserve(&h.worker, &h.scope, &transaction("a"), 0),
        Err(EngineError::InvalidTime)
    );
    h.admin.advance_time(u64::MAX).unwrap();
    assert_eq!(h.admin.advance_time(1), Err(EngineError::InvalidTime));
    assert_eq!(
        h.engine.reserve(&h.worker, &h.scope, &transaction("a"), 1),
        Err(EngineError::InvalidTime)
    );
    assert_eq!(
        h.engine
            .transaction(&h.worker, &h.scope, &transaction("a"))
            .unwrap()
            .state,
        TransactionState::Analyzed
    );
}

#[test]
fn abort_is_terminal_and_releases_coordination_scopes_with_an_auditable_reason() {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    h.engine
        .abort(&h.worker, &h.scope, &transaction("a"), "cancelled".into())
        .unwrap();
    h.engine
        .abort(&h.worker, &h.scope, &transaction("a"), "cancelled".into())
        .unwrap();
    assert!(matches!(
        h.commit("a", "a"),
        Err(EngineError::IllegalTransition { .. })
    ));
    assert!(
        h.engine
            .revalidate(&h.worker, &h.scope, &transaction("a"))
            .is_err()
    );
    h.ready(
        "replacement",
        accesses(&[], &["x"]),
        &[("x", Some(1), Some(20))],
    );
    h.commit("replacement", "replacement").unwrap();
    assert_eq!(h.current().state()[&element("x")], 20);
    let history = h
        .engine
        .lease_history(&h.worker, &h.scope, &transaction("a"))
        .unwrap();
    assert!(
        matches!(&history[1].kind,celetixo_engine::engine::LeaseEventKind::Released{reason} if reason=="cancelled")
    );
}

#[test]
fn illegal_order_wrong_snapshot_and_undeclared_write_are_rejected_atomically() {
    let h = Harness::new();
    let intent = h.intent(accesses(&[], &["x"]), vec![]);
    h.engine
        .propose(
            &h.worker,
            &h.scope,
            transaction("a"),
            h.base.id(),
            intent.clone(),
        )
        .unwrap();
    assert!(matches!(
        h.commit("a", "too-early"),
        Err(EngineError::IllegalTransition { .. })
    ));
    // Repeating the same immutable proposal is idempotent; changing its intent is not.
    h.engine
        .propose(&h.worker, &h.scope, transaction("a"), h.base.id(), intent)
        .unwrap();
    assert_eq!(
        h.engine.propose(
            &h.worker,
            &h.scope,
            transaction("a"),
            h.base.id(),
            h.intent(accesses(&[], &["y"]), vec![])
        ),
        Err(EngineError::AlreadyExists)
    );
    h.engine
        .analyze(&h.worker, &h.scope, &transaction("a"))
        .unwrap();
    h.engine
        .reserve(&h.worker, &h.scope, &transaction("a"), 100)
        .unwrap();
    h.engine
        .execute(&h.worker, &h.scope, &transaction("a"))
        .unwrap();
    let fake = SnapshotId(Digest::of("snapshot", b"wrong"));
    let wrong_base = Patch::new(
        fake,
        [(
            element("x"),
            Change {
                expected: Some(1),
                value: Some(10),
            },
        )]
        .into(),
    )
    .unwrap();
    assert_eq!(
        h.engine
            .submit_patch(&h.worker, &h.scope, &transaction("a"), wrong_base),
        Err(EngineError::WrongPatchBase)
    );
    let wrong_scope = Patch::new(
        h.base.id(),
        [(
            element("y"),
            Change {
                expected: Some(2),
                value: Some(10),
            },
        )]
        .into(),
    )
    .unwrap();
    assert_eq!(
        h.engine
            .submit_patch(&h.worker, &h.scope, &transaction("a"), wrong_scope),
        Err(EngineError::UndeclaredWrite)
    );
    assert_eq!(h.current(), h.base);
}

#[test]
fn all_required_checks_must_be_present_and_transaction_dependencies_must_have_committed() {
    let h = Harness::new();
    let mut intent = h.intent(accesses(&[], &["x"]), vec![]);
    intent.depends_on.insert(transaction("missing"));
    assert_eq!(
        h.engine
            .propose(&h.worker, &h.scope, transaction("a"), h.base.id(), intent),
        Err(EngineError::UnsatisfiedDependency)
    );
    let mut intent = h.intent(accesses(&[], &["x"]), vec![]);
    let second = CheckSpec::new(
        CheckId::new("second").unwrap(),
        vec!["reference-second/v1".into()],
        "v1".into(),
        h.check.environment(),
        h.check.configuration(),
    )
    .unwrap();
    intent.required_checks.push(second.clone());
    h.engine
        .propose(&h.worker, &h.scope, transaction("a"), h.base.id(), intent)
        .unwrap();
    h.engine
        .analyze(&h.worker, &h.scope, &transaction("a"))
        .unwrap();
    let binding = h.execute("a", &[("x", Some(1), Some(10))]);
    h.verify("a");
    assert_eq!(
        h.commit("a", "missing-second"),
        Err(EngineError::MissingEvidence(second.id().clone()))
    );
    let evidence = h.evidence(binding, second, CheckResult::Passed { exit_code: 0 });
    h.engine
        .record_evidence(&h.verifier, &h.scope, &transaction("a"), evidence)
        .unwrap();
    h.commit("a", "complete").unwrap();
}

#[test]
fn idempotent_success_and_rejection_have_stable_outcomes_and_keys_cannot_be_reassigned() {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    let receipt = h.commit("a", "stable").unwrap();
    assert_eq!(h.commit("a", "stable"), Ok(receipt));
    assert_eq!(h.current().revision(), Revision(1));
    h.propose("b", accesses(&[], &["y"]), vec![]);
    assert_eq!(
        h.commit("b", "stable"),
        Err(EngineError::IdempotencyConflict)
    );
    let rejection = h.commit("b", "rejected");
    assert!(matches!(
        rejection,
        Err(EngineError::IllegalTransition { .. })
    ));
    assert_eq!(h.commit("b", "rejected"), rejection);
    // Replay still requires a currently authorized scoped capability.
    h.admin.revoke(&h.worker).unwrap();
    assert_eq!(h.commit("a", "stable"), Err(EngineError::Unauthorized));
}

#[test]
fn named_commit_failures_leave_no_partial_publication_and_lost_response_recovers_one_outcome() {
    for fault in [
        CommitFault::BeforeValidation,
        CommitFault::DuringValidation,
        CommitFault::BeforePublication,
        CommitFault::DuringPublication,
        CommitFault::AfterPublication,
    ] {
        let h = Harness::new();
        h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
        let outcome =
            h.engine
                .commit_with_fault(&h.worker, &h.scope, &transaction("a"), key("a"), fault);
        if fault == CommitFault::AfterPublication {
            assert_eq!(outcome, Err(EngineError::ResponseLost));
            assert_eq!(h.current().revision(), Revision(1));
            assert_eq!(
                h.engine
                    .transaction(&h.worker, &h.scope, &transaction("a"))
                    .unwrap()
                    .state,
                TransactionState::Committed
            );
        } else {
            assert_eq!(outcome, Err(EngineError::InjectedFailure(fault)));
            assert_eq!(h.current(), h.base);
            assert_eq!(
                h.engine
                    .transaction(&h.worker, &h.scope, &transaction("a"))
                    .unwrap()
                    .state,
                TransactionState::Validating
            );
        }
        let restarted_handle = Engine::new(h.store.clone());
        let receipt = restarted_handle
            .commit(&h.worker, &h.scope, &transaction("a"), key("a"))
            .unwrap();
        assert_eq!(receipt.revision, Revision(1));
        assert_eq!(h.commit("a", "a"), Ok(receipt));
        assert_eq!(h.current().revision(), Revision(1));
    }
}

#[test]
fn independent_engine_handles_race_at_one_gate_without_losing_compatible_writes() {
    let h = Harness::new();
    h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
    h.ready("b", accesses(&[], &["y"]), &[("y", Some(2), Some(20))]);
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|id| {
            let engine = Engine::new(h.store.clone());
            let session = h.worker.clone();
            let scope = h.scope.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (
                    id,
                    engine.commit(&session, &scope, &transaction(id), key(id)),
                )
            })
        })
        .collect();
    barrier.wait();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
        1
    );
    let loser = outcomes.iter().find(|(_, result)| result.is_err()).unwrap();
    assert!(matches!(
        loser.1,
        Err(EngineError::RevalidationRequired { .. })
    ));
    h.engine
        .revalidate(&h.worker, &h.scope, &transaction(loser.0))
        .unwrap();
    h.verify(loser.0);
    h.commit(loser.0, "fresh-loser").unwrap();
    assert_eq!(h.current().revision(), Revision(2));
    assert_eq!(h.current().state()[&element("x")], 10);
    assert_eq!(h.current().state()[&element("y")], 20);
}

#[test]
fn concurrent_write_skew_is_rejected_even_when_write_reservations_are_disjoint() {
    let h = Harness::new();
    h.ready("a", accesses(&["y"], &["x"]), &[("x", Some(1), Some(10))]);
    h.ready("b", accesses(&["x"], &["y"]), &[("y", Some(2), Some(20))]);
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|id| {
            let engine = Engine::new(h.store.clone());
            let session = h.worker.clone();
            let scope = h.scope.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (
                    id,
                    engine.commit(&session, &scope, &transaction(id), key(id)),
                )
            })
        })
        .collect();
    barrier.wait();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|(_, r)| r.is_ok()).count(), 1);
    let loser = outcomes.iter().find(|(_, r)| r.is_err()).unwrap();
    let before = h.current();
    assert!(matches!(
        h.engine
            .revalidate(&h.worker, &h.scope, &transaction(loser.0)),
        Err(EngineError::Conflict(_))
    ));
    assert_eq!(h.current(), before);
    assert_eq!(before.revision(), Revision(1));
    assert!(!(before.state()[&element("x")] == 10 && before.state()[&element("y")] == 20));
}

#[test]
fn multiple_revision_advances_preserve_prior_attempts_and_require_fresh_evidence_each_time() {
    let h = Harness::new();
    for (id, field, before, after) in [("a", "x", 1, 10), ("b", "y", 2, 20), ("c", "z", 3, 30)] {
        h.ready(
            id,
            accesses(&[], &[field]),
            &[(field, Some(before), Some(after))],
        );
    }
    h.commit("a", "a").unwrap();
    assert!(matches!(
        h.commit("b", "b:0"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    h.engine
        .revalidate(&h.worker, &h.scope, &transaction("b"))
        .unwrap();
    h.verify("b");
    assert!(matches!(
        h.commit("c", "c:0"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    h.engine
        .revalidate(&h.worker, &h.scope, &transaction("c"))
        .unwrap();
    h.verify("c");
    h.commit("c", "c:1").unwrap();
    let attempts_before = h
        .engine
        .attempts(&h.worker, &h.scope, &transaction("b"))
        .unwrap();
    assert!(matches!(
        h.commit("b", "b:1"),
        Err(EngineError::RevalidationRequired { .. })
    ));
    h.engine
        .revalidate(&h.worker, &h.scope, &transaction("b"))
        .unwrap();
    assert_eq!(h.commit("b", "b:0"), Err(EngineError::IdempotencyConflict));
    assert!(matches!(
        h.commit("b", "b:2:missing"),
        Err(EngineError::MissingEvidence(_))
    ));
    h.verify("b");
    let receipt = h.commit("b", "b:2").unwrap();
    assert_eq!(receipt.revision, Revision(3));
    let attempts = h
        .engine
        .attempts(&h.worker, &h.scope, &transaction("b"))
        .unwrap();
    assert_eq!(&attempts[..2], &attempts_before);
    assert_eq!(attempts[2].number, 2);
    assert_eq!(
        h.current().state(),
        &[(element("x"), 10), (element("y"), 20), (element("z"), 30)].into()
    );
}

#[test]
fn cancellation_and_revocation_schedules_have_one_authoritative_outcome() {
    // Named orderings deterministically exercise both sides of the same critical section.
    for commit_first in [false, true] {
        let h = Harness::new();
        h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
        let other = Engine::new(h.store.clone());
        if commit_first {
            h.commit("a", "a").unwrap();
            assert!(matches!(
                other.abort(&h.worker, &h.scope, &transaction("a"), "cancel".into()),
                Err(EngineError::IllegalTransition { .. })
            ));
            assert_eq!(h.current().revision(), Revision(1));
        } else {
            other
                .abort(&h.worker, &h.scope, &transaction("a"), "cancel".into())
                .unwrap();
            assert!(matches!(
                h.commit("a", "a"),
                Err(EngineError::IllegalTransition { .. })
            ));
            assert_eq!(h.current(), h.base);
        }
        let h = Harness::new();
        h.ready("a", accesses(&[], &["x"]), &[("x", Some(1), Some(10))]);
        if commit_first {
            h.commit("a", "a").unwrap();
            h.admin.revoke(&h.worker).unwrap();
            assert_eq!(
                h.engine.current(&h.operator, &h.scope).unwrap().revision(),
                Revision(1)
            );
        } else {
            h.admin.revoke(&h.worker).unwrap();
            assert_eq!(h.commit("a", "a"), Err(EngineError::Unauthorized));
            assert_eq!(h.engine.current(&h.operator, &h.scope).unwrap(), h.base);
        }
    }
}

#[test]
fn reservation_requires_proposal_ownership_even_with_other_scoped_permissions() {
    let h = Harness::new();
    h.propose("a", accesses(&[], &["x"]), vec![]);
    let other = h
        .admin
        .grant(
            PrincipalId::new("different-worker").unwrap(),
            h.scope.clone(),
            [Permission::Reserve].into(),
        )
        .unwrap();
    assert_eq!(
        h.engine.reserve(&other, &h.scope, &transaction("a"), 100),
        Err(EngineError::Unauthorized)
    );
    assert!(
        h.engine
            .lease_history(&h.worker, &h.scope, &transaction("a"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        h.engine
            .transaction(&h.worker, &h.scope, &transaction("a"))
            .unwrap()
            .state,
        TransactionState::Analyzed
    );
}
