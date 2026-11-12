//! Exact native commit request binding. The canonical service TaskIntent stays
//! separate; these native Engine values do not replace its complete DSL.
use super::*;
use crate::persistence::NativeIntentRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    pub transaction: TransactionId,
    pub attempt: u64,
    pub authorization_grant: u64,
    pub fence: u64,
    pub base_snapshot: SnapshotId,
    pub candidate_snapshot: SnapshotId,
    pub patch: PatchId,
    pub analysis: Digest,
    pub intent: Digest,
    pub evidence: BTreeMap<CheckId, EvidenceId>,
    pub key: IdempotencyKey,
}

impl CommitRequest {
    pub const FORMAT: &str = "celetixo/commit-request/v1";
    /// Scope and independently authenticated principal are part of identity;
    /// idempotency key selects an outcome and is not itself semantic content.
    pub fn digest(&self, scope: &Scope, principal: &PrincipalId) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes(scope, principal))
    }

    /// Native persistence identity preimage; not a competing service wire schema.
    pub fn canonical_bytes(&self, scope: &Scope, principal: &PrincipalId) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(scope.organization.as_str());
        c.str(scope.workspace.as_str());
        c.str(scope.repository.as_str());
        c.str(principal.as_str());
        c.str(self.transaction.as_str());
        c.u64(self.attempt);
        c.u64(self.authorization_grant);
        c.u64(self.fence);
        c.digest(self.base_snapshot.0);
        c.digest(self.candidate_snapshot.0);
        c.digest(self.patch.0);
        c.digest(self.analysis);
        c.digest(self.intent);
        c.u64(self.evidence.len() as u64);
        for (check, evidence) in &self.evidence {
            c.str(check.as_str());
            c.digest(evidence.0);
        }
        c.into_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservationView {
    pub transaction: TransactionId,
    pub attempt: u64,
    pub owner: PrincipalId,
    /// Attribution only; the corresponding opaque Session is still required.
    pub authorization_grant: u64,
    pub fence: u64,
    pub acquired_at: u64,
    pub acquired_attempt: u64,
    pub expires_at: u64,
    pub writes: BTreeSet<ElementId>,
}

impl Engine {
    pub fn reservation(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<ReservationView, EngineError> {
        self.store
            .session(session, scope, Permission::Read)
            .read(|db| {
                self.authorize(db, session, scope, Permission::Read)?;
                let tx = db
                    .transactions
                    .get(&(scope.clone(), id.clone()))
                    .ok_or(EngineError::NotFound)?;
                let lease = tx.lease.as_ref().ok_or(EngineError::InvalidLease)?;
                let owner = db
                    .grants
                    .get(&lease.owner)
                    .ok_or(EngineError::Unauthorized)?;
                Ok(ReservationView {
                    transaction: tx.id.clone(),
                    attempt: tx.attempt().number,
                    owner: owner.principal.clone(),
                    authorization_grant: lease.owner,
                    fence: lease.fence,
                    acquired_at: lease.acquired_at,
                    acquired_attempt: lease.acquired_attempt,
                    expires_at: lease.expires_at,
                    writes: lease.writes.clone(),
                })
            })
    }

    /// Prepare exact request data, not a commit permission. It remains subject
    /// to the current opaque session, lease, evidence and complete commit gate.
    pub fn prepare_commit_request(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        key: IdempotencyKey,
    ) -> Result<CommitRequest, EngineError> {
        self.store
            .session(session, scope, Permission::Commit)
            .read(|db| {
                self.authorize(db, session, scope, Permission::Commit)?;
                let tx = db
                    .transactions
                    .get(&(scope.clone(), id.clone()))
                    .ok_or(EngineError::NotFound)?;
                if tx.owner != *session.principal() {
                    return Err(EngineError::Unauthorized);
                }
                require_lease(db, tx, session)?;
                let bindings = tx.bindings()?;
                Ok(CommitRequest {
                    transaction: id.clone(),
                    attempt: bindings.attempt,
                    authorization_grant: session.token,
                    fence: tx.lease.as_ref().ok_or(EngineError::InvalidLease)?.fence,
                    base_snapshot: bindings.base_snapshot,
                    candidate_snapshot: bindings.candidate_snapshot,
                    patch: bindings.patch,
                    analysis: bindings.analysis,
                    intent: intent_digest(tx)?,
                    evidence: tx.attempt().evidence.clone(),
                    key,
                })
            })
    }

    pub fn commit_prepared(
        &self,
        session: &Session,
        scope: &Scope,
        request: CommitRequest,
    ) -> Result<CommitReceipt, EngineError> {
        self.commit_prepared_with_fault(session, scope, request, CommitFault::None)
    }

    /// Trusted deterministic embedding/test injection; it cannot confer authority.
    pub fn commit_prepared_with_fault(
        &self,
        session: &Session,
        scope: &Scope,
        request: CommitRequest,
        fault: CommitFault,
    ) -> Result<CommitReceipt, EngineError> {
        self.commit_inner(
            session,
            scope,
            &request.transaction,
            request.key.clone(),
            fault,
            Some(&request),
        )
    }
}

pub(super) fn validate_request(
    tx: &Transaction,
    request: &CommitRequest,
) -> Result<(), EngineError> {
    let bindings = tx.bindings()?;
    if request.transaction != tx.id
        || request.attempt != bindings.attempt
        || request.base_snapshot != bindings.base_snapshot
        || request.candidate_snapshot != bindings.candidate_snapshot
        || request.patch != bindings.patch
        || request.analysis != bindings.analysis
        || request.intent != intent_digest(tx)?
        || request.evidence != tx.attempt().evidence
    {
        return Err(EngineError::CommitRequestMismatch);
    }
    if request.fence == 0
        || tx
            .lease
            .as_ref()
            .is_none_or(|lease| lease.fence != request.fence)
    {
        return Err(EngineError::InvalidLease);
    }
    Ok(())
}

fn intent_digest(tx: &Transaction) -> Result<Digest, EngineError> {
    NativeIntentRecord::new(&tx.attempts[0].base, tx.intent.clone())
        .map(|record| record.digest())
        .map_err(|_| EngineError::InvalidIntent("stored intent failed validation".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::Completeness;
    use crate::evidence::CheckSpec;
    use crate::snapshot::{InclusionPolicy, SnapshotManifest};

    #[test]
    fn fence_exhaustion_and_conflict_do_not_partially_acquire_or_consume_a_generation() {
        let scope = Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("ws").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        };
        let snapshot = Snapshot::new(
            scope.clone(),
            Revision(0),
            SnapshotManifest {
                base_commit: "fixture".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "fence-test/v1".into(),
                inputs: vec![],
                environment: BTreeMap::new(),
            },
            BTreeMap::new(),
        )
        .unwrap();
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store.clone());
        let session = admin
            .grant(
                PrincipalId::new("worker").unwrap(),
                scope.clone(),
                Permission::ALL.into(),
            )
            .unwrap();
        let digest = Digest::of("fence-test", b"config");
        engine
            .register(
                &session,
                snapshot.clone(),
                EngineeringModel {
                    generation: 1,
                    analyzer: "reference".into(),
                    version: "v1".into(),
                    configuration: digest,
                    dependencies: BTreeMap::new(),
                    completeness: Completeness::Complete,
                },
            )
            .unwrap();
        let a = TransactionId::new("a").unwrap();
        let b = TransactionId::new("b").unwrap();
        for id in [&a, &b] {
            engine
                .propose(
                    &session,
                    &scope,
                    id.clone(),
                    snapshot.id(),
                    TransactionIntent {
                        objective: "fence failure atomicity".into(),
                        accesses: AccessSet {
                            writes: BTreeSet::from([ElementId::new("x").unwrap()]),
                            ..Default::default()
                        },
                        invariants: vec![],
                        required_checks: vec![
                            CheckSpec::new(
                                CheckId::new("check").unwrap(),
                                vec!["check".into()],
                                "reference/v1".into(),
                                digest,
                                digest,
                            )
                            .unwrap(),
                        ],
                        depends_on: BTreeSet::new(),
                    },
                )
                .unwrap();
            engine.analyze(&session, &scope, id).unwrap();
        }
        store
            .update(|db| {
                db.reservation_fences.insert(scope.clone(), u64::MAX);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            engine.reserve(&session, &scope, &a, 100),
            Err(EngineError::CounterExhausted)
        );
        assert_eq!(
            engine.transaction(&session, &scope, &a).unwrap().state,
            TransactionState::Analyzed
        );
        assert!(
            engine
                .lease_history(&session, &scope, &a)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            engine.reservation(&session, &scope, &a),
            Err(EngineError::InvalidLease)
        );
        assert_eq!(
            store.read(|db| Ok(db.reservation_fences[&scope])).unwrap(),
            u64::MAX
        );
        // Private test injection only; production has no counter-reset operation.
        store
            .update(|db| {
                db.reservation_fences.insert(scope.clone(), 0);
                Ok(())
            })
            .unwrap();
        engine.reserve(&session, &scope, &a, 100).unwrap();
        assert_eq!(
            engine.reserve(&session, &scope, &b, 100),
            Err(EngineError::ReservationConflict)
        );
        assert_eq!(
            store.read(|db| Ok(db.reservation_fences[&scope])).unwrap(),
            1
        );
        assert!(
            engine
                .lease_history(&session, &scope, &b)
                .unwrap()
                .is_empty()
        );
        engine
            .abort(&session, &scope, &a, "release for replacement".into())
            .unwrap();
        engine.reserve(&session, &scope, &b, 100).unwrap();
        assert_eq!(engine.reservation(&session, &scope, &b).unwrap().fence, 2);
    }
}
