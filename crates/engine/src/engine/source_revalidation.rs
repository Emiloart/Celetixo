//! Fresh source revalidation discards prior execution eligibility. This does not
//! adapt a patch, reuse evidence or declare a transaction committable.
use super::*;

mod publication;
mod receipt;
mod selection;
pub(super) use publication::{apply, observe};
pub(super) use selection::select;

/// Original reservation observation, checked against its immutable event prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLeaseObservation {
    pub event_count: u64,
    pub grant_active: Option<bool>,
}

/// Independently observed metadata for checking an original decision. This
/// cannot construct a caller capability or certify a new revalidation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRevalidationContext {
    pub head: Option<SnapshotId>,
    pub model_generation: Option<u64>,
    pub analysis: Option<Digest>,
    pub policy: Option<Digest>,
    pub reservation_fence_before: u64,
    pub leases: BTreeMap<TransactionId, SourceLeaseObservation>,
    pub source_event_counts: BTreeMap<SourceAttemptId, u64>,
}

/// Exact optimistic selectors. Constructing or decoding them grants no authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRevalidationRequest {
    pub scope: Scope,
    pub principal: PrincipalId,
    pub authorization_grant: u64,
    pub transaction: TransactionId,
    pub previous_attempt: u64,
    pub previous_base: SnapshotId,
    pub previous_model_generation: u64,
    pub previous_impact: Digest,
    pub intent: Digest,
    pub fence: u64,
    pub head: SnapshotId,
    pub revision: Revision,
    pub model_generation: u64,
    pub analysis: Digest,
    pub index_generation: Digest,
    pub policy: Digest,
    pub policy_generation: u64,
    pub ttl_ms: u64,
    pub key: IdempotencyKey,
}

/// Historical decision metadata, never a Session, live lease or launch permit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRevalidationReceipt {
    pub scope: Scope,
    pub transaction: TransactionId,
    pub previous_attempt: u64,
    pub attempt: u64,
    pub previous_base: SnapshotId,
    pub base: SnapshotId,
    pub revision: Revision,
    pub model_generation: u64,
    pub analysis: Digest,
    pub index_generation: Digest,
    pub policy: Digest,
    pub policy_generation: u64,
    pub previous_fence: u64,
    pub fence: u64,
    pub expires_at_ms: u64,
    pub accesses: AccessSet,
    pub impact: Digest,
    pub invalidated_source_attempts: BTreeSet<SourceAttemptId>,
    pub intervening_revisions: Vec<Revision>,
    pub request: Digest,
    pub revalidated_at_ms: u64,
}

impl SourceRevalidationRequest {
    pub const FORMAT: &str = "celetixo/source-revalidation-request/v1";
    pub fn digest(&self, scope: &Scope, principal: &PrincipalId) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes(scope, principal))
    }
    pub fn canonical_bytes(&self, scope: &Scope, principal: &PrincipalId) -> Vec<u8> {
        let mut c = Canonical::default();
        for value in [
            scope.organization.as_str(),
            scope.workspace.as_str(),
            scope.repository.as_str(),
            principal.as_str(),
        ] {
            c.str(value);
        }
        self.encode_body(&mut c);
        c.into_bytes()
    }
    pub(crate) fn encode_body(&self, c: &mut Canonical) {
        for value in [
            self.scope.organization.as_str(),
            self.scope.workspace.as_str(),
            self.scope.repository.as_str(),
            self.principal.as_str(),
        ] {
            c.str(value);
        }
        c.u64(self.authorization_grant);
        c.str(self.transaction.as_str());
        c.u64(self.previous_attempt);
        c.digest(self.previous_base.0);
        c.u64(self.previous_model_generation);
        c.digest(self.previous_impact);
        c.digest(self.intent);
        c.u64(self.fence);
        c.digest(self.head.0);
        c.u64(self.revision.0);
        c.u64(self.model_generation);
        c.digest(self.analysis);
        c.digest(self.index_generation);
        c.digest(self.policy);
        c.u64(self.policy_generation);
        c.u64(self.ttl_ms);
    }
}

impl Engine {
    /// Current authoritative analysis is already approved through model/policy
    /// installation. This preview neither creates an attempt nor renews a lease.
    pub fn prepare_source_revalidation_request(
        &self,
        owner: &Session,
        scope: &Scope,
        transaction: &TransactionId,
        ttl_ms: u64,
        key: IdempotencyKey,
    ) -> Result<SourceRevalidationRequest, EngineError> {
        self.store
            .session(owner, scope, Permission::Analyze)
            .read(|db| {
                for permission in [
                    Permission::Analyze,
                    Permission::Reserve,
                    Permission::Execute,
                ] {
                    self.authorize(db, owner, scope, permission)?;
                }
                let tx = db
                    .transactions
                    .get(&(scope.clone(), transaction.clone()))
                    .ok_or(EngineError::NotFound)?;
                require_lease(db, tx, owner)?;
                let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
                let analysis = repo
                    .source_analysis
                    .as_ref()
                    .ok_or(SourceAttemptError::SourceRepositoryRequired)?;
                let policy = db
                    .source_policies
                    .get(scope)
                    .ok_or(SourceAttemptError::PolicyChanged)?;
                let request = SourceRevalidationRequest {
                    scope: scope.clone(),
                    principal: owner.principal.clone(),
                    authorization_grant: owner.token,
                    transaction: transaction.clone(),
                    previous_attempt: tx.attempt().number,
                    previous_base: tx.attempt().base.id(),
                    previous_model_generation: tx.attempt().model.generation,
                    previous_impact: tx
                        .attempt()
                        .analysis
                        .as_ref()
                        .ok_or(EngineError::IncompleteAnalysis)?
                        .digest(),
                    intent: source_attempts::intent_digest(&tx.intent),
                    fence: tx.lease.as_ref().ok_or(EngineError::InvalidLease)?.fence,
                    head: repo.current.id(),
                    revision: repo.current.revision(),
                    model_generation: repo.model.generation,
                    analysis: analysis.digest(),
                    index_generation: analysis.index_generation(),
                    policy: policy.digest(),
                    policy_generation: policy.input().generation,
                    ttl_ms,
                    key,
                };
                let actor = MutationActor::Session {
                    principal: owner.principal.clone(),
                    grant: owner.token,
                };
                selection::select(db, scope, &actor, &request)?;
                Ok(request)
            })
    }
}

impl Engine {
    pub fn revalidate_source_prepared(
        &self,
        owner: &Session,
        scope: &Scope,
        request: SourceRevalidationRequest,
    ) -> Result<SourceRevalidationReceipt, EngineError> {
        self.revalidate_source_prepared_with_fault(owner, scope, request, CommitFault::None)
    }
    /// Failure injection uses the same atomic stage as the original-outcome
    /// journal. A recovered receipt describes history, never a renewed lease.
    pub fn revalidate_source_prepared_with_fault(
        &self,
        owner: &Session,
        scope: &Scope,
        request: SourceRevalidationRequest,
        fault: CommitFault,
    ) -> Result<SourceRevalidationReceipt, EngineError> {
        let key = request.key.clone();
        let request = MutationRequest::SourceRevalidate(Box::new(request));
        request.check_recovery_limits()?;
        // Exact recovery authenticates the current original actor, then returns
        // before an update, clock publication, new fence or source execution.
        if let Some(original) =
            self.store
                .session(owner, scope, Permission::Analyze)
                .read(|db| {
                    self.authorize(db, owner, scope, Permission::Analyze)?;
                    let actor = MutationActor::Session {
                        principal: owner.principal.clone(),
                        grant: owner.token,
                    };
                    let lookup = (
                        scope.clone(),
                        MutationKey {
                            actor,
                            key: key.clone(),
                        },
                    );
                    if let Some(record) = db.mutations.get(&lookup) {
                        if record.request != request {
                            return Err(EngineError::IdempotencyConflict);
                        }
                        return Ok(Some(revalidation_result(record.outcome.clone())));
                    }
                    Ok(None)
                })?
        {
            return original;
        }
        self.store
            .session(owner, scope, Permission::Analyze)
            .atomic(|db| {
                if let Err(error) = self.authorize(db, owner, scope, Permission::Analyze) {
                    return Ok(Err(error));
                }
                let actor = MutationActor::Session {
                    principal: owner.principal.clone(),
                    grant: owner.token,
                };
                let lookup = (
                    scope.clone(),
                    MutationKey {
                        actor: actor.clone(),
                        key: key.clone(),
                    },
                );
                if let Some(record) = db.mutations.get(&lookup) {
                    if record.request != request {
                        return Ok(Err(EngineError::IdempotencyConflict));
                    }
                    return Ok(revalidation_result(record.outcome.clone()));
                }
                if matches!(
                    fault,
                    CommitFault::BeforeValidation
                        | CommitFault::DuringValidation
                        | CommitFault::BeforePublication
                ) {
                    return Ok(Err(EngineError::InjectedFailure(fault)));
                }
                let mut staged = db.clone();
                let response = mutation::apply_once(
                    &mut staged,
                    scope,
                    actor.clone(),
                    key.clone(),
                    request.clone(),
                    None,
                    |db, request| {
                        let MutationRequest::SourceRevalidate(request) = request else {
                            return Err(EngineError::Unauthorized);
                        };
                        apply(db, scope, &actor, request)
                            .map(|r| MutationValue::SourceRevalidated(Box::new(r)))
                    },
                )?;
                let response = match response {
                    Ok(response) => response,
                    Err(error) => return Ok(Err(error)),
                };
                if response.outcome.is_ok() && fault == CommitFault::DuringPublication {
                    return Ok(Err(EngineError::InjectedFailure(fault)));
                }
                let result = revalidation_result(response.outcome);
                *db = staged;
                if result.is_ok() && fault == CommitFault::AfterPublication {
                    return Ok(Err(EngineError::ResponseLost));
                }
                Ok(result)
            })?
    }
}
fn revalidation_result(
    result: Result<MutationValue, EngineError>,
) -> Result<SourceRevalidationReceipt, EngineError> {
    match result {
        Ok(MutationValue::SourceRevalidated(receipt)) => Ok(*receipt),
        Err(error) => Err(error),
        _ => Err(authority::AuthorityFailure::Integrity.into()),
    }
}
