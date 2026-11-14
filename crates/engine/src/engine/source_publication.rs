//! Read-only source publication preview for independently authorized analyzers.
//! A preview neither installs analyzer output nor grants commit authority.
use super::*;
use crate::source_evidence::SourceEvidenceAssessment;
use crate::source_publication::{SourcePublicationCandidate, SourcePublicationError};

mod commit;
mod gate;
mod receipt;
mod request;
pub(super) use gate::validate_source_commit_metadata;
pub use receipt::{
    SourceCommitArtifactRead, SourceCommitContext, SourceCommitReceipt, SourceCommitSourceContext,
};
pub use request::{SourceCommitPreparation, SourceCommitRequest};

impl Engine {
    /// Replay a sealed source result into its proposed revision successor.
    /// Scoped readers may analyze this candidate without receiving the owner's
    /// commit or model-management permission. No external object I/O is done;
    /// a later commit must independently verify current artifact availability.
    pub fn source_publication_candidate(
        &self,
        session: &Session,
        scope: &Scope,
        admission: SourceAdmissionId,
    ) -> Result<SourcePublicationCandidate, EngineError> {
        self.store
            .session(session, scope, Permission::Read)
            .read(|db| {
                self.authorize(db, session, scope, Permission::Read)?;
                publication_candidate_in(db, scope, admission)
            })
    }
}

fn source_record<'a>(
    db: &'a Database,
    scope: &Scope,
    admission: SourceAdmissionId,
) -> Result<&'a source_attempts::SourceAttemptRecord, EngineError> {
    db.source_attempts
        .iter()
        .find_map(|((s, _), record)| {
            (*s == *scope && record.admission.id() == admission).then_some(record)
        })
        .ok_or(EngineError::NotFound)
}

fn publication_candidate_in(
    db: &Database,
    scope: &Scope,
    admission: SourceAdmissionId,
) -> Result<SourcePublicationCandidate, EngineError> {
    let record = source_record(db, scope, admission)?;
    if record.state != SourceAttemptState::EvidenceSealed {
        return Err(SourceAttemptError::InvalidState.into());
    }
    let binding = record.admission.binding();
    let context = source_attempts::source_publication_context(
        db,
        scope,
        &binding.transaction,
        record.owner_capability,
    )?;
    if record.admission.expires_at_ms() <= db.now {
        return Err(SourceAttemptError::Expired.into());
    }
    if context.transaction_attempt != binding.transaction_attempt
        || context.base_snapshot != binding.base_snapshot
        || context.base_revision != binding.base_revision
        || context.principal != binding.principal
        || context.policy != record.admission.policy()
        || context.analysis_artifact != binding.analysis_artifact
        || context.index_generation != binding.index_generation
        || context.impact_request != binding.impact_request
        || context.impact_result != binding.impact_result
    {
        return Err(SourceAttemptError::BindingMismatch.into());
    }
    let tx = db
        .transactions
        .get(&(scope.clone(), binding.transaction.clone()))
        .ok_or(EngineError::NotFound)?;
    if tx
        .lease
        .as_ref()
        .is_none_or(|l| l.fence != record.admission_fence)
    {
        return Err(EngineError::InvalidLease);
    }
    let evidence_id = record.evidence.ok_or(SourceAttemptError::InvalidState)?;
    let retained = db
        .source_evidence_records
        .get(&(scope.clone(), evidence_id))
        .ok_or(EngineError::NotFound)?;
    if retained.receipt.admission != admission
        || retained.receipt.evidence != evidence_id
        || retained.evidence.id() != evidence_id
        || retained.evidence.binding().admission() != admission
        || retained.receipt.assessment != SourceEvidenceAssessment::Passed
    {
        return Err(SourceAttemptError::BindingMismatch.into());
    }
    let policy = db
        .source_policies
        .get(scope)
        .ok_or(SourceAttemptError::PolicyChanged)?;
    let expected = SourceAdmission::restore_checked(
        &tx.attempt().base,
        &record.patch,
        policy,
        record.admission.plan().clone(),
        SourceAuthorizationProvenance {
            principal: tx.owner.clone(),
            capability_reference: source_attempts::authorization_digest(
                db,
                record.owner_capability,
            )?,
        },
        record.admission.admitted_at_ms(),
        &tx.attempt()
            .effective
            .as_ref()
            .ok_or(EngineError::IncompleteAnalysis)?
            .writes,
    )?;
    if expected != record.admission {
        return Err(SourceAttemptError::BindingMismatch.into());
    }
    let candidate = SourcePublicationCandidate::new(&tx.attempt().base, &record.patch)
        .map_err(publication_error)?;
    if candidate.application() != &record.application {
        return Err(SourceAttemptError::BindingMismatch.into());
    }
    Ok(candidate)
}

fn publication_error(error: SourcePublicationError) -> EngineError {
    match error {
        SourcePublicationError::Patch(error) => SourceAttemptError::Patch(error).into(),
        SourcePublicationError::Snapshot(error) => error.into(),
        SourcePublicationError::Analysis(error) => error.into(),
        SourcePublicationError::RevisionExhausted => EngineError::CounterExhausted,
        SourcePublicationError::IncompleteAnalysis => EngineError::IncompleteAnalysis,
        SourcePublicationError::BindingMismatch => SourceAttemptError::BindingMismatch.into(),
    }
}
