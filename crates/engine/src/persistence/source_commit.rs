//! Original source commit selectors. Restoration grants no approval, permission,
//! current artifact availability or commit authority.
use super::*;
use crate::engine::{
    SourceCommitArtifactRead, SourceCommitContext, SourceCommitReceipt, SourceCommitRequest,
    SourceCommitSourceContext,
};
use crate::ids::{
    Canonical, IdempotencyKey, PrincipalId, SourceAdmissionId, SourceAttemptId, SourceEvidenceId,
    SourcePatchId, SourcePlanId, TransactionId,
};

pub fn restore_source_commit_request(
    scope: &Scope,
    principal: &PrincipalId,
    key: IdempotencyKey,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceCommitRequest, RestoreError> {
    let mut r = Reader::new(expected, SourceCommitRequest::FORMAT, bytes, limits)?;
    if r.scope()? != *scope || r.text()? != principal.as_str() {
        return Err(RestoreError::BindingMismatch);
    }
    let request = decode_request(&mut r, key)?;
    r.end()?;
    lifecycle::canonical(
        request.digest(scope, principal),
        expected,
        &request.canonical_bytes(scope, principal),
        bytes,
    )?;
    Ok(request)
}

pub fn restore_source_commit_receipt(
    scope: &Scope,
    principal: &PrincipalId,
    request: &SourceCommitRequest,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceCommitReceipt, RestoreError> {
    let mut r = Reader::new(expected, SourceCommitReceipt::FORMAT, bytes, limits)?;
    let receipt = decode_receipt(&mut r)?;
    r.end()?;
    receipt.validate_request(scope, principal, request)?;
    lifecycle::canonical(
        receipt.digest(),
        expected,
        &receipt.canonical_bytes(),
        bytes,
    )?;
    Ok(receipt)
}

pub(super) fn decode_receipt(r: &mut Reader<'_>) -> Result<SourceCommitReceipt, RestoreError> {
    Ok(SourceCommitReceipt {
        scope: r.scope()?,
        transaction: TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        attempt: r.number()?,
        source_attempt: SourceAttemptId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        admission: SourceAdmissionId(r.digest()?),
        patch: SourcePatchId(r.digest()?),
        evidence: SourceEvidenceId(r.digest()?),
        verified_snapshot: SnapshotId(r.digest()?),
        publication_snapshot: SnapshotId(r.digest()?),
        revision: Revision(r.number()?),
        model_generation: r.number()?,
        analysis: r.digest()?,
        request: r.digest()?,
        committed_at_ms: r.number()?,
    })
}

fn encode_optional<T>(c: &mut Canonical, value: Option<T>, encode: impl FnOnce(&mut Canonical, T)) {
    match value {
        None => c.u64(0),
        Some(value) => {
            c.u64(1);
            encode(c, value);
        }
    }
}
fn decode_optional<T>(
    r: &mut Reader<'_>,
    decode: impl FnOnce(&mut Reader<'_>) -> Result<T, RestoreError>,
) -> Result<Option<T>, RestoreError> {
    match r.number()? {
        0 => Ok(None),
        1 => Ok(Some(decode(r)?)),
        _ => Err(RestoreError::Encoding),
    }
}

fn decode_bool(r: &mut Reader<'_>) -> Result<bool, RestoreError> {
    match r.number()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(RestoreError::Encoding),
    }
}

pub(super) fn encode_context(c: &mut Canonical, context: &SourceCommitContext) {
    c.str(context.analysis_principal.as_str());
    c.u64(context.analysis_grant);
    encode_optional(c, context.head, |c, value| c.digest(value.0));
    encode_optional(c, context.model_generation, |c, value| c.u64(value));
    encode_optional(c, context.analysis, |c, value| c.digest(value));
    encode_optional(c, context.policy, |c, value| c.digest(value));
    encode_optional(c, context.source.as_ref(), |c, source| {
        c.digest(source.admission.0);
        c.str(source.source_attempt.as_str());
        c.str(source.state.name());
        c.u64(source.event_count);
        encode_optional(c, source.evidence, |c, value| c.digest(value.0));
        encode_optional(c, source.executor_grant, |c, value| c.u64(value));
        encode_optional(c, source.executor_active, |c, value| {
            c.u64(u64::from(value))
        });
        encode_optional(c, source.store_registration, |c, value| c.u64(value));
        encode_optional(c, source.store_active, |c, value| c.u64(u64::from(value)));
    });
    encode_optional(c, context.artifacts.as_ref(), |c, artifacts| {
        c.digest(artifacts.evidence.0);
        c.u64(artifacts.registration);
        c.digest(artifacts.provenance);
        c.u64(artifacts.observed_at_ms);
    });
}

pub(super) fn decode_context(r: &mut Reader<'_>) -> Result<SourceCommitContext, RestoreError> {
    Ok(SourceCommitContext {
        analysis_principal: PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        analysis_grant: r.number()?,
        head: decode_optional(r, |r| Ok(SnapshotId(r.digest()?)))?,
        model_generation: decode_optional(r, |r| r.number())?,
        analysis: decode_optional(r, |r| r.digest())?,
        policy: decode_optional(r, |r| r.digest())?,
        source: decode_optional(r, |r| {
            Ok(SourceCommitSourceContext {
                admission: SourceAdmissionId(r.digest()?),
                source_attempt: SourceAttemptId::new(r.text()?)
                    .map_err(|_| RestoreError::Identity)?,
                state: source_event::state(&r.text()?)?,
                event_count: r.number()?,
                evidence: decode_optional(r, |r| Ok(SourceEvidenceId(r.digest()?)))?,
                executor_grant: decode_optional(r, |r| r.number())?,
                executor_active: decode_optional(r, decode_bool)?,
                store_registration: decode_optional(r, |r| r.number())?,
                store_active: decode_optional(r, decode_bool)?,
            })
        })?,
        artifacts: decode_optional(r, |r| {
            Ok(SourceCommitArtifactRead {
                evidence: SourceEvidenceId(r.digest()?),
                registration: r.number()?,
                provenance: r.digest()?,
                observed_at_ms: r.number()?,
            })
        })?,
    })
}

pub(super) fn decode_request(
    r: &mut Reader<'_>,
    key: IdempotencyKey,
) -> Result<SourceCommitRequest, RestoreError> {
    // Invalid raw selectors must survive when restoring an original rejection.
    // Independent authenticated context is above; body metadata cannot replace it.
    let binding = source_mutation::binding(r)?;
    let authorization_grant = r.number()?;
    let fence = r.number()?;
    let analysis_principal = PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let analysis_grant = r.number()?;
    let admission = SourceAdmissionId(r.digest()?);
    let patch = SourcePatchId(r.digest()?);
    let operations = r.digest()?;
    let policy = r.digest()?;
    let policy_generation = r.number()?;
    let plan = SourcePlanId(r.digest()?);
    let verified_snapshot = SnapshotId(r.digest()?);
    let verified_manifest = r.digest()?;
    let application_report = r.digest()?;
    let publication_snapshot = SnapshotId(r.digest()?);
    let publication_revision = Revision(r.number()?);
    let publication_proof = r.digest()?;
    let evidence = SourceEvidenceId(r.digest()?);
    let evidence_receipt = r.digest()?;
    let store_registration = r.number()?;
    let storage_provenance = r.digest()?;
    let analysis_digest = r.digest()?;
    let analysis_bytes = r.field()?;
    let mut analyzer = Reader::new(
        analysis_digest,
        "celetixo/repository-analysis/v1",
        analysis_bytes,
        DomainDecodeLimits {
            collection_entries: r
                .limits
                .collection_entries
                .checked_sub(r.entries)
                .ok_or(RestoreError::Limit)?,
            ..r.limits
        },
    )?;
    let analysis = semantic::decode_original_analysis(&mut analyzer)?;
    analyzer.end()?;
    if analysis.digest() != analysis_digest || analysis.canonical_bytes() != analysis_bytes {
        return Err(RestoreError::NonCanonical);
    }
    r.entries = r
        .entries
        .checked_add(analyzer.entries)
        .ok_or(RestoreError::Limit)?;
    let request = SourceCommitRequest {
        binding,
        authorization_grant,
        fence,
        analysis_principal,
        analysis_grant,
        admission,
        patch,
        operations,
        policy,
        policy_generation,
        plan,
        verified_snapshot,
        verified_manifest,
        application_report,
        publication_snapshot,
        publication_revision,
        publication_proof,
        evidence,
        evidence_receipt,
        store_registration,
        storage_provenance,
        analysis,
        key,
    };
    Ok(request)
}
