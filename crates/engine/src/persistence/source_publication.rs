//! Checked publication analysis values. No head, grant or receipt is installed.
use super::source_execution::{Cost, charge};
use super::*;
use crate::source_patch::{PatchOperation, SourcePatch};
use crate::source_publication::{SourcePublicationAnalysis, SourcePublicationError};

pub fn restore_source_publication_analysis(
    base: &Snapshot,
    patch: &SourcePatch,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourcePublicationAnalysis, RestoreError> {
    let mut r = Reader::new(expected, SourcePublicationAnalysis::FORMAT, bytes, limits)?;
    if r.scope()? != *base.scope()
        || r.digest()? != base.id().0
        || r.number()? != base.revision().0
        || r.digest()? != patch.id().0
    {
        return Err(RestoreError::BindingMismatch);
    }
    let application = r.digest()?;
    let report = r.digest()?;
    let publication = r.digest()?;
    let revision = r.number()?;
    let analysis_digest = r.digest()?;
    let analysis_bytes = r.field()?;
    r.end()?;

    // Bound dependencies before replay creates application/publication copies.
    // Analyzer bytes are already charged to the enclosing encoded byte budget.
    charge(&mut r, bytes.len(), replay_cost(base, patch)?)?;
    let mut analyzer = Reader::new(
        analysis_digest,
        "celetixo/repository-analysis/v1",
        analysis_bytes,
        DomainDecodeLimits {
            collection_entries: limits.collection_entries - r.entries,
            ..limits
        },
    )?;
    let analysis = semantic::decode_original_analysis(&mut analyzer)?;
    analyzer.end()?;
    if analysis.digest() != analysis_digest || analysis.canonical_bytes() != analysis_bytes {
        return Err(RestoreError::NonCanonical);
    }
    let result =
        SourcePublicationAnalysis::new(base, patch, analysis).map_err(|error| match error {
            SourcePublicationError::Patch(error) => RestoreError::SourcePatch(error),
            SourcePublicationError::Snapshot(error) => RestoreError::Snapshot(error),
            SourcePublicationError::Analysis(error) => RestoreError::RepositoryAnalysis(error),
            _ => RestoreError::InvalidDomainValue,
        })?;
    let candidate = result.candidate();
    if application != candidate.application().snapshot().id().0
        || report != candidate.application().report().digest()
        || publication != candidate.snapshot().id().0
        || revision != candidate.snapshot().revision().0
    {
        return Err(RestoreError::BindingMismatch);
    }
    lifecycle::canonical(result.digest(), expected, &result.canonical_bytes(), bytes)?;
    Ok(result)
}

fn replay_cost(base: &Snapshot, patch: &SourcePatch) -> Result<Cost, RestoreError> {
    let mut cost = Cost::default();
    // Original dependencies, application and revision successor. Each result
    // can contain at most the original input plus every patch payload/path.
    for _ in 0..3 {
        super::source_admission::snapshot_cost(&mut cost, base)?;
        cost.count(patch.operations().len())?;
        for text in [
            patch.binding().scope.organization.as_str(),
            patch.binding().scope.workspace.as_str(),
            patch.binding().scope.repository.as_str(),
            patch.binding().principal.as_str(),
            patch.binding().transaction.as_str(),
            patch.binding().source_attempt.as_str(),
            &patch.binding().producer,
        ] {
            cost.text(text)?;
        }
        for operation in patch.operations() {
            cost.text(operation.source_path())?;
            if let Some(path) = operation.destination_path() {
                cost.text(path)?;
            }
            for _ in 0..10 {
                cost.digest()?;
            }
            if let PatchOperation::Create { entry, .. } | PatchOperation::Replace { entry, .. } =
                operation
            {
                let len = entry.payload().bytes().len();
                cost.max_field = cost.max_field.max(len);
                cost.bytes(8)?;
                cost.bytes(len)?;
            }
        }
    }
    cost.bytes(4096)?;
    cost.max_field = cost.max_field.max(512);
    cost.count(8)?;
    Ok(cost)
}
