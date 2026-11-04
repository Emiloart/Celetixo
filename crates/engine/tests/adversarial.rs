//! Regression histories for trust boundaries and stale work. No repository code executes.
use celetixo_engine::access::{AccessSet, Completeness, Predicate};
use celetixo_engine::engine::{
    Administrator, Engine, EngineError, InMemoryStore, Permission, Session,
};
use celetixo_engine::evidence::{
    Artifact, CheckResult, CheckSpec, EvidenceBindings, ExecutionProvenance, VerificationEvidence,
};
use celetixo_engine::ids::{
    CheckId, Digest, ElementId, IdempotencyKey, OrganizationId, PrincipalId, RepositoryId,
    Revision, Scope, TransactionId, WorkspaceId,
};
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::patch::{Change, Patch, PatchError};
use celetixo_engine::snapshot::{InclusionPolicy, Snapshot, SnapshotManifest, State};
use celetixo_engine::transaction::{TransactionIntent, TransactionState};
use std::collections::{BTreeMap, BTreeSet};

fn element(value: &str) -> ElementId {
    ElementId::new(value).unwrap()
}

fn transaction(value: &str) -> TransactionId {
    TransactionId::new(value).unwrap()
}

fn key(value: &str) -> IdempotencyKey {
    IdempotencyKey::new(value).unwrap()
}

fn digest(value: &str) -> Digest {
    Digest::of("adversarial-fixture/v1", value.as_bytes())
}

fn worker_permissions() -> BTreeSet<Permission> {
    [
        Permission::Read,
        Permission::Propose,
        Permission::Analyze,
        Permission::Reserve,
        Permission::Execute,
        Permission::Commit,
        Permission::Cancel,
    ]
    .into()
}

struct Fixture {
    engine: Engine,
    admin: Administrator,
    manager: Session,
    worker: Session,
    verifier: Session,
    scope: Scope,
    base: Snapshot,
    model: EngineeringModel,
    check: CheckSpec,
}

impl Fixture {
    fn new(dependencies: BTreeMap<ElementId, BTreeSet<ElementId>>) -> Self {
        let scope = Scope {
            organization: OrganizationId::new("adversarial-org").unwrap(),
            workspace: WorkspaceId::new("adversarial-workspace").unwrap(),
            repository: RepositoryId::new("adversarial-repository").unwrap(),
        };
        let state = State::from([
            (element("a"), 0),
            (element("b"), 0),
            (element("config:x"), 0),
            (element("module:a"), 0),
            (element("module:top"), 0),
        ]);
        let base = Snapshot::new(
            scope.clone(),
            Revision(0),
            SnapshotManifest {
                base_commit: "reference:initial".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "trusted-adversarial-fixture/v1".into(),
                inputs: vec![],
                environment: BTreeMap::from([("evaluator".into(), "nonnegative/v1".into())]),
            },
            state,
        )
        .unwrap();
        let model = EngineeringModel {
            generation: 1,
            analyzer: "finite-dependency-model".into(),
            version: "v1".into(),
            configuration: digest("model-config"),
            dependencies,
            completeness: Completeness::Complete,
        };
        let (store, admin) = InMemoryStore::new();
        let manager = admin
            .grant(
                PrincipalId::new("trusted-manager").unwrap(),
                scope.clone(),
                Permission::ALL.into(),
            )
            .unwrap();
        let worker = admin
            .grant(
                PrincipalId::new("worker").unwrap(),
                scope.clone(),
                worker_permissions(),
            )
            .unwrap();
        let verifier = admin
            .grant(
                PrincipalId::new("trusted-verifier").unwrap(),
                scope.clone(),
                [Permission::Read, Permission::Verify].into(),
            )
            .unwrap();
        let engine = Engine::new(store);
        engine
            .register(&manager, base.clone(), model.clone())
            .unwrap();
        let check = CheckSpec::new(
            CheckId::new("nonnegative-state").unwrap(),
            vec!["reference-evaluator".into(), "nonnegative-state".into()],
            "finite-integer-evaluator/v1".into(),
            digest("verification-environment"),
            digest("nonnegative-config"),
        )
        .unwrap();
        Self {
            engine,
            admin,
            manager,
            worker,
            verifier,
            scope,
            base,
            model,
            check,
        }
    }

    fn intent(&self, accesses: AccessSet) -> TransactionIntent {
        TransactionIntent {
            objective: "exercise a bounded reference-state transition".into(),
            accesses,
            invariants: vec![],
            required_checks: vec![self.check.clone()],
            depends_on: BTreeSet::new(),
        }
    }

    fn propose_and_reserve(&self, id: &TransactionId, accesses: AccessSet) -> AccessSet {
        self.engine
            .propose(
                &self.worker,
                &self.scope,
                id.clone(),
                self.base.id(),
                self.intent(accesses),
            )
            .unwrap();
        let effective = self.engine.analyze(&self.worker, &self.scope, id).unwrap();
        self.engine
            .reserve(&self.worker, &self.scope, id, 100)
            .unwrap();
        effective
    }

    fn patch(&self, changes: &[(&str, i64, i64)]) -> Patch {
        Patch::new(
            self.base.id(),
            changes
                .iter()
                .map(|(id, expected, value)| {
                    (
                        element(id),
                        Change {
                            expected: Some(*expected),
                            value: Some(*value),
                        },
                    )
                })
                .collect(),
        )
        .unwrap()
    }

    fn prepare(&self, id: &TransactionId, accesses: AccessSet, patch: Patch) -> EvidenceBindings {
        self.propose_and_reserve(id, accesses);
        self.engine.execute(&self.worker, &self.scope, id).unwrap();
        self.engine
            .submit_patch(&self.worker, &self.scope, id, patch)
            .unwrap()
    }

    /// Trusted, deterministic evaluator over the exact Engine-produced candidate snapshot.
    /// The retained artifact is the evaluator's actual input and result, not a green stub.
    fn evaluate(&self, id: &TransactionId, run: &str) -> VerificationEvidence {
        let candidate = self
            .engine
            .candidate(&self.verifier, &self.scope, id)
            .unwrap();
        let bindings = self
            .engine
            .bindings(&self.verifier, &self.scope, id)
            .unwrap();
        assert_eq!(bindings.candidate_snapshot, candidate.id());
        let passed = candidate.state().values().all(|value| *value >= 0);
        let result = if passed {
            CheckResult::Passed { exit_code: 0 }
        } else {
            CheckResult::Failed { exit_code: 1 }
        };
        let report = format!(
            "candidate={}\nstate={:?}\nnonnegative={}\n",
            candidate.id(),
            candidate.state(),
            passed
        );
        VerificationEvidence::new(
            bindings,
            self.check.clone(),
            result,
            ExecutionProvenance {
                verifier: self.verifier.principal().clone(),
                runner: "finite-integer-evaluator/v1".into(),
                run_id: run.into(),
                started_at: 0,
                finished_at: 0,
            },
            vec![],
            vec![Artifact::new("evaluation.txt", "text/plain", report.into_bytes()).unwrap()],
        )
        .unwrap()
    }

    fn verify(&self, id: &TransactionId, run: &str) -> VerificationEvidence {
        let evidence = self.evaluate(id, run);
        self.engine
            .record_evidence(&self.verifier, &self.scope, id, evidence.clone())
            .unwrap();
        evidence
    }

    fn current(&self) -> Snapshot {
        self.engine.current(&self.worker, &self.scope).unwrap()
    }
}

fn writes(ids: &[&str]) -> AccessSet {
    AccessSet {
        writes: ids.iter().map(|id| element(id)).collect(),
        ..Default::default()
    }
}

#[test]
fn stale_evidence_cannot_follow_a_transaction_into_a_new_revalidation_attempt() {
    let f = Fixture::new(BTreeMap::new());
    let first = transaction("first");
    let second = transaction("second");
    f.prepare(&first, writes(&["a"]), f.patch(&[("a", 0, 1)]));
    let original = f.prepare(&second, writes(&["b"]), f.patch(&[("b", 0, 2)]));
    f.verify(&first, "first:0");
    let stale = f.verify(&second, "second:0");
    f.engine
        .commit(&f.worker, &f.scope, &first, key("first:commit"))
        .unwrap();
    assert!(matches!(
        f.engine
            .commit(&f.worker, &f.scope, &second, key("second:stale")),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let fresh = f.engine.revalidate(&f.worker, &f.scope, &second).unwrap();
    assert_eq!(fresh.attempt, original.attempt + 1);
    assert_ne!(fresh.base_snapshot, original.base_snapshot);
    assert_ne!(fresh.candidate_snapshot, original.candidate_snapshot);
    assert_ne!(fresh.patch, original.patch);
    assert_ne!(fresh.analysis, original.analysis);
    assert_eq!(
        f.engine
            .record_evidence(&f.verifier, &f.scope, &second, stale),
        Err(EngineError::EvidenceMismatch)
    );
    assert!(matches!(
        f.engine
            .commit(&f.worker, &f.scope, &second, key("second:missing")),
        Err(EngineError::MissingEvidence(_))
    ));
    f.verify(&second, "second:1");
    let expected_candidate = f.engine.candidate(&f.verifier, &f.scope, &second).unwrap();
    let receipt = f
        .engine
        .commit(&f.worker, &f.scope, &second, key("second:fresh"))
        .unwrap();
    assert_eq!(receipt.snapshot, expected_candidate.id());
    assert_eq!(f.current(), expected_candidate);
    assert_eq!(f.current().revision(), Revision(2));
    assert_eq!(f.current().state()[&element("a")], 1);
    assert_eq!(f.current().state()[&element("b")], 2);
    let view = f.engine.transaction(&f.worker, &f.scope, &second).unwrap();
    assert_eq!(view.attempts, 2);
    assert_eq!(view.original_snapshot, f.base.id());
}

#[test]
fn same_principal_with_a_different_capability_cannot_use_the_lease() {
    let f = Fixture::new(BTreeMap::new());
    let id = transaction("capability-fence");
    f.propose_and_reserve(&id, writes(&["a"]));
    let twin = f
        .admin
        .grant(
            f.worker.principal().clone(),
            f.scope.clone(),
            worker_permissions(),
        )
        .unwrap();
    assert_eq!(
        f.engine.renew(&twin, &f.scope, &id, 100),
        Err(EngineError::InvalidLease)
    );
    assert_eq!(
        f.engine.execute(&twin, &f.scope, &id),
        Err(EngineError::InvalidLease)
    );
    assert_eq!(
        f.engine
            .transaction(&f.worker, &f.scope, &id)
            .unwrap()
            .state,
        TransactionState::Reserved
    );
    f.engine.execute(&f.worker, &f.scope, &id).unwrap();
    assert_eq!(
        f.engine
            .submit_patch(&twin, &f.scope, &id, f.patch(&[("a", 0, 1)])),
        Err(EngineError::InvalidLease)
    );
    f.engine
        .submit_patch(&f.worker, &f.scope, &id, f.patch(&[("a", 0, 1)]))
        .unwrap();
    f.verify(&id, "capability-fence:verification");
    assert_eq!(
        f.engine.commit(&twin, &f.scope, &id, key("wrong-token")),
        Err(EngineError::InvalidLease)
    );
    assert_eq!(f.current().revision(), Revision(0));
    f.engine
        .commit(&f.worker, &f.scope, &id, key("correct-token"))
        .unwrap();
    assert_eq!(f.current().state()[&element("a")], 1);
}

#[test]
fn a_stale_before_image_is_atomic_and_cannot_install_a_candidate() {
    let f = Fixture::new(BTreeMap::new());
    let id = transaction("before-image");
    f.propose_and_reserve(&id, writes(&["a", "b"]));
    f.engine.execute(&f.worker, &f.scope, &id).unwrap();
    let malformed = f.patch(&[("a", 0, 7), ("b", 99, 8)]);
    assert!(matches!(
        f.engine.submit_patch(&f.worker, &f.scope, &id, malformed),
        Err(EngineError::Patch(PatchError::BeforeImage { .. }))
    ));
    assert_eq!(f.current(), f.base);
    assert_eq!(
        f.engine
            .transaction(&f.worker, &f.scope, &id)
            .unwrap()
            .state,
        TransactionState::Executing
    );
    assert_eq!(
        f.engine.bindings(&f.worker, &f.scope, &id),
        Err(EngineError::EvidenceMismatch)
    );
    assert!(f.engine.candidate(&f.worker, &f.scope, &id).is_err());
    f.engine
        .submit_patch(
            &f.worker,
            &f.scope,
            &id,
            f.patch(&[("a", 0, 7), ("b", 0, 8)]),
        )
        .unwrap();
    f.verify(&id, "before-image:corrected");
    f.engine
        .commit(&f.worker, &f.scope, &id, key("corrected"))
        .unwrap();
    assert_eq!(f.current().state()[&element("a")], 7);
    assert_eq!(f.current().state()[&element("b")], 8);
}

#[test]
fn aborted_work_rejects_late_verification_and_cannot_commit() {
    let f = Fixture::new(BTreeMap::new());
    let id = transaction("aborted-verification");
    f.prepare(&id, writes(&["a"]), f.patch(&[("a", 0, 1)]));
    let late = f.evaluate(&id, "finished-after-cancellation");
    f.engine
        .abort(&f.worker, &f.scope, &id, "cancelled by supervisor".into())
        .unwrap();
    assert!(matches!(
        f.engine.record_evidence(&f.verifier, &f.scope, &id, late),
        Err(EngineError::IllegalTransition {
            from: TransactionState::Aborted,
            ..
        })
    ));
    assert!(matches!(
        f.engine.commit(&f.worker, &f.scope, &id, key("cancelled")),
        Err(EngineError::IllegalTransition {
            from: TransactionState::Aborted,
            ..
        })
    ));
    assert_eq!(f.current(), f.base);
}

#[test]
fn verifier_attribution_cannot_substitute_for_verifier_authorization() {
    let f = Fixture::new(BTreeMap::new());
    let id = transaction("verifier-boundary");
    f.prepare(&id, writes(&["a"]), f.patch(&[("a", 0, 1)]));
    let evidence = f.evaluate(&id, "verifier-boundary:0");
    assert_eq!(
        f.engine
            .record_evidence(&f.worker, &f.scope, &id, evidence.clone()),
        Err(EngineError::Unauthorized)
    );
    let other_verifier = f
        .admin
        .grant(
            PrincipalId::new("different-verifier").unwrap(),
            f.scope.clone(),
            [Permission::Verify].into(),
        )
        .unwrap();
    assert_eq!(
        f.engine
            .record_evidence(&other_verifier, &f.scope, &id, evidence.clone()),
        Err(EngineError::EvidenceMismatch)
    );
    assert!(matches!(
        f.engine
            .commit(&f.worker, &f.scope, &id, key("untrusted-evidence")),
        Err(EngineError::MissingEvidence(_))
    ));
    assert_eq!(f.current(), f.base);
    f.engine
        .record_evidence(&f.verifier, &f.scope, &id, evidence)
        .unwrap();
    f.engine
        .commit(&f.worker, &f.scope, &id, key("authenticated-evidence"))
        .unwrap();
    assert_eq!(f.current().state()[&element("a")], 1);
}

#[test]
fn ordinary_analysis_permission_cannot_replace_authoritative_model_facts() {
    let f = Fixture::new(BTreeMap::from([(
        element("module:a"),
        [element("config:x")].into(),
    )]));
    let mut empty = f.model.clone();
    empty.generation += 1;
    empty.dependencies.clear();
    assert_eq!(
        f.engine.update_model(&f.worker, &f.scope, empty.clone()),
        Err(EngineError::Unauthorized)
    );
    let id = transaction("model-still-authoritative");
    let accesses = AccessSet {
        reads: [element("module:a")].into(),
        ..writes(&["a"])
    };
    let effective = f.propose_and_reserve(&id, accesses);
    assert!(effective.reads.contains(&element("config:x")));
    f.engine.update_model(&f.manager, &f.scope, empty).unwrap();
}

#[test]
fn prefix_reads_expand_declared_dependencies_and_bind_the_full_analysis() {
    let dependencies = BTreeMap::from([
        (element("module:a"), [element("config:x")].into()),
        (element("module:top"), [element("module:a")].into()),
    ]);
    let f = Fixture::new(dependencies);
    let reader = transaction("prefix-reader");
    let writer = transaction("config-writer");
    let accesses = AccessSet {
        predicates: [Predicate::Prefix("module:".into())].into(),
        ..writes(&["a"])
    };
    let effective = f.propose_and_reserve(&reader, accesses.clone());
    assert!(effective.reads.contains(&element("config:x")));
    f.engine.execute(&f.worker, &f.scope, &reader).unwrap();
    let bindings = f
        .engine
        .submit_patch(&f.worker, &f.scope, &reader, f.patch(&[("a", 0, 1)]))
        .unwrap();
    let analysis = f.model.analyze(f.base.id(), &accesses);
    assert_eq!(bindings.analysis, analysis.digest());
    assert_ne!(bindings.analysis, analysis.stamp().digest());
    f.verify(&reader, "prefix-reader:0");
    f.prepare(
        &writer,
        writes(&["config:x"]),
        f.patch(&[("config:x", 0, 1)]),
    );
    f.verify(&writer, "config-writer:0");
    f.engine
        .commit(&f.worker, &f.scope, &writer, key("config-writer:commit"))
        .unwrap();
    assert!(matches!(
        f.engine
            .commit(&f.worker, &f.scope, &reader, key("prefix-reader:stale")),
        Err(EngineError::RevalidationRequired { .. })
    ));
    assert!(matches!(
        f.engine.revalidate(&f.worker, &f.scope, &reader),
        Err(EngineError::Conflict(_))
    ));
    assert_eq!(f.current().revision(), Revision(1));
    assert_eq!(f.current().state()[&element("a")], 0);
    assert_eq!(f.current().state()[&element("config:x")], 1);
}

#[test]
fn reverse_impact_reaches_transitive_consumers() {
    let f = Fixture::new(BTreeMap::from([
        (element("module:top"), [element("module:a")].into()),
        (element("module:a"), [element("config:x")].into()),
    ]));
    let explicit = writes(&["config:x"]);
    let analysis = f.model.analyze(f.base.id(), &explicit);
    assert!(analysis.derived().reads.contains(&element("module:a")));
    assert!(analysis.derived().reads.contains(&element("module:top")));
}

#[test]
fn caller_declared_invariant_scopes_expand_dependencies_and_reject_invalidated_inputs() {
    let f = Fixture::new(BTreeMap::from([
        (element("module:top"), [element("module:a")].into()),
        (element("module:a"), [element("config:x")].into()),
    ]));
    let reader = transaction("declared-invariant-reader");
    let writer = transaction("declared-invariant-input-writer");
    // This is a caller-declared scope rather than a Condition contract. Its modeled
    // dependencies still matter even when the caller omits the corresponding reads.
    let scoped = AccessSet {
        invariant_scopes: [element("module:top")].into(),
        ..writes(&["a"])
    };
    let effective = f.propose_and_reserve(&reader, scoped);
    assert!(effective.reads.contains(&element("module:a")));
    assert!(effective.reads.contains(&element("config:x")));
    f.engine.execute(&f.worker, &f.scope, &reader).unwrap();
    f.engine
        .submit_patch(&f.worker, &f.scope, &reader, f.patch(&[("a", 0, 1)]))
        .unwrap();
    f.verify(&reader, "declared-invariant-reader:0");
    f.prepare(
        &writer,
        writes(&["config:x"]),
        f.patch(&[("config:x", 0, 1)]),
    );
    f.verify(&writer, "declared-invariant-input-writer:0");
    f.engine
        .commit(&f.worker, &f.scope, &writer, key("declared-input:commit"))
        .unwrap();
    assert!(matches!(
        f.engine.commit(
            &f.worker,
            &f.scope,
            &reader,
            key("declared-invariant:stale")
        ),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let result = f.engine.revalidate(&f.worker, &f.scope, &reader);
    let Err(EngineError::Conflict(reports)) = result else {
        panic!("expected a dependency conflict, got {result:?}");
    };
    assert!(reports.iter().any(|report| {
        report.contender == reader
            && report.committed == writer
            && report.committed_revision == Revision(1)
            && report
                .contender_provenance
                .explicit
                .invariant_scopes
                .contains(&element("module:top"))
            && report
                .contender_provenance
                .derived
                .reads
                .contains(&element("config:x"))
            && report
                .conflicts
                .iter()
                .any(|conflict| conflict.element == element("config:x"))
    }));
    assert_eq!(f.current().revision(), Revision(1));
    assert_eq!(f.current().state()[&element("a")], 0);
    assert_eq!(f.current().state()[&element("config:x")], 1);
}
