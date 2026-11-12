//! Shared in-memory authoritative reference store and its atomic commit gate.
//!
//! Session capabilities authenticate callers only inside this trusted embedding process.
//! The mutex models atomic publication; it supplies neither distributed coordination nor
//! crash durability. No worker code, shell command or dependency is executed by this crate.
use crate::access::{AccessSet, Conflict, InvariantViolation};
use crate::evidence::{EvidenceBindings, VerificationEvidence};
use crate::ids::*;
use crate::model::EngineeringModel;
use crate::patch::{Patch, PatchError};
use crate::repository::{RepositoryAnalysis, RepositoryAnalysisError, RepositoryImpact};
use crate::snapshot::{Snapshot, SnapshotError};
use crate::transaction::{TransactionIntent, TransactionState, TransactionView};
use runtime::{SourceArtifactReaders, StoreIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

mod runtime;
mod source_attempts;
mod source_cleanup;
mod source_evidence_gate;
mod state_access;
pub use source_attempts::{
    SourceAdmission, SourceAttemptError, SourceAttemptEvent, SourceAttemptOptions,
    SourceAttemptState, SourceAttemptView, SourceAuthorizationProvenance, SourceExecutorCapability,
    SourcePatchContext, SourceRejectionView,
};
pub use source_cleanup::SourceCleanupCapability;
pub use source_evidence_gate::{
    SourceArtifactStoreCapability, SourceEvidenceFault, SourceEvidenceReceipt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Permission {
    Register,
    ManageModel,
    Read,
    Propose,
    Analyze,
    Reserve,
    Execute,
    Verify,
    Commit,
    Cancel,
}

impl Permission {
    pub const ALL: [Self; 10] = [
        Self::Register,
        Self::ManageModel,
        Self::Read,
        Self::Propose,
        Self::Analyze,
        Self::Reserve,
        Self::Execute,
        Self::Verify,
        Self::Commit,
        Self::Cancel,
    ];
}

/// Opaque in-process capability. Identifiers alone cannot construct or authenticate it.
#[derive(Clone)]
pub struct Session {
    store: Arc<StoreIdentity>,
    token: u64,
    principal: PrincipalId,
}
impl Session {
    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }
}

#[derive(Clone)]
pub struct InMemoryStore {
    inner: Arc<Mutex<Database>>,
    // Live capabilities and adapters belong to this embedding, never to stored state.
    identity: Arc<StoreIdentity>,
    source_artifact_readers: Arc<Mutex<SourceArtifactReaders>>,
}

/// Held only by the trusted embedding application, separate from worker Engine handles.
#[derive(Clone)]
pub struct Administrator {
    store: InMemoryStore,
}

#[derive(Clone)]
pub struct Engine {
    store: InMemoryStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReceipt {
    pub transaction: TransactionId,
    pub attempt: u64,
    pub revision: Revision,
    pub snapshot: SnapshotId,
    pub patch: PatchId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessProvenance {
    pub explicit: AccessSet,
    pub derived: AccessSet,
    pub invariant_scopes: BTreeSet<ElementId>,
    pub analysis: crate::access::AnalysisStamp,
    pub completeness: crate::access::Completeness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAction {
    AbortOrNewIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictReport {
    pub contender: TransactionId,
    pub contender_attempt: u64,
    pub committed: TransactionId,
    pub committed_attempt: u64,
    pub committed_revision: Revision,
    pub conflicts: Vec<Conflict>,
    pub contender_provenance: AccessProvenance,
    pub committed_provenance: AccessProvenance,
    pub action: ConflictAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptView {
    pub number: u64,
    pub base: SnapshotId,
    pub candidate: Option<SnapshotId>,
    pub patch: Option<PatchId>,
    pub analysis: Option<crate::access::ImpactAnalysis>,
    pub evidence: BTreeMap<CheckId, EvidenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseEventKind {
    Acquired {
        expires_at: u64,
    },
    Renewed {
        previous_expiry: u64,
        expires_at: u64,
    },
    Expired,
    Revoked,
    Released {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseEvent {
    pub at: u64,
    pub owner: PrincipalId,
    /// Canonical acquisition order; an entire set is acquired atomically.
    pub scopes: BTreeSet<ElementId>,
    pub kind: LeaseEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFault {
    None,
    BeforeValidation,
    DuringValidation,
    BeforePublication,
    /// Staged state exists only in a local copy; simulate failure before atomic swap.
    DuringPublication,
    AfterPublication,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EngineError {
    #[error("authoritative store unavailable or poisoned")]
    StoreUnavailable,
    #[error("unauthorized scoped action")]
    Unauthorized,
    #[error("record not found")]
    NotFound,
    #[error("record identity already exists; identities are never recycled")]
    AlreadyExists,
    #[error("invalid intent: {0}")]
    InvalidIntent(String),
    #[error("illegal state transition from {from:?} to {to:?}")]
    IllegalTransition {
        from: TransactionState,
        to: TransactionState,
    },
    #[error("unsupported, incomplete or stale engineering analysis")]
    IncompleteAnalysis,
    #[error("analysis generation changed; fresh proposal required")]
    ModelChanged,
    #[error("declared transaction dependency has not committed")]
    UnsatisfiedDependency,
    #[error("coordination scope is held by another live attempt")]
    ReservationConflict,
    #[error("reservation expired, revoked, missing or owned by another capability")]
    InvalidLease,
    #[error("invalid clock or lease duration")]
    InvalidTime,
    #[error("revision or attempt space exhausted")]
    CounterExhausted,
    #[error("versioned patch does not match attempt snapshot")]
    WrongPatchBase,
    #[error("patch writes an element outside the analyzed write set")]
    UndeclaredWrite,
    #[error("required invariant was not satisfied: {0}")]
    Invariant(#[from] InvariantViolation),
    #[error("patch application failed: {0}")]
    Patch(#[from] PatchError),
    #[error("snapshot construction failed: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("source evidence rejected: {0}")]
    SourceEvidence(#[from] crate::source_evidence::SourceEvidenceError),
    #[error("current revision requires explicit revalidation against {current}")]
    RevalidationRequired { current: SnapshotId },
    #[error("intervening committed access invalidated this attempt: {0:?}")]
    Conflict(Vec<ConflictReport>),
    #[error("evidence does not match the exact attempt, snapshot, patch, analysis or check")]
    EvidenceMismatch,
    #[error("missing required evidence for check {0}")]
    MissingEvidence(CheckId),
    #[error("failed or incomplete evidence for check {0}")]
    EvidenceNotPassed(CheckId),
    #[error("idempotency key already belongs to a different request or attempt")]
    IdempotencyConflict,
    #[error("injected failure before publication at {0:?}")]
    InjectedFailure(CommitFault),
    #[error("commit published but response was lost; retry the same key")]
    ResponseLost,
    #[error("repository analysis rejected: {0}")]
    RepositoryAnalysis(#[from] RepositoryAnalysisError),
    #[error("source repositories require a snapshot-bound analysis artifact")]
    SourceAnalysisRequired,
    #[error("integer reference patches cannot mutate a captured source repository")]
    SourceMutationUnsupported,
    #[error("source execution attempt rejected: {0}")]
    SourceAttempt(#[from] SourceAttemptError),
}

#[derive(Clone)]
struct Grant {
    principal: PrincipalId,
    scope: Scope,
    permissions: BTreeSet<Permission>,
    active: bool,
}

#[derive(Clone)]
struct Lease {
    owner: u64,
    expires_at: u64,
    writes: BTreeSet<ElementId>,
}

#[derive(Clone)]
struct Attempt {
    number: u64,
    base: Snapshot,
    model: EngineeringModel,
    effective: Option<AccessSet>,
    analysis: Option<crate::access::ImpactAnalysis>,
    patch: Option<Patch>,
    candidate: Option<Snapshot>,
    evidence: BTreeMap<CheckId, EvidenceId>,
}

#[derive(Clone)]
struct Transaction {
    id: TransactionId,
    owner: PrincipalId,
    scope: Scope,
    intent: TransactionIntent,
    state: TransactionState,
    attempts: Vec<Attempt>,
    lease: Option<Lease>,
    lease_history: Vec<LeaseEvent>,
    last_reason: Option<String>,
}

impl Transaction {
    fn attempt(&self) -> &Attempt {
        self.attempts
            .last()
            .expect("registered transaction has an attempt")
    }
    fn attempt_mut(&mut self) -> &mut Attempt {
        self.attempts
            .last_mut()
            .expect("registered transaction has an attempt")
    }
    fn transition(&mut self, next: TransactionState) -> Result<(), EngineError> {
        if !self.state.allows(next) {
            return Err(EngineError::IllegalTransition {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }
    fn view(&self) -> TransactionView {
        TransactionView {
            id: self.id.clone(),
            scope: self.scope.clone(),
            owner: self.owner.clone(),
            state: self.state,
            attempt: self.attempt().number,
            original_snapshot: self.attempts[0].base.id(),
            attempt_snapshot: self.attempt().base.id(),
            attempts: self.attempts.len(),
            last_reason: self.last_reason.clone(),
        }
    }
    fn bindings(&self) -> Result<EvidenceBindings, EngineError> {
        let attempt = self.attempt();
        Ok(EvidenceBindings {
            transaction: self.id.clone(),
            attempt: attempt.number,
            base_snapshot: attempt.base.id(),
            candidate_snapshot: attempt
                .candidate
                .as_ref()
                .ok_or(EngineError::EvidenceMismatch)?
                .id(),
            patch: attempt
                .patch
                .as_ref()
                .ok_or(EngineError::EvidenceMismatch)?
                .id(),
            analysis: attempt
                .analysis
                .as_ref()
                .ok_or(EngineError::IncompleteAnalysis)?
                .digest(),
        })
    }
}

#[derive(Clone)]
struct CommittedChange {
    transaction: TransactionId,
    attempt: u64,
    revision: Revision,
    accesses: AccessSet,
    provenance: AccessProvenance,
}

#[derive(Clone)]
struct Repository {
    current: Snapshot,
    snapshots: BTreeMap<SnapshotId, Snapshot>,
    model: EngineeringModel,
    history: Vec<CommittedChange>,
    source_analysis: Option<RepositoryAnalysis>,
}

#[derive(Clone)]
struct RecordedOperation {
    transaction: TransactionId,
    attempt: u64,
    outcome: Result<CommitReceipt, EngineError>,
}

#[derive(Clone, Default)]
// Adapter-free engineering records. Cloning records must not copy live authority
// or retain storage readers. This remains an in-memory reference representation;
// a durable backend must decode checked, normalized records, not serialize this blob.
struct Database {
    now: u64,
    next_token: u64,
    grants: BTreeMap<u64, Grant>,
    repositories: BTreeMap<Scope, Repository>,
    transactions: BTreeMap<(Scope, TransactionId), Transaction>,
    evidence: BTreeMap<(Scope, EvidenceId), VerificationEvidence>,
    evidence_sources: BTreeMap<(Scope, EvidenceId), u64>,
    operations: BTreeMap<(Scope, IdempotencyKey), RecordedOperation>,
    next_source_executor: u64,
    source_executors: BTreeMap<u64, source_attempts::SourceExecutorGrant>,
    source_policies: BTreeMap<Scope, crate::source_execution::SourceExecutionPolicy>,
    source_attempts: BTreeMap<(Scope, SourceAttemptId), source_attempts::SourceAttemptRecord>,
    source_operations: BTreeMap<(Scope, IdempotencyKey), source_attempts::RecordedSourceOperation>,
    source_rejections: BTreeMap<(Scope, SourceAttemptId), SourceRejectionView>,
    next_source_cleanup: u64,
    source_cleanup_grants: BTreeMap<u64, source_cleanup::SourceCleanupGrant>,
    next_source_artifact_store: u64,
    source_artifact_stores: BTreeMap<u64, source_evidence_gate::RegisteredSourceArtifactStore>,
    source_evidence_records:
        BTreeMap<(Scope, SourceEvidenceId), source_evidence_gate::RetainedSourceEvidence>,
}

impl InMemoryStore {
    pub fn new() -> (Self, Administrator) {
        let store = Self {
            inner: Arc::new(Mutex::new(Database::default())),
            identity: Arc::new(StoreIdentity),
            source_artifact_readers: Arc::new(Mutex::new(SourceArtifactReaders::default())),
        };
        let admin = Administrator {
            store: store.clone(),
        };
        (store, admin)
    }
    fn lock(&self) -> Result<MutexGuard<'_, Database>, EngineError> {
        self.inner.lock().map_err(|_| EngineError::StoreUnavailable)
    }
}

impl Administrator {
    pub fn grant(
        &self,
        principal: PrincipalId,
        scope: Scope,
        permissions: BTreeSet<Permission>,
    ) -> Result<Session, EngineError> {
        self.store.update(|db| {
            let token = db
                .next_token
                .checked_add(1)
                .ok_or(EngineError::CounterExhausted)?;
            db.next_token = token;
            db.grants.insert(
                token,
                Grant {
                    principal: principal.clone(),
                    scope,
                    permissions,
                    active: true,
                },
            );
            Ok(Session {
                store: self.store.identity.clone(),
                token,
                principal,
            })
        })
    }
    pub fn revoke(&self, session: &Session) -> Result<(), EngineError> {
        if !Arc::ptr_eq(&self.store.identity, &session.store) {
            return Err(EngineError::Unauthorized);
        }
        self.store.update(|db| {
            db.grants
                .get_mut(&session.token)
                .ok_or(EngineError::Unauthorized)?
                .active = false;
            let now = db.now;
            for tx in db.transactions.values_mut() {
                if let Some(lease) = &tx.lease
                    && lease.owner == session.token
                {
                    tx.lease_history.push(LeaseEvent {
                        at: now,
                        owner: session.principal.clone(),
                        scopes: lease.writes.clone(),
                        kind: LeaseEventKind::Revoked,
                    });
                }
            }
            Ok(())
        })
    }
    /// Monotonic test clock shared by every Engine handle. Actors cannot supply commit time.
    pub fn advance_time(&self, ticks: u64) -> Result<u64, EngineError> {
        self.store.update(|db| {
            let previous = db.now;
            db.now = db.now.checked_add(ticks).ok_or(EngineError::InvalidTime)?;
            let now = db.now;
            let principals: BTreeMap<_, _> = db
                .grants
                .iter()
                .map(|(token, g)| (*token, g.principal.clone()))
                .collect();
            for tx in db.transactions.values_mut() {
                if let Some(lease) = &tx.lease
                    && lease.expires_at > previous
                    && lease.expires_at <= now
                {
                    tx.lease_history.push(LeaseEvent {
                        at: lease.expires_at,
                        owner: principals[&lease.owner].clone(),
                        scopes: lease.writes.clone(),
                        kind: LeaseEventKind::Expired,
                    });
                }
            }
            Ok(db.now)
        })
    }
}

impl Engine {
    pub fn new(store: InMemoryStore) -> Self {
        Self { store }
    }
    fn authorize(
        &self,
        db: &Database,
        session: &Session,
        scope: &Scope,
        action: Permission,
    ) -> Result<(), EngineError> {
        if !Arc::ptr_eq(&self.store.identity, &session.store) {
            return Err(EngineError::Unauthorized);
        }
        let grant = db
            .grants
            .get(&session.token)
            .ok_or(EngineError::Unauthorized)?;
        if !grant.active
            || &grant.scope != scope
            || grant.principal != session.principal
            || !grant.permissions.contains(&action)
        {
            return Err(EngineError::Unauthorized);
        }
        Ok(())
    }
    pub fn register(
        &self,
        session: &Session,
        snapshot: Snapshot,
        model: EngineeringModel,
    ) -> Result<(), EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, snapshot.scope(), Permission::Register)?;
            // The integer reference harness has no source inputs. Captured workspaces
            // must enter through the artifact-bound repository registration path.
            if captured_workspace(&snapshot) {
                return Err(EngineError::SourceAnalysisRequired);
            }
            validate_model(&model)?;
            if snapshot.revision() != Revision(0) {
                return Err(EngineError::InvalidIntent(
                    "initial revision must be zero".into(),
                ));
            }
            if db.repositories.contains_key(snapshot.scope()) {
                return Err(EngineError::AlreadyExists);
            }
            db.repositories.insert(
                snapshot.scope().clone(),
                Repository {
                    snapshots: BTreeMap::from([(snapshot.id(), snapshot.clone())]),
                    current: snapshot,
                    model,
                    history: vec![],
                    source_analysis: None,
                },
            );
            Ok(())
        })
    }
    /// Register captured source state with an exact analysis artifact. This does not
    /// execute or mutate source. The reference integer-patch path remains unavailable.
    pub fn register_repository(
        &self,
        session: &Session,
        snapshot: Snapshot,
        analysis: RepositoryAnalysis,
    ) -> Result<(), EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, snapshot.scope(), Permission::Register)?;
            self.authorize(db, session, snapshot.scope(), Permission::ManageModel)?;
            analysis.validate(&snapshot, analysis.index_generation())?;
            if snapshot.revision() != Revision(0) {
                return Err(EngineError::InvalidIntent(
                    "initial revision must be zero".into(),
                ));
            }
            if db.repositories.contains_key(snapshot.scope()) {
                return Err(EngineError::AlreadyExists);
            }
            db.repositories.insert(
                snapshot.scope().clone(),
                Repository {
                    snapshots: BTreeMap::from([(snapshot.id(), snapshot.clone())]),
                    current: snapshot,
                    model: analysis.model(1),
                    history: vec![],
                    source_analysis: Some(analysis),
                },
            );
            Ok(())
        })
    }
    /// A fresh analysis may replace facts only for the exact current captured snapshot.
    /// It cannot advance repository revision or import an unverified workspace mutation.
    /// `expected_artifact` is a compare-and-swap precondition, not an analyzer's
    /// claim that its own generation is current. Exact installation retries are
    /// idempotent; a delayed competing update cannot replace newer facts.
    pub fn install_repository_analysis(
        &self,
        session: &Session,
        scope: &Scope,
        expected_artifact: Digest,
        analysis: RepositoryAnalysis,
    ) -> Result<(), EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::ManageModel)?;
            let repo = db
                .repositories
                .get_mut(scope)
                .ok_or(EngineError::NotFound)?;
            if repo.source_analysis.is_none() {
                return Err(EngineError::SourceAnalysisRequired);
            }
            analysis.validate(&repo.current, analysis.index_generation())?;
            if repo.source_analysis.as_ref() == Some(&analysis) {
                return Ok(());
            }
            if repo
                .source_analysis
                .as_ref()
                .is_none_or(|current| current.digest() != expected_artifact)
            {
                return Err(RepositoryAnalysisError::StaleGeneration.into());
            }
            let generation = repo
                .model
                .generation
                .checked_add(1)
                .ok_or(EngineError::CounterExhausted)?;
            repo.model = analysis.model(generation);
            repo.source_analysis = Some(analysis);
            Ok(())
        })
    }
    /// Query even an incomplete graph. Returned uncertainty does not authorize admission
    /// or commit, and requested snapshot/generation must match authoritative registration.
    pub fn inspect_repository_impact(
        &self,
        session: &Session,
        scope: &Scope,
        snapshot: SnapshotId,
        index_generation: Digest,
        explicit: &AccessSet,
    ) -> Result<RepositoryImpact, EngineError> {
        self.store.read(|db| {
            self.authorize(db, session, scope, Permission::Analyze)?;
            let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
            let analysis = repo
                .source_analysis
                .as_ref()
                .ok_or(EngineError::SourceAnalysisRequired)?;
            if repo.current.id() != snapshot {
                return Err(RepositoryAnalysisError::SnapshotMismatch.into());
            }
            analysis.validate(&repo.current, index_generation)?;
            Ok(analysis.inspect(repo.model.generation, explicit)?)
        })
    }
    pub fn current(&self, session: &Session, scope: &Scope) -> Result<Snapshot, EngineError> {
        self.store.read(|db| {
            self.authorize(db, session, scope, Permission::Read)?;
            Ok(db
                .repositories
                .get(scope)
                .ok_or(EngineError::NotFound)?
                .current
                .clone())
        })
    }
    pub fn transaction(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<TransactionView, EngineError> {
        self.store.read(|db| {
            self.authorize(db, session, scope, Permission::Read)?;
            Ok(db
                .transactions
                .get(&(scope.clone(), id.clone()))
                .ok_or(EngineError::NotFound)?
                .view())
        })
    }
    pub fn attempts(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<Vec<AttemptView>, EngineError> {
        self.store.read(|db| {
            self.authorize(db, session, scope, Permission::Read)?;
            let tx = db
                .transactions
                .get(&(scope.clone(), id.clone()))
                .ok_or(EngineError::NotFound)?;
            Ok(tx
                .attempts
                .iter()
                .map(|a| AttemptView {
                    number: a.number,
                    base: a.base.id(),
                    candidate: a.candidate.as_ref().map(Snapshot::id),
                    patch: a.patch.as_ref().map(Patch::id),
                    analysis: a.analysis.clone(),
                    evidence: a.evidence.clone(),
                })
                .collect())
        })
    }
    pub fn lease_history(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<Vec<LeaseEvent>, EngineError> {
        self.store.read(|db| {
            self.authorize(db, session, scope, Permission::Read)?;
            Ok(db
                .transactions
                .get(&(scope.clone(), id.clone()))
                .ok_or(EngineError::NotFound)?
                .lease_history
                .clone())
        })
    }
    pub fn update_model(
        &self,
        session: &Session,
        scope: &Scope,
        model: EngineeringModel,
    ) -> Result<(), EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::ManageModel)?;
            validate_model(&model)?;
            let repository = db
                .repositories
                .get_mut(scope)
                .ok_or(EngineError::NotFound)?;
            if repository.source_analysis.is_some() {
                return Err(EngineError::SourceAnalysisRequired);
            }
            if model.generation <= repository.model.generation {
                return Err(EngineError::ModelChanged);
            }
            repository.model = model;
            Ok(())
        })
    }
    pub fn propose(
        &self,
        session: &Session,
        scope: &Scope,
        id: TransactionId,
        base: SnapshotId,
        intent: TransactionIntent,
    ) -> Result<TransactionView, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Propose)?;
            validate_intent(&intent)?;
            let key = (scope.clone(), id.clone());
            if let Some(existing) = db.transactions.get(&key) {
                if existing.intent == intent
                    && existing.attempts[0].base.id() == base
                    && existing.owner == session.principal
                {
                    return Ok(existing.view());
                }
                return Err(EngineError::AlreadyExists);
            }
            let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
            let snapshot = repo
                .snapshots
                .get(&base)
                .ok_or(EngineError::NotFound)?
                .clone();
            for dependency in &intent.depends_on {
                if dependency == &id
                    || !db
                        .transactions
                        .get(&(scope.clone(), dependency.clone()))
                        .is_some_and(|tx| tx.state == TransactionState::Committed)
                {
                    return Err(EngineError::UnsatisfiedDependency);
                }
            }
            let tx = Transaction {
                id,
                owner: session.principal.clone(),
                scope: scope.clone(),
                intent,
                state: TransactionState::Proposed,
                attempts: vec![Attempt {
                    number: 0,
                    base: snapshot,
                    model: repo.model.clone(),
                    effective: None,
                    analysis: None,
                    patch: None,
                    candidate: None,
                    evidence: BTreeMap::new(),
                }],
                lease: None,
                lease_history: vec![],
                last_reason: None,
            };
            let view = tx.view();
            db.transactions.insert(key, tx);
            Ok(view)
        })
    }
    pub fn analyze(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<AccessSet, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Analyze)?;
            let key = (scope.clone(), id.clone());
            let mut tx = db
                .transactions
                .get(&key)
                .ok_or(EngineError::NotFound)?
                .clone();
            tx.transition(TransactionState::Analyzed)?;
            let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
            if tx.attempt().model != repo.model {
                return Err(EngineError::ModelChanged);
            }
            analyze_attempt(
                &mut tx,
                repo.source_analysis.as_ref(),
                db.source_policies.get(scope),
            )?;
            let accesses = tx
                .attempt()
                .effective
                .clone()
                .ok_or(EngineError::IncompleteAnalysis)?;
            db.transactions.insert(key, tx);
            Ok(accesses)
        })
    }
    pub fn reserve(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        ttl: u64,
    ) -> Result<u64, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Reserve)?;
            let key = (scope.clone(), id.clone());
            let mut tx = db
                .transactions
                .get(&key)
                .ok_or(EngineError::NotFound)?
                .clone();
            if tx.owner != session.principal {
                return Err(EngineError::Unauthorized);
            }
            tx.transition(TransactionState::Reserved)?;
            let expires_at = expiration(db.now, ttl)?;
            let writes = tx
                .attempt()
                .effective
                .as_ref()
                .ok_or(EngineError::IncompleteAnalysis)?
                .writes
                .clone();
            for ((other_scope, other_id), other) in &db.transactions {
                if other_scope == scope
                    && other_id != id
                    && let Some(lease) = &other.lease
                    && lease_active(db, lease)
                    && !lease.writes.is_disjoint(&writes)
                {
                    return Err(EngineError::ReservationConflict);
                }
            }
            tx.lease_history.push(LeaseEvent {
                at: db.now,
                owner: session.principal.clone(),
                scopes: writes.clone(),
                kind: LeaseEventKind::Acquired { expires_at },
            });
            tx.lease = Some(Lease {
                owner: session.token,
                expires_at,
                writes,
            });
            db.transactions.insert(key, tx);
            Ok(expires_at)
        })
    }
    pub fn renew(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        ttl: u64,
    ) -> Result<u64, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Reserve)?;
            let key = (scope.clone(), id.clone());
            let tx = db.transactions.get(&key).ok_or(EngineError::NotFound)?;
            require_lease(db, tx, session)?;
            let expires = expiration(db.now, ttl)?;
            let now = db.now;
            let tx = db.transactions.get_mut(&key).ok_or(EngineError::NotFound)?;
            let lease = tx.lease.as_mut().ok_or(EngineError::InvalidLease)?;
            // Renewal never accidentally shortens a still-valid lease.
            let previous_expiry = lease.expires_at;
            lease.expires_at = expires.max(lease.expires_at);
            tx.lease_history.push(LeaseEvent {
                at: now,
                owner: session.principal.clone(),
                scopes: lease.writes.clone(),
                kind: LeaseEventKind::Renewed {
                    previous_expiry,
                    expires_at: lease.expires_at,
                },
            });
            Ok(lease.expires_at)
        })
    }
    pub fn execute(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<(), EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Execute)?;
            let key = (scope.clone(), id.clone());
            let tx = db.transactions.get(&key).ok_or(EngineError::NotFound)?;
            require_lease(db, tx, session)?;
            db.transactions
                .get_mut(&key)
                .ok_or(EngineError::NotFound)?
                .transition(TransactionState::Executing)
        })
    }
    pub fn submit_patch(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        patch: Patch,
    ) -> Result<EvidenceBindings, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Execute)?;
            if db
                .repositories
                .get(scope)
                .ok_or(EngineError::NotFound)?
                .source_analysis
                .is_some()
            {
                return Err(EngineError::SourceMutationUnsupported);
            }
            let key = (scope.clone(), id.clone());
            let mut tx = db
                .transactions
                .get(&key)
                .ok_or(EngineError::NotFound)?
                .clone();
            require_lease(db, &tx, session)?;
            tx.transition(TransactionState::Validating)?;
            if patch.base_snapshot() != tx.attempt().base.id() {
                return Err(EngineError::WrongPatchBase);
            }
            let writes = &tx
                .attempt()
                .effective
                .as_ref()
                .ok_or(EngineError::IncompleteAnalysis)?
                .writes;
            if patch
                .changes()
                .keys()
                .any(|element| !writes.contains(element))
            {
                return Err(EngineError::UndeclaredWrite);
            }
            let state = patch.apply(tx.attempt().base.state())?;
            for invariant in &tx.intent.invariants {
                invariant.evaluate(&state)?;
            }
            let revision = next_revision(tx.attempt().base.revision())?;
            let candidate = tx.attempt().base.successor(revision, state)?;
            tx.attempt_mut().patch = Some(patch);
            tx.attempt_mut().candidate = Some(candidate);
            let bindings = tx.bindings()?;
            db.transactions.insert(key, tx);
            Ok(bindings)
        })
    }
    pub fn bindings(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<EvidenceBindings, EngineError> {
        self.store.read(|db| {
            self.authorize(db, session, scope, Permission::Read)?;
            db.transactions
                .get(&(scope.clone(), id.clone()))
                .ok_or(EngineError::NotFound)?
                .bindings()
        })
    }
    pub fn candidate(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<Snapshot, EngineError> {
        self.store.read(|db| {
            self.authorize(db, session, scope, Permission::Read)?;
            db.transactions
                .get(&(scope.clone(), id.clone()))
                .ok_or(EngineError::NotFound)?
                .attempt()
                .candidate
                .clone()
                .ok_or(EngineError::NotFound)
        })
    }
    pub fn record_evidence(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        evidence: VerificationEvidence,
    ) -> Result<EvidenceId, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Verify)?;
            let key = (scope.clone(), id.clone());
            let tx = db.transactions.get(&key).ok_or(EngineError::NotFound)?;
            if tx.state != TransactionState::Validating {
                return Err(EngineError::IllegalTransition {
                    from: tx.state,
                    to: TransactionState::Validating,
                });
            }
            if evidence.provenance().verifier != session.principal
                || evidence.bindings() != &tx.bindings()?
            {
                return Err(EngineError::EvidenceMismatch);
            }
            let required = tx
                .intent
                .required_checks
                .iter()
                .find(|check| check.id() == evidence.check().id())
                .ok_or(EngineError::EvidenceMismatch)?;
            if !evidence.matches(&tx.bindings()?, required) {
                return Err(EngineError::EvidenceMismatch);
            }
            if evidence.provenance().finished_at > db.now {
                return Err(EngineError::EvidenceMismatch);
            }
            let evidence_id = evidence.id();
            let check_id = evidence.check().id().clone();
            db.evidence.insert((scope.clone(), evidence_id), evidence);
            db.evidence_sources
                .insert((scope.clone(), evidence_id), session.token);
            db.transactions
                .get_mut(&key)
                .ok_or(EngineError::NotFound)?
                .attempt_mut()
                .evidence
                .insert(check_id, evidence_id);
            Ok(evidence_id)
        })
    }
    pub fn abort(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        reason: String,
    ) -> Result<(), EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Cancel)?;
            if reason.trim().is_empty() {
                return Err(EngineError::InvalidIntent("abort needs reason".into()));
            }
            let now = db.now;
            let tx = db
                .transactions
                .get_mut(&(scope.clone(), id.clone()))
                .ok_or(EngineError::NotFound)?;
            if tx.state == TransactionState::Aborted && tx.last_reason.as_ref() == Some(&reason) {
                return Ok(());
            }
            tx.transition(TransactionState::Aborted)?;
            if let Some(lease) = &tx.lease {
                tx.lease_history.push(LeaseEvent {
                    at: now,
                    owner: tx.owner.clone(),
                    scopes: lease.writes.clone(),
                    kind: LeaseEventKind::Released {
                        reason: reason.clone(),
                    },
                });
            }
            tx.lease = None;
            tx.last_reason = Some(reason);
            source_attempts::cancel_transaction_attempts(db, scope, id)?;
            Ok(())
        })
    }
    pub fn revalidate(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<EvidenceBindings, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Analyze)?;
            if db
                .repositories
                .get(scope)
                .ok_or(EngineError::NotFound)?
                .source_analysis
                .is_some()
            {
                return Err(EngineError::SourceMutationUnsupported);
            }
            let key = (scope.clone(), id.clone());
            let mut tx = db
                .transactions
                .get(&key)
                .ok_or(EngineError::NotFound)?
                .clone();
            require_lease(db, &tx, session)?;
            tx.transition(TransactionState::Validating)?;
            let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
            check_intervening(&tx, repo)?;
            if tx.attempt().model != repo.model {
                return Err(EngineError::ModelChanged);
            }
            let old = tx.attempt();
            let old_patch = old.patch.as_ref().ok_or(EngineError::WrongPatchBase)?;
            let patch = Patch::new(repo.current.id(), old_patch.changes().clone())?;
            let candidate = repo.current.successor(
                next_revision(repo.current.revision())?,
                patch.apply(repo.current.state())?,
            )?;
            for invariant in &tx.intent.invariants {
                invariant.evaluate(candidate.state())?;
            }
            let number = old
                .number
                .checked_add(1)
                .ok_or(EngineError::CounterExhausted)?;
            tx.attempts.push(Attempt {
                number,
                base: repo.current.clone(),
                model: repo.model.clone(),
                effective: None,
                analysis: None,
                patch: Some(patch),
                candidate: Some(candidate),
                evidence: BTreeMap::new(),
            });
            analyze_attempt(
                &mut tx,
                repo.source_analysis.as_ref(),
                db.source_policies.get(scope),
            )?;
            tx.last_reason = Some("compatible revision advance; fresh evidence required".into());
            let bindings = tx.bindings()?;
            db.transactions.insert(key, tx);
            Ok(bindings)
        })
    }
    pub fn commit(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        key: IdempotencyKey,
    ) -> Result<CommitReceipt, EngineError> {
        self.commit_with_fault(session, scope, id, key, CommitFault::None)
    }
    /// Test harness injection points. This operation still runs the complete commit gate.
    pub fn commit_with_fault(
        &self,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        key: IdempotencyKey,
        fault: CommitFault,
    ) -> Result<CommitReceipt, EngineError> {
        self.store.update(|db| {
            self.authorize(db, session, scope, Permission::Commit)?;
            let tx_key = (scope.clone(), id.clone());
            let tx = db
                .transactions
                .get(&tx_key)
                .ok_or(EngineError::NotFound)?
                .clone();
            let operation_key = (scope.clone(), key);
            if let Some(record) = db.operations.get(&operation_key) {
                if record.transaction != *id || record.attempt != tx.attempt().number {
                    return Err(EngineError::IdempotencyConflict);
                }
                return record.outcome.clone();
            }
            if fault == CommitFault::BeforeValidation {
                return Err(EngineError::InjectedFailure(fault));
            }
            let result = validate_commit(db, &tx, session);
            if fault == CommitFault::DuringValidation {
                return Err(EngineError::InjectedFailure(fault));
            }
            let receipt = match result {
                Ok(receipt) => receipt,
                Err(error) => {
                    if let EngineError::RevalidationRequired { .. } = &error {
                        let stored = db
                            .transactions
                            .get_mut(&tx_key)
                            .ok_or(EngineError::NotFound)?;
                        stored.transition(TransactionState::RevalidationRequired)?;
                        stored.last_reason = Some(error.to_string());
                    }
                    db.operations.insert(
                        operation_key,
                        RecordedOperation {
                            transaction: id.clone(),
                            attempt: tx.attempt().number,
                            outcome: Err(error.clone()),
                        },
                    );
                    return Err(error);
                }
            };
            if fault == CommitFault::BeforePublication {
                return Err(EngineError::InjectedFailure(fault));
            }
            // Stage *every* side effect together. No authoritative field is changed before swap.
            // A durable adapter must replace this boundary with one storage transaction/CAS.
            let mut staged = db.clone();
            let repo = staged
                .repositories
                .get_mut(scope)
                .ok_or(EngineError::NotFound)?;
            let candidate = tx
                .attempt()
                .candidate
                .clone()
                .ok_or(EngineError::EvidenceMismatch)?;
            repo.current = candidate.clone();
            repo.snapshots.insert(candidate.id(), candidate);
            repo.history.push(CommittedChange {
                transaction: tx.id.clone(),
                attempt: tx.attempt().number,
                revision: receipt.revision,
                accesses: tx
                    .attempt()
                    .effective
                    .clone()
                    .ok_or(EngineError::IncompleteAnalysis)?,
                provenance: provenance(&tx)?,
            });
            let stored = staged
                .transactions
                .get_mut(&tx_key)
                .ok_or(EngineError::NotFound)?;
            stored.transition(TransactionState::Committed)?;
            if let Some(lease) = &stored.lease {
                stored.lease_history.push(LeaseEvent {
                    at: db.now,
                    owner: session.principal.clone(),
                    scopes: lease.writes.clone(),
                    kind: LeaseEventKind::Released {
                        reason: "committed".into(),
                    },
                });
            }
            stored.lease = None;
            stored.last_reason = Some("atomic validation and publication".into());
            staged.operations.insert(
                operation_key,
                RecordedOperation {
                    transaction: id.clone(),
                    attempt: tx.attempt().number,
                    outcome: Ok(receipt.clone()),
                },
            );
            if fault == CommitFault::DuringPublication {
                return Err(EngineError::InjectedFailure(fault));
            }
            *db = staged;
            if fault == CommitFault::AfterPublication {
                return Err(EngineError::ResponseLost);
            }
            Ok(receipt)
        })
    }
}

/// Fail closed for every nonempty source/input layer, capture provenance and
/// capture metadata (including an empty captured Git repository). This does not
/// establish the truth of a producer's census; snapshot producers remain trusted.
fn captured_workspace(snapshot: &Snapshot) -> bool {
    let manifest = snapshot.manifest();
    !manifest.inputs.is_empty()
        || !manifest.inclusion_policy.excluded_paths.is_empty()
        || manifest.provenance.starts_with("celetixo/git-capture/")
        || manifest
            .environment
            .keys()
            .any(|key| key.starts_with("celetixo.capture.") || key.starts_with("celetixo.overlay."))
}

fn validate_model(model: &EngineeringModel) -> Result<(), EngineError> {
    model
        .validate()
        .map_err(|_| EngineError::IncompleteAnalysis)
}

fn validate_intent(intent: &TransactionIntent) -> Result<(), EngineError> {
    if intent.objective.trim().is_empty() || intent.required_checks.is_empty() {
        return Err(EngineError::InvalidIntent(
            "objective and required checks must be nonempty".into(),
        ));
    }
    let mut checks = BTreeSet::new();
    for check in &intent.required_checks {
        if !checks.insert(check.id()) {
            return Err(EngineError::InvalidIntent(
                "duplicate required check identity".into(),
            ));
        }
    }
    let mut invariants = BTreeSet::new();
    for invariant in &intent.invariants {
        if !invariants.insert(&invariant.id) {
            return Err(EngineError::InvalidIntent(
                "duplicate invariant identity".into(),
            ));
        }
    }
    Ok(())
}

fn analyze_attempt(
    tx: &mut Transaction,
    source: Option<&RepositoryAnalysis>,
    policy: Option<&crate::source_execution::SourceExecutionPolicy>,
) -> Result<(), EngineError> {
    let mut explicit = tx.intent.accesses.clone();
    for invariant in &tx.intent.invariants {
        explicit.invariant_scopes.extend(invariant.scope());
        // Invariants are actual dependencies, even if a requester omits their read set.
        explicit.reads.extend(invariant.scope());
    }
    let (effective, analysis) = if let Some(source) = source {
        source.validate(&tx.attempt().base, source.index_generation())?;
        let (effective, impact, _) = source_attempts::derive_source_impact(
            source,
            tx.attempt().model.generation,
            &explicit,
            policy,
        )?;
        (effective, impact.analysis)
    } else {
        let analysis = tx
            .attempt()
            .model
            .analyze(tx.attempt().base.id(), &explicit);
        (explicit.union(analysis.derived()), analysis)
    };
    if !matches!(
        analysis.completeness(),
        crate::access::Completeness::Complete
    ) {
        return Err(EngineError::IncompleteAnalysis);
    }
    tx.attempt_mut().effective = Some(effective);
    tx.attempt_mut().analysis = Some(analysis);
    Ok(())
}

fn expiration(now: u64, ttl: u64) -> Result<u64, EngineError> {
    if ttl == 0 {
        return Err(EngineError::InvalidTime);
    }
    now.checked_add(ttl).ok_or(EngineError::InvalidTime)
}
fn lease_active(db: &Database, lease: &Lease) -> bool {
    lease.expires_at > db.now
        && db
            .grants
            .get(&lease.owner)
            .is_some_and(|grant| grant.active)
}
fn require_lease(db: &Database, tx: &Transaction, session: &Session) -> Result<(), EngineError> {
    if !tx
        .lease
        .as_ref()
        .is_some_and(|lease| lease.owner == session.token && lease_active(db, lease))
    {
        return Err(EngineError::InvalidLease);
    }
    Ok(())
}
fn next_revision(revision: Revision) -> Result<Revision, EngineError> {
    Ok(Revision(
        revision
            .0
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?,
    ))
}
fn check_intervening(tx: &Transaction, repo: &Repository) -> Result<(), EngineError> {
    let accesses = tx
        .attempt()
        .effective
        .as_ref()
        .ok_or(EngineError::IncompleteAnalysis)?;
    let mut conflicts = Vec::new();
    for change in &repo.history {
        if change.revision > tx.attempt().base.revision() {
            let details = accesses.conflicts(&change.accesses);
            if !details.is_empty() {
                conflicts.push(ConflictReport {
                    contender: tx.id.clone(),
                    contender_attempt: tx.attempt().number,
                    committed: change.transaction.clone(),
                    committed_attempt: change.attempt,
                    committed_revision: change.revision,
                    conflicts: details,
                    contender_provenance: provenance(tx)?,
                    committed_provenance: change.provenance.clone(),
                    action: ConflictAction::AbortOrNewIntent,
                });
            }
        }
    }
    if !conflicts.is_empty() {
        return Err(EngineError::Conflict(conflicts));
    }
    Ok(())
}
fn provenance(tx: &Transaction) -> Result<AccessProvenance, EngineError> {
    let analysis = tx
        .attempt()
        .analysis
        .as_ref()
        .ok_or(EngineError::IncompleteAnalysis)?;
    Ok(AccessProvenance {
        explicit: tx.intent.accesses.clone(),
        derived: analysis.derived().clone(),
        invariant_scopes: tx
            .attempt()
            .effective
            .as_ref()
            .ok_or(EngineError::IncompleteAnalysis)?
            .invariant_scopes
            .clone(),
        analysis: analysis.stamp().clone(),
        completeness: analysis.completeness().clone(),
    })
}
fn validate_commit(
    db: &Database,
    tx: &Transaction,
    session: &Session,
) -> Result<CommitReceipt, EngineError> {
    let repo = db
        .repositories
        .get(&tx.scope)
        .ok_or(EngineError::NotFound)?;
    if repo.source_analysis.is_some() || captured_workspace(&repo.current) {
        return Err(EngineError::SourceMutationUnsupported);
    }
    if tx.state != TransactionState::Validating {
        return Err(EngineError::IllegalTransition {
            from: tx.state,
            to: TransactionState::Committed,
        });
    }
    require_lease(db, tx, session)?;
    let attempt = tx.attempt();
    if attempt.model != repo.model {
        return Err(EngineError::ModelChanged);
    }
    if !repo.snapshots.contains_key(&attempt.base.id()) {
        return Err(EngineError::NotFound);
    }
    if attempt.base.id() != repo.current.id() || attempt.base.revision() != repo.current.revision()
    {
        return Err(EngineError::RevalidationRequired {
            current: repo.current.id(),
        });
    }
    check_intervening(tx, repo)?;
    let analysis = attempt
        .analysis
        .as_ref()
        .ok_or(EngineError::IncompleteAnalysis)?;
    if analysis.stamp().snapshot != attempt.base.id()
        || !matches!(
            analysis.completeness(),
            crate::access::Completeness::Complete
        )
    {
        return Err(EngineError::IncompleteAnalysis);
    }
    let patch = attempt.patch.as_ref().ok_or(EngineError::WrongPatchBase)?;
    if patch.base_snapshot() != attempt.base.id() {
        return Err(EngineError::WrongPatchBase);
    }
    let effective = attempt
        .effective
        .as_ref()
        .ok_or(EngineError::IncompleteAnalysis)?;
    if patch
        .changes()
        .keys()
        .any(|id| !effective.writes.contains(id))
    {
        return Err(EngineError::UndeclaredWrite);
    }
    let state = patch.apply(repo.current.state())?;
    for invariant in &tx.intent.invariants {
        invariant.evaluate(&state)?;
    }
    let candidate = repo
        .current
        .successor(next_revision(repo.current.revision())?, state)?;
    if attempt.candidate.as_ref() != Some(&candidate) {
        return Err(EngineError::EvidenceMismatch);
    }
    let bindings = tx.bindings()?;
    for check in &tx.intent.required_checks {
        let evidence_id = attempt
            .evidence
            .get(check.id())
            .ok_or_else(|| EngineError::MissingEvidence(check.id().clone()))?;
        let evidence = db
            .evidence
            .get(&(tx.scope.clone(), *evidence_id))
            .ok_or_else(|| EngineError::MissingEvidence(check.id().clone()))?;
        let source = db
            .evidence_sources
            .get(&(tx.scope.clone(), *evidence_id))
            .ok_or(EngineError::EvidenceMismatch)?;
        let signer = db.grants.get(source).ok_or(EngineError::Unauthorized)?;
        if !signer.active
            || signer.scope != tx.scope
            || !signer.permissions.contains(&Permission::Verify)
            || signer.principal != evidence.provenance().verifier
        {
            return Err(EngineError::Unauthorized);
        }
        if !evidence.matches(&bindings, check) {
            return Err(EngineError::EvidenceMismatch);
        }
        if !evidence.passed() {
            return Err(EngineError::EvidenceNotPassed(check.id().clone()));
        }
    }
    for dependency in &tx.intent.depends_on {
        if !db
            .transactions
            .get(&(tx.scope.clone(), dependency.clone()))
            .is_some_and(|other| other.state == TransactionState::Committed)
        {
            return Err(EngineError::UnsatisfiedDependency);
        }
    }
    Ok(CommitReceipt {
        transaction: tx.id.clone(),
        attempt: attempt.number,
        revision: candidate.revision(),
        snapshot: candidate.id(),
        patch: patch.id(),
    })
}
