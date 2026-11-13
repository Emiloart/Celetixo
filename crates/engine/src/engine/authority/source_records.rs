//! Complete captured-source metadata graph. No live capability or reader is
//! reconstructed, and current object availability is never inferred from rows.
use super::*;
use crate::engine::source_attempts::{
    self, RecordedSourceOperation, SourceAttemptRecord, SourceExecutorGrant,
};
use crate::engine::source_cleanup::SourceCleanupGrant;
use crate::engine::source_evidence_gate::{
    self, RegisteredSourceArtifactStore, RetainedSourceEvidence,
};
use crate::persistence::{
    SourceAdmissionDependencies, SourceOutcomeContext, SourceOutcomeRecord, SourceRequestRecord,
    restore_source_admission,
};
use crate::source_evidence::{SourceEvidence, SourceEvidenceAssessment};
use crate::source_execution::{
    BackendIdentity, ExecutionTier, PlanRequirements, ResourceLimits, SourceExecutionPolicy,
    SourcePlanBinding, VerificationPlan,
};
use crate::source_patch::SourcePatch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAnalysisContext {
    pub analysis: Digest,
    pub policy: Option<Digest>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecords {
    /// Shared rows do not select a runtime profile. Native-only entry points
    /// still reject source content; these rows require the whole source graph.
    pub base: BaseRecords,
    pub analysis_head: Option<Digest>,
    pub analyses: BTreeMap<Digest, RepositoryAnalysis>,
    pub policy_head: Option<Digest>,
    pub policies: BTreeMap<Digest, SourceExecutionPolicy>,
    pub attempt_contexts: BTreeMap<(TransactionId, u64), SourceAnalysisContext>,
    pub executor_counter: u64,
    pub executors: BTreeMap<u64, SourceExecutorRecord>,
    pub cleanup_counter: u64,
    pub cleanup: BTreeMap<u64, SourceCleanupRecord>,
    pub artifact_counter: u64,
    pub artifact_stores: BTreeMap<u64, SourceArtifactStoreRecord>,
    pub attempts: BTreeMap<SourceAttemptId, SourceAttemptMetadata>,
    pub operations: BTreeMap<IdempotencyKey, SourceOperationRecord>,
    pub rejections: BTreeMap<SourceAttemptId, SourceRejectionView>,
    pub evidence: BTreeMap<SourceEvidenceId, SourceEvidenceMetadata>,
    /// Existing source calls to the unprepared native commit API can record a
    /// rejected result. Retain it explicitly; never manufacture a prepared request.
    pub unprepared_commits: BTreeMap<IdempotencyKey, SourceUnpreparedCommitRecord>,
}
impl SourceRecords {
    /// Empty scoped source-profile facts; authentication still belongs to the
    /// trusted embedding and every import is checked before evaluation.
    pub fn empty(scope: Scope) -> Self {
        Self {
            base: BaseRecords::empty(scope),
            analysis_head: None,
            analyses: BTreeMap::new(),
            policy_head: None,
            policies: BTreeMap::new(),
            attempt_contexts: BTreeMap::new(),
            executor_counter: 0,
            executors: BTreeMap::new(),
            cleanup_counter: 0,
            cleanup: BTreeMap::new(),
            artifact_counter: 0,
            artifact_stores: BTreeMap::new(),
            attempts: BTreeMap::new(),
            operations: BTreeMap::new(),
            rejections: BTreeMap::new(),
            evidence: BTreeMap::new(),
            unprepared_commits: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceExecutorRecord {
    pub principal: PrincipalId,
    pub backend: BackendIdentity,
    pub tiers: BTreeSet<ExecutionTier>,
    pub resource_ceiling: ResourceLimits,
    pub expires_at_ms: u64,
    pub active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCleanupRecord {
    pub principal: PrincipalId,
    pub backend: Digest,
    pub expires_at_ms: u64,
    pub active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceArtifactStoreRecord {
    /// None is the existing local profile, without a durable adapter selector.
    pub adapter: Option<SourceArtifactAdapter>,
    pub retention: Digest,
    pub provenance: Digest,
    pub active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAttemptMetadata {
    pub admission: SourceAdmission,
    pub patch: SourcePatch,
    pub options: SourceAttemptOptions,
    pub admission_fence: u64,
    pub admission_lease_event_count: u64,
    pub state: SourceAttemptState,
    pub owner_grant: u64,
    pub executor_grant: Option<u64>,
    pub evidence: Option<SourceEvidenceId>,
    pub events: Vec<SourceAttemptEvent>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOperationRecord {
    pub request: SourceRequestRecord,
    pub target_attempt: u64,
    pub recorded_at_ms: u64,
    pub fence: Option<u64>,
    pub lease_event_count: u64,
    pub observed_analysis: Option<Digest>,
    pub observed_policy: Option<Digest>,
    pub observed_model_generation: u64,
    pub owner_grant: u64,
    pub outcome: Result<SourceAdmissionId, EngineError>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidenceMetadata {
    pub evidence: SourceEvidence,
    pub receipt: SourceEvidenceReceipt,
    pub store_registration: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceUnpreparedCommitRecord {
    pub transaction: TransactionId,
    pub attempt: u64,
    pub outcome: Result<CommitReceipt, EngineError>,
}

impl AuthorityState {
    pub fn from_source_records(
        records: SourceRecords,
        limits: NativeRecordLimits,
    ) -> Result<Self, RestoreError> {
        check_source_budget(&records, limits)?;
        validate_catalogs(&records)?;
        let scope = records.base.scope.clone();
        let mut db = records::restore_base(records.base.clone(), Some(&records))?;
        install_source_graph(&mut db, &records)?;
        mutation::restore(&mut db, &scope, records.base.mutations.clone())?;
        let state = Self { scope, db };
        // Import never changes lazy expiry/reconciliation states. Exact readback
        // also ensures that no unsupported or orphaned supplied fact was dropped.
        if export(&state.db, &state.scope)? != records {
            return Err(RestoreError::NonCanonical);
        }
        Ok(state)
    }
    pub fn source_records(
        &self,
        limits: NativeRecordLimits,
    ) -> Result<SourceRecords, RestoreError> {
        // Source state currently enters only through checked import. Evaluation
        // callbacks retain that one scoped database; export audits all families.
        ensure_scope(&self.db, &self.scope)?;
        check_database_budget(&self.db, &self.scope, limits)?;
        let records = export(&self.db, &self.scope)?;
        check_source_budget(&records, limits)?;
        Ok(records)
    }
}

pub(super) fn export(db: &Database, scope: &Scope) -> Result<SourceRecords, RestoreError> {
    let base = records::export_base(db, scope, true)?;
    let analyses = db
        .source_analysis_records
        .iter()
        .filter(|((s, _), _)| s == scope)
        .map(|((_, d), v)| (*d, v.clone()))
        .collect();
    let policies = db
        .source_policy_records
        .iter()
        .filter(|((s, _), _)| s == scope)
        .map(|((_, d), v)| (*d, v.clone()))
        .collect();
    let mut attempt_contexts = BTreeMap::new();
    for ((s, id), tx) in &db.transactions {
        if s == scope {
            for a in &tx.attempts {
                attempt_contexts.insert(
                    (id.clone(), a.number),
                    SourceAnalysisContext {
                        analysis: a.source_analysis.ok_or(RestoreError::BindingMismatch)?,
                        policy: a.source_policy,
                    },
                );
            }
        }
    }

    let mut result = SourceRecords {
        base,
        analysis_head: db
            .repositories
            .get(scope)
            .and_then(|r| r.source_analysis.as_ref().map(RepositoryAnalysis::digest)),
        analyses,
        policy_head: db
            .source_policies
            .get(scope)
            .map(SourceExecutionPolicy::digest),
        policies,
        attempt_contexts,
        executor_counter: db.next_source_executor,
        executors: BTreeMap::new(),
        cleanup_counter: db.next_source_cleanup,
        cleanup: BTreeMap::new(),
        artifact_counter: db.next_source_artifact_store,
        artifact_stores: BTreeMap::new(),
        attempts: BTreeMap::new(),
        operations: BTreeMap::new(),
        rejections: BTreeMap::new(),
        evidence: BTreeMap::new(),
        unprepared_commits: BTreeMap::new(),
    };
    for (id, g) in &db.source_executors {
        if g.scope == *scope {
            result.executors.insert(
                *id,
                SourceExecutorRecord {
                    principal: g.principal.clone(),
                    backend: g.backend.clone(),
                    tiers: g.tiers.clone(),
                    resource_ceiling: g.resource_ceiling,
                    expires_at_ms: g.expires_at_ms,
                    active: g.active,
                },
            );
        }
    }
    for (id, g) in &db.source_cleanup_grants {
        if g.scope == *scope {
            result.cleanup.insert(
                *id,
                SourceCleanupRecord {
                    principal: g.principal.clone(),
                    backend: g.backend,
                    expires_at_ms: g.expires_at_ms,
                    active: g.active,
                },
            );
        }
    }
    for (id, g) in &db.source_artifact_stores {
        if g.scope == *scope {
            result.artifact_stores.insert(
                *id,
                SourceArtifactStoreRecord {
                    adapter: g.adapter.clone(),
                    retention: g.retention,
                    provenance: g.provenance,
                    active: g.active,
                },
            );
        }
    }
    for ((s, id), a) in &db.source_attempts {
        if s == scope {
            result.attempts.insert(
                id.clone(),
                SourceAttemptMetadata {
                    admission: a.admission.clone(),
                    patch: a.patch.clone(),
                    options: a.options.clone(),
                    admission_fence: a.admission_fence,
                    admission_lease_event_count: a.admission_lease_event_count,
                    state: a.state,
                    owner_grant: a.owner_capability,
                    executor_grant: a.executor_capability,
                    evidence: a.evidence,
                    events: a.events.clone(),
                },
            );
        }
    }
    for ((s, key), o) in &db.source_operations {
        if s == scope {
            result.operations.insert(
                key.clone(),
                SourceOperationRecord {
                    request: SourceRequestRecord::new(o.patch.clone(), o.options.clone())?,
                    target_attempt: o.target_attempt,
                    recorded_at_ms: o.recorded_at_ms,
                    fence: o.fence,
                    lease_event_count: o.lease_event_count,
                    observed_analysis: o.observed_analysis,
                    observed_policy: o.observed_policy,
                    observed_model_generation: o.observed_model_generation,
                    owner_grant: o.owner_capability,
                    outcome: o.outcome.clone(),
                },
            );
        }
    }
    for ((s, id), r) in &db.source_rejections {
        if s == scope {
            result.rejections.insert(id.clone(), r.clone());
        }
    }
    for ((s, id), e) in &db.source_evidence_records {
        if s == scope {
            result.evidence.insert(
                *id,
                SourceEvidenceMetadata {
                    evidence: e.evidence.clone(),
                    receipt: e.receipt.clone(),
                    store_registration: e.store_registration,
                },
            );
        }
    }
    for ((s, key), o) in &db.operations {
        if s == scope && o.request.is_none() {
            result.unprepared_commits.insert(
                key.clone(),
                SourceUnpreparedCommitRecord {
                    transaction: o.transaction.clone(),
                    attempt: o.attempt,
                    outcome: o.outcome.clone(),
                },
            );
        }
    }
    Ok(result)
}

fn invalid<T>() -> Result<T, RestoreError> {
    Err(RestoreError::BindingMismatch)
}
fn validate_catalogs(r: &SourceRecords) -> Result<(), RestoreError> {
    let scope = &r.base.scope;
    if !r.base.evidence.is_empty() || !r.base.operations.is_empty() {
        return Err(RestoreError::UnsupportedDomainValue);
    }
    let Some(repo) = &r.base.repository else {
        if r.analysis_head.is_some()
            || r.policy_head.is_some()
            || !r.analyses.is_empty()
            || !r.policies.is_empty()
            || !r.attempt_contexts.is_empty()
            || !r.attempts.is_empty()
            || !r.operations.is_empty()
            || !r.rejections.is_empty()
            || !r.evidence.is_empty()
            || !r.unprepared_commits.is_empty()
        {
            return invalid();
        }
        return Ok(());
    };
    // Source revision publication has no accepted implementation yet.
    if repo.initial != repo.current || !repo.history.is_empty() || repo.registered.len() != 1 {
        return Err(RestoreError::UnsupportedDomainValue);
    }
    let current = repo
        .registered
        .get(&repo.current)
        .ok_or(RestoreError::BindingMismatch)?;
    let head = r
        .analysis_head
        .and_then(|d| r.analyses.get(&d))
        .ok_or(RestoreError::BindingMismatch)?;
    if head.model(repo.model.generation) != repo.model {
        return invalid();
    }
    head.validate(current, head.index_generation())?;
    for (id, a) in &r.analyses {
        let base = repo
            .registered
            .get(&a.snapshot())
            .ok_or(RestoreError::BindingMismatch)?;
        if *id != a.digest() || a.scope() != scope {
            return invalid();
        }
        a.validate(base, a.index_generation())?;
    }
    let mut generations = BTreeSet::new();
    for (id, p) in &r.policies {
        let a = r
            .analyses
            .get(&p.input().analysis_artifact)
            .ok_or(RestoreError::BindingMismatch)?;
        let base = repo
            .registered
            .get(&p.input().snapshot)
            .ok_or(RestoreError::BindingMismatch)?;
        if *id != p.digest()
            || p.input().scope != *scope
            || p.input().index_generation != a.index_generation()
            || !generations.insert(p.input().generation)
        {
            return invalid();
        }
        p.validate_snapshot(base)?;
        let files = crate::snapshot::effective_workspace(base)
            .map_err(|_| RestoreError::BindingMismatch)?;
        for (path, rule) in &p.input().paths {
            let actual = files
                .get(path)
                .map(crate::source_patch::Preimage::of)
                .transpose()?;
            if actual != rule.expected_base || !rule.required_writes.is_subset(a.elements()) {
                return invalid();
            }
        }
        if p.invariants()
            .values()
            .any(|i| !i.scope.is_subset(a.elements()))
        {
            return invalid();
        }
    }
    if r.policy_head.is_some_and(|id| {
        r.policies.get(&id).is_none_or(|p| {
            r.policies
                .values()
                .any(|q| q.input().generation > p.input().generation)
        })
    }) {
        return invalid();
    }
    // A policy head may intentionally be stale relative to a replaced analysis.
    if r.policy_head.is_none() && !r.policies.is_empty() {
        return invalid();
    }
    let expected: BTreeSet<_> = r
        .base
        .transactions
        .iter()
        .flat_map(|(id, tx)| tx.attempts.iter().map(move |a| (id.clone(), a.number)))
        .collect();
    if r.attempt_contexts.keys().cloned().collect::<BTreeSet<_>>() != expected {
        return invalid();
    }
    Ok(())
}

pub(super) fn validate_transaction(
    tx: &Transaction,
    r: &SourceRecords,
) -> Result<(), RestoreError> {
    if tx.attempts.len() != 1 {
        return Err(RestoreError::UnsupportedDomainValue);
    }
    let a = tx.attempt();
    let context = r
        .attempt_contexts
        .get(&(tx.id.clone(), a.number))
        .ok_or(RestoreError::BindingMismatch)?;
    let source = r
        .analyses
        .get(&context.analysis)
        .ok_or(RestoreError::BindingMismatch)?;
    source.validate(&a.base, source.index_generation())?;
    if source.model(a.model.generation) != a.model
        || a.patch.is_some()
        || a.candidate.is_some()
        || !a.evidence.is_empty()
    {
        return invalid();
    }
    if a.analysis.is_some() {
        let policy = context
            .policy
            .map(|d| r.policies.get(&d).ok_or(RestoreError::BindingMismatch))
            .transpose()?;
        let mut expected = tx.clone();
        analyze_attempt(&mut expected, Some(source), policy)
            .map_err(|_| RestoreError::BindingMismatch)?;
        if expected.attempt().effective != a.effective || expected.attempt().analysis != a.analysis
        {
            return invalid();
        }
    } else if context.policy.is_some() {
        return invalid();
    }
    let analyzed = a.analysis.is_some();
    let leased = tx.lease.is_some();
    let valid = match tx.state {
        TransactionState::Proposed => !analyzed && !leased,
        TransactionState::Analyzed => analyzed && !leased,
        TransactionState::Reserved | TransactionState::Executing | TransactionState::Validating => {
            analyzed && leased
        }
        TransactionState::Aborted => {
            !leased
                && tx
                    .last_reason
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty())
        }
        TransactionState::Committed | TransactionState::RevalidationRequired => false,
    };
    if !valid {
        return invalid();
    }
    Ok(())
}

fn install_source_graph(db: &mut Database, r: &SourceRecords) -> Result<(), RestoreError> {
    let scope = &r.base.scope;
    for (id, a) in &r.analyses {
        db.source_analysis_records
            .insert((scope.clone(), *id), a.clone());
    }
    for (id, p) in &r.policies {
        db.source_policy_records
            .insert((scope.clone(), *id), p.clone());
    }
    if let Some(id) = r.policy_head {
        db.source_policies
            .insert(scope.clone(), r.policies[&id].clone());
    }
    db.next_source_executor = r.executor_counter;
    db.next_source_cleanup = r.cleanup_counter;
    db.next_source_artifact_store = r.artifact_counter;
    for (id, g) in &r.executors {
        if *id == 0
            || *id > r.executor_counter
            || g.expires_at_ms == 0
            || g.tiers.is_empty()
            || g.tiers.contains(&ExecutionTier::T0)
            || !source_attempts::valid_text(&g.backend.name)
            || !source_attempts::valid_text(&g.backend.version)
        {
            return invalid();
        }
        g.resource_ceiling.validate()?;
        db.source_executors.insert(
            *id,
            SourceExecutorGrant {
                principal: g.principal.clone(),
                scope: scope.clone(),
                backend: g.backend.clone(),
                tiers: g.tiers.clone(),
                resource_ceiling: g.resource_ceiling,
                expires_at_ms: g.expires_at_ms,
                active: g.active,
            },
        );
    }
    for (id, g) in &r.cleanup {
        if *id == 0 || *id > r.cleanup_counter || g.expires_at_ms == 0 {
            return invalid();
        }
        db.source_cleanup_grants.insert(
            *id,
            SourceCleanupGrant {
                principal: g.principal.clone(),
                scope: scope.clone(),
                backend: g.backend,
                expires_at_ms: g.expires_at_ms,
                active: g.active,
            },
        );
    }
    for (id, g) in &r.artifact_stores {
        if *id == 0
            || *id > r.artifact_counter
            || source_evidence_gate::artifact_store_provenance(
                scope,
                *id,
                g.retention,
                g.adapter.as_ref(),
            ) != g.provenance
        {
            return invalid();
        }
        db.source_artifact_stores.insert(
            *id,
            RegisteredSourceArtifactStore {
                scope: scope.clone(),
                adapter: g.adapter.clone(),
                retention: g.retention,
                provenance: g.provenance,
                active: g.active,
            },
        );
    }
    for (id, a) in &r.attempts {
        restore_attempt(db, r, id, a)?;
    }
    restore_operations(db, r)?;
    restore_evidence(db, r)?;
    for ((s, _), tx) in &db.transactions {
        if s != scope {
            return invalid();
        }
        if tx.state == TransactionState::Aborted
            && db
                .source_attempts
                .values()
                .any(|a| a.admission.binding().transaction == tx.id && !a.state.is_terminal())
        {
            return invalid();
        }
        let passed = db.source_evidence_records.values().any(|e| {
            e.evidence.binding().scope() == scope
                && e.receipt.assessment == SourceEvidenceAssessment::Passed
                && db.source_attempts.values().any(|a| {
                    a.admission.id() == e.receipt.admission
                        && a.admission.binding().transaction == tx.id
                })
        });
        if (tx.state == TransactionState::Validating) != passed
            && tx.state != TransactionState::Aborted
        {
            return invalid();
        }
    }
    for (key, o) in &r.unprepared_commits {
        let tx = db
            .transactions
            .get(&(scope.clone(), o.transaction.clone()))
            .ok_or(RestoreError::BindingMismatch)?;
        if o.attempt >= tx.attempts.len() as u64
            || o.outcome != Err(EngineError::SourceMutationUnsupported)
            || db.operations.contains_key(&(scope.clone(), key.clone()))
        {
            return invalid();
        }
        db.operations.insert(
            (scope.clone(), key.clone()),
            RecordedOperation {
                transaction: o.transaction.clone(),
                attempt: o.attempt,
                request: None,
                outcome: o.outcome.clone(),
            },
        );
    }
    Ok(())
}

fn lease_at(
    tx: &Transaction,
    fence: u64,
    owner: u64,
    at: u64,
    event_count: u64,
) -> Result<u64, RestoreError> {
    let count = usize::try_from(event_count).map_err(|_| RestoreError::Limit)?;
    if count == 0 || count > tx.lease_history.len() {
        return invalid();
    }
    let mut expiry = None;
    for e in &tx.lease_history[..count] {
        if e.at > at {
            return invalid();
        }
        if e.fence != fence {
            continue;
        }
        if e.authorization_grant != owner {
            return invalid();
        }
        match e.kind {
            LeaseEventKind::Acquired { expires_at }
            | LeaseEventKind::Renewed { expires_at, .. } => expiry = Some(expires_at),
            LeaseEventKind::Expired | LeaseEventKind::Revoked | LeaseEventKind::Released { .. } => {
                return invalid();
            }
        }
    }
    expiry
        .filter(|expiry| *expiry > at)
        .ok_or(RestoreError::BindingMismatch)
}

/// Pure historical metadata input, never runtime authorization. SQL loaders must
/// validate the complete assembled SourceRecords graph after restoring children.
pub struct SourceAdmissionRecordInput<'a> {
    pub request: &'a SourceRequestRecord,
    pub owner_grant: u64,
    pub policy: Digest,
    pub expected: SourceAdmissionId,
    pub bytes: &'a [u8],
    pub decode_limits: crate::persistence::DomainDecodeLimits,
}
impl SourceRecords {
    /// Nonallocating aggregate preflight for adapters before owning another copy
    /// or assembling derived reference projections. This does not validate trust.
    pub fn check_recovery_limits(&self, limits: NativeRecordLimits) -> Result<(), RestoreError> {
        check_source_budget(self, limits)
    }
    pub fn restore_admission(
        &self,
        input: SourceAdmissionRecordInput<'_>,
        limits: NativeRecordLimits,
    ) -> Result<SourceAdmission, RestoreError> {
        let mut cost = patch_cost(input.request.patch())?;
        cost.count(input.request.options().optional_checks.len())?;
        for check in &input.request.options().optional_checks {
            cost.text(check.as_str())?;
        }
        cost.bytes(input.bytes.len())?;
        let base = self
            .base
            .repository
            .as_ref()
            .and_then(|repo| {
                repo.registered
                    .get(&input.request.patch().binding().base_snapshot)
            })
            .ok_or(RestoreError::BindingMismatch)?;
        for _ in 0..4 {
            crate::persistence::source_snapshot_cost(&mut cost, base)?;
            cost.merge(patch_cost(input.request.patch())?)?;
        }
        check_source_budget_with_extra(self, limits, Some(cost))?;
        validate_catalogs(self)?;
        let db = records::restore_base(self.base.clone(), Some(self))?;
        reconstruct_admission(&db, self, input).map(|(admission, _)| admission)
    }
}
fn reconstruct_admission(
    db: &Database,
    r: &SourceRecords,
    input: SourceAdmissionRecordInput<'_>,
) -> Result<(SourceAdmission, crate::source_patch::AppliedSourcePatch), RestoreError> {
    let scope = &r.base.scope;
    let b = input.request.patch().binding();
    if b.scope != *scope {
        return invalid();
    }
    let tx = db
        .transactions
        .get(&(scope.clone(), b.transaction.clone()))
        .ok_or(RestoreError::BindingMismatch)?;
    let attempt = tx
        .attempts
        .get(b.transaction_attempt as usize)
        .ok_or(RestoreError::BindingMismatch)?;
    let source = r
        .analyses
        .get(&b.analysis_artifact)
        .ok_or(RestoreError::BindingMismatch)?;
    let policy = r
        .policies
        .get(&input.policy)
        .ok_or(RestoreError::BindingMismatch)?;
    let grant = db
        .grants
        .get(&input.owner_grant)
        .ok_or(RestoreError::BindingMismatch)?;
    if grant.principal != tx.owner
        || b.principal != tx.owner
        || !grant.permissions.contains(&Permission::Execute)
        || attempt.source_analysis != Some(source.digest())
        || attempt.source_policy != Some(policy.digest())
        || !tx.intent.invariants.is_empty()
    {
        return invalid();
    }
    let (_, impact, invariants) = source_attempts::derive_source_impact(
        source,
        attempt.model.generation,
        &tx.intent.accesses,
        Some(policy),
    )
    .map_err(|_| RestoreError::BindingMismatch)?;
    let request = source_attempts::intent_digest(&tx.intent);
    if b.impact_request != request
        || b.impact_result != source_attempts::impact_digest(request, &impact)
    {
        return invalid();
    }
    let application = input.request.patch().apply(&attempt.base)?;
    let plan = VerificationPlan::resolve(
        policy,
        SourcePlanBinding {
            scope: scope.clone(),
            base_snapshot: b.base_snapshot,
            analysis_artifact: b.analysis_artifact,
            index_generation: b.index_generation,
            impact_request: b.impact_request,
            impact_result: b.impact_result,
            patch: input.request.patch().id(),
            operation_digest: input.request.patch().operation_digest(),
            resulting_snapshot: application.snapshot().id(),
            resulting_manifest: application.snapshot().manifest().digest(),
        },
        PlanRequirements {
            declared_checks: tx.intent.required_checks.clone(),
            impact_checks: impact.justified_checks,
            invariants,
            optional_checks: input.request.options().optional_checks.clone(),
        },
    )?;
    let authorization = SourceAuthorizationProvenance {
        principal: grant.principal.clone(),
        capability_reference: source_attempts::authorization_digest(db, input.owner_grant)
            .map_err(|_| RestoreError::BindingMismatch)?,
    };
    let writes = &attempt
        .effective
        .as_ref()
        .ok_or(RestoreError::BindingMismatch)?
        .writes;
    let admission = restore_source_admission(
        SourceAdmissionDependencies {
            base: &attempt.base,
            patch: input.request.patch(),
            policy,
            plan: &plan,
            effective_writes: writes,
            authorization: &authorization,
        },
        input.expected,
        input.bytes,
        input.decode_limits,
    )?;
    Ok((admission, application))
}
fn restore_attempt(
    db: &mut Database,
    r: &SourceRecords,
    id: &SourceAttemptId,
    a: &SourceAttemptMetadata,
) -> Result<(), RestoreError> {
    let b = a.patch.binding();
    let scope = &r.base.scope;
    if b.scope != *scope
        || b.source_attempt != *id
        || a.admission.binding() != b
        || a.admission.admitted_at_ms() > db.now
    {
        return invalid();
    }
    a.options.validate()?;
    let request = SourceRequestRecord::new(a.patch.clone(), a.options.clone())?;
    let (admission, application) = reconstruct_admission(
        db,
        r,
        SourceAdmissionRecordInput {
            request: &request,
            owner_grant: a.owner_grant,
            policy: a.admission.policy(),
            expected: a.admission.id(),
            bytes: &a.admission.canonical_bytes(),
            decode_limits: Default::default(),
        },
    )?;
    if admission != a.admission {
        return invalid();
    }
    let tx = db
        .transactions
        .get(&(scope.clone(), b.transaction.clone()))
        .ok_or(RestoreError::BindingMismatch)?;
    let expiry = lease_at(
        tx,
        a.admission_fence,
        a.owner_grant,
        a.admission.admitted_at_ms(),
        a.admission_lease_event_count,
    )?;
    if b.expires_at_ms > expiry {
        return invalid();
    }
    validate_events(
        &a.events,
        a.state,
        a.admission.admitted_at_ms(),
        db.now,
        true,
    )?;
    if a.events
        .first()
        .is_none_or(|e| e.to != SourceAttemptState::Admitted)
    {
        return invalid();
    }
    let mut state = SourceAttemptState::Admitted;
    for e in &a.events {
        state = e.to;
    }
    if state != a.state {
        return invalid();
    }
    if let Some(executor) = a.executor_grant {
        let g = r
            .executors
            .get(&executor)
            .ok_or(RestoreError::BindingMismatch)?;
        let claim = a
            .events
            .iter()
            .find(|e| e.to == SourceAttemptState::Patching)
            .ok_or(RestoreError::BindingMismatch)?;
        if g.expires_at_ms <= claim.at_ms
            || a.admission.plan().checks().iter().any(|c| {
                c.definition().input().backend != g.backend
                    || !g.tiers.contains(&c.definition().input().tier)
                    || !c
                        .definition()
                        .input()
                        .resources
                        .is_within(&g.resource_ceiling)
            })
        {
            return invalid();
        }
    } else if a.events.iter().any(|e| {
        matches!(
            e.to,
            SourceAttemptState::Patching
                | SourceAttemptState::PatchApplied
                | SourceAttemptState::Verifying
                | SourceAttemptState::EvidenceSealed
        )
    }) {
        return invalid();
    }
    if !a.state.is_terminal()
        && db.source_attempts.values().any(|other| {
            other.admission.binding().transaction == b.transaction && !other.state.is_terminal()
        })
    {
        return invalid();
    }
    db.source_attempts.insert(
        (scope.clone(), id.clone()),
        SourceAttemptRecord {
            admission: a.admission.clone(),
            patch: a.patch.clone(),
            options: a.options.clone(),
            admission_fence: a.admission_fence,
            admission_lease_event_count: a.admission_lease_event_count,
            application,
            state: a.state,
            owner_capability: a.owner_grant,
            executor_capability: a.executor_grant,
            evidence: a.evidence,
            events: a.events.clone(),
        },
    );
    Ok(())
}
fn validate_events(
    events: &[SourceAttemptEvent],
    state: SourceAttemptState,
    at: u64,
    now: u64,
    worker_reason: bool,
) -> Result<(), RestoreError> {
    let mut previous = SourceAttemptState::Proposed;
    let mut previous_at = at;
    for (sequence, e) in events.iter().enumerate() {
        if e.sequence != sequence as u64
            || e.from != previous
            || e.at_ms < previous_at
            || e.at_ms > now
            || !e.from.allows(e.to)
            || e.reason.trim().is_empty()
            || (worker_reason && (e.reason.len() > 4096 || e.reason.chars().any(char::is_control)))
        {
            return invalid();
        }
        previous = e.to;
        previous_at = e.at_ms;
    }
    if events.first().is_none_or(|e| e.at_ms != at) || previous != state {
        return invalid();
    }
    Ok(())
}

fn restore_operations(db: &mut Database, r: &SourceRecords) -> Result<(), RestoreError> {
    let scope = &r.base.scope;
    let mut accepted = BTreeSet::new();
    for (key, o) in &r.operations {
        let p = o.request.patch();
        let b = p.binding();
        let grant = db
            .grants
            .get(&o.owner_grant)
            .ok_or(RestoreError::BindingMismatch)?;
        let tx = db
            .transactions
            .get(&(scope.clone(), b.transaction.clone()))
            .ok_or(RestoreError::BindingMismatch)?;
        if b.scope != *scope
            || grant.principal != tx.owner
            || o.target_attempt >= tx.attempts.len() as u64
            || o.recorded_at_ms > db.now
            || o.lease_event_count > tx.lease_history.len() as u64
            || o.observed_analysis
                .is_some_and(|d| !r.analyses.contains_key(&d))
            || o.observed_policy
                .is_some_and(|d| !r.policies.contains_key(&d))
            || o.observed_model_generation == 0
            || o.observed_model_generation > db.repositories[scope].model.generation
        {
            return invalid();
        }
        SourceOutcomeRecord::new(
            SourceOutcomeContext {
                scope: scope.clone(),
                transaction: b.transaction.clone(),
                target_attempt: o.target_attempt,
                key: key.clone(),
                request: o.request.request_digest(),
            },
            o.outcome.clone(),
        )?;
        if let Ok(id) = o.outcome {
            let a = db
                .source_attempts
                .get(&(scope.clone(), b.source_attempt.clone()))
                .ok_or(RestoreError::BindingMismatch)?;
            if a.admission.id() != id
                || a.owner_capability != o.owner_grant
                || a.admission.admitted_at_ms() != o.recorded_at_ms
                || a.admission_fence != o.fence.ok_or(RestoreError::BindingMismatch)?
                || a.patch != *p
                || a.options != *o.request.options()
                || b.transaction_attempt != o.target_attempt
                || a.admission_lease_event_count != o.lease_event_count
                || o.observed_analysis != Some(b.analysis_artifact)
                || o.observed_policy != Some(a.admission.policy())
                || o.observed_model_generation
                    != tx.attempts[o.target_attempt as usize].model.generation
                || !accepted.insert(b.source_attempt.clone())
            {
                return invalid();
            }
        }
        if let Err(error) = &o.outcome {
            if *error == EngineError::SourceAttempt(SourceAttemptError::AttemptIdentity) {
                if !r.attempts.contains_key(&b.source_attempt)
                    && !r.rejections.contains_key(&b.source_attempt)
                {
                    return invalid();
                }
            } else {
                let rejection = r
                    .rejections
                    .get(&b.source_attempt)
                    .ok_or(RestoreError::BindingMismatch)?;
                if r.attempts.contains_key(&b.source_attempt)
                    || rejection.request != o.request.request_digest()
                    || rejection.error != *error
                    || rejection.rejected_at_ms != o.recorded_at_ms
                {
                    return invalid();
                }
            }
        }
        db.source_operations.insert(
            (scope.clone(), key.clone()),
            RecordedSourceOperation {
                request: o.request.request_digest(),
                patch: p.clone(),
                options: o.request.options().clone(),
                target_attempt: o.target_attempt,
                recorded_at_ms: o.recorded_at_ms,
                fence: o.fence,
                lease_event_count: o.lease_event_count,
                observed_analysis: o.observed_analysis,
                observed_policy: o.observed_policy,
                observed_model_generation: o.observed_model_generation,
                attempt: b.source_attempt.clone(),
                owner_capability: o.owner_grant,
                outcome: o.outcome.clone(),
            },
        );
    }
    if accepted != r.attempts.keys().cloned().collect() {
        return invalid();
    }
    for (id, rejection) in &r.rejections {
        if rejection.scope != *scope
            || rejection.source_attempt != *id
            || r.attempts.contains_key(id)
        {
            return invalid();
        }
        let matching = r.operations.values().any(|o| {
            let b = o.request.patch().binding();
            b.transaction == rejection.transaction
                && b.transaction_attempt == rejection.transaction_attempt
                && b.source_attempt == *id
                && o.request.request_digest() == rejection.request
                && o.recorded_at_ms == rejection.rejected_at_ms
                && o.outcome == Err(rejection.error.clone())
                && db
                    .grants
                    .get(&o.owner_grant)
                    .is_some_and(|g| g.principal == rejection.owner)
        });
        if !matching {
            return invalid();
        }
        validate_events(
            &rejection.events,
            SourceAttemptState::Rejected,
            rejection.rejected_at_ms,
            db.now,
            false,
        )?;
        if rejection.events.len() != 1 || rejection.events[0].reason != rejection.error.to_string()
        {
            return invalid();
        }
        db.source_rejections
            .insert((scope.clone(), id.clone()), rejection.clone());
    }
    Ok(())
}
fn restore_evidence(db: &mut Database, r: &SourceRecords) -> Result<(), RestoreError> {
    let scope = &r.base.scope;
    let mut selected = BTreeSet::new();
    for a in r.attempts.values() {
        if let Some(id) = a.evidence {
            if !selected.insert(id) || !r.evidence.contains_key(&id) {
                return invalid();
            }
        } else if a.state == SourceAttemptState::EvidenceSealed {
            return invalid();
        }
    }
    if selected != r.evidence.keys().copied().collect() {
        return invalid();
    }
    for (id, e) in &r.evidence {
        let a = db
            .source_attempts
            .values()
            .find(|a| a.evidence == Some(*id))
            .ok_or(RestoreError::BindingMismatch)?;
        let executor = a
            .executor_capability
            .and_then(|id| db.source_executors.get(&id))
            .ok_or(RestoreError::BindingMismatch)?;
        let store = db
            .source_artifact_stores
            .get(&e.store_registration)
            .ok_or(RestoreError::BindingMismatch)?;
        let receipt = &e.receipt;
        if e.evidence.id() != *id
            || receipt.evidence != *id
            || receipt.admission != a.admission.id()
            || receipt.recorded_at_ms > db.now
            || receipt.recorded_at_ms < e.evidence.finished_at_ms()
            || receipt.recorded_at_ms >= a.admission.expires_at_ms()
            || receipt.recorded_at_ms >= executor.expires_at_ms
            || receipt.storage_provenance != store.provenance
            || store.retention != a.admission.retention_policy()
        {
            return invalid();
        }
        let assessment = e.evidence.validate_historical_metadata(
            &a.admission,
            &executor.principal,
            &receipt.envelope,
            &source_evidence_gate::source_provenance(db, a),
        )?;
        let expected_state = match assessment {
            SourceEvidenceAssessment::Passed => SourceAttemptState::EvidenceSealed,
            SourceEvidenceAssessment::Failed { .. } => SourceAttemptState::ExecutionFailed,
            SourceEvidenceAssessment::Incomplete { .. } => SourceAttemptState::EvidenceIncomplete,
        };
        if receipt.assessment != assessment
            || a.state != expected_state
            || a.events
                .last()
                .is_none_or(|event| event.at_ms != receipt.recorded_at_ms)
        {
            return invalid();
        }
        db.source_evidence_records.insert(
            (scope.clone(), *id),
            RetainedSourceEvidence {
                evidence: e.evidence.clone(),
                receipt: receipt.clone(),
                store_registration: e.store_registration,
            },
        );
    }
    Ok(())
}

fn check_source_budget(
    records: &SourceRecords,
    limits: NativeRecordLimits,
) -> Result<(), RestoreError> {
    check_source_budget_with_extra(records, limits, None)
}
fn check_source_budget_with_extra(
    records: &SourceRecords,
    limits: NativeRecordLimits,
    extra: Option<Cost>,
) -> Result<(), RestoreError> {
    if !records.base.evidence.is_empty() || !records.base.operations.is_empty() {
        return Err(RestoreError::UnsupportedDomainValue);
    }
    if records
        .base
        .repository
        .as_ref()
        .is_some_and(|repo| !repo.history.is_empty())
        || records.base.transactions.values().any(|tx| {
            tx.attempts
                .iter()
                .any(|a| a.patch.is_some() || a.candidate.is_some() || !a.evidence.is_empty())
        })
    {
        return Err(RestoreError::UnsupportedDomainValue);
    }
    let mut b = SourceBudget::new(limits)?;
    b.base(&records.base)?;
    if let Some(extra) = extra {
        b.value(extra)?;
    }
    for analysis in records.analyses.values() {
        b.value(analysis_cost(analysis)?)?;
    }
    for policy in records.policies.values() {
        b.value(policy_cost(policy)?)?;
    }
    b.entries(records.attempt_contexts.len())?;
    for g in records.executors.values() {
        let mut c = Cost::default();
        c.text(g.principal.as_str())?;
        c.text(&g.backend.name)?;
        c.text(&g.backend.version)?;
        c.count(g.tiers.len())?;
        b.value(c)?;
    }
    for g in records.cleanup.values() {
        let mut c = Cost::default();
        c.text(g.principal.as_str())?;
        b.value(c)?;
    }
    for store in records.artifact_stores.values() {
        let mut c = Cost::default();
        c.bytes(128)?;
        if let Some(adapter) = &store.adapter {
            c.text(adapter.identity.as_str())?;
            c.digest()?;
        }
        b.value(c)?;
    }
    for a in records.attempts.values() {
        let mut c = patch_cost(&a.patch)?;
        c.count(a.events.len())?;
        for e in &a.events {
            c.text(&e.reason)?;
        }
        c.count(a.options.optional_checks.len())?;
        for id in &a.options.optional_checks {
            c.text(id.as_str())?;
        }
        c.merge(plan_cost(a.admission.plan())?)?;
        // Patch replay, report provenance and later evidence validation use the
        // same base/candidate data. Reserve a conservative bound before either.
        let repo = records
            .base
            .repository
            .as_ref()
            .ok_or(RestoreError::BindingMismatch)?;
        let tx = records
            .base
            .transactions
            .get(&a.patch.binding().transaction)
            .ok_or(RestoreError::BindingMismatch)?;
        let base = tx
            .attempts
            .get(a.patch.binding().transaction_attempt as usize)
            .and_then(|a| repo.registered.get(&a.base))
            .ok_or(RestoreError::BindingMismatch)?;
        for _ in 0..4 {
            crate::persistence::source_snapshot_cost(&mut c, base)?;
            c.merge(patch_cost(&a.patch)?)?;
        }
        b.value(c)?;
    }
    for o in records.operations.values() {
        let mut c = patch_cost(o.request.patch())?;
        c.count(o.request.options().optional_checks.len())?;
        for id in &o.request.options().optional_checks {
            c.text(id.as_str())?;
        }
        if let Err(e) = &o.outcome {
            error_cost(&mut c, e)?;
        }
        b.value(c)?;
    }
    for r in records.rejections.values() {
        let mut c = Cost::default();
        c.count(r.events.len())?;
        for e in &r.events {
            c.text(&e.reason)?;
        }
        error_cost(&mut c, &r.error)?;
        b.value(c)?;
    }
    for e in records.evidence.values() {
        b.value(evidence_cost(&e.evidence, &e.receipt)?)?;
    }
    for o in records.unprepared_commits.values() {
        let mut c = Cost::default();
        c.text(o.transaction.as_str())?;
        if let Err(e) = &o.outcome {
            error_cost(&mut c, e)?;
        }
        b.value(c)?;
    }
    // Native common validators may now serialize bounded values. Their original
    // checks are preserved; source accounting above shares one aggregate budget.
    super::check_budget(&records.base, limits)
}
use crate::persistence::RecoveryCost as Cost;
struct SourceBudget {
    limits: NativeRecordLimits,
    entries: usize,
    bytes: usize,
}
impl SourceBudget {
    fn new(limits: NativeRecordLimits) -> Result<Self, RestoreError> {
        if limits.entries == 0 || limits.value_bytes == 0 || limits.total_value_bytes == 0 {
            return Err(RestoreError::Limit);
        }
        Ok(Self {
            limits,
            entries: 0,
            bytes: 0,
        })
    }
    fn entries(&mut self, n: usize) -> Result<(), RestoreError> {
        self.entries = self.entries.checked_add(n).ok_or(RestoreError::Limit)?;
        if self.entries > self.limits.entries {
            return Err(RestoreError::Limit);
        }
        Ok(())
    }
    fn value(&mut self, mut c: Cost) -> Result<(), RestoreError> {
        c.bytes(1024)?;
        self.entries(c.entries.checked_add(1).ok_or(RestoreError::Limit)?)?;
        self.bytes = self.bytes.checked_add(c.bytes).ok_or(RestoreError::Limit)?;
        if c.bytes > self.limits.value_bytes || self.bytes > self.limits.total_value_bytes {
            return Err(RestoreError::Limit);
        }
        Ok(())
    }
    fn base(&mut self, r: &BaseRecords) -> Result<(), RestoreError> {
        let mut c = Cost::default();
        for id in [
            r.scope.organization.as_str(),
            r.scope.workspace.as_str(),
            r.scope.repository.as_str(),
        ] {
            c.text(id)?;
        }
        self.value(c)?;
        self.entries(r.mutations.len())?;
        for (key, record) in &r.mutations {
            let size = mutation::recovery_size(&r.scope, key, record)?;
            self.value(Cost {
                entries: size.entries,
                bytes: size.bytes,
                max_field: size.bytes,
            })?;
        }
        self.entries(r.grants.len())?;
        for g in r.grants.values() {
            let mut c = Cost::default();
            c.text(g.principal.as_str())?;
            c.count(g.permissions.len())?;
            self.value(c)?;
        }
        if let Some(repo) = &r.repository {
            for snap in repo.registered.values() {
                let mut c = Cost::default();
                crate::persistence::source_snapshot_cost(&mut c, snap)?;
                self.value(c)?;
            }
            let mut c = Cost::default();
            model_cost(&mut c, &repo.model)?;
            self.value(c)?;
            self.entries(repo.history.len())?;
        }
        for (id, tx) in &r.transactions {
            let mut c = Cost::default();
            c.text(id.as_str())?;
            c.text(tx.owner.as_str())?;
            c.text(&tx.intent.objective)?;
            access_cost(&mut c, &tx.intent.accesses)?;
            c.count(tx.intent.required_checks.len())?;
            for check in &tx.intent.required_checks {
                c.spec(check)?;
            }
            c.count(tx.intent.depends_on.len())?;
            for id in &tx.intent.depends_on {
                c.text(id.as_str())?;
            }
            c.count(tx.intent.invariants.len())?;
            for i in &tx.intent.invariants {
                c.text(i.id.as_str())?;
                invariant_cost(&mut c, i)?;
            }
            c.count(tx.lease_history.len())?;
            for e in &tx.lease_history {
                c.count(e.scopes.len())?;
                for id in &e.scopes {
                    c.text(id.as_str())?;
                }
                if let LeaseEventKind::Released { reason } = &e.kind {
                    c.text(reason)?;
                }
            }
            if let Some(reason) = &tx.last_reason {
                c.text(reason)?;
            }
            self.value(c)?;
            for a in &tx.attempts {
                let mut c = Cost::default();
                model_cost(&mut c, &a.model)?;
                if let Some(e) = &a.effective {
                    access_cost(&mut c, e)?;
                }
                if let Some(analysis) = &a.analysis {
                    access_cost(&mut c, analysis.derived())?;
                    c.text(&analysis.stamp().analyzer)?;
                    c.text(&analysis.stamp().version)?;
                    if let crate::access::Completeness::Incomplete { reasons } =
                        analysis.completeness()
                    {
                        c.count(reasons.len())?;
                        for s in reasons {
                            c.text(s)?;
                        }
                    }
                }
                self.value(c)?;
            }
        }
        Ok(())
    }
}
fn access_cost(c: &mut Cost, a: &AccessSet) -> Result<(), RestoreError> {
    for set in [&a.reads, &a.writes, &a.invariant_scopes] {
        c.count(set.len())?;
        for id in set {
            c.text(id.as_str())?;
        }
    }
    c.count(a.predicates.len())?;
    for p in &a.predicates {
        match p {
            crate::access::Predicate::Prefix(s) => c.text(s)?,
        }
    }
    Ok(())
}
fn model_cost(c: &mut Cost, m: &EngineeringModel) -> Result<(), RestoreError> {
    c.text(&m.analyzer)?;
    c.text(&m.version)?;
    c.count(m.dependencies.len())?;
    for (id, deps) in &m.dependencies {
        c.text(id.as_str())?;
        c.count(deps.len())?;
        for id in deps {
            c.text(id.as_str())?;
        }
    }
    if let crate::access::Completeness::Incomplete { reasons } = &m.completeness {
        c.count(reasons.len())?;
        for s in reasons {
            c.text(s)?;
        }
    }
    Ok(())
}
fn patch_cost(p: &SourcePatch) -> Result<Cost, RestoreError> {
    let mut c = Cost::default();
    c.count(p.operations().len())?;
    c.text(&p.binding().producer)?;
    for id in [
        p.binding().scope.organization.as_str(),
        p.binding().scope.workspace.as_str(),
        p.binding().scope.repository.as_str(),
    ] {
        c.text(id)?;
    }
    for id in [
        p.binding().principal.as_str(),
        p.binding().transaction.as_str(),
        p.binding().source_attempt.as_str(),
    ] {
        c.text(id)?;
    }
    for op in p.operations() {
        c.text(op.source_path())?;
        if let Some(path) = op.destination_path() {
            c.text(path)?;
        }
        if let crate::source_patch::PatchOperation::Create { entry, .. }
        | crate::source_patch::PatchOperation::Replace { entry, .. } = op
        {
            c.bytes(entry.payload().bytes().len())?;
        }
        c.bytes(512)?;
    }
    Ok(c)
}
fn plan_cost(p: &VerificationPlan) -> Result<Cost, RestoreError> {
    let mut c = Cost::default();
    c.count(p.checks().len())?;
    for check in p.checks() {
        c.bytes(256)?;
        c.merge(crate::persistence::source_definition_cost(
            check.definition(),
        )?)?;
        c.count(check.rationales().len())?;
        for rationale in check.rationales() {
            c.number()?;
            match rationale {
                crate::source_execution::CheckRationale::Invariant(id) => c.text(id.as_str())?,
                crate::source_execution::CheckRationale::DependencyOf(id) => c.text(id.as_str())?,
                _ => {}
            }
        }
    }
    c.count(p.invariant_contracts().len())?;
    for id in p.invariant_contracts().keys() {
        c.text(id.as_str())?;
        c.digest()?;
    }
    Ok(c)
}
fn policy_cost(p: &SourceExecutionPolicy) -> Result<Cost, RestoreError> {
    crate::persistence::source_policy_cost(p)
}
fn artifact_cost(
    c: &mut Cost,
    o: &crate::source_evidence::SourceArtifactReference,
) -> Result<(), RestoreError> {
    for s in [
        o.scope.organization.as_str(),
        o.scope.workspace.as_str(),
        o.scope.repository.as_str(),
        &o.name,
        &o.media_type,
    ] {
        c.text(s)?;
    }
    c.bytes(256)
}
fn error_cost(c: &mut Cost, e: &EngineError) -> Result<(), RestoreError> {
    match e {
        EngineError::InvalidIntent(s) => c.text(s)?,
        EngineError::MissingEvidence(id) | EngineError::EvidenceNotPassed(id) => {
            c.text(id.as_str())?
        }
        EngineError::Invariant(error) => match error {
            crate::access::InvariantViolation::Missing { invariant, element } => {
                c.text(invariant.as_str())?;
                c.text(element.as_str())?;
            }
            crate::access::InvariantViolation::Unsatisfied { invariant }
            | crate::access::InvariantViolation::Overflow { invariant } => {
                c.text(invariant.as_str())?
            }
        },
        EngineError::Patch(error) => match error {
            crate::patch::PatchError::NoOp(id)
            | crate::patch::PatchError::BeforeImage { element: id, .. } => c.text(id.as_str())?,
            crate::patch::PatchError::Empty => {}
        },
        EngineError::SourceAttempt(e) => match e {
            SourceAttemptError::Plan(e) => plan_error_cost(c, e)?,
            SourceAttemptError::Patch(e) => {
                use crate::source_patch::SourcePatchError::*;
                match e {
                    InvalidPath(s)
                    | ReservedPath(s)
                    | ReservedEnvironment(s)
                    | ConflictingOperations(s)
                    | PreimageMismatch(s)
                    | OccupiedDestination(s)
                    | NoOp(s)
                    | ExcludedPath(s)
                    | PathCollision(s)
                    | SymlinkDenied(s)
                    | UnsafeSymlink(s) => c.text(s)?,
                    _ => {}
                }
            }
            SourceAttemptError::Credential(
                crate::credentials::CredentialError::SensitiveInput(s),
            ) => c.text(s)?,
            _ => {}
        },
        EngineError::SourceEvidence(e) => {
            use crate::source_evidence::{SourceArtifactError as A, SourceEvidenceError as E};
            match e {
                E::ArtifactMismatch(s) | E::Artifact(A::Unavailable(s) | A::Persistence(s)) => {
                    c.text(s)?
                }
                E::CheckMismatch(id) => c.text(id.as_str())?,
                _ => {}
            }
        }
        EngineError::RepositoryAnalysis(
            crate::repository::RepositoryAnalysisError::UnknownElement(id),
        ) => c.text(id.as_str())?,
        EngineError::Conflict(reports) => {
            c.count(reports.len())?;
            for r in reports {
                c.bytes(2048)?;
                c.text(r.contender.as_str())?;
                c.text(r.committed.as_str())?;
                c.count(r.conflicts.len())?;
                for v in &r.conflicts {
                    c.bytes(128)?;
                    c.text(v.element.as_str())?;
                    if let Some(crate::access::Predicate::Prefix(prefix)) = &v.predicate {
                        c.text(prefix)?;
                    }
                }
                for p in [&r.contender_provenance, &r.committed_provenance] {
                    c.text(&p.analysis.analyzer)?;
                    c.text(&p.analysis.version)?;
                    if let crate::access::Completeness::Incomplete { reasons } = &p.completeness {
                        c.count(reasons.len())?;
                        for text in reasons {
                            c.text(text)?;
                        }
                    }
                    access_cost(c, &p.explicit)?;
                    access_cost(c, &p.derived)?;
                    c.count(p.invariant_scopes.len())?;
                    for id in &p.invariant_scopes {
                        c.text(id.as_str())?;
                    }
                }
            }
        }
        _ => {}
    }
    c.bytes(1024)
}
pub(crate) fn error_recovery_size(
    error: &EngineError,
) -> Result<crate::persistence::RecoverySize, RestoreError> {
    let mut cost = Cost::default();
    error_cost(&mut cost, error)?;
    Ok(crate::persistence::RecoverySize {
        entries: cost.entries,
        bytes: cost.bytes,
    })
}
fn plan_error_cost(
    c: &mut Cost,
    e: &crate::source_execution::SourcePlanError,
) -> Result<(), RestoreError> {
    use crate::source_execution::SourcePlanError::*;
    match e {
        InvalidPath(s) | UnauthorizedPath(s) | PathPreimage(s) | CheckInputMismatch(s) => {
            c.text(s)?
        }
        DuplicateCheck(id)
        | MissingCheck(id)
        | SubstitutedCheck(id)
        | ResourceCeiling(id)
        | UnapprovedBackend(id)
        | ConflictingRequirement(id) => c.text(id.as_str())?,
        MissingInvariant(id) => c.text(id.as_str())?,
        Credential(crate::credentials::CredentialError::SensitiveInput(s)) => c.text(s)?,
        _ => {}
    }
    Ok(())
}

fn analysis_cost(a: &RepositoryAnalysis) -> Result<Cost, RestoreError> {
    let mut c = Cost::default();
    c.text(a.analyzer_version())?;
    c.count(a.elements().len())?;
    for id in a.elements() {
        c.text(id.as_str())?;
    }
    c.count(a.dependencies().len())?;
    for (id, deps) in a.dependencies() {
        c.text(id.as_str())?;
        c.count(deps.len())?;
        for id in deps {
            c.text(id.as_str())?;
        }
    }
    c.count(a.checks().len())?;
    for (id, checks) in a.checks() {
        c.text(id.as_str())?;
        c.count(checks.len())?;
        for id in checks {
            c.text(id.as_str())?;
        }
    }
    c.count(a.gaps().len())?;
    for gap in a.gaps() {
        c.number()?;
        c.text(&gap.stage)?;
        if let Some(path) = &gap.path {
            c.text(path)?;
        }
        c.text(&gap.detail)?;
    }
    Ok(c)
}
fn invariant_cost(c: &mut Cost, i: &crate::access::Invariant) -> Result<(), RestoreError> {
    use crate::access::Condition;
    match &i.condition {
        Condition::Equals { element, .. } | Condition::Present(element) => {
            c.text(element.as_str())?
        }
        Condition::SumAtMost { elements, .. } => {
            c.count(elements.len())?;
            for id in elements {
                c.text(id.as_str())?;
            }
        }
    }
    Ok(())
}

impl Administrator {
    /// Trusted read-only metadata inspection. This cannot attach readers, issue
    /// capabilities, or imply that required objects are still retrievable.
    pub fn inspect_source_records(
        &self,
        scope: &Scope,
        limits: NativeRecordLimits,
    ) -> Result<SourceRecords, EngineError> {
        self.store.administration(scope).read(|db| {
            check_database_budget(db, scope, limits)
                .map_err(|_| EngineError::Authority(AuthorityFailure::Integrity))?;
            let records = export(db, scope)
                .map_err(|_| EngineError::Authority(AuthorityFailure::Integrity))?;
            check_source_budget(&records, limits)
                .map_err(|_| EngineError::Authority(AuthorityFailure::Integrity))?;
            Ok(records)
        })
    }
}
// Bound the borrowed reference representation before export owns any copies.
fn check_database_budget(
    db: &Database,
    scope: &Scope,
    limits: NativeRecordLimits,
) -> Result<(), RestoreError> {
    let mut b = SourceBudget::new(limits)?;
    for ((stored, key), record) in &db.mutations {
        if stored == scope {
            let size = mutation::recovery_size(scope, key, record)?;
            b.value(Cost {
                entries: size.entries,
                bytes: size.bytes,
                max_field: size.bytes,
            })?;
        }
    }
    for g in db.grants.values().filter(|g| g.scope == *scope) {
        let mut c = Cost::default();
        c.text(g.principal.as_str())?;
        c.count(g.permissions.len())?;
        b.value(c)?;
    }
    if let Some(repo) = db.repositories.get(scope) {
        let mut c = Cost::default();
        model_cost(&mut c, &repo.model)?;
        b.value(c)?;
        for snap in repo.snapshots.values() {
            let mut c = Cost::default();
            crate::persistence::source_snapshot_cost(&mut c, snap)?;
            b.value(c)?;
        }
    }
    for ((s, id), tx) in &db.transactions {
        if s != scope {
            continue;
        }
        let mut c = Cost::default();
        c.text(id.as_str())?;
        c.text(&tx.intent.objective)?;
        c.text(tx.owner.as_str())?;
        access_cost(&mut c, &tx.intent.accesses)?;
        c.count(tx.intent.required_checks.len())?;
        for check in &tx.intent.required_checks {
            c.spec(check)?;
        }
        c.count(tx.intent.invariants.len())?;
        for i in &tx.intent.invariants {
            c.text(i.id.as_str())?;
            invariant_cost(&mut c, i)?;
        }
        c.count(tx.intent.depends_on.len())?;
        for id in &tx.intent.depends_on {
            c.text(id.as_str())?;
        }
        c.count(tx.lease_history.len())?;
        for e in &tx.lease_history {
            c.count(e.scopes.len())?;
            for id in &e.scopes {
                c.text(id.as_str())?;
            }
            if let LeaseEventKind::Released { reason } = &e.kind {
                c.text(reason)?;
            }
        }
        if let Some(reason) = &tx.last_reason {
            c.text(reason)?;
        }
        for a in &tx.attempts {
            model_cost(&mut c, &a.model)?;
            if let Some(a) = &a.effective {
                access_cost(&mut c, a)?;
            }
            if let Some(a) = &a.analysis {
                access_cost(&mut c, a.derived())?;
            }
        }
        b.value(c)?;
    }
    for ((s, _), a) in &db.source_analysis_records {
        if s == scope {
            b.value(analysis_cost(a)?)?;
        }
    }
    for ((s, _), p) in &db.source_policy_records {
        if s == scope {
            b.value(policy_cost(p)?)?;
        }
    }
    for g in db.source_executors.values().filter(|g| g.scope == *scope) {
        let mut c = Cost::default();
        c.text(g.principal.as_str())?;
        c.text(&g.backend.name)?;
        c.text(&g.backend.version)?;
        c.count(g.tiers.len())?;
        b.value(c)?;
    }
    for g in db
        .source_cleanup_grants
        .values()
        .filter(|g| g.scope == *scope)
    {
        let mut c = Cost::default();
        c.text(g.principal.as_str())?;
        b.value(c)?;
    }
    for store in db
        .source_artifact_stores
        .values()
        .filter(|g| g.scope == *scope)
    {
        let mut c = Cost::default();
        c.bytes(128)?;
        if let Some(adapter) = &store.adapter {
            c.text(adapter.identity.as_str())?;
            c.digest()?;
        }
        b.value(c)?;
    }
    for ((s, _), a) in &db.source_attempts {
        if s != scope {
            continue;
        }
        let mut c = patch_cost(&a.patch)?;
        c.merge(plan_cost(a.admission.plan())?)?;
        crate::persistence::source_snapshot_cost(&mut c, a.application.snapshot())?;
        crate::persistence::source_snapshot_cost(
            &mut c,
            &db.transactions[&(scope.clone(), a.admission.binding().transaction.clone())].attempts
                [a.admission.binding().transaction_attempt as usize]
                .base,
        )?;
        c.count(a.events.len())?;
        for e in &a.events {
            c.text(&e.reason)?;
        }
        c.count(a.options.optional_checks.len())?;
        for id in &a.options.optional_checks {
            c.text(id.as_str())?;
        }
        b.value(c)?;
    }
    for ((s, _), o) in &db.source_operations {
        if s != scope {
            continue;
        }
        let mut c = patch_cost(&o.patch)?;
        c.count(o.options.optional_checks.len())?;
        for id in &o.options.optional_checks {
            c.text(id.as_str())?;
        }
        if let Err(e) = &o.outcome {
            error_cost(&mut c, e)?;
        }
        b.value(c)?;
    }
    for ((s, _), r) in &db.source_rejections {
        if s != scope {
            continue;
        }
        let mut c = Cost::default();
        c.count(r.events.len())?;
        for e in &r.events {
            c.text(&e.reason)?;
        }
        error_cost(&mut c, &r.error)?;
        b.value(c)?;
    }
    for ((s, _), e) in &db.source_evidence_records {
        if s == scope {
            b.value(evidence_cost(&e.evidence, &e.receipt)?)?;
        }
    }
    for ((s, _), o) in &db.operations {
        if s == scope {
            let mut c = Cost::default();
            if let Err(e) = &o.outcome {
                error_cost(&mut c, e)?;
            }
            b.value(c)?;
        }
    }
    Ok(())
}

fn evidence_cost(
    e: &SourceEvidence,
    receipt: &SourceEvidenceReceipt,
) -> Result<Cost, RestoreError> {
    let mut c = Cost::default();
    for id in [
        e.binding().scope().organization.as_str(),
        e.binding().scope().workspace.as_str(),
        e.binding().scope().repository.as_str(),
        e.binding().executor().as_str(),
    ] {
        c.text(id)?;
    }
    c.count(e.checks().len())?;
    for check in e.checks().values() {
        let i = check.input();
        c.text(i.check.as_str())?;
        c.count(i.artifacts.len())?;
        for object in i.artifacts.values() {
            artifact_cost(&mut c, object)?;
        }
        c.count(i.diagnostics.len())?;
        for text in &i.diagnostics {
            c.text(text)?;
        }
        for reason in [
            match &i.outcome {
                crate::source_evidence::SourceCheckOutcome::Infrastructure { reason }
                | crate::source_evidence::SourceCheckOutcome::Skipped { reason }
                | crate::source_evidence::SourceCheckOutcome::UnknownTermination { reason } => {
                    Some(reason)
                }
                _ => None,
            },
            match &i.termination {
                crate::source_evidence::SourceTermination::Unknown { reason } => Some(reason),
                _ => None,
            },
            match &i.cleanup {
                crate::source_evidence::SourceCleanup::Quarantined { reason }
                | crate::source_evidence::SourceCleanup::Failed { reason } => Some(reason),
                _ => None,
            },
        ]
        .into_iter()
        .flatten()
        {
            c.text(reason)?;
        }
        c.bytes(1024)?;
    }
    c.count(e.provenance().len())?;
    for object in e.provenance().values() {
        artifact_cost(&mut c, object)?;
    }
    // Envelope diagnostics are exposed by the immutable type, never parsed
    // from Display text or treated as an attestation.
    c.count(e.diagnostics().len())?;
    for text in e.diagnostics() {
        c.text(text)?;
    }
    artifact_cost(&mut c, &receipt.envelope)?;
    match &receipt.assessment {
        SourceEvidenceAssessment::Passed => {}
        SourceEvidenceAssessment::Failed { checks } => {
            c.count(checks.len())?;
            for id in checks {
                c.text(id.as_str())?;
            }
        }
        SourceEvidenceAssessment::Incomplete { reasons } => {
            c.count(reasons.len())?;
            for text in reasons {
                c.text(text)?;
            }
        }
    }

    Ok(c)
}

fn ensure_scope(db: &Database, scope: &Scope) -> Result<(), RestoreError> {
    if db.repositories.keys().any(|s| s != scope)
        || db.grants.values().any(|g| g.scope != *scope)
        || db.transactions.keys().any(|(s, _)| s != scope)
        || db.evidence.keys().any(|(s, _)| s != scope)
        || db.evidence_sources.keys().any(|(s, _)| s != scope)
        || db.operations.keys().any(|(s, _)| s != scope)
        || db.mutations.keys().any(|(s, _)| s != scope)
        || db.reservation_fences.keys().any(|s| s != scope)
        || db.source_executors.values().any(|g| g.scope != *scope)
        || db.source_cleanup_grants.values().any(|g| g.scope != *scope)
        || db
            .source_artifact_stores
            .values()
            .any(|g| g.scope != *scope)
        || db.source_policies.keys().any(|s| s != scope)
        || db.source_policy_records.keys().any(|(s, _)| s != scope)
        || db.source_analysis_records.keys().any(|(s, _)| s != scope)
        || db.source_attempts.keys().any(|(s, _)| s != scope)
        || db.source_operations.keys().any(|(s, _)| s != scope)
        || db.source_rejections.keys().any(|(s, _)| s != scope)
        || db.source_evidence_records.keys().any(|(s, _)| s != scope)
    {
        return invalid();
    }
    Ok(())
}
