//! Exact source request values. Preparation and serialization grant no commit
//! permission; the effectful publication gate authenticates and validates anew.
use super::*;
use crate::persistence::SourceEvidenceReceiptRecord;
use crate::source_patch::PatchBinding;
use crate::source_publication::SourcePublicationAnalysis;

pub struct SourceCommitPreparation {
    pub admission: SourceAdmissionId,
    pub analysis: RepositoryAnalysis,
    pub key: IdempotencyKey,
}

/// Original selectors, including rejected values, rather than live authority.
/// The source patch/evidence IDs remain distinct from native integer-patch IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommitRequest {
    pub binding: PatchBinding,
    pub authorization_grant: u64,
    pub fence: u64,
    pub analysis_principal: PrincipalId,
    pub analysis_grant: u64,
    pub admission: SourceAdmissionId,
    pub patch: SourcePatchId,
    pub operations: Digest,
    pub policy: Digest,
    pub policy_generation: u64,
    pub plan: SourcePlanId,
    pub verified_snapshot: SnapshotId,
    pub verified_manifest: Digest,
    pub application_report: Digest,
    pub publication_snapshot: SnapshotId,
    pub publication_revision: Revision,
    pub publication_proof: Digest,
    pub evidence: SourceEvidenceId,
    pub evidence_receipt: Digest,
    pub store_registration: u64,
    pub storage_provenance: Digest,
    pub analysis: RepositoryAnalysis,
    pub key: IdempotencyKey,
}
impl SourceCommitRequest {
    pub const FORMAT: &str = "celetixo/source-commit-request/v1";
    pub fn digest(&self, scope: &Scope, principal: &PrincipalId) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes(scope, principal))
    }
    /// The key selects an original outcome and is excluded from semantic body
    /// identity. Scope and both independently authenticated actors are included.
    pub fn canonical_bytes(&self, scope: &Scope, principal: &PrincipalId) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(scope.organization.as_str());
        c.str(scope.workspace.as_str());
        c.str(scope.repository.as_str());
        c.str(principal.as_str());
        self.encode_body(&mut c);
        c.into_bytes()
    }
    pub(crate) fn encode_body(&self, c: &mut Canonical) {
        crate::source_patch::encode_binding(c, &self.binding);
        c.u64(self.authorization_grant);
        c.u64(self.fence);
        c.str(self.analysis_principal.as_str());
        c.u64(self.analysis_grant);
        c.digest(self.admission.0);
        c.digest(self.patch.0);
        c.digest(self.operations);
        c.digest(self.policy);
        c.u64(self.policy_generation);
        c.digest(self.plan.0);
        c.digest(self.verified_snapshot.0);
        c.digest(self.verified_manifest);
        c.digest(self.application_report);
        c.digest(self.publication_snapshot.0);
        c.u64(self.publication_revision.0);
        c.digest(self.publication_proof);
        c.digest(self.evidence.0);
        c.digest(self.evidence_receipt);
        c.u64(self.store_registration);
        c.digest(self.storage_provenance);
        c.digest(self.analysis.digest());
        c.bytes(&self.analysis.canonical_bytes());
    }

    pub(crate) fn recovery_size(
        &self,
    ) -> Result<crate::persistence::RecoverySize, crate::persistence::RestoreError> {
        let mut size = mutation::repository_analysis_size(&self.analysis)?;
        size.bytes = size
            .bytes
            .checked_add(16 * 1024)
            .and_then(|n| n.checked_add(self.binding.producer.len()))
            .ok_or(crate::persistence::RestoreError::Limit)?;
        Ok(size)
    }
}

impl Engine {
    /// Prepare exact request metadata under two independent opaque sessions.
    /// The committer needs Commit; the analyzer approver needs ManageModel.
    /// Neither role must acquire the other role's permission. Source publication
    /// freshly authenticates both actors and rechecks the entire gate.
    pub fn prepare_source_commit_request(
        &self,
        owner: &Session,
        analysis_authority: &Session,
        scope: &Scope,
        preparation: SourceCommitPreparation,
    ) -> Result<SourceCommitRequest, EngineError> {
        // Measure caller-owned facts before a database callback or any replay,
        // clone or canonical allocation. This uses the existing accounting
        // contract; the allowance bounds the fixed selectors and patch binding
        // (including its bounded 4096-byte producer) retained by the Engine.
        let limits = authority::NativeRecordLimits::default();
        let size = mutation::repository_analysis_size(&preparation.analysis).map_err(|_| {
            EngineError::InvalidIntent("source commit analysis exceeds recovery limits".into())
        })?;
        if size.entries > limits.entries
            || size
                .bytes
                .checked_add(16 * 1024)
                .is_none_or(|n| n > limits.value_bytes)
        {
            return Err(EngineError::InvalidIntent(
                "source commit analysis exceeds recovery limits".into(),
            ));
        }
        self.store
            .session(owner, scope, Permission::Commit)
            .read(|db| {
                self.authorize(db, owner, scope, Permission::Commit)?;
                self.authorize(db, analysis_authority, scope, Permission::ManageModel)?;
                let record = source_record(db, scope, preparation.admission)?;
                let binding = record.admission.binding();
                if owner.principal != binding.principal || owner.token != record.owner_capability {
                    return Err(EngineError::Unauthorized);
                }
                let candidate = publication_candidate_in(db, scope, preparation.admission)?;
                let tx = db
                    .transactions
                    .get(&(scope.clone(), binding.transaction.clone()))
                    .ok_or(EngineError::NotFound)?;
                require_lease(db, tx, owner)?;
                let publication = SourcePublicationAnalysis::new(
                    &tx.attempt().base,
                    &record.patch,
                    preparation.analysis,
                )
                .map_err(publication_error)?;
                if publication.candidate() != &candidate {
                    return Err(SourceAttemptError::BindingMismatch.into());
                }
                let evidence_id = record.evidence.ok_or(SourceAttemptError::InvalidState)?;
                let retained = db
                    .source_evidence_records
                    .get(&(scope.clone(), evidence_id))
                    .ok_or(EngineError::NotFound)?;
                let receipt = SourceEvidenceReceiptRecord::new(
                    scope.clone(),
                    retained.store_registration,
                    retained.receipt.clone(),
                )
                .map_err(|_| SourceAttemptError::BindingMismatch)?;
                Ok(SourceCommitRequest {
                    binding: binding.clone(),
                    authorization_grant: owner.token,
                    fence: tx.lease.as_ref().ok_or(EngineError::InvalidLease)?.fence,
                    analysis_principal: analysis_authority.principal.clone(),
                    analysis_grant: analysis_authority.token,
                    admission: preparation.admission,
                    patch: record.patch.id(),
                    operations: record.patch.operation_digest(),
                    policy: record.admission.policy(),
                    policy_generation: record.admission.policy_generation(),
                    plan: record.admission.plan().id(),
                    verified_snapshot: record.admission.resulting_snapshot(),
                    verified_manifest: record.admission.resulting_manifest(),
                    application_report: record.admission.application_report(),
                    publication_snapshot: candidate.snapshot().id(),
                    publication_revision: candidate.snapshot().revision(),
                    publication_proof: publication.digest(),
                    evidence: evidence_id,
                    evidence_receipt: receipt.digest(),
                    store_registration: retained.store_registration,
                    storage_provenance: retained.receipt.storage_provenance,
                    analysis: publication.analysis().clone(),
                    key: preparation.key,
                })
            })
    }
}
