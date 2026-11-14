//! Deterministic source publication values, without transaction authority.
//!
//! Verification binds the original application snapshot. Advancing repository
//! revision changes snapshot identity, so a prepared source commit must retain
//! both identities and prove that the published content is exactly that result.
use crate::ids::{Canonical, Digest, Revision, SnapshotId, SourcePatchId};
use crate::repository::{RepositoryAnalysis, RepositoryAnalysisError};
use crate::snapshot::{Snapshot, SnapshotError};
use crate::source_patch::{AppliedSourcePatch, SourcePatch, SourcePatchError};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourcePublicationError {
    #[error(transparent)]
    Patch(#[from] SourcePatchError),
    #[error("source publication revision space is exhausted")]
    RevisionExhausted,
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Analysis(#[from] RepositoryAnalysisError),
    #[error("source publication analysis contains unresolved gaps")]
    IncompleteAnalysis,
    #[error("publication candidate differs from exact source replay and revision advancement")]
    BindingMismatch,
}

/// Complete analyzer output for an exact proposed publication. This is an
/// immutable value, not an installed model or authenticated staging decision.
/// In particular, constructing or restoring it cannot authorize publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePublicationAnalysis {
    base: SnapshotId,
    patch: SourcePatchId,
    candidate: SourcePublicationCandidate,
    analysis: RepositoryAnalysis,
}
impl SourcePublicationAnalysis {
    pub const FORMAT: &str = "celetixo/source-publication-analysis/v1";

    pub fn new(
        base: &Snapshot,
        patch: &SourcePatch,
        analysis: RepositoryAnalysis,
    ) -> Result<Self, SourcePublicationError> {
        let candidate = SourcePublicationCandidate::new(base, patch)?;
        analysis.validate(candidate.snapshot(), analysis.index_generation())?;
        if !analysis.is_complete() {
            return Err(SourcePublicationError::IncompleteAnalysis);
        }
        Ok(Self {
            base: base.id(),
            patch: patch.id(),
            candidate,
            analysis,
        })
    }

    pub fn base(&self) -> SnapshotId {
        self.base
    }
    pub fn patch(&self) -> SourcePatchId {
        self.patch
    }
    pub fn candidate(&self) -> &SourcePublicationCandidate {
        &self.candidate
    }
    pub fn analysis(&self) -> &RepositoryAnalysis {
        &self.analysis
    }
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }

    /// Canonical storage preimage, separate from the service protocol. Preserve
    /// both the verification-bound application and the revision successor.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        let scope = self.candidate.snapshot().scope();
        c.str(scope.organization.as_str());
        c.str(scope.workspace.as_str());
        c.str(scope.repository.as_str());
        c.digest(self.base.0);
        c.u64(self.candidate.application().snapshot().revision().0);
        c.digest(self.patch.0);
        c.digest(self.candidate.application().snapshot().id().0);
        c.digest(self.candidate.application().report().digest());
        c.digest(self.candidate.snapshot().id().0);
        c.u64(self.candidate.snapshot().revision().0);
        c.digest(self.analysis.digest());
        // Inline fields share a single decoding budget; do not reset it for a
        // nested analyzer record or trust its self-supplied content address.
        c.bytes(&self.analysis.canonical_bytes());
        c.into_bytes()
    }

    pub fn validate(
        &self,
        base: &Snapshot,
        patch: &SourcePatch,
    ) -> Result<(), SourcePublicationError> {
        if *self != Self::new(base, patch, self.analysis.clone())? {
            return Err(SourcePublicationError::BindingMismatch);
        }
        Ok(())
    }
}

/// A proposed revision successor, never an admitted commit or selected head.
/// Authorization, semantic analysis, invariants, evidence, leases, OCC and
/// atomic publication must still be decided by the Engine commit gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePublicationCandidate {
    application: AppliedSourcePatch,
    snapshot: Snapshot,
}
impl SourcePublicationCandidate {
    pub fn new(base: &Snapshot, patch: &SourcePatch) -> Result<Self, SourcePublicationError> {
        // Preserve the published source-application format and its base revision.
        // Do not rewrite an evidence-bound snapshot into another revision in place.
        let application = patch.apply(base)?;
        let revision = Revision(
            base.revision()
                .0
                .checked_add(1)
                .ok_or(SourcePublicationError::RevisionExhausted)?,
        );
        let snapshot = application
            .snapshot()
            .successor(revision, application.snapshot().state().clone())?;
        Ok(Self {
            application,
            snapshot,
        })
    }

    /// Original exact application and report, as selected by verification.
    pub fn application(&self) -> &AppliedSourcePatch {
        &self.application
    }

    /// Exact manifest/content with one revision increment. Constructing this
    /// value does not install it in an Engine repository or authorize a commit.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Recompute both values from the immutable original base and full patch.
    /// Matching self-supplied digests alone cannot establish this relation.
    pub fn validate(
        &self,
        base: &Snapshot,
        patch: &SourcePatch,
    ) -> Result<(), SourcePublicationError> {
        if *self != Self::new(base, patch)? {
            return Err(SourcePublicationError::BindingMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
