//! Engine-owned, process-local source execution admission and attempt records.
//!
//! A pure candidate is replayed before issuance. No method in this module runs
//! repository code or publishes source. Trusted executor capabilities are
//! separate from ordinary transaction/verifier sessions.

use super::*;
use crate::access::{Completeness, Condition, ImpactAnalysis};
use crate::credentials::CredentialError;
use crate::source_execution::{
    BackendIdentity, ExecutionTier, PlanRequirements, ResourceLimits, SourceExecutionPolicy,
    SourcePlanBinding, SourcePlanError, VerificationPlan,
};
use crate::source_patch::{AppliedSourcePatch, PatchBinding, SourcePatch, SourcePatchError};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceAttemptError {
    #[error("source execution policy is missing, stale or changed")]
    PolicyChanged,
    #[error("source admission or executor expired")]
    Expired,
    #[error("source patch binding does not match Engine-derived context")]
    BindingMismatch,
    #[error("source attempt has an invalid state for this action")]
    InvalidState,
    #[error("another source attempt is active for this transaction")]
    AttemptActive,
    #[error("source attempt identity is already bound to different work")]
    AttemptIdentity,
    #[error("source attempt is already claimed by an executor")]
    AlreadyClaimed,
    #[error("executor is not trusted for this exact scope, backend or resource policy")]
    UntrustedExecutor,
    #[error("numeric reference invariants cannot establish source invariants")]
    UnsupportedInvariant,
    #[error("source policy contains an unknown semantic write or invariant identity")]
    UnknownPolicyElement,
    #[error("source execution requires a scoped source repository")]
    SourceRepositoryRequired,
    #[error("source attempt needs a bounded, nonempty failure reason")]
    InvalidReason,
    #[error("source attempt options exceed the supported check budget")]
    InvalidOptions,
    #[error("source planning failed: {0}")]
    Plan(#[from] SourcePlanError),
    #[error("source patch application failed: {0}")]
    Patch(#[from] SourcePatchError),
    #[error("candidate credential admission rejected: {0}")]
    Credential(#[from] CredentialError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAttemptState {
    Proposed,
    Admitted,
    Patching,
    PatchApplied,
    Verifying,
    EvidenceSealed,
    Rejected,
    PatchConflict,
    ExecutionFailed,
    TimedOut,
    Cancelled,
    EvidenceIncomplete,
    RevalidationRequired,
}

impl SourceAttemptState {
    pub fn is_terminal(self) -> bool {
        !matches!(
            self,
            Self::Proposed | Self::Admitted | Self::Patching | Self::PatchApplied | Self::Verifying
        )
    }

    pub fn allows(self, next: Self) -> bool {
        use SourceAttemptState::*;
        if self.is_terminal() {
            return false;
        }
        matches!(
            (self, next),
            (Proposed, Admitted | Rejected)
                | (Admitted, Patching | Rejected)
                | (Patching, PatchApplied | PatchConflict | ExecutionFailed)
                | (PatchApplied, Verifying | ExecutionFailed)
                | (
                    Verifying,
                    EvidenceSealed | ExecutionFailed | EvidenceIncomplete
                )
        ) || matches!(next, TimedOut | Cancelled | RevalidationRequired)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAttemptEvent {
    pub sequence: u64,
    pub at_ms: u64,
    pub from: SourceAttemptState,
    pub to: SourceAttemptState,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthorizationProvenance {
    pub principal: PrincipalId,
    /// Scoped opaque attribution to the active in-process Engine capability.
    /// This digest is provenance, not an independently usable capability.
    pub capability_reference: Digest,
}

/// Optional selection among installed policy-owned definitions. This cannot
/// remove mandatory checks, alter definitions or widen execution grants.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceAttemptOptions {
    /// At most 4096 unique IDs from the installed registry. Structurally
    /// oversized options are rejected before an idempotency key is retained.
    pub optional_checks: BTreeSet<CheckId>,
}

impl SourceAttemptOptions {
    fn validate(&self) -> Result<(), SourceAttemptError> {
        if self.optional_checks.len() > 4096 {
            return Err(SourceAttemptError::InvalidOptions);
        }
        Ok(())
    }

    fn request_digest(&self, patch: SourcePatchId) -> Digest {
        // Preserve the published empty-options operation identity. Nonempty
        // selections use a separate domain and bind every sorted CheckId.
        if self.optional_checks.is_empty() {
            return patch.0;
        }
        let mut c = Canonical::default();
        c.digest(patch.0);
        c.u64(self.optional_checks.len() as u64);
        for id in &self.optional_checks {
            c.str(id.as_str());
        }
        c.finish("celetixo/source-attempt-request/v1")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAdmission {
    id: SourceAdmissionId,
    binding: PatchBinding,
    patch: SourcePatchId,
    operation_digest: Digest,
    resulting_snapshot: SnapshotId,
    resulting_manifest: Digest,
    application_report: Digest,
    policy: Digest,
    policy_generation: u64,
    plan: VerificationPlan,
    authorization: SourceAuthorizationProvenance,
    admitted_at_ms: u64,
    expires_at_ms: u64,
    retention_policy: Digest,
}

impl SourceAdmission {
    pub fn id(&self) -> SourceAdmissionId {
        self.id
    }
    pub fn binding(&self) -> &PatchBinding {
        &self.binding
    }
    pub fn patch(&self) -> SourcePatchId {
        self.patch
    }
    pub fn operation_digest(&self) -> Digest {
        self.operation_digest
    }
    pub fn resulting_snapshot(&self) -> SnapshotId {
        self.resulting_snapshot
    }
    pub fn resulting_manifest(&self) -> Digest {
        self.resulting_manifest
    }
    pub fn application_report(&self) -> Digest {
        self.application_report
    }
    pub fn policy(&self) -> Digest {
        self.policy
    }
    pub fn policy_generation(&self) -> u64 {
        self.policy_generation
    }
    pub fn plan(&self) -> &VerificationPlan {
        &self.plan
    }
    pub fn authorization(&self) -> &SourceAuthorizationProvenance {
        &self.authorization
    }
    pub fn admitted_at_ms(&self) -> u64 {
        self.admitted_at_ms
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
    pub fn retention_policy(&self) -> Digest {
        self.retention_policy
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.digest(self.patch.0);
        c.digest(self.operation_digest);
        c.digest(self.resulting_snapshot.0);
        c.digest(self.resulting_manifest);
        c.digest(self.application_report);
        c.digest(self.policy);
        c.u64(self.policy_generation);
        c.digest(self.plan.id().0);
        c.str(self.authorization.principal.as_str());
        c.digest(self.authorization.capability_reference);
        c.u64(self.admitted_at_ms);
        c.u64(self.expires_at_ms);
        c.digest(self.retention_policy);
        c.into_bytes()
    }

    fn compute_id(&self) -> SourceAdmissionId {
        SourceAdmissionId(Digest::of(
            "celetixo/source-admission/v1",
            &self.canonical_bytes(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAttemptView {
    pub admission: SourceAdmission,
    pub state: SourceAttemptState,
    pub executor: Option<PrincipalId>,
    pub evidence: Option<SourceEvidenceId>,
    pub events: Vec<SourceAttemptEvent>,
}

/// An immutable failed admission, not a runnable capability. An authenticated
/// owner cannot later recycle its identity after changing patch or policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRejectionView {
    pub scope: Scope,
    pub owner: PrincipalId,
    pub transaction: TransactionId,
    pub transaction_attempt: u64,
    pub source_attempt: SourceAttemptId,
    pub request: Digest,
    pub error: EngineError,
    pub rejected_at_ms: u64,
    pub events: Vec<SourceAttemptEvent>,
}

#[derive(Clone)]
pub struct SourceExecutorCapability {
    store: Arc<Mutex<Database>>,
    token: u64,
    principal: PrincipalId,
    backend: BackendIdentity,
}

impl SourceExecutorCapability {
    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }
    pub fn backend(&self) -> &BackendIdentity {
        &self.backend
    }
    pub(super) fn token(&self) -> u64 {
        self.token
    }
}

#[derive(Clone)]
pub(super) struct SourceExecutorGrant {
    principal: PrincipalId,
    scope: Scope,
    backend: BackendIdentity,
    tiers: BTreeSet<ExecutionTier>,
    resource_ceiling: ResourceLimits,
    expires_at_ms: u64,
    active: bool,
}

#[derive(Clone)]
pub(super) struct SourceAttemptRecord {
    pub(super) admission: SourceAdmission,
    pub(super) application: AppliedSourcePatch,
    pub(super) patch: SourcePatch,
    pub(super) state: SourceAttemptState,
    pub(super) owner_capability: u64,
    pub(super) executor_capability: Option<u64>,
    pub(super) evidence: Option<SourceEvidenceId>,
    pub(super) events: Vec<SourceAttemptEvent>,
}

impl SourceAttemptRecord {
    pub(super) fn transition(
        &mut self,
        now: u64,
        next: SourceAttemptState,
        reason: &str,
    ) -> Result<(), EngineError> {
        if !self.state.allows(next) {
            return Err(SourceAttemptError::InvalidState.into());
        }
        let sequence =
            u64::try_from(self.events.len()).map_err(|_| EngineError::CounterExhausted)?;
        self.events.push(SourceAttemptEvent {
            sequence,
            at_ms: now,
            from: self.state,
            to: next,
            reason: reason.into(),
        });
        self.state = next;
        Ok(())
    }

    pub(super) fn view(&self, db: &Database) -> SourceAttemptView {
        SourceAttemptView {
            admission: self.admission.clone(),
            state: self.state,
            executor: self
                .executor_capability
                .and_then(|token| db.source_executors.get(&token))
                .map(|grant| grant.principal.clone()),
            evidence: self.evidence,
            events: self.events.clone(),
        }
    }
}

#[derive(Clone)]
pub(super) struct RecordedSourceOperation {
    request: Digest,
    attempt: SourceAttemptId,
    owner_capability: u64,
    outcome: Result<(), EngineError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePatchContext {
    pub scope: Scope,
    pub principal: PrincipalId,
    pub transaction: TransactionId,
    pub transaction_attempt: u64,
    pub base_snapshot: SnapshotId,
    pub base_revision: Revision,
    pub analysis_artifact: Digest,
    pub index_generation: Digest,
    pub impact_request: Digest,
    pub impact_result: Digest,
    pub policy: Digest,
    pub created_at_ms: u64,
    pub maximum_expiry_ms: u64,
}

impl SourcePatchContext {
    pub fn bind(
        &self,
        source_attempt: SourceAttemptId,
        producer: String,
        expires_at_ms: u64,
    ) -> Result<PatchBinding, SourceAttemptError> {
        if expires_at_ms <= self.created_at_ms || expires_at_ms > self.maximum_expiry_ms {
            return Err(SourceAttemptError::Expired);
        }
        Ok(PatchBinding {
            scope: self.scope.clone(),
            principal: self.principal.clone(),
            transaction: self.transaction.clone(),
            transaction_attempt: self.transaction_attempt,
            source_attempt,
            base_snapshot: self.base_snapshot,
            base_revision: self.base_revision,
            analysis_artifact: self.analysis_artifact,
            index_generation: self.index_generation,
            impact_request: self.impact_request,
            impact_result: self.impact_result,
            created_at_ms: self.created_at_ms,
            expires_at_ms,
            producer,
        })
    }
}

impl Administrator {
    /// Trusted embedding boundary. Backends are registered only after their
    /// external threat-boundary validation; this function does not prove it.
    pub fn grant_source_executor(
        &self,
        principal: PrincipalId,
        scope: Scope,
        backend: BackendIdentity,
        tiers: BTreeSet<ExecutionTier>,
        resource_ceiling: ResourceLimits,
        expires_at_ms: u64,
    ) -> Result<SourceExecutorCapability, EngineError> {
        let mut db = self.store.lock()?;
        if tiers.is_empty()
            || tiers.contains(&ExecutionTier::T0)
            || !valid_text(&backend.name)
            || !valid_text(&backend.version)
            || expires_at_ms <= db.now
        {
            return Err(SourceAttemptError::UntrustedExecutor.into());
        }
        resource_ceiling
            .validate()
            .map_err(SourceAttemptError::from)?;
        let token = db
            .next_source_executor
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?;
        db.next_source_executor = token;
        db.source_executors.insert(
            token,
            SourceExecutorGrant {
                principal: principal.clone(),
                scope,
                backend: backend.clone(),
                tiers,
                resource_ceiling,
                expires_at_ms,
                active: true,
            },
        );
        Ok(SourceExecutorCapability {
            store: self.store.inner.clone(),
            token,
            principal,
            backend,
        })
    }

    pub fn revoke_source_executor(
        &self,
        executor: &SourceExecutorCapability,
    ) -> Result<(), EngineError> {
        if !Arc::ptr_eq(&self.store.inner, &executor.store) {
            return Err(EngineError::Unauthorized);
        }
        let mut db = self.store.lock()?;
        db.source_executors
            .get_mut(&executor.token)
            .ok_or(EngineError::Unauthorized)?
            .active = false;
        let now = db.now;
        for record in db.source_attempts.values_mut() {
            if record.executor_capability == Some(executor.token) && !record.state.is_terminal() {
                record.transition(now, SourceAttemptState::Cancelled, "executor grant revoked")?;
            }
        }
        Ok(())
    }
}

impl Engine {
    pub fn install_source_policy(
        &self,
        session: &Session,
        scope: &Scope,
        expected_current: Option<Digest>,
        policy: SourceExecutionPolicy,
    ) -> Result<Digest, EngineError> {
        let mut db = self.store.lock()?;
        self.authorize(&db, session, scope, Permission::ManageModel)?;
        let current = db.source_policies.get(scope);
        if current == Some(&policy) {
            return Ok(policy.digest());
        }
        if current.map(SourceExecutionPolicy::digest) != expected_current {
            return Err(SourceAttemptError::PolicyChanged.into());
        }
        if current.is_some_and(|old| policy.input().generation <= old.input().generation)
            || policy.input().expires_at_ms <= db.now
        {
            return Err(SourceAttemptError::PolicyChanged.into());
        }
        let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
        let analysis = repo
            .source_analysis
            .as_ref()
            .ok_or(SourceAttemptError::SourceRepositoryRequired)?;
        if !analysis.is_complete() {
            return Err(EngineError::IncompleteAnalysis);
        }
        policy
            .validate_snapshot(&repo.current)
            .map_err(SourceAttemptError::from)?;
        if policy.input().scope != *scope
            || policy.input().analysis_artifact != analysis.digest()
            || policy.input().index_generation != analysis.index_generation()
        {
            return Err(SourceAttemptError::BindingMismatch.into());
        }
        let files = crate::snapshot::effective_workspace(&repo.current)
            .map_err(|_| SourceAttemptError::BindingMismatch)?;
        for (path, rule) in &policy.input().paths {
            let actual = files
                .get(path)
                .map(crate::source_patch::Preimage::of)
                .transpose()
                .map_err(SourceAttemptError::from)?;
            if actual != rule.expected_base {
                return Err(
                    SourceAttemptError::Plan(SourcePlanError::PathPreimage(path.clone())).into(),
                );
            }
            if !rule.required_writes.is_subset(analysis.elements()) {
                return Err(SourceAttemptError::UnknownPolicyElement.into());
            }
        }
        if policy
            .invariants()
            .values()
            .any(|invariant| !invariant.scope.is_subset(analysis.elements()))
        {
            return Err(SourceAttemptError::UnknownPolicyElement.into());
        }
        let digest = policy.digest();
        let generation = repo
            .model
            .generation
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?;
        let next_model = analysis.model(generation);
        db.repositories
            .get_mut(scope)
            .ok_or(EngineError::NotFound)?
            .model = next_model;
        db.source_policies.insert(scope.clone(), policy);
        Ok(digest)
    }

    pub fn source_patch_context(
        &self,
        session: &Session,
        scope: &Scope,
        transaction: &TransactionId,
    ) -> Result<SourcePatchContext, EngineError> {
        let db = self.store.lock()?;
        self.authorize(&db, session, scope, Permission::Execute)?;
        source_context(&db, scope, transaction, session.token)
    }

    pub fn admit_source_attempt(
        &self,
        session: &Session,
        scope: &Scope,
        patch: SourcePatch,
        key: IdempotencyKey,
    ) -> Result<SourceAdmission, EngineError> {
        self.admit_source_attempt_with_options(
            session,
            scope,
            patch,
            SourceAttemptOptions::default(),
            key,
        )
    }

    pub fn admit_source_attempt_with_options(
        &self,
        session: &Session,
        scope: &Scope,
        patch: SourcePatch,
        options: SourceAttemptOptions,
        key: IdempotencyKey,
    ) -> Result<SourceAdmission, EngineError> {
        let mut db = self.store.lock()?;
        self.authorize(&db, session, scope, Permission::Execute)?;
        // An unauthorized caller cannot reserve another principal's attempt
        // identity merely by submitting a malformed patch envelope.
        if patch.binding().scope != *scope {
            return Err(EngineError::Unauthorized);
        }
        let tx = db
            .transactions
            .get(&(scope.clone(), patch.binding().transaction.clone()))
            .ok_or(EngineError::NotFound)?;
        if tx.owner != session.principal {
            return Err(EngineError::Unauthorized);
        }
        if tx
            .lease
            .as_ref()
            .is_some_and(|lease| lease.owner != session.token)
        {
            return Err(EngineError::InvalidLease);
        }
        // Structurally invalid options reserve neither an attempt identity nor
        // an idempotency key, like a patch rejected before construction.
        options.validate()?;
        let operation_key = (scope.clone(), key);
        let request = options.request_digest(patch.id());
        if let Some(previous) = db.source_operations.get(&operation_key) {
            if previous.owner_capability != session.token {
                return Err(EngineError::Unauthorized);
            }
            if previous.request != request {
                return Err(EngineError::IdempotencyConflict);
            }
            previous.outcome.clone()?;
            let record = db
                .source_attempts
                .get(&(scope.clone(), previous.attempt.clone()))
                .ok_or(EngineError::NotFound)?;
            if record.owner_capability != session.token {
                return Err(EngineError::Unauthorized);
            }
            return Ok(record.admission.clone());
        }
        let binding = patch.binding().clone();
        let identity = (scope.clone(), binding.source_attempt.clone());
        let result = if db.source_rejections.contains_key(&identity)
            || db.source_attempts.contains_key(&identity)
        {
            Err(SourceAttemptError::AttemptIdentity.into())
        } else {
            Self::admit_source_attempt_locked(&mut db, session, scope, patch, options)
        };
        if let Err(error) = &result
            && !db.source_attempts.contains_key(&identity)
            && !db.source_rejections.contains_key(&identity)
        {
            let now = db.now;
            db.source_rejections.insert(
                identity,
                SourceRejectionView {
                    scope: scope.clone(),
                    owner: session.principal.clone(),
                    transaction: binding.transaction,
                    transaction_attempt: binding.transaction_attempt,
                    source_attempt: binding.source_attempt.clone(),
                    request,
                    error: error.clone(),
                    rejected_at_ms: now,
                    events: vec![SourceAttemptEvent {
                        sequence: 0,
                        at_ms: now,
                        from: SourceAttemptState::Proposed,
                        to: SourceAttemptState::Rejected,
                        reason: error.to_string(),
                    }],
                },
            );
        }
        db.source_operations.insert(
            operation_key,
            RecordedSourceOperation {
                request,
                attempt: binding.source_attempt,
                owner_capability: session.token,
                outcome: result.as_ref().map(|_| ()).map_err(Clone::clone),
            },
        );
        result
    }

    fn admit_source_attempt_locked(
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        patch: SourcePatch,
        options: SourceAttemptOptions,
    ) -> Result<SourceAdmission, EngineError> {
        let binding = patch.binding();
        let context = source_context(db, scope, &binding.transaction, session.token)?;
        if binding.scope != context.scope
            || binding.principal != context.principal
            || binding.transaction_attempt != context.transaction_attempt
            || binding.base_snapshot != context.base_snapshot
            || binding.base_revision != context.base_revision
            || binding.analysis_artifact != context.analysis_artifact
            || binding.index_generation != context.index_generation
            || binding.impact_request != context.impact_request
            || binding.impact_result != context.impact_result
            || binding.created_at_ms > db.now
            || binding.expires_at_ms <= db.now
            || binding.expires_at_ms > context.maximum_expiry_ms
        {
            return Err(SourceAttemptError::BindingMismatch.into());
        }
        let record_key = (scope.clone(), binding.source_attempt.clone());
        if db.source_attempts.contains_key(&record_key) {
            return Err(SourceAttemptError::AttemptIdentity.into());
        }
        if db.source_attempts.values().any(|record| {
            record.admission.binding.scope == *scope
                && record.admission.binding.transaction == binding.transaction
                && record.admission.binding.transaction_attempt == binding.transaction_attempt
                && !record.state.is_terminal()
        }) {
            return Err(SourceAttemptError::AttemptActive.into());
        }
        let tx = db
            .transactions
            .get(&(scope.clone(), binding.transaction.clone()))
            .ok_or(EngineError::NotFound)?;
        if !tx.intent.invariants.is_empty() {
            return Err(SourceAttemptError::UnsupportedInvariant.into());
        }
        let policy = db
            .source_policies
            .get(scope)
            .ok_or(SourceAttemptError::PolicyChanged)?;
        let effective = tx
            .attempt()
            .effective
            .as_ref()
            .ok_or(EngineError::IncompleteAnalysis)?;
        policy
            .validate_patch_paths(&patch, &tx.attempt().base, &effective.writes)
            .map_err(SourceAttemptError::from)?;
        let application = patch
            .apply(&tx.attempt().base)
            .map_err(SourceAttemptError::from)?;
        application
            .validate(&tx.attempt().base, &patch)
            .map_err(SourceAttemptError::from)?;
        for (path, input) in crate::snapshot::effective_workspace(application.snapshot())
            .map_err(|_| SourceAttemptError::BindingMismatch)?
        {
            if let Some(bytes) = input.bytes {
                policy
                    .input()
                    .credentials
                    .admit_input(&path, &bytes)
                    .map_err(SourceAttemptError::from)?;
            }
        }
        let source = db.repositories[scope]
            .source_analysis
            .as_ref()
            .ok_or(SourceAttemptError::SourceRepositoryRequired)?;
        let (_, impact, invariants) = derive_source_impact(
            source,
            tx.attempt().model.generation,
            &tx.intent.accesses,
            Some(policy),
        )?;
        let plan = VerificationPlan::resolve(
            policy,
            SourcePlanBinding {
                scope: scope.clone(),
                base_snapshot: binding.base_snapshot,
                analysis_artifact: binding.analysis_artifact,
                index_generation: binding.index_generation,
                impact_request: binding.impact_request,
                impact_result: binding.impact_result,
                patch: patch.id(),
                operation_digest: patch.operation_digest(),
                resulting_snapshot: application.snapshot().id(),
                resulting_manifest: application.snapshot().manifest().digest(),
            },
            PlanRequirements {
                declared_checks: tx.intent.required_checks.clone(),
                impact_checks: impact.justified_checks,
                invariants,
                optional_checks: options.optional_checks,
            },
        )
        .map_err(SourceAttemptError::from)?;
        plan.validate_resulting_snapshot(application.snapshot())
            .map_err(SourceAttemptError::from)?;
        let mut admission = SourceAdmission {
            id: SourceAdmissionId(Digest::of("celetixo/source-admission/uninitialized", b"")),
            binding: binding.clone(),
            patch: patch.id(),
            operation_digest: patch.operation_digest(),
            resulting_snapshot: application.snapshot().id(),
            resulting_manifest: application.snapshot().manifest().digest(),
            application_report: application.report().digest(),
            policy: policy.digest(),
            policy_generation: policy.input().generation,
            plan,
            authorization: SourceAuthorizationProvenance {
                principal: session.principal.clone(),
                capability_reference: authorization_digest(db, session.token)?,
            },
            admitted_at_ms: db.now,
            expires_at_ms: binding.expires_at_ms,
            retention_policy: policy.input().retention_policy,
        };
        admission.id = admission.compute_id();
        let record = SourceAttemptRecord {
            admission: admission.clone(),
            application,
            patch,
            state: SourceAttemptState::Admitted,
            owner_capability: session.token,
            executor_capability: None,
            evidence: None,
            events: vec![SourceAttemptEvent {
                sequence: 0,
                at_ms: db.now,
                from: SourceAttemptState::Proposed,
                to: SourceAttemptState::Admitted,
                reason: "Engine admission validated".into(),
            }],
        };
        db.source_attempts.insert(record_key, record);
        Ok(admission)
    }

    pub fn source_rejection(
        &self,
        session: &Session,
        scope: &Scope,
        attempt: &SourceAttemptId,
    ) -> Result<SourceRejectionView, EngineError> {
        let db = self.store.lock()?;
        self.authorize(&db, session, scope, Permission::Read)?;
        db.source_rejections
            .get(&(scope.clone(), attempt.clone()))
            .cloned()
            .ok_or(EngineError::NotFound)
    }

    pub fn source_attempt(
        &self,
        session: &Session,
        scope: &Scope,
        attempt: &SourceAttemptId,
    ) -> Result<SourceAttemptView, EngineError> {
        let mut db = self.store.lock()?;
        self.authorize(&db, session, scope, Permission::Read)?;
        let key = (scope.clone(), attempt.clone());
        reconcile_attempt(&mut db, &key)?;
        Ok(db
            .source_attempts
            .get(&key)
            .ok_or(EngineError::NotFound)?
            .view(&db))
    }

    /// An exact retry observes the existing claim. It does not issue a second
    /// permission to launch. The caller must reconcile its workspace/run ID.
    pub fn claim_source_attempt(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
    ) -> Result<SourceAttemptView, EngineError> {
        let mut db = self.store.lock()?;
        validate_executor_store(self, executor)?;
        let key = attempt_key(admission);
        reconcile_attempt(&mut db, &key)?;
        validate_source_admission(&db, executor, admission, false)?;
        let now = db.now;
        let record = db
            .source_attempts
            .get_mut(&key)
            .ok_or(EngineError::NotFound)?;
        if record.executor_capability.is_some() {
            return Err(SourceAttemptError::AlreadyClaimed.into());
        }
        record.transition(
            now,
            SourceAttemptState::Patching,
            "trusted executor claimed attempt",
        )?;
        record.executor_capability = Some(executor.token());
        Ok(db.source_attempts[&key].view(&db))
    }

    pub fn source_application(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
    ) -> Result<AppliedSourcePatch, EngineError> {
        let db = self.store.lock()?;
        validate_executor_store(self, executor)?;
        validate_source_admission(&db, executor, admission, true)?;
        Ok(db.source_attempts[&attempt_key(admission)]
            .application
            .clone())
    }

    /// Revalidate a claimed execution without cloning source bytes. Executors
    /// poll this before side effects and while supervising running checks.
    pub fn validate_source_attempt(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
    ) -> Result<SourceAttemptState, EngineError> {
        let mut db = self.store.lock()?;
        validate_executor_store(self, executor)?;
        let key = attempt_key(admission);
        reconcile_attempt(&mut db, &key)?;
        validate_source_admission(&db, executor, admission, true)?;
        Ok(db.source_attempts[&key].state)
    }

    /// Authenticated monotonic Engine clock used for receipt bindings. The
    /// trusted embedding drives the shared development clock; executor-supplied
    /// timestamps never alter admission or lease authority.
    pub fn source_execution_time(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
    ) -> Result<u64, EngineError> {
        let mut db = self.store.lock()?;
        validate_executor_store(self, executor)?;
        let key = attempt_key(admission);
        reconcile_attempt(&mut db, &key)?;
        validate_source_admission(&db, executor, admission, true)?;
        Ok(db.now)
    }

    pub fn mark_source_patch_applied(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
    ) -> Result<SourceAttemptView, EngineError> {
        source_transition(
            self,
            executor,
            admission,
            SourceAttemptState::PatchApplied,
            "exact candidate materialized",
        )
    }

    pub fn begin_source_verification(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
    ) -> Result<SourceAttemptView, EngineError> {
        source_transition(
            self,
            executor,
            admission,
            SourceAttemptState::Verifying,
            "policy verification started",
        )
    }

    pub fn finish_source_attempt(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        outcome: SourceAttemptState,
        reason: String,
    ) -> Result<SourceAttemptView, EngineError> {
        if !valid_text(&reason)
            || !outcome.is_terminal()
            || outcome == SourceAttemptState::EvidenceSealed
        {
            return Err(SourceAttemptError::InvalidReason.into());
        }
        source_transition(self, executor, admission, outcome, &reason)
    }
}

fn source_transition(
    engine: &Engine,
    executor: &SourceExecutorCapability,
    admission: &SourceAdmission,
    next: SourceAttemptState,
    reason: &str,
) -> Result<SourceAttemptView, EngineError> {
    let mut db = engine.store.lock()?;
    validate_executor_store(engine, executor)?;
    let key = attempt_key(admission);
    reconcile_attempt(&mut db, &key)?;
    validate_source_admission(&db, executor, admission, true)?;
    let now = db.now;
    db.source_attempts
        .get_mut(&key)
        .ok_or(EngineError::NotFound)?
        .transition(now, next, reason)?;
    Ok(db.source_attempts[&key].view(&db))
}

pub(super) fn attempt_key(admission: &SourceAdmission) -> (Scope, SourceAttemptId) {
    (
        admission.binding.scope.clone(),
        admission.binding.source_attempt.clone(),
    )
}

pub(super) fn validate_executor_store(
    engine: &Engine,
    executor: &SourceExecutorCapability,
) -> Result<(), EngineError> {
    if !Arc::ptr_eq(&engine.store.inner, &executor.store) {
        return Err(EngineError::Unauthorized);
    }
    Ok(())
}

pub(super) fn validate_source_admission(
    db: &Database,
    executor: &SourceExecutorCapability,
    admission: &SourceAdmission,
    require_claim: bool,
) -> Result<(), EngineError> {
    validate_executor_for_admission(db, executor, admission, require_claim)?;
    let record = db
        .source_attempts
        .get(&attempt_key(admission))
        .ok_or(EngineError::NotFound)?;
    validate_record_current(db, record)?;
    Ok(())
}

/// Current authenticated executor and exact admission/claim validation, without
/// reviving a terminal attempt. Used only to acknowledge already retained
/// immutable outcomes; new side effects must use validate_source_admission.
pub(super) fn validate_executor_for_admission(
    db: &Database,
    executor: &SourceExecutorCapability,
    admission: &SourceAdmission,
    require_claim: bool,
) -> Result<(), EngineError> {
    let record = db
        .source_attempts
        .get(&attempt_key(admission))
        .ok_or(EngineError::NotFound)?;
    if &record.admission != admission || admission.id != admission.compute_id() {
        return Err(SourceAttemptError::BindingMismatch.into());
    }
    let executor_grant = db
        .source_executors
        .get(&executor.token)
        .ok_or(SourceAttemptError::UntrustedExecutor)?;
    if !executor_grant.active
        || executor_grant.scope != admission.binding.scope
        || executor_grant.principal != executor.principal
        || executor_grant.backend != executor.backend
        || executor_grant.expires_at_ms <= db.now
    {
        return Err(SourceAttemptError::UntrustedExecutor.into());
    }
    for check in admission.plan.checks() {
        let input = check.definition().input();
        if input.backend != executor_grant.backend
            || !executor_grant.tiers.contains(&input.tier)
            || !input.resources.is_within(&executor_grant.resource_ceiling)
        {
            return Err(SourceAttemptError::UntrustedExecutor.into());
        }
    }
    if require_claim && record.executor_capability != Some(executor.token) {
        return Err(SourceAttemptError::UntrustedExecutor.into());
    }
    Ok(())
}

fn validate_record_current(db: &Database, record: &SourceAttemptRecord) -> Result<(), EngineError> {
    if record.state.is_terminal() {
        return Err(SourceAttemptError::InvalidState.into());
    }
    if record.patch.id() != record.admission.patch
        || record.patch.operation_digest() != record.admission.operation_digest
        || record.application.report().digest() != record.admission.application_report
    {
        return Err(SourceAttemptError::BindingMismatch.into());
    }
    if record.admission.expires_at_ms <= db.now {
        return Err(SourceAttemptError::Expired.into());
    }
    let context = source_context(
        db,
        &record.admission.binding.scope,
        &record.admission.binding.transaction,
        record.owner_capability,
    )?;
    if context.policy != record.admission.policy
        || context.analysis_artifact != record.admission.binding.analysis_artifact
        || context.index_generation != record.admission.binding.index_generation
        || context.impact_request != record.admission.binding.impact_request
        || context.impact_result != record.admission.binding.impact_result
        || context.transaction_attempt != record.admission.binding.transaction_attempt
    {
        return Err(SourceAttemptError::BindingMismatch.into());
    }
    Ok(())
}

fn source_context(
    db: &Database,
    scope: &Scope,
    id: &TransactionId,
    owner_capability: u64,
) -> Result<SourcePatchContext, EngineError> {
    let tx = db
        .transactions
        .get(&(scope.clone(), id.clone()))
        .ok_or(EngineError::NotFound)?;
    let owner = db
        .grants
        .get(&owner_capability)
        .ok_or(EngineError::Unauthorized)?;
    if !owner.active
        || owner.scope != *scope
        || owner.principal != tx.owner
        || !owner.permissions.contains(&Permission::Execute)
    {
        return Err(EngineError::Unauthorized);
    }
    if tx.state != TransactionState::Executing {
        return Err(SourceAttemptError::InvalidState.into());
    }
    let lease = tx.lease.as_ref().ok_or(EngineError::InvalidLease)?;
    if lease.owner != owner_capability || !lease_active(db, lease) {
        return Err(EngineError::InvalidLease);
    }
    let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
    if repo.current.id() != tx.attempt().base.id()
        || repo.current.revision() != tx.attempt().base.revision()
    {
        return Err(EngineError::RevalidationRequired {
            current: repo.current.id(),
        });
    }
    if repo.model != tx.attempt().model {
        return Err(EngineError::ModelChanged);
    }
    let source = repo
        .source_analysis
        .as_ref()
        .ok_or(SourceAttemptError::SourceRepositoryRequired)?;
    source.validate(&tx.attempt().base, source.index_generation())?;
    if !source.is_complete() {
        return Err(EngineError::IncompleteAnalysis);
    }
    let policy = db
        .source_policies
        .get(scope)
        .ok_or(SourceAttemptError::PolicyChanged)?;
    if policy.input().expires_at_ms <= db.now {
        return Err(SourceAttemptError::Expired.into());
    }
    if policy.input().analysis_artifact != source.digest()
        || policy.input().index_generation != source.index_generation()
    {
        return Err(SourceAttemptError::PolicyChanged.into());
    }
    policy
        .validate_snapshot(&repo.current)
        .map_err(SourceAttemptError::from)?;
    let mut explicit = tx.intent.accesses.clone();
    for invariant in &tx.intent.invariants {
        explicit.reads.extend(invariant.scope());
        explicit.invariant_scopes.extend(invariant.scope());
    }
    let (effective, impact, _) = derive_source_impact(
        source,
        tx.attempt().model.generation,
        &explicit,
        Some(policy),
    )?;
    let stored = tx
        .attempt()
        .analysis
        .as_ref()
        .ok_or(EngineError::IncompleteAnalysis)?;
    if impact.analysis != *stored
        || !matches!(stored.completeness(), Completeness::Complete)
        || tx.attempt().effective.as_ref() != Some(&effective)
    {
        return Err(EngineError::IncompleteAnalysis);
    }
    let request = intent_digest(&tx.intent);
    let result = impact_digest(request, &impact);
    Ok(SourcePatchContext {
        scope: scope.clone(),
        principal: owner.principal.clone(),
        transaction: id.clone(),
        transaction_attempt: tx.attempt().number,
        base_snapshot: tx.attempt().base.id(),
        base_revision: tx.attempt().base.revision(),
        analysis_artifact: source.digest(),
        index_generation: source.index_generation(),
        impact_request: request,
        impact_result: result,
        policy: policy.digest(),
        created_at_ms: db.now,
        maximum_expiry_ms: lease.expires_at.min(policy.input().expires_at_ms),
    })
}

fn intent_digest(intent: &TransactionIntent) -> Digest {
    let mut c = Canonical::default();
    c.str(&intent.objective);
    c.digest(intent.accesses.digest());
    c.u64(intent.invariants.len() as u64);
    for invariant in &intent.invariants {
        c.str(invariant.id.as_str());
        match &invariant.condition {
            Condition::Equals { element, value } => {
                c.u64(0);
                c.str(element.as_str());
                c.i64(*value);
            }
            Condition::Present(element) => {
                c.u64(1);
                c.str(element.as_str());
            }
            Condition::SumAtMost { elements, maximum } => {
                c.u64(2);
                c.u64(elements.len() as u64);
                for element in elements {
                    c.str(element.as_str());
                }
                c.i64(*maximum);
            }
        }
    }
    c.u64(intent.required_checks.len() as u64);
    for check in &intent.required_checks {
        c.digest(check.digest());
    }
    c.u64(intent.depends_on.len() as u64);
    for dependency in &intent.depends_on {
        c.str(dependency.as_str());
    }
    c.finish("celetixo/source-impact-request/v1")
}

fn impact_digest(request: Digest, impact: &RepositoryImpact) -> Digest {
    let mut c = Canonical::default();
    c.digest(request);
    c.str(impact.scope.organization.as_str());
    c.str(impact.scope.workspace.as_str());
    c.str(impact.scope.repository.as_str());
    c.digest(impact.snapshot.0);
    c.digest(impact.index_generation);
    c.digest(impact.artifact);
    c.digest(impact.analysis.digest());
    c.u64(impact.justified_checks.len() as u64);
    for check in &impact.justified_checks {
        c.str(check.as_str());
    }
    c.finish("celetixo/source-impact-result/v1")
}

fn authorization_digest(db: &Database, token: u64) -> Result<Digest, EngineError> {
    let grant = db.grants.get(&token).ok_or(EngineError::Unauthorized)?;
    let mut c = Canonical::default();
    c.u64(token);
    c.str(grant.principal.as_str());
    c.str(grant.scope.organization.as_str());
    c.str(grant.scope.workspace.as_str());
    c.str(grant.scope.repository.as_str());
    for permission in Permission::ALL {
        c.u64(u64::from(grant.permissions.contains(&permission)));
    }
    Ok(c.finish("celetixo/source-authorization-provenance/v1"))
}

fn reconcile_attempt(db: &mut Database, key: &(Scope, SourceAttemptId)) -> Result<(), EngineError> {
    let record = db.source_attempts.get(key).ok_or(EngineError::NotFound)?;
    if record.state.is_terminal() {
        return Ok(());
    }
    if let Some(token) = record.executor_capability {
        let outcome = match db.source_executors.get(&token) {
            Some(grant) if grant.active && grant.expires_at_ms > db.now => None,
            Some(grant) if grant.expires_at_ms <= db.now => Some(SourceAttemptState::TimedOut),
            _ => Some(SourceAttemptState::Cancelled),
        };
        if let Some(outcome) = outcome {
            let now = db.now;
            db.source_attempts
                .get_mut(key)
                .ok_or(EngineError::NotFound)?
                .transition(now, outcome, "executor capability expired or revoked")?;
            return Ok(());
        }
    }
    let failure = validate_record_current(db, record).err();
    if let Some(error) = failure {
        let next = match error {
            EngineError::Unauthorized | EngineError::IllegalTransition { .. } => {
                SourceAttemptState::Cancelled
            }
            EngineError::InvalidLease | EngineError::SourceAttempt(SourceAttemptError::Expired) => {
                SourceAttemptState::TimedOut
            }
            _ => SourceAttemptState::RevalidationRequired,
        };
        let now = db.now;
        db.source_attempts
            .get_mut(key)
            .ok_or(EngineError::NotFound)?
            .transition(now, next, &error.to_string())?;
    }
    Ok(())
}

pub(super) fn cancel_transaction_attempts(
    db: &mut Database,
    scope: &Scope,
    transaction: &TransactionId,
) -> Result<(), EngineError> {
    let now = db.now;
    for record in db.source_attempts.values_mut() {
        if record.admission.binding.scope == *scope
            && record.admission.binding.transaction == *transaction
            && !record.state.is_terminal()
        {
            // Cancellation is allowed from every nonterminal source state.
            record.transition(now, SourceAttemptState::Cancelled, "transaction cancelled")?;
        }
    }
    Ok(())
}

/// Least fixed point of semantic dependencies and policy-owned invariant
/// scopes. Policy additions retain derived provenance rather than becoming
/// producer declarations. Source invariant checks cannot be omitted by intent.
pub(super) fn derive_source_impact(
    source: &RepositoryAnalysis,
    generation: u64,
    explicit: &AccessSet,
    policy: Option<&SourceExecutionPolicy>,
) -> Result<(AccessSet, RepositoryImpact, BTreeSet<InvariantId>), EngineError> {
    if !source.is_complete() {
        return Err(EngineError::IncompleteAnalysis);
    }
    if let Some(policy) = policy
        && (policy.input().scope != *source.scope()
            || policy.input().snapshot != source.snapshot()
            || policy.input().analysis_artifact != source.digest()
            || policy.input().index_generation != source.index_generation())
    {
        return Err(SourceAttemptError::PolicyChanged.into());
    }
    let mut expanded = explicit.clone();
    let mut invariants = BTreeSet::new();
    loop {
        let mut impact = source.inspect(generation, &expanded)?;
        let effective = expanded.union(impact.analysis.derived());
        let touched: BTreeSet<_> = effective
            .reads
            .union(&effective.writes)
            .chain(&effective.invariant_scopes)
            .cloned()
            .collect();
        let previous = expanded.clone();
        if let Some(policy) = policy {
            for invariant in policy.invariants().values() {
                if !invariant.scope.is_disjoint(&touched) {
                    invariants.insert(invariant.id.clone());
                    expanded.reads.extend(invariant.scope.iter().cloned());
                    expanded
                        .invariant_scopes
                        .extend(invariant.scope.iter().cloned());
                    impact
                        .justified_checks
                        .extend(invariant.required_checks.iter().cloned());
                }
            }
        }
        if expanded == previous {
            let mut derived = impact.analysis.derived().clone();
            derived
                .reads
                .extend(expanded.reads.difference(&explicit.reads).cloned());
            derived.invariant_scopes.extend(
                expanded
                    .invariant_scopes
                    .difference(&explicit.invariant_scopes)
                    .cloned(),
            );
            impact.analysis = ImpactAnalysis::new(
                impact.analysis.stamp().clone(),
                derived,
                impact.analysis.completeness().clone(),
            );
            return Ok((
                explicit.union(impact.analysis.derived()),
                impact,
                invariants,
            ));
        }
    }
}

fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}
