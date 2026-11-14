//! Pure authoritative-metadata validation. A result is not a commit permit:
//! object I/O, final recheck and atomic publication are separate required steps.
use super::*;
use crate::persistence::SourceEvidenceReceiptRecord;
use crate::source_evidence::{SourceAttestationLevel, SourceEvidenceError};
use crate::source_publication::SourcePublicationAnalysis;

pub(in crate::engine) struct SourceCommitSelection {
    pub receipt: SourceCommitReceipt,
    pub publication: SourcePublicationAnalysis,
}

/// Actor metadata is attribution, never authentication. Live effectful callers
/// must independently authenticate the actual two opaque Sessions first.
pub(in crate::engine) fn validate_source_commit_metadata(
    db: &Database,
    scope: &Scope,
    owner: &MutationActor,
    analyst: &MutationActor,
    request: &SourceCommitRequest,
) -> Result<SourceCommitSelection, EngineError> {
    let MutationActor::Session { principal, grant } = owner else {
        return Err(EngineError::Unauthorized);
    };
    let MutationActor::Session {
        principal: analyst_principal,
        grant: analyst_grant,
    } = analyst
    else {
        return Err(EngineError::Unauthorized);
    };
    for (principal, grant, permission) in [
        (principal, grant, Permission::Commit),
        (analyst_principal, analyst_grant, Permission::ManageModel),
    ] {
        let actor = db.grants.get(grant).ok_or(EngineError::Unauthorized)?;
        if !actor.active
            || actor.scope != *scope
            || actor.principal != *principal
            || !actor.permissions.contains(&permission)
        {
            return Err(EngineError::Unauthorized);
        }
    }
    if request.authorization_grant != *grant
        || request.binding.scope != *scope
        || request.binding.principal != *principal
        || request.analysis_grant != *analyst_grant
        || request.analysis_principal != *analyst_principal
    {
        return Err(EngineError::Unauthorized);
    }
    let source = source_record(db, scope, request.admission)?;
    let admission = &source.admission;
    if source.owner_capability != *grant || admission.binding().principal != *principal {
        return Err(EngineError::Unauthorized);
    }
    if request.binding != *admission.binding()
        || request.patch != admission.patch()
        || request.operations != admission.operation_digest()
        || request.policy != admission.policy()
        || request.policy_generation != admission.policy_generation()
        || request.plan != admission.plan().id()
        || request.verified_snapshot != admission.resulting_snapshot()
        || request.verified_manifest != admission.resulting_manifest()
        || request.application_report != admission.application_report()
    {
        return Err(EngineError::CommitRequestMismatch);
    }
    let tx = db
        .transactions
        .get(&(scope.clone(), request.binding.transaction.clone()))
        .ok_or(EngineError::NotFound)?;
    if tx.owner != *principal {
        return Err(EngineError::Unauthorized);
    }
    if tx.attempt().number != request.binding.transaction_attempt
        || tx.state != TransactionState::Validating
        || source.state != SourceAttemptState::EvidenceSealed
    {
        return Err(SourceAttemptError::InvalidState.into());
    }
    let lease = tx.lease.as_ref().ok_or(EngineError::InvalidLease)?;
    if lease.fence != request.fence || request.fence != source.admission_fence {
        return Err(EngineError::InvalidLease);
    }
    let candidate = publication_candidate_in(db, scope, request.admission)?;
    if request.publication_snapshot != candidate.snapshot().id()
        || request.publication_revision != candidate.snapshot().revision()
        || request
            .analysis
            .validate(candidate.snapshot(), request.analysis.index_generation())
            .is_err()
    {
        return Err(EngineError::CommitRequestMismatch);
    }
    if !request.analysis.is_complete() {
        return Err(EngineError::IncompleteAnalysis);
    }
    let publication =
        SourcePublicationAnalysis::new(&tx.attempt().base, &source.patch, request.analysis.clone())
            .map_err(publication_error)?;
    if publication.digest() != request.publication_proof {
        return Err(EngineError::CommitRequestMismatch);
    }
    let evidence_id = source.evidence.ok_or(SourceAttemptError::InvalidState)?;
    let retained = db
        .source_evidence_records
        .get(&(scope.clone(), evidence_id))
        .ok_or(EngineError::NotFound)?;
    let receipt = SourceEvidenceReceiptRecord::new(
        scope.clone(),
        retained.store_registration,
        retained.receipt.clone(),
    )
    .map_err(|_| SourceEvidenceError::BindingMismatch)?;
    if request.evidence != evidence_id
        || request.evidence_receipt != receipt.digest()
        || request.store_registration != retained.store_registration
        || request.storage_provenance != retained.receipt.storage_provenance
    {
        return Err(EngineError::CommitRequestMismatch);
    }
    let storage = db
        .source_artifact_stores
        .get(&retained.store_registration)
        .ok_or(EngineError::Unauthorized)?;
    if !storage.active
        || storage.scope != *scope
        || storage.provenance != retained.receipt.storage_provenance
        || storage.retention != admission.retention_policy()
    {
        return Err(EngineError::Unauthorized);
    }
    let executor = source
        .executor_capability
        .and_then(|id| db.source_executors.get(&id))
        .ok_or(SourceAttemptError::UntrustedExecutor)?;
    if !executor.active
        || executor.scope != *scope
        || executor.expires_at_ms <= db.now
        || &executor.principal != retained.evidence.binding().executor()
        || retained.evidence.binding().attestation() != SourceAttestationLevel::EngineScopedExecutor
    {
        return Err(SourceAttemptError::UntrustedExecutor.into());
    }
    for check in admission.plan().checks() {
        let check = check.definition().input();
        if executor.backend != check.backend
            || !executor.tiers.contains(&check.tier)
            || !check.resources.is_within(&executor.resource_ceiling)
        {
            return Err(SourceAttemptError::UntrustedExecutor.into());
        }
    }
    let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
    check_intervening(tx, repo)?;
    for dependency in &tx.intent.depends_on {
        if !db
            .transactions
            .get(&(scope.clone(), dependency.clone()))
            .is_some_and(|other| other.state == TransactionState::Committed)
            || !repo.history.iter().any(|change| {
                change.transaction == *dependency && change.revision <= repo.current.revision()
            })
        {
            return Err(EngineError::UnsatisfiedDependency);
        }
    }
    let model_generation = repo
        .model
        .generation
        .checked_add(1)
        .ok_or(EngineError::CounterExhausted)?;
    let receipt = SourceCommitReceipt {
        scope: scope.clone(),
        transaction: tx.id.clone(),
        attempt: tx.attempt().number,
        source_attempt: request.binding.source_attempt.clone(),
        admission: admission.id(),
        patch: source.patch.id(),
        evidence: evidence_id,
        verified_snapshot: source.application.snapshot().id(),
        publication_snapshot: candidate.snapshot().id(),
        revision: candidate.snapshot().revision(),
        model_generation,
        analysis: publication.analysis().digest(),
        request: request.digest(scope, principal),
        committed_at_ms: db.now,
    };
    Ok(SourceCommitSelection {
        receipt,
        publication,
    })
}
