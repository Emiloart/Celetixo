//! Original typed mutation requests and outcomes. These native domain values
//! are storage contracts, not a replacement for the canonical service protocol.
use super::*;
use authority::AuthorityFailure;
mod administrative;
mod execution;
mod repository;
mod source_commit;
mod source_executor;
pub use administrative::{AdministrativeContext, AdministrativeMutation, AdministrativeResource};
use execution::{
    check_shape as check_execution_shape, error_size as execution_error_size, evidence_size,
    patch_size, validate as validate_execution,
};
pub use repository::RepositoryMutation;
pub(super) fn repository_analysis_size(
    analysis: &RepositoryAnalysis,
) -> Result<crate::persistence::RecoverySize, crate::persistence::RestoreError> {
    repository::analysis_size(analysis)
}
pub use source_executor::{
    SourceExecutorAction, SourceExecutorMutation, SourceMutationContext, SourceMutationResult,
};

/// Attribution retained from an independently authenticated runtime capability.
/// Constructing this metadata never creates a Session or executor capability.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum MutationActor {
    EmbeddingAdministrator,
    Session { principal: PrincipalId, grant: u64 },
    SourceExecutor { principal: PrincipalId, grant: u64 },
}
impl MutationActor {
    pub fn digest(&self) -> Digest {
        let mut c = Canonical::default();
        self.encode(&mut c);
        c.finish("celetixo/engine-mutation-actor/v1")
    }
    pub(crate) fn encode(&self, c: &mut Canonical) {
        match self {
            Self::EmbeddingAdministrator => c.str("EmbeddingAdministrator"),
            Self::Session { principal, grant } => {
                c.str("Session");
                c.str(principal.as_str());
                c.u64(*grant);
            }
            Self::SourceExecutor { principal, grant } => {
                c.str("SourceExecutor");
                c.str(principal.as_str());
                c.u64(*grant);
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MutationKey {
    pub actor: MutationActor,
    pub key: IdempotencyKey,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationRequest {
    SourceCommit(Box<SourceCommitRequest>),
    SourceExecutor(Box<SourceExecutorMutation>),
    Administrative(Box<AdministrativeMutation>),
    Repository(Box<RepositoryMutation>),
    SubmitPatch {
        transaction: TransactionId,
        patch: Patch,
    },
    RecordEvidence {
        transaction: TransactionId,
        evidence: Box<VerificationEvidence>,
    },
    Revalidate {
        transaction: TransactionId,
    },
    Propose {
        transaction: TransactionId,
        base: SnapshotId,
        intent: Box<TransactionIntent>,
    },
    Analyze {
        transaction: TransactionId,
    },
    Reserve {
        transaction: TransactionId,
        ttl_ms: u64,
    },
    Renew {
        transaction: TransactionId,
        ttl_ms: u64,
    },
    Execute {
        transaction: TransactionId,
    },
    Abort {
        transaction: TransactionId,
        reason: String,
    },
}
impl MutationRequest {
    pub const FORMAT: &str = "celetixo/engine-mutation-request/v1";
    pub fn transaction(&self) -> Option<&TransactionId> {
        match self {
            Self::SourceCommit(request) => Some(&request.binding.transaction),
            Self::SourceExecutor(request) => Some(&request.binding.transaction),
            Self::Repository(_) | Self::Administrative(_) => None,
            Self::Analyze { transaction }
            | Self::Propose { transaction, .. }
            | Self::Reserve { transaction, .. }
            | Self::Renew { transaction, .. }
            | Self::Execute { transaction }
            | Self::SubmitPatch { transaction, .. }
            | Self::RecordEvidence { transaction, .. }
            | Self::Revalidate { transaction }
            | Self::Abort { transaction, .. } => Some(transaction),
        }
    }
    fn permission(&self) -> Option<Permission> {
        Some(match self {
            Self::SourceCommit(_) => Permission::Commit,
            Self::Administrative(_) | Self::SourceExecutor(_) => return None,
            Self::Repository(request) => request.permission(),
            Self::Analyze { .. } => Permission::Analyze,
            Self::Revalidate { .. } => Permission::Analyze,
            Self::SubmitPatch { .. } => Permission::Execute,
            Self::RecordEvidence { .. } => Permission::Verify,
            Self::Propose { .. } => Permission::Propose,
            Self::Reserve { .. } | Self::Renew { .. } => Permission::Reserve,
            Self::Execute { .. } => Permission::Execute,
            Self::Abort { .. } => Permission::Cancel,
        })
    }
    pub(super) fn check_recovery_limits(&self) -> Result<(), EngineError> {
        let limits = authority::NativeRecordLimits::default();
        let mut bytes = 4096usize;
        let mut entries = 1usize;
        match self {
            Self::SourceCommit(request) => {
                let size = request.recovery_size().map_err(|_| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                bytes = bytes.checked_add(size.bytes).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                entries = entries.checked_add(size.entries).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
            }
            Self::SourceExecutor(request) => {
                let size = request.recovery_size().map_err(|_| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                bytes = bytes.checked_add(size.bytes).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                entries = entries.checked_add(size.entries).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
            }
            Self::Administrative(request) => {
                let size = request.recovery_size().map_err(|_| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                bytes = bytes.checked_add(size.bytes).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                entries = entries.checked_add(size.entries).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
            }
            Self::Repository(request) => {
                let size = request.recovery_size().map_err(|_| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                bytes = bytes.checked_add(size.bytes).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                entries = entries.checked_add(size.entries).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
            }
            Self::SubmitPatch { patch, .. } => {
                let size = patch_size(patch).map_err(|_| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                bytes = bytes.checked_add(size.bytes).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                entries = entries.checked_add(size.entries).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
            }
            Self::RecordEvidence { evidence, .. } => {
                let size = evidence_size(evidence).map_err(|_| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                bytes = bytes.checked_add(size.bytes).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                entries = entries.checked_add(size.entries).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
            }
            Self::Propose { intent, .. } => {
                let size = proposal_size(intent).map_err(|_| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                bytes = bytes.checked_add(size.bytes).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
                entries = entries.checked_add(size.entries).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?;
            }
            Self::Abort { reason, .. } => {
                bytes = bytes.checked_add(reason.len()).ok_or_else(|| {
                    EngineError::InvalidIntent("mutation request exceeds recovery limits".into())
                })?
            }
            _ => {}
        }
        if bytes > limits.value_bytes || entries > limits.entries {
            return Err(EngineError::InvalidIntent(
                "mutation request exceeds recovery limits".into(),
            ));
        }
        Ok(())
    }
    pub fn digest(&self, scope: &Scope, actor: &MutationActor) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes(scope, actor))
    }
    pub fn canonical_bytes(&self, scope: &Scope, actor: &MutationActor) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(scope.organization.as_str());
        c.str(scope.workspace.as_str());
        c.str(scope.repository.as_str());
        actor.encode(&mut c);
        match self {
            Self::SourceCommit(request) => {
                c.str("CommitSource");
                request.encode_body(&mut c);
            }
            Self::SourceExecutor(request) => request.encode(&mut c),
            Self::Administrative(request) => request.encode(&mut c),
            Self::Repository(request) => request.encode(&mut c),
            Self::SubmitPatch { transaction, patch } => {
                c.str("SubmitPatch");
                c.str(transaction.as_str());
                c.digest(patch.id().0);
                c.bytes(&patch.canonical_bytes());
            }
            Self::RecordEvidence {
                transaction,
                evidence,
            } => {
                c.str("RecordEvidence");
                c.str(transaction.as_str());
                c.digest(evidence.bindings().base_snapshot.0);
                crate::persistence::encode_verification_fields(&mut c, evidence);
            }
            Self::Revalidate { transaction } => {
                c.str("Revalidate");
                c.str(transaction.as_str());
            }
            Self::Propose {
                transaction,
                base,
                intent,
            } => {
                c.str("Propose");
                c.str(transaction.as_str());
                c.digest(base.0);
                crate::persistence::encode_intent(&mut c, intent);
            }
            Self::Analyze { transaction } => {
                c.str("Analyze");
                c.str(transaction.as_str());
            }
            Self::Reserve {
                transaction,
                ttl_ms,
            }
            | Self::Renew {
                transaction,
                ttl_ms,
            } => {
                c.str(if matches!(self, Self::Reserve { .. }) {
                    "Reserve"
                } else {
                    "Renew"
                });
                c.str(transaction.as_str());
                c.u64(*ttl_ms);
            }
            Self::Execute { transaction } => {
                c.str("Execute");
                c.str(transaction.as_str());
            }
            Self::Abort {
                transaction,
                reason,
            } => {
                c.str("Abort");
                c.str(transaction.as_str());
                c.str(reason);
            }
        }
        c.into_bytes()
    }
}

/// A historical result, never a currently valid lease or launch permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationValue {
    SourceCommitted(Box<SourceCommitReceipt>),
    SourceAttempt(Box<SourceMutationResult>),
    Allocation {
        resource: AdministrativeResource,
        id: u64,
    },
    Revocation {
        resource: AdministrativeResource,
        id: u64,
    },
    Policy {
        policy: Digest,
        model_generation: u64,
    },
    Repository {
        snapshot: SnapshotId,
        model: Box<EngineeringModel>,
    },
    Bindings {
        bindings: EvidenceBindings,
    },
    Evidence {
        evidence: EvidenceId,
    },
    Analysis {
        attempt: u64,
        accesses: AccessSet,
    },
    Lease {
        expires_at_ms: u64,
        fence: u64,
        acquisition_attempt: u64,
    },
    State {
        state: TransactionState,
        attempt: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationRecord {
    pub request: MutationRequest,
    pub recorded_at_ms: u64,
    pub resolved_attempt: Option<u64>,
    pub state_before: Option<TransactionState>,
    pub lease_events_before: u64,
    pub administration: Option<AdministrativeContext>,
    pub source_execution: Option<SourceMutationContext>,
    pub source_commit: Option<Box<SourceCommitContext>>,
    pub outcome: Result<MutationValue, EngineError>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationResponse {
    pub request: Digest,
    pub recorded_at_ms: u64,
    pub outcome: Result<MutationValue, EngineError>,
    /// Delivery information only; the semantic outcome remains the original one.
    pub replayed: bool,
}
impl MutationRecord {
    pub(crate) fn recovery_size(
        &self,
        scope: &Scope,
        key: &MutationKey,
    ) -> Result<crate::persistence::RecoverySize, crate::persistence::RestoreError> {
        recovery_size(scope, key, self)
    }
    pub(crate) fn check_shape(
        &self,
        actor: &MutationActor,
    ) -> Result<(), crate::persistence::RestoreError> {
        use crate::persistence::RestoreError;
        if let MutationRequest::SourceCommit(request) = &self.request {
            return source_commit::check_record(request, self, actor);
        }
        if self.source_commit.is_some() {
            return Err(RestoreError::BindingMismatch);
        }
        if let MutationRequest::SourceExecutor(request) = &self.request {
            return if matches!(actor, MutationActor::SourceExecutor {grant,..} if *grant != 0) {
                request.check_record(self)
            } else {
                Err(RestoreError::UnsupportedDomainValue)
            };
        }
        if self.source_execution.is_some() {
            return Err(RestoreError::BindingMismatch);
        }
        if let MutationRequest::Administrative(request) = &self.request {
            return if *actor == MutationActor::EmbeddingAdministrator {
                request.check_record(self)
            } else {
                Err(RestoreError::UnsupportedDomainValue)
            };
        }
        if self.administration.is_some() {
            return Err(RestoreError::BindingMismatch);
        }
        if !matches!(actor, MutationActor::Session { grant, .. } if *grant != 0) {
            return Err(RestoreError::UnsupportedDomainValue);
        }
        if self.resolved_attempt.is_some() != self.state_before.is_some() {
            return Err(RestoreError::BindingMismatch);
        }
        if self.resolved_attempt.is_none() && self.lease_events_before != 0 {
            return Err(RestoreError::BindingMismatch);
        }
        if let MutationRequest::Repository(request) = &self.request {
            return request.check_record(self);
        }
        if matches!(
            self.request,
            MutationRequest::SubmitPatch { .. }
                | MutationRequest::RecordEvidence { .. }
                | MutationRequest::Revalidate { .. }
        ) {
            return check_execution_shape(self);
        }
        if matches!(self.request, MutationRequest::Propose { .. }) {
            return check_proposal_shape(self);
        }
        if matches!(self.request, MutationRequest::Analyze { .. }) {
            return check_analysis_shape(self);
        }
        if matches!(
            self.request,
            MutationRequest::Execute { .. } | MutationRequest::Abort { .. }
        ) {
            return check_transition_shape(self);
        }
        if self.resolved_attempt.is_none() {
            return if self.lease_events_before == 0 && self.outcome == Err(EngineError::NotFound) {
                Ok(())
            } else {
                Err(RestoreError::BindingMismatch)
            };
        }
        let before = self.state_before.ok_or(RestoreError::BindingMismatch)?;
        let is_reserve = matches!(self.request, MutationRequest::Reserve { .. });
        let ttl = match self.request {
            MutationRequest::Reserve { ttl_ms, .. } | MutationRequest::Renew { ttl_ms, .. } => {
                ttl_ms
            }
            _ => return Err(RestoreError::UnsupportedDomainValue),
        };
        match &self.outcome {
            Err(error) if lease_error(&self.request, error) && *error != EngineError::NotFound => {
                let consistent = match error {
                    EngineError::Unauthorized | EngineError::InvalidLease => true,
                    EngineError::IllegalTransition { from, to } => {
                        *from == before && *to == TransactionState::Reserved && !before.allows(*to)
                    }
                    EngineError::InvalidTime => {
                        expiration(self.recorded_at_ms, ttl).is_err()
                            && (!is_reserve || before.allows(TransactionState::Reserved))
                    }
                    _ => {
                        before.allows(TransactionState::Reserved)
                            && expiration(self.recorded_at_ms, ttl).is_ok()
                    }
                };
                if consistent {
                    Ok(())
                } else {
                    Err(RestoreError::BindingMismatch)
                }
            }
            Err(_) => Err(RestoreError::UnsupportedDomainValue),
            Ok(MutationValue::Lease {
                expires_at_ms,
                fence,
                acquisition_attempt,
            }) if *expires_at_ms > self.recorded_at_ms
                && *fence != 0
                && self
                    .resolved_attempt
                    .is_some_and(|a| *acquisition_attempt <= a)
                && (!is_reserve || before.allows(TransactionState::Reserved)) =>
            {
                Ok(())
            }
            _ => Err(RestoreError::InvalidDomainValue),
        }
    }
    fn response(&self, scope: &Scope, actor: &MutationActor, replayed: bool) -> MutationResponse {
        MutationResponse {
            request: self.request.digest(scope, actor),
            recorded_at_ms: self.recorded_at_ms,
            outcome: self.outcome.clone(),
            replayed,
        }
    }
}

impl Engine {
    /// Apply once or return the original result after current authentication.
    /// A returned lease result describes history; normal lease checks still apply.
    pub fn mutate(
        &self,
        session: &Session,
        scope: &Scope,
        key: IdempotencyKey,
        request: MutationRequest,
    ) -> Result<MutationResponse, EngineError> {
        // A one-session dispatch cannot supply the separate approval role.
        if matches!(request, MutationRequest::SourceCommit(_)) {
            return Err(EngineError::Unauthorized);
        }
        let permission = request.permission().ok_or(EngineError::Unauthorized)?;
        self.store.session(session, scope, permission).atomic(|db| {
            if let Err(error) = self.authorize(db, session, scope, permission) {return Ok(Err(error));}
            if let MutationRequest::Repository(registered) = &request
                && let RepositoryMutation::Register {snapshot, ..} | RepositoryMutation::RegisterRepository {snapshot, ..} = registered.as_ref()
                && snapshot.scope() != scope {return Ok(Err(EngineError::Unauthorized));}
            if matches!(&request, MutationRequest::Repository(request) if matches!(request.as_ref(), RepositoryMutation::RegisterRepository {..}))
                && let Err(error) = self.authorize(db, session, scope, Permission::ManageModel) {return Ok(Err(error));}
            let actor = MutationActor::Session {principal: session.principal.clone(), grant: session.token};
            apply_once(db, scope, actor, key, request, None, |db, request| self.apply_session_mutation(db, session, scope, request))
        })?
    }
    fn apply_session_mutation(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        request: &MutationRequest,
    ) -> Result<MutationValue, EngineError> {
        match request {
            MutationRequest::SourceCommit(_) => Err(EngineError::Unauthorized),
            MutationRequest::Administrative(_) | MutationRequest::SourceExecutor(_) => {
                Err(EngineError::Unauthorized)
            }
            MutationRequest::Repository(request) => request.apply(self, db, session, scope),
            MutationRequest::SubmitPatch { transaction, patch } => self
                .submit_patch_in(db, session, scope, transaction, patch.clone())
                .map(|bindings| MutationValue::Bindings { bindings }),
            MutationRequest::RecordEvidence {
                transaction,
                evidence,
            } => self
                .record_evidence_in(db, session, scope, transaction, *evidence.clone())
                .map(|evidence| MutationValue::Evidence { evidence }),
            MutationRequest::Revalidate { transaction } => self
                .revalidate_in(db, session, scope, transaction)
                .map(|bindings| MutationValue::Bindings { bindings }),
            MutationRequest::Propose {
                transaction,
                base,
                intent,
            } => self
                .propose_in(
                    db,
                    session,
                    scope,
                    transaction.clone(),
                    *base,
                    *intent.clone(),
                )
                .map(|view| MutationValue::State {
                    state: view.state,
                    attempt: view.attempt,
                }),
            MutationRequest::Analyze { transaction } => self
                .analyze_in(db, session, scope, transaction)
                .and_then(|accesses| {
                    let tx = db
                        .transactions
                        .get(&(scope.clone(), transaction.clone()))
                        .ok_or(AuthorityFailure::Integrity)?;
                    Ok(MutationValue::Analysis {
                        attempt: tx.attempt().number,
                        accesses,
                    })
                }),
            MutationRequest::Reserve {
                transaction,
                ttl_ms,
            } => self
                .reserve_in(db, session, scope, transaction, *ttl_ms)
                .and_then(|expiry| lease_value(db, scope, transaction, expiry)),
            MutationRequest::Renew {
                transaction,
                ttl_ms,
            } => self
                .renew_in(db, session, scope, transaction, *ttl_ms)
                .and_then(|expiry| lease_value(db, scope, transaction, expiry)),
            MutationRequest::Execute { transaction } => self
                .execute_in(db, session, scope, transaction)
                .and_then(|()| state_value(db, scope, transaction)),
            MutationRequest::Abort {
                transaction,
                reason,
            } => self
                .abort_in(db, session, scope, transaction, reason.clone())
                .and_then(|()| state_value(db, scope, transaction)),
        }
    }

    /// Read only this authenticated actor's original outcome, without rerunning it.
    pub fn mutation_outcome(
        &self,
        session: &Session,
        scope: &Scope,
        key: &IdempotencyKey,
    ) -> Result<Option<MutationResponse>, EngineError> {
        self.store
            .session(session, scope, Permission::Read)
            .read(|db| {
                self.authorize(db, session, scope, Permission::Read)?;
                let actor = MutationActor::Session {
                    principal: session.principal.clone(),
                    grant: session.token,
                };
                Ok(db
                    .mutations
                    .get(&(
                        scope.clone(),
                        MutationKey {
                            actor: actor.clone(),
                            key: key.clone(),
                        },
                    ))
                    .map(|r| r.response(scope, &actor, true)))
            })
    }
}

/// One atomic journal path shared by separately authenticated actor entry points.
pub(super) fn apply_once(
    db: &mut Database,
    scope: &Scope,
    actor: MutationActor,
    key: IdempotencyKey,
    request: MutationRequest,
    source_commit: Option<Box<SourceCommitContext>>,
    execute: impl FnOnce(&mut Database, &MutationRequest) -> Result<MutationValue, EngineError>,
) -> Result<Result<MutationResponse, EngineError>, AuthorityFailure> {
    if let Err(error) = request.check_recovery_limits() {
        return Ok(Err(error));
    }
    let key = MutationKey { actor, key };
    let storage_key = (scope.clone(), key.clone());
    if let Some(original) = db.mutations.get(&storage_key) {
        if original.request != request {
            return Ok(Err(EngineError::IdempotencyConflict));
        }
        return Ok(Ok(original.response(scope, &key.actor, true)));
    }
    if db
        .mutations
        .keys()
        .filter(|(stored, _)| stored == scope)
        .count()
        >= authority::NativeRecordLimits::default().entries
    {
        return Err(AuthorityFailure::Integrity);
    }
    let original = request
        .transaction()
        .and_then(|tx| db.transactions.get(&(scope.clone(), tx.clone())));
    let resolved_attempt = original.map(|tx| tx.attempt().number);
    let state_before = original.map(|tx| tx.state);
    let lease_events_before = original.map_or(Ok(0), |tx| {
        u64::try_from(tx.lease_history.len()).map_err(|_| AuthorityFailure::Integrity)
    })?;
    let administration = if let MutationRequest::Administrative(request) = &request {
        Some(request.observe(db, scope))
    } else {
        None
    };
    let mut source_execution = if let MutationRequest::SourceExecutor(request) = &request {
        Some(request.observe(db, scope))
    } else {
        None
    };
    let outcome = match execute(db, &request) {
        Ok(value) => Ok(value),
        Err(error) if mutation_error(&request, &error) => Err(error),
        Err(_) => return Err(AuthorityFailure::Protocol),
    };
    if let MutationRequest::SourceExecutor(request) = &request
        && let Some(context) = &mut source_execution
    {
        request.observe_after(db, scope, context);
    }
    let record = MutationRecord {
        request,
        recorded_at_ms: db.now,
        resolved_attempt,
        state_before,
        lease_events_before,
        administration,
        source_execution,
        source_commit,
        outcome,
    };
    validate(db, scope, &key, &record).map_err(|_| AuthorityFailure::Integrity)?;
    let response = record.response(scope, &key.actor, false);
    db.mutations.insert(storage_key, record);
    Ok(Ok(response))
}

fn lease_error(request: &MutationRequest, error: &EngineError) -> bool {
    match error {
        EngineError::NotFound | EngineError::InvalidTime => true,
        EngineError::Unauthorized
        | EngineError::IllegalTransition { .. }
        | EngineError::IncompleteAnalysis
        | EngineError::ReservationConflict
        | EngineError::CounterExhausted => matches!(request, MutationRequest::Reserve { .. }),
        EngineError::InvalidLease => matches!(request, MutationRequest::Renew { .. }),
        _ => false,
    }
}

fn mutation_error(request: &MutationRequest, error: &EngineError) -> bool {
    match request {
        MutationRequest::SourceCommit(_) => source_commit::domain_error(error),
        MutationRequest::SourceExecutor(request) => request.domain_error(error),
        MutationRequest::Administrative(request) => request.domain_error(error),
        MutationRequest::Repository(request) => request.domain_error(error),
        MutationRequest::SubmitPatch { .. } => matches!(
            error,
            EngineError::NotFound
                | EngineError::SourceMutationUnsupported
                | EngineError::InvalidLease
                | EngineError::IllegalTransition { .. }
                | EngineError::WrongPatchBase
                | EngineError::UndeclaredWrite
                | EngineError::IncompleteAnalysis
                | EngineError::Patch(_)
                | EngineError::Invariant(_)
                | EngineError::CounterExhausted
        ),
        MutationRequest::RecordEvidence { .. } => matches!(
            error,
            EngineError::NotFound
                | EngineError::IllegalTransition { .. }
                | EngineError::EvidenceMismatch
                | EngineError::WrongPatchBase
                | EngineError::IncompleteAnalysis
        ),
        MutationRequest::Revalidate { .. } => matches!(
            error,
            EngineError::NotFound
                | EngineError::SourceMutationUnsupported
                | EngineError::InvalidLease
                | EngineError::IllegalTransition { .. }
                | EngineError::Conflict(_)
                | EngineError::ModelChanged
                | EngineError::WrongPatchBase
                | EngineError::Patch(_)
                | EngineError::Invariant(_)
                | EngineError::CounterExhausted
                | EngineError::IncompleteAnalysis
                | EngineError::RepositoryAnalysis(_)
        ),
        MutationRequest::Propose { .. } => matches!(
            error,
            EngineError::NotFound
                | EngineError::AlreadyExists
                | EngineError::UnsatisfiedDependency
                | EngineError::InvalidIntent(_)
        ),
        MutationRequest::Analyze { .. } => matches!(
            error,
            EngineError::NotFound
                | EngineError::IllegalTransition { .. }
                | EngineError::ModelChanged
                | EngineError::IncompleteAnalysis
                | EngineError::RepositoryAnalysis(_)
                | EngineError::SourceAttempt(SourceAttemptError::PolicyChanged)
        ),
        MutationRequest::Reserve { .. } | MutationRequest::Renew { .. } => {
            lease_error(request, error)
        }
        MutationRequest::Execute { .. } => matches!(
            error,
            EngineError::NotFound
                | EngineError::InvalidLease
                | EngineError::IllegalTransition { .. }
        ),
        MutationRequest::Abort { .. } => {
            matches!(
                error,
                EngineError::NotFound | EngineError::IllegalTransition { .. }
            ) || matches!(error,EngineError::InvalidIntent(reason) if reason=="abort needs reason")
        }
    }
}

fn check_proposal_shape(r: &MutationRecord) -> Result<(), crate::persistence::RestoreError> {
    use crate::persistence::RestoreError;
    let invalid = RestoreError::BindingMismatch;
    let MutationRequest::Propose { intent, .. } = &r.request else {
        return Err(invalid);
    };
    if let Err(error) = validate_intent(intent) {
        return if r.outcome == Err(error) {
            Ok(())
        } else {
            Err(invalid)
        };
    }
    match &r.outcome {
        Ok(MutationValue::State { state, attempt })
            if *state == r.state_before.unwrap_or(TransactionState::Proposed)
                && *attempt == r.resolved_attempt.unwrap_or(0) =>
        {
            Ok(())
        }
        Err(EngineError::AlreadyExists) if r.resolved_attempt.is_some() => Ok(()),
        Err(EngineError::NotFound) if r.resolved_attempt.is_none() => Ok(()),
        Err(EngineError::UnsatisfiedDependency)
            if r.resolved_attempt.is_none() && !intent.depends_on.is_empty() =>
        {
            Ok(())
        }
        _ => Err(invalid),
    }
}

fn proposal_size(
    intent: &TransactionIntent,
) -> Result<crate::persistence::RecoverySize, crate::persistence::RestoreError> {
    use crate::access::{Condition, Predicate};
    use crate::persistence::{RecoverySize, RestoreError};
    fn charge(size: &mut RecoverySize, entries: usize, bytes: usize) -> Result<(), RestoreError> {
        size.entries = size
            .entries
            .checked_add(entries)
            .ok_or(RestoreError::Limit)?;
        size.bytes = size.bytes.checked_add(bytes).ok_or(RestoreError::Limit)?;
        Ok(())
    }
    fn field(size: &mut RecoverySize, value: &str) -> Result<(), RestoreError> {
        charge(
            size,
            1,
            value.len().checked_add(16).ok_or(RestoreError::Limit)?,
        )
    }
    let mut size = RecoverySize {
        entries: 0,
        bytes: 512,
    };
    field(&mut size, &intent.objective)?;
    for set in [
        &intent.accesses.reads,
        &intent.accesses.writes,
        &intent.accesses.invariant_scopes,
    ] {
        for element in set {
            field(&mut size, element.as_str())?;
        }
    }
    for Predicate::Prefix(prefix) in &intent.accesses.predicates {
        field(&mut size, prefix)?;
    }
    for invariant in &intent.invariants {
        field(&mut size, invariant.id.as_str())?;
        charge(&mut size, 0, 32)?;
        match &invariant.condition {
            Condition::Equals { element, .. } | Condition::Present(element) => {
                field(&mut size, element.as_str())?
            }
            Condition::SumAtMost { elements, .. } => {
                for element in elements {
                    field(&mut size, element.as_str())?;
                }
            }
        }
    }
    for check in &intent.required_checks {
        charge(&mut size, 0, 128)?;
        field(&mut size, check.id().as_str())?;
        field(&mut size, check.toolchain())?;
        for argument in check.command() {
            field(&mut size, argument)?;
        }
    }
    for dependency in &intent.depends_on {
        field(&mut size, dependency.as_str())?;
    }
    Ok(size)
}

fn check_analysis_shape(r: &MutationRecord) -> Result<(), crate::persistence::RestoreError> {
    use crate::persistence::RestoreError;
    let invalid = RestoreError::BindingMismatch;
    let Some(before) = r.state_before else {
        return if r.outcome == Err(EngineError::NotFound) {
            Ok(())
        } else {
            Err(invalid)
        };
    };
    match &r.outcome {
        Ok(MutationValue::Analysis { attempt, .. })
            if before.allows(TransactionState::Analyzed)
                && r.resolved_attempt == Some(*attempt) =>
        {
            Ok(())
        }
        Err(EngineError::IllegalTransition { from, to })
            if *from == before && *to == TransactionState::Analyzed && !before.allows(*to) =>
        {
            Ok(())
        }
        Err(error)
            if before.allows(TransactionState::Analyzed)
                && mutation_error(&r.request, error)
                && !matches!(
                    error,
                    EngineError::NotFound | EngineError::IllegalTransition { .. }
                ) =>
        {
            Ok(())
        }
        _ => Err(invalid),
    }
}
fn lease_value(
    db: &Database,
    scope: &Scope,
    id: &TransactionId,
    expires_at_ms: u64,
) -> Result<MutationValue, EngineError> {
    let lease = db
        .transactions
        .get(&(scope.clone(), id.clone()))
        .and_then(|t| t.lease.as_ref())
        .ok_or(AuthorityFailure::Integrity)?;
    Ok(MutationValue::Lease {
        expires_at_ms,
        fence: lease.fence,
        acquisition_attempt: lease.acquired_attempt,
    })
}
fn state_value(
    db: &Database,
    scope: &Scope,
    id: &TransactionId,
) -> Result<MutationValue, EngineError> {
    let tx = db
        .transactions
        .get(&(scope.clone(), id.clone()))
        .ok_or(AuthorityFailure::Integrity)?;
    Ok(MutationValue::State {
        state: tx.state,
        attempt: tx.attempt().number,
    })
}
fn check_transition_shape(r: &MutationRecord) -> Result<(), crate::persistence::RestoreError> {
    use crate::persistence::RestoreError;
    let invalid = RestoreError::BindingMismatch;
    let (target, invalid_reason) = match &r.request {
        MutationRequest::Execute { .. } => (TransactionState::Executing, false),
        MutationRequest::Abort { reason, .. } => {
            (TransactionState::Aborted, reason.trim().is_empty())
        }
        _ => return Err(invalid),
    };
    if let Err(EngineError::InvalidIntent(reason)) = &r.outcome {
        return if invalid_reason && reason == "abort needs reason" {
            Ok(())
        } else {
            Err(invalid)
        };
    }
    if invalid_reason {
        return Err(invalid);
    }
    let Some(before) = r.state_before else {
        return if r.resolved_attempt.is_none()
            && r.lease_events_before == 0
            && r.outcome == Err(EngineError::NotFound)
        {
            Ok(())
        } else {
            Err(invalid)
        };
    };
    match &r.outcome {
        Ok(MutationValue::State { state, attempt })
            if *state == target
                && r.resolved_attempt == Some(*attempt)
                && (before.allows(target)
                    || (target == TransactionState::Aborted && before == target)) =>
        {
            Ok(())
        }
        Err(EngineError::InvalidLease) if target == TransactionState::Executing => Ok(()),
        Err(EngineError::IllegalTransition { from, to })
            if *from == before && *to == target && !before.allows(target) =>
        {
            Ok(())
        }
        _ => Err(invalid),
    }
}

pub(crate) fn recovery_size(
    scope: &Scope,
    key: &MutationKey,
    record: &MutationRecord,
) -> Result<crate::persistence::RecoverySize, crate::persistence::RestoreError> {
    use crate::persistence::{RecoverySize, RestoreError};
    record.check_shape(&key.actor)?;
    // Both canonical records repeat scope/actor identity. This nonallocating
    // bound also covers fixed-width fields, tags and native transition errors.
    let mut bytes = 1024usize;
    for text in [
        scope.organization.as_str(),
        scope.workspace.as_str(),
        scope.repository.as_str(),
    ] {
        bytes = bytes
            .checked_add(text.len().checked_mul(2).ok_or(RestoreError::Limit)?)
            .ok_or(RestoreError::Limit)?;
    }
    let principal = match &key.actor {
        MutationActor::EmbeddingAdministrator => "",
        MutationActor::Session { principal, .. }
        | MutationActor::SourceExecutor { principal, .. } => principal.as_str(),
    };
    for count in [
        principal.len(),
        principal.len(),
        key.key.as_str().len(),
        record
            .request
            .transaction()
            .map_or(0, |tx| tx.as_str().len()),
    ] {
        bytes = bytes.checked_add(count).ok_or(RestoreError::Limit)?;
    }
    if let MutationRequest::Abort { reason, .. } = &record.request {
        bytes = bytes.checked_add(reason.len()).ok_or(RestoreError::Limit)?;
    }
    let mut entries = 1usize;
    if let Some(context) = &record.source_commit {
        bytes = bytes
            .checked_add(8192)
            .and_then(|n| n.checked_add(context.analysis_principal.as_str().len()))
            .ok_or(RestoreError::Limit)?;
        if let Some(source) = &context.source {
            bytes = bytes
                .checked_add(source.source_attempt.as_str().len())
                .ok_or(RestoreError::Limit)?;
        }
    }
    let input_size = match &record.request {
        MutationRequest::SourceCommit(request) => Some(request.recovery_size()?),
        MutationRequest::SourceExecutor(request) => Some(request.recovery_size()?),
        MutationRequest::Administrative(request) => Some(request.recovery_size()?),
        MutationRequest::Repository(request) => Some(request.recovery_size()?),
        MutationRequest::SubmitPatch { patch, .. } => Some(patch_size(patch)?),
        MutationRequest::RecordEvidence { evidence, .. } => Some(evidence_size(evidence)?),
        _ => None,
    };
    if let Some(size) = input_size {
        bytes = bytes.checked_add(size.bytes).ok_or(RestoreError::Limit)?;
        entries = entries
            .checked_add(size.entries)
            .ok_or(RestoreError::Limit)?;
    }
    if let Ok(MutationValue::SourceAttempt(value)) = &record.outcome {
        let size = value.recovery_size()?;
        bytes = bytes.checked_add(size.bytes).ok_or(RestoreError::Limit)?;
        entries = entries
            .checked_add(size.entries)
            .ok_or(RestoreError::Limit)?;
    }
    if let Ok(MutationValue::Repository { model, .. }) = &record.outcome {
        let size = repository::model_size(model)?;
        bytes = bytes.checked_add(size.bytes).ok_or(RestoreError::Limit)?;
        entries = entries
            .checked_add(size.entries)
            .ok_or(RestoreError::Limit)?;
    }
    if let MutationRequest::Propose { intent, .. } = &record.request {
        let size = proposal_size(intent)?;
        bytes = bytes.checked_add(size.bytes).ok_or(RestoreError::Limit)?;
        entries = entries
            .checked_add(size.entries)
            .ok_or(RestoreError::Limit)?;
    }
    if let Ok(MutationValue::Analysis { accesses, .. }) = &record.outcome {
        for set in [
            &accesses.reads,
            &accesses.writes,
            &accesses.invariant_scopes,
        ] {
            entries = entries.checked_add(set.len()).ok_or(RestoreError::Limit)?;
            for element in set {
                bytes = bytes
                    .checked_add(element.as_str().len())
                    .and_then(|n| n.checked_add(8))
                    .ok_or(RestoreError::Limit)?;
            }
        }
        entries = entries
            .checked_add(accesses.predicates.len())
            .ok_or(RestoreError::Limit)?;
        for crate::access::Predicate::Prefix(prefix) in &accesses.predicates {
            bytes = bytes
                .checked_add(prefix.len())
                .and_then(|n| n.checked_add(16))
                .ok_or(RestoreError::Limit)?;
        }
    }
    if let Err(EngineError::RepositoryAnalysis(RepositoryAnalysisError::UnknownElement(element))) =
        &record.outcome
    {
        bytes = bytes
            .checked_add(element.as_str().len())
            .ok_or(RestoreError::Limit)?;
    }
    if let Err(error) = &record.outcome {
        let size = execution_error_size(error)?;
        bytes = bytes.checked_add(size.bytes).ok_or(RestoreError::Limit)?;
        entries = entries
            .checked_add(size.entries)
            .ok_or(RestoreError::Limit)?;
    }
    Ok(RecoverySize { entries, bytes })
}

pub(super) fn restore(
    db: &mut Database,
    scope: &Scope,
    records: BTreeMap<MutationKey, MutationRecord>,
) -> Result<(), crate::persistence::RestoreError> {
    let mut effects = BTreeSet::new();
    let mut analysis_effects = BTreeSet::new();
    let mut proposals = BTreeSet::new();
    let mut revalidations = BTreeSet::new();
    let mut patches = BTreeSet::new();
    let mut allocations = BTreeSet::new();
    let mut source_events = BTreeSet::new();
    let mut models = BTreeSet::new();
    for ((stored, _), r) in &db.mutations {
        if stored == scope && matches!(r.outcome, Ok(MutationValue::Lease { .. })) {
            effects.insert((r.request.transaction().cloned(), r.lease_events_before));
        }
        if stored == scope
            && let Ok(MutationValue::Analysis { attempt, .. }) = &r.outcome
        {
            analysis_effects.insert((r.request.transaction().cloned(), *attempt));
        }
    }
    for (key, record) in records {
        validate(db, scope, &key, &record)?;
        if let MutationRequest::SourceExecutor(request) = &record.request
            && let Some(context) = &record.source_execution
            && context.events_after > context.events_before
            && !source_events.insert((
                request.binding.source_attempt.clone(),
                context.events_before,
            ))
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if let Ok(MutationValue::Allocation { resource, id }) = &record.outcome
            && !allocations.insert((*resource, *id))
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if matches!(&record.request, MutationRequest::Repository(request) if matches!(request.as_ref(), RepositoryMutation::Register {..} | RepositoryMutation::UpdateModel {..}))
            && let Ok(MutationValue::Repository { model, .. }) = &record.outcome
            && !models.insert(model.generation)
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if matches!(record.request, MutationRequest::SubmitPatch { .. })
            && let Ok(MutationValue::Bindings { bindings }) = &record.outcome
            && !patches.insert((record.request.transaction().cloned(), bindings.attempt))
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if matches!(record.request, MutationRequest::Revalidate { .. })
            && let Ok(MutationValue::Bindings { bindings }) = &record.outcome
            && !revalidations.insert((record.request.transaction().cloned(), bindings.attempt))
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if matches!(record.request, MutationRequest::Propose { .. })
            && record.resolved_attempt.is_none()
            && record.outcome.is_ok()
            && !proposals.insert(record.request.transaction().cloned())
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if let Ok(MutationValue::Analysis { attempt, .. }) = &record.outcome
            && !analysis_effects.insert((record.request.transaction().cloned(), *attempt))
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if matches!(record.outcome, Ok(MutationValue::Lease { .. }))
            && !effects.insert((
                record.request.transaction().cloned(),
                record.lease_events_before,
            ))
        {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
        if db.mutations.insert((scope.clone(), key), record).is_some() {
            return Err(crate::persistence::RestoreError::NonCanonical);
        }
    }
    Ok(())
}

pub(super) fn validate(
    db: &Database,
    scope: &Scope,
    key: &MutationKey,
    r: &MutationRecord,
) -> Result<(), crate::persistence::RestoreError> {
    use crate::persistence::RestoreError;
    r.check_shape(&key.actor)?;
    let invalid = RestoreError::BindingMismatch;
    if let MutationRequest::SourceExecutor(request) = &r.request {
        return request.validate(db, scope, key, r);
    }
    if let MutationRequest::SourceCommit(request) = &r.request {
        return source_commit::validate(db, scope, key, request, r);
    }
    if let MutationRequest::Administrative(request) = &r.request {
        if key.actor != MutationActor::EmbeddingAdministrator || r.recorded_at_ms > db.now {
            return Err(invalid);
        }
        return request.validate(db, scope, r);
    }
    let MutationActor::Session { principal, grant } = &key.actor else {
        return Err(invalid);
    };
    let actor = db.grants.get(grant).ok_or(invalid.clone())?;
    if *grant == 0
        || actor.scope != *scope
        || actor.principal != *principal
        || !actor
            .permissions
            .contains(&r.request.permission().ok_or(invalid.clone())?)
        || r.recorded_at_ms > db.now
        || r.resolved_attempt.is_some() != r.state_before.is_some()
    {
        return Err(invalid);
    }
    if let MutationRequest::Repository(request) = &r.request {
        if matches!(
            request.as_ref(),
            RepositoryMutation::RegisterRepository { .. }
        ) && !actor.permissions.contains(&Permission::ManageModel)
        {
            return Err(invalid);
        }
        return request.validate(db, scope, r);
    }
    let transaction = r.request.transaction().ok_or(invalid.clone())?;
    let tx = db.transactions.get(&(scope.clone(), transaction.clone()));
    if let Some(attempt) = r.resolved_attempt {
        let tx = tx.ok_or(invalid.clone())?;
        if attempt > tx.attempt().number
            || r.lease_events_before > tx.lease_history.len() as u64
            || r.lease_events_before
                .checked_sub(1)
                .is_some_and(|sequence| tx.lease_history[sequence as usize].at > r.recorded_at_ms)
        {
            return Err(invalid);
        }
    } else if r.lease_events_before != 0 {
        return Err(invalid);
    }
    if matches!(
        r.request,
        MutationRequest::Execute { .. } | MutationRequest::Abort { .. }
    ) {
        return validate_transition(tx, grant, r);
    }
    if matches!(
        r.request,
        MutationRequest::SubmitPatch { .. }
            | MutationRequest::RecordEvidence { .. }
            | MutationRequest::Revalidate { .. }
    ) {
        return validate_execution(db, scope, tx, principal, *grant, r);
    }
    if let MutationRequest::Propose { base, intent, .. } = &r.request {
        return match &r.outcome {
            Ok(MutationValue::State { .. }) => {
                let tx = tx.ok_or(invalid.clone())?;
                if tx.intent != **intent
                    || tx.attempts[0].base.id() != *base
                    || tx.owner != *principal
                    || (r.resolved_attempt.is_none() && tx.owner_capability != *grant)
                {
                    Err(invalid)
                } else {
                    Ok(())
                }
            }
            Err(EngineError::AlreadyExists) => {
                let tx = tx.ok_or(invalid.clone())?;
                if tx.intent == **intent
                    && tx.attempts[0].base.id() == *base
                    && tx.owner == *principal
                {
                    Err(invalid)
                } else {
                    Ok(())
                }
            }
            Err(_) => check_proposal_shape(r),
            _ => Err(invalid),
        };
    }
    if matches!(r.request, MutationRequest::Analyze { .. }) {
        if let Ok(MutationValue::Analysis { attempt, accesses }) = &r.outcome {
            let tx = tx.ok_or(invalid.clone())?;
            let original = tx
                .attempts
                .get(usize::try_from(*attempt).map_err(|_| RestoreError::Limit)?)
                .ok_or(invalid.clone())?;
            if original.effective.as_ref() != Some(accesses) || original.analysis.is_none() {
                return Err(invalid);
            }
        }
        return check_analysis_shape(r);
    }
    match &r.outcome {
        Err(error) if lease_error(&r.request, error) => {
            if *error == EngineError::NotFound {
                return if r.resolved_attempt.is_none() {
                    Ok(())
                } else {
                    Err(invalid)
                };
            }
            let tx = tx.ok_or(invalid.clone())?;
            let before = r.state_before.ok_or(invalid.clone())?;
            if matches!(r.request, MutationRequest::Reserve { .. })
                && *error != EngineError::Unauthorized
                && tx.owner != *principal
            {
                return Err(invalid);
            }
            let original_lease_active = r
                .lease_events_before
                .checked_sub(1)
                .and_then(|sequence| tx.lease_history.get(sequence as usize))
                .is_some_and(|event| {
                    let expiry = match event.kind {
                        LeaseEventKind::Acquired { expires_at }
                        | LeaseEventKind::Renewed { expires_at, .. } => expires_at,
                        _ => return false,
                    };
                    event.authorization_grant == *grant && expiry > r.recorded_at_ms
                });
            match error {
                EngineError::Unauthorized if tx.owner != *principal => Ok(()),
                EngineError::IllegalTransition { from, to }
                    if *from == before
                        && *to == TransactionState::Reserved
                        && !before.allows(TransactionState::Reserved) =>
                {
                    Ok(())
                }
                EngineError::InvalidTime => {
                    let ttl = match &r.request {
                        MutationRequest::Reserve { ttl_ms, .. }
                        | MutationRequest::Renew { ttl_ms, .. } => *ttl_ms,
                        _ => return Err(invalid),
                    };
                    let preceding_checks = match r.request {
                        MutationRequest::Reserve { .. } => {
                            before.allows(TransactionState::Reserved)
                        }
                        MutationRequest::Renew { .. } => original_lease_active,
                        _ => return Err(invalid),
                    };
                    if preceding_checks && expiration(r.recorded_at_ms, ttl).is_err() {
                        Ok(())
                    } else {
                        Err(invalid)
                    }
                }
                EngineError::CounterExhausted
                    if db.reservation_fences.get(scope) == Some(&u64::MAX)
                        && before.allows(TransactionState::Reserved) =>
                {
                    Ok(())
                }
                EngineError::ReservationConflict | EngineError::IncompleteAnalysis
                    if before.allows(TransactionState::Reserved) =>
                {
                    Ok(())
                }
                EngineError::InvalidLease if !original_lease_active => Ok(()),
                _ => Err(invalid),
            }
        }
        Err(_) => Err(invalid),
        Ok(
            MutationValue::SourceCommitted(_)
            | MutationValue::SourceAttempt(_)
            | MutationValue::State { .. }
            | MutationValue::Repository { .. }
            | MutationValue::Policy { .. }
            | MutationValue::Allocation { .. }
            | MutationValue::Revocation { .. },
        ) => Err(invalid),
        Ok(MutationValue::Bindings { .. } | MutationValue::Evidence { .. }) => Err(invalid),
        Ok(MutationValue::Analysis { .. }) => Err(invalid),
        Ok(MutationValue::Lease {
            expires_at_ms,
            fence,
            acquisition_attempt,
        }) => {
            let tx = tx.ok_or(invalid.clone())?;
            let sequence =
                usize::try_from(r.lease_events_before).map_err(|_| RestoreError::Limit)?;
            let event = tx.lease_history.get(sequence).ok_or(invalid.clone())?;
            if event.at != r.recorded_at_ms
                || event.fence != *fence
                || event.authorization_grant != *grant
                || event.owner != *principal
                || event.acquisition_attempt != *acquisition_attempt
                || *expires_at_ms <= event.at
            {
                return Err(invalid);
            }
            let ttl = match &r.request {
                MutationRequest::Reserve { ttl_ms, .. } | MutationRequest::Renew { ttl_ms, .. } => {
                    *ttl_ms
                }
                _ => return Err(invalid),
            };
            let expiry = expiration(event.at, ttl).map_err(|_| invalid.clone())?;
            match (&r.request, &event.kind) {
                (MutationRequest::Reserve { .. }, LeaseEventKind::Acquired { expires_at })
                    if *expires_at == expiry
                        && *expires_at == *expires_at_ms
                        && r.resolved_attempt == Some(*acquisition_attempt)
                        && tx.owner == *principal
                        && r.state_before
                            .is_some_and(|s| s.allows(TransactionState::Reserved)) =>
                {
                    Ok(())
                }
                (
                    MutationRequest::Renew { .. },
                    LeaseEventKind::Renewed {
                        previous_expiry,
                        expires_at,
                    },
                ) if *previous_expiry > event.at
                    && *expires_at == expiry.max(*previous_expiry)
                    && *expires_at == *expires_at_ms =>
                {
                    Ok(())
                }
                _ => Err(invalid),
            }
        }
    }
}

fn validate_transition(
    tx: Option<&Transaction>,
    grant: &u64,
    r: &MutationRecord,
) -> Result<(), crate::persistence::RestoreError> {
    use crate::persistence::RestoreError;
    let invalid = RestoreError::BindingMismatch;
    if matches!(r.outcome, Err(EngineError::InvalidIntent(_))) {
        return check_transition_shape(r);
    }
    if r.resolved_attempt.is_none() {
        return check_transition_shape(r);
    }
    let tx = tx.ok_or(invalid.clone())?;
    let sequence = usize::try_from(r.lease_events_before).map_err(|_| RestoreError::Limit)?;
    let previous = sequence
        .checked_sub(1)
        .and_then(|n| tx.lease_history.get(n));
    let original_lease = previous.and_then(|event| match event.kind {
        LeaseEventKind::Acquired { expires_at } | LeaseEventKind::Renewed { expires_at, .. } => {
            Some((event, expires_at))
        }
        _ => None,
    });
    let active = original_lease.is_some_and(|(event, expiry)| {
        event.authorization_grant == *grant && expiry > r.recorded_at_ms
    });
    match (&r.request, &r.outcome) {
        (MutationRequest::Execute { .. }, Ok(MutationValue::State { .. }))
            if active
                && !matches!(
                    tx.state,
                    TransactionState::Proposed
                        | TransactionState::Analyzed
                        | TransactionState::Reserved
                ) =>
        {
            Ok(())
        }
        (MutationRequest::Execute { .. }, Err(EngineError::InvalidLease)) if !active => Ok(()),
        (MutationRequest::Execute { .. }, Err(EngineError::IllegalTransition { .. })) if active => {
            Ok(())
        }
        (MutationRequest::Abort { reason, .. }, Ok(MutationValue::State { .. }))
            if tx.state == TransactionState::Aborted && tx.last_reason.as_ref() == Some(reason) =>
        {
            let had_lease =
                r.state_before != Some(TransactionState::Aborted) && original_lease.is_some();
            if had_lease {
                let event = tx.lease_history.get(sequence).ok_or(invalid.clone())?;
                let (old, _) = original_lease.ok_or(invalid.clone())?;
                if event.at != r.recorded_at_ms
                    || event.fence != old.fence
                    || event.authorization_grant != old.authorization_grant
                    || event.acquisition_attempt != old.acquisition_attempt
                    || event.owner != tx.owner
                    || !matches!(&event.kind,LeaseEventKind::Released {reason:actual} if actual==reason)
                {
                    return Err(invalid);
                }
            } else if tx.lease_history.len() != sequence {
                return Err(invalid);
            }
            Ok(())
        }
        (MutationRequest::Abort { reason, .. }, Err(EngineError::IllegalTransition { .. }))
            if r.state_before != Some(TransactionState::Aborted)
                || tx.last_reason.as_ref() != Some(reason) =>
        {
            Ok(())
        }
        _ => Err(invalid),
    }
}
