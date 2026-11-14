//! Independently installed object readers never come from request metadata.
use super::*;
use crate::source_evidence::{SourceArtifactError, SourceArtifactReference, SourceEvidence};

pub(super) struct ArtifactPreparation {
    admission: SourceAdmission,
    evidence: SourceEvidence,
    envelope: SourceArtifactReference,
    registration: u64,
    storage: source_evidence_gate::RegisteredSourceArtifactStore,
    reader: Arc<dyn SourceArtifactReader>,
    expected: BTreeMap<String, Vec<u8>>,
}
impl ArtifactPreparation {
    pub(super) fn new(
        engine: &Engine,
        db: &Database,
        scope: &Scope,
        request: &SourceCommitRequest,
    ) -> Result<Self, EngineError> {
        let source = source_record(db, scope, request.admission)?;
        let retained = db
            .source_evidence_records
            .get(&(scope.clone(), request.evidence))
            .ok_or(EngineError::NotFound)?;
        let storage = db
            .source_artifact_stores
            .get(&retained.store_registration)
            .ok_or(EngineError::Unauthorized)?
            .clone();
        let mut bytes = retained.receipt.envelope.bytes;
        for object in retained.evidence.provenance().values().chain(
            retained
                .evidence
                .checks()
                .values()
                .flat_map(|check| check.input().artifacts.values()),
        ) {
            bytes = bytes
                .checked_add(object.bytes)
                .ok_or(SourceEvidenceError::Artifact(SourceArtifactError::Limit))?;
        }
        if bytes
            > db.source_policies
                .get(scope)
                .ok_or(SourceAttemptError::PolicyChanged)?
                .input()
                .resource_ceiling
                .artifact_bytes
        {
            return Err(SourceEvidenceError::Artifact(SourceArtifactError::Limit).into());
        }
        Ok(Self {
            admission: source.admission.clone(),
            evidence: retained.evidence.clone(),
            envelope: retained.receipt.envelope.clone(),
            registration: retained.store_registration,
            storage,
            reader: engine
                .store
                .artifact_reader(scope, retained.store_registration)?,
            expected: source_evidence_gate::source_provenance(db, source),
        })
    }
    pub(super) fn validate(&self) -> Result<(), EngineError> {
        let assessment = self.evidence.validate_persisted(
            &self.admission,
            self.evidence.binding().executor(),
            &self.envelope,
            self.reader.as_ref(),
            &self.expected,
        )?;
        if assessment != SourceEvidenceAssessment::Passed {
            return Err(SourceEvidenceError::BindingMismatch.into());
        }
        Ok(())
    }
    pub(super) fn recheck(
        &self,
        engine: &Engine,
        db: &Database,
        scope: &Scope,
        request: &SourceCommitRequest,
    ) -> Result<(), AuthorityFailure> {
        let storage = db
            .source_artifact_stores
            .get(&self.registration)
            .ok_or(AuthorityFailure::Integrity)?;
        let reader = engine
            .store
            .artifact_reader(scope, self.registration)
            .map_err(|_| AuthorityFailure::Unavailable)?;
        if !Arc::ptr_eq(&reader, &self.reader)
            || storage.adapter != self.storage.adapter
            || storage.retention != self.storage.retention
            || storage.provenance != self.storage.provenance
            || !storage.active
            || request.evidence != self.evidence.id()
            || request.admission != self.admission.id()
            || request.store_registration != self.registration
        {
            return Err(AuthorityFailure::Contention);
        }
        Ok(())
    }
}
