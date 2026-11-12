//! Native reference request/fence semantics; actual SQL coordination is separate.
use celetixo_engine::access::{AccessSet, Completeness};
use celetixo_engine::engine::*;
use celetixo_engine::evidence::*;
use celetixo_engine::ids::*;
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::patch::{Change, Patch};
use celetixo_engine::snapshot::*;
use celetixo_engine::transaction::*;
use std::collections::{BTreeMap, BTreeSet};

fn d(s: &str) -> Digest {
    Digest::of("prepared-commit-test/v1", s.as_bytes())
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
    engine: Engine,
    admin: Administrator,
    worker: Session,
    verifier: Session,
    base: Snapshot,
    check: CheckSpec,
}
impl Harness {
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
                provenance: "prepared-commit-test/v1".into(),
                inputs: vec![],
                environment: BTreeMap::new(),
            },
            BTreeMap::from([(e("x"), 1), (e("y"), 2)]),
        )
        .unwrap();
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
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
                BTreeSet::from([
                    Permission::Read,
                    Permission::Propose,
                    Permission::Analyze,
                    Permission::Reserve,
                    Permission::Execute,
                    Permission::Commit,
                    Permission::Cancel,
                ]),
            )
            .unwrap();
        let verifier = admin
            .grant(
                PrincipalId::new("verifier").unwrap(),
                scope.clone(),
                BTreeSet::from([Permission::Verify]),
            )
            .unwrap();
        engine
            .register(
                &operator,
                base.clone(),
                EngineeringModel {
                    generation: 1,
                    analyzer: "finite-reference".into(),
                    version: "v1".into(),
                    configuration: d("config"),
                    dependencies: BTreeMap::new(),
                    completeness: Completeness::Complete,
                },
            )
            .unwrap();
        let check = CheckSpec::new(
            CheckId::new("verify").unwrap(),
            vec!["not-executed-reference-check".into()],
            "reference/v1".into(),
            d("env"),
            d("check"),
        )
        .unwrap();
        Self {
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
    fn intent(&self, write: &str) -> TransactionIntent {
        TransactionIntent {
            objective: "exact request and fenced lease".into(),
            accesses: AccessSet {
                writes: BTreeSet::from([e(write)]),
                ..Default::default()
            },
            invariants: vec![],
            required_checks: vec![self.check.clone()],
            depends_on: BTreeSet::new(),
        }
    }
    fn ready(&self, name: &str, write: &str, value: i64, verify: bool) {
        self.engine
            .propose(
                &self.worker,
                self.scope(),
                id(name),
                self.base.id(),
                self.intent(write),
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
        if verify {
            self.verify(name);
        }
    }
    fn verify(&self, name: &str) {
        let evidence = VerificationEvidence::new(
            self.engine
                .bindings(&self.worker, self.scope(), &id(name))
                .unwrap(),
            self.check.clone(),
            CheckResult::Passed { exit_code: 0 },
            ExecutionProvenance {
                verifier: self.verifier.principal().clone(),
                runner: "reference".into(),
                run_id: name.into(),
                started_at: 0,
                finished_at: 0,
            },
            vec![],
            vec![Artifact::new("stdout", "text/plain", b"ok".to_vec()).unwrap()],
        )
        .unwrap();
        self.engine
            .record_evidence(&self.verifier, self.scope(), &id(name), evidence)
            .unwrap();
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
}

#[test]
fn exact_request_reconciles_lost_response_and_expired_lease_without_second_advancement() {
    let h = Harness::new();
    h.ready("a", "x", 10, true);
    let request = h.request("a", "commit:a");
    assert_eq!(
        h.engine.commit_prepared_with_fault(
            &h.worker,
            h.scope(),
            request.clone(),
            CommitFault::AfterPublication
        ),
        Err(EngineError::ResponseLost)
    );
    h.admin.advance_time(200).unwrap();
    let receipt = h.commit(request.clone()).unwrap();
    assert_eq!(receipt.revision, Revision(1));
    assert_eq!(h.commit(request.clone()).unwrap(), receipt);
    assert_eq!(
        h.engine.current(&h.worker, h.scope()).unwrap().revision(),
        Revision(1)
    );
    h.admin.revoke(&h.worker).unwrap();
    assert_eq!(h.commit(request), Err(EngineError::Unauthorized));
}

#[test]
fn canonical_request_identity_binds_every_semantic_field_and_excludes_only_the_key() {
    let h = Harness::new();
    h.ready("a", "x", 10, true);
    let request = h.request("a", "commit:a");
    let digest = request.digest(h.scope(), h.worker.principal());
    let mut other_key = request.clone();
    other_key.key = key("other-key");
    assert_eq!(digest, other_key.digest(h.scope(), h.worker.principal()));
    let mut changed = Vec::new();
    let mut r = request.clone();
    r.transaction = id("other");
    changed.push(r);
    let mut r = request.clone();
    r.attempt += 1;
    changed.push(r);
    let mut r = request.clone();
    r.authorization_grant += 1;
    changed.push(r);
    let mut r = request.clone();
    r.fence += 1;
    changed.push(r);
    let mut r = request.clone();
    r.base_snapshot = SnapshotId(d("base"));
    changed.push(r);
    let mut r = request.clone();
    r.candidate_snapshot = SnapshotId(d("candidate"));
    changed.push(r);
    let mut r = request.clone();
    r.patch = PatchId(d("patch"));
    changed.push(r);
    let mut r = request.clone();
    r.analysis = d("analysis");
    changed.push(r);
    let mut r = request.clone();
    r.intent = d("intent");
    changed.push(r);
    let mut r = request.clone();
    r.evidence
        .insert(CheckId::new("verify").unwrap(), EvidenceId(d("evidence")));
    changed.push(r);
    for changed in changed {
        assert_ne!(digest, changed.digest(h.scope(), h.worker.principal()));
    }
    let mut scope = h.scope().clone();
    scope.repository = RepositoryId::new("other").unwrap();
    assert_ne!(digest, request.digest(&scope, h.worker.principal()));
    assert_ne!(
        digest,
        request.digest(h.scope(), &PrincipalId::new("other").unwrap())
    );
}

#[test]
fn changed_evidence_cannot_reassign_a_rejected_request_key() {
    let h = Harness::new();
    h.ready("a", "x", 10, false);
    let request = h.request("a", "sticky");
    let rejection = h.commit(request.clone());
    assert_eq!(
        rejection,
        Err(EngineError::MissingEvidence(h.check.id().clone()))
    );
    h.verify("a");
    assert_eq!(h.commit(request), rejection);
    let changed = h.request("a", "sticky");
    assert_eq!(h.commit(changed), Err(EngineError::IdempotencyConflict));
    let accepted = h.commit(h.request("a", "fresh-request")).unwrap();
    assert_eq!(accepted.revision, Revision(1));
}

#[test]
fn malformed_or_unauthorized_requests_do_not_poison_an_owners_operation_key() {
    let h = Harness::new();
    h.ready("a", "x", 10, true);
    let request = h.request("a", "owned");
    let mut changed = request.clone();
    changed.patch = PatchId(d("forged"));
    assert_eq!(h.commit(changed), Err(EngineError::CommitRequestMismatch));
    let mut changed = request.clone();
    changed.fence = 0;
    assert_eq!(h.commit(changed), Err(EngineError::InvalidLease));
    let other = h
        .admin
        .grant(
            PrincipalId::new("other-worker").unwrap(),
            h.scope().clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let other_proposal = h
        .engine
        .propose(&other, h.scope(), id("other"), h.base.id(), h.intent("y"))
        .unwrap();
    let mut forged = request.clone();
    forged.authorization_grant = other_proposal.proposal_grant_id;
    assert_eq!(
        h.engine.commit_prepared(&other, h.scope(), forged),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(h.engine.current(&h.worker, h.scope()).unwrap(), h.base);
    assert_eq!(h.commit(request).unwrap().revision, Revision(1));
}

#[test]
fn proposal_attribution_survives_a_new_lease_grant_for_the_same_principal() {
    let h = Harness::new();
    let proposal = h
        .engine
        .propose(
            &h.worker,
            h.scope(),
            id("new-session"),
            h.base.id(),
            h.intent("x"),
        )
        .unwrap();
    let renewed = h
        .admin
        .grant(
            h.worker.principal().clone(),
            h.scope().clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    h.engine
        .analyze(&renewed, h.scope(), &id("new-session"))
        .unwrap();
    h.engine
        .reserve(&renewed, h.scope(), &id("new-session"), 100)
        .unwrap();
    let reservation = h
        .engine
        .reservation(&renewed, h.scope(), &id("new-session"))
        .unwrap();
    assert_ne!(proposal.proposal_grant_id, reservation.authorization_grant);
    assert_eq!(
        proposal.proposal_grant_id,
        h.engine
            .transaction(&renewed, h.scope(), &id("new-session"))
            .unwrap()
            .proposal_grant_id
    );
    assert_eq!(renewed.scope(), h.scope());
    // A new grant does not inherit the previous grant's lease authority.
    assert_eq!(
        h.engine.execute(&h.worker, h.scope(), &id("new-session")),
        Err(EngineError::InvalidLease)
    );
    h.engine
        .execute(&renewed, h.scope(), &id("new-session"))
        .unwrap();
}

#[test]
fn disjoint_fences_remain_valid_and_revalidation_requires_new_exact_attempt_data() {
    let h = Harness::new();
    h.ready("a", "x", 10, true);
    h.ready("b", "y", 20, true);
    let first = h.request("a", "a");
    let second = h.request("b", "b:0");
    assert!(first.fence < second.fence);
    h.commit(first).unwrap();
    let rejection = h.commit(second.clone());
    assert!(matches!(
        rejection,
        Err(EngineError::RevalidationRequired { .. })
    ));
    h.engine.revalidate(&h.worker, h.scope(), &id("b")).unwrap();
    h.verify("b");
    assert_eq!(h.commit(second.clone()), rejection);
    let mut stale = second.clone();
    stale.key = key("stale-with-new-key");
    assert_eq!(h.commit(stale), Err(EngineError::CommitRequestMismatch));
    let fresh = h.request("b", "b:1");
    assert_eq!(fresh.attempt, 1);
    assert_eq!(fresh.fence, second.fence);
    assert_eq!(h.commit(fresh).unwrap().revision, Revision(2));
}

#[test]
fn released_fence_and_changed_request_cannot_authorize_a_replacement_commit() {
    let h = Harness::new();
    h.ready("old", "x", 10, true);
    let old = h.request("old", "old");
    h.engine
        .abort(
            &h.worker,
            h.scope(),
            &id("old"),
            "replace cancelled work".into(),
        )
        .unwrap();
    h.ready("new", "x", 11, true);
    let new = h.request("new", "new");
    assert!(new.fence > old.fence);
    assert_eq!(h.commit(old), Err(EngineError::InvalidLease));
    let mut stale = new.clone();
    stale.fence -= 1;
    assert_eq!(h.commit(stale), Err(EngineError::InvalidLease));
    let receipt = h.commit(new.clone()).unwrap();
    let mut substitution = new;
    substitution.analysis = d("changed");
    assert_eq!(
        h.commit(substitution),
        Err(EngineError::IdempotencyConflict)
    );
    assert_eq!(receipt.revision, Revision(1));
    assert_eq!(
        h.engine.current(&h.worker, h.scope()).unwrap().state()[&e("x")],
        11
    );
}
