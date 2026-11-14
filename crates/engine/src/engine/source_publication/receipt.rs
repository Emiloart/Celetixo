//! Source-specific historical publication facts. None is a live capability.
use super::*;
use crate::persistence::RestoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommitReceipt {
    pub scope: Scope,
    pub transaction: TransactionId,
    pub attempt: u64,
    pub source_attempt: SourceAttemptId,
    pub admission: SourceAdmissionId,
    pub patch: SourcePatchId,
    pub evidence: SourceEvidenceId,
    pub verified_snapshot: SnapshotId,
    pub publication_snapshot: SnapshotId,
    pub revision: Revision,
    pub model_generation: u64,
    pub analysis: Digest,
    pub request: Digest,
    pub committed_at_ms: u64,
}
impl SourceCommitReceipt {
    pub const FORMAT: &str = "celetixo/source-commit-receipt/v1";

    /// Check the result's relationship to original request metadata. This does
    /// not establish authorization, object availability or an admitted commit.
    pub fn validate_request(
        &self,
        scope: &Scope,
        principal: &PrincipalId,
        request: &SourceCommitRequest,
    ) -> Result<(), RestoreError> {
        let binding = &request.binding;
        if self.scope != *scope
            || binding.scope != *scope
            || binding.principal != *principal
            || self.transaction != binding.transaction
            || self.attempt != binding.transaction_attempt
            || self.source_attempt != binding.source_attempt
            || self.admission != request.admission
            || self.patch != request.patch
            || self.evidence != request.evidence
            || self.verified_snapshot != request.verified_snapshot
            || self.publication_snapshot != request.publication_snapshot
            || self.revision != request.publication_revision
            || self.revision.0
                != binding
                    .base_revision
                    .0
                    .checked_add(1)
                    .ok_or(RestoreError::BindingMismatch)?
            || self.analysis != request.analysis.digest()
            || self.model_generation == 0
            || request.analysis.snapshot() != self.publication_snapshot
            || request.analysis.scope() != scope
            || !request.analysis.is_complete()
            || self.request != request.digest(scope, principal)
            || request.authorization_grant == 0
            || request.analysis_grant == 0
            || request.fence == 0
            || request.store_registration == 0
            || self.committed_at_ms < binding.created_at_ms
            || self.committed_at_ms >= binding.expires_at_ms
        {
            return Err(RestoreError::BindingMismatch);
        }
        Ok(())
    }

    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        self.encode_fields(&mut c);
        c.into_bytes()
    }
    pub(crate) fn encode_fields(&self, c: &mut Canonical) {
        c.str(self.scope.organization.as_str());
        c.str(self.scope.workspace.as_str());
        c.str(self.scope.repository.as_str());
        c.str(self.transaction.as_str());
        c.u64(self.attempt);
        c.str(self.source_attempt.as_str());
        c.digest(self.admission.0);
        c.digest(self.patch.0);
        c.digest(self.evidence.0);
        c.digest(self.verified_snapshot.0);
        c.digest(self.publication_snapshot.0);
        c.u64(self.revision.0);
        c.u64(self.model_generation);
        c.digest(self.analysis);
        c.digest(self.request);
        c.u64(self.committed_at_ms);
    }
}

/// Engine-observed artifact validation, retained as historical provenance.
/// Its presence or successful decoding cannot authorize a new object read,
/// validate today's storage availability or replace a current commit gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommitArtifactRead {
    pub evidence: SourceEvidenceId,
    pub registration: u64,
    pub provenance: Digest,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommitSourceContext {
    pub admission: SourceAdmissionId,
    pub source_attempt: SourceAttemptId,
    pub state: SourceAttemptState,
    pub event_count: u64,
    pub evidence: Option<SourceEvidenceId>,
    pub executor_grant: Option<u64>,
    pub executor_active: Option<bool>,
    pub store_registration: Option<u64>,
    pub store_active: Option<bool>,
}

/// Original observed inputs, independent of the possibly invalid raw request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommitContext {
    pub analysis_principal: PrincipalId,
    pub analysis_grant: u64,
    pub head: Option<SnapshotId>,
    pub model_generation: Option<u64>,
    pub analysis: Option<Digest>,
    pub policy: Option<Digest>,
    pub source: Option<SourceCommitSourceContext>,
    pub artifacts: Option<SourceCommitArtifactRead>,
}
