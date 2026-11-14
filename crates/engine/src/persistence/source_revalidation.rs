//! Checked historical selectors and outcomes; no live lease is reconstructed.
use super::*;
use crate::engine::{
    SourceLeaseObservation, SourceRevalidationContext, SourceRevalidationReceipt,
    SourceRevalidationRequest,
};
use crate::ids::{Canonical, IdempotencyKey, PrincipalId, SourceAttemptId, TransactionId};

pub fn restore_source_revalidation_request(
    scope: &Scope,
    principal: &PrincipalId,
    key: IdempotencyKey,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceRevalidationRequest, RestoreError> {
    let mut r = Reader::new(expected, SourceRevalidationRequest::FORMAT, bytes, limits)?;
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
pub fn restore_source_revalidation_receipt(
    scope: &Scope,
    principal: &PrincipalId,
    request: &SourceRevalidationRequest,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceRevalidationReceipt, RestoreError> {
    let mut r = Reader::new(expected, SourceRevalidationReceipt::FORMAT, bytes, limits)?;
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
pub(super) fn decode_request(
    r: &mut Reader<'_>,
    key: IdempotencyKey,
) -> Result<SourceRevalidationRequest, RestoreError> {
    Ok(SourceRevalidationRequest {
        scope: r.scope()?,
        principal: PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        authorization_grant: r.number()?,
        transaction: TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        previous_attempt: r.number()?,
        previous_base: SnapshotId(r.digest()?),
        previous_model_generation: r.number()?,
        previous_impact: r.digest()?,
        intent: r.digest()?,
        fence: r.number()?,
        head: SnapshotId(r.digest()?),
        revision: Revision(r.number()?),
        model_generation: r.number()?,
        analysis: r.digest()?,
        index_generation: r.digest()?,
        policy: r.digest()?,
        policy_generation: r.number()?,
        ttl_ms: r.number()?,
        key,
    })
}
pub(super) fn decode_receipt(
    r: &mut Reader<'_>,
) -> Result<SourceRevalidationReceipt, RestoreError> {
    let scope = r.scope()?;
    let transaction = TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let previous_attempt = r.number()?;
    let attempt = r.number()?;
    let previous_base = SnapshotId(r.digest()?);
    let base = SnapshotId(r.digest()?);
    let revision = Revision(r.number()?);
    let model_generation = r.number()?;
    let analysis = r.digest()?;
    let index_generation = r.digest()?;
    let policy = r.digest()?;
    let policy_generation = r.number()?;
    let previous_fence = r.number()?;
    let fence = r.number()?;
    let expires_at_ms = r.number()?;
    let accesses = lifecycle::decode_access(r)?;
    let impact = r.digest()?;
    let mut invalidated_source_attempts = BTreeSet::new();
    for _ in 0..r.count()? {
        if !invalidated_source_attempts
            .insert(SourceAttemptId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        {
            return Err(RestoreError::NonCanonical);
        }
    }
    let mut intervening_revisions = Vec::new();
    for _ in 0..r.count()? {
        intervening_revisions.push(Revision(r.number()?));
    }
    Ok(SourceRevalidationReceipt {
        scope,
        transaction,
        previous_attempt,
        attempt,
        previous_base,
        base,
        revision,
        model_generation,
        analysis,
        index_generation,
        policy,
        policy_generation,
        previous_fence,
        fence,
        expires_at_ms,
        accesses,
        impact,
        invalidated_source_attempts,
        intervening_revisions,
        request: r.digest()?,
        revalidated_at_ms: r.number()?,
    })
}
fn optional<T>(c: &mut Canonical, value: Option<T>, encode: impl FnOnce(&mut Canonical, T)) {
    match value {
        None => c.u64(0),
        Some(v) => {
            c.u64(1);
            encode(c, v);
        }
    }
}
fn read_optional<T>(
    r: &mut Reader<'_>,
    decode: impl FnOnce(&mut Reader<'_>) -> Result<T, RestoreError>,
) -> Result<Option<T>, RestoreError> {
    match r.number()? {
        0 => Ok(None),
        1 => Ok(Some(decode(r)?)),
        _ => Err(RestoreError::Encoding),
    }
}
pub(super) fn encode_context(c: &mut Canonical, value: &SourceRevalidationContext) {
    optional(c, value.head, |c, v| c.digest(v.0));
    optional(c, value.model_generation, |c, v| c.u64(v));
    optional(c, value.analysis, |c, v| c.digest(v));
    optional(c, value.policy, |c, v| c.digest(v));
    c.u64(value.reservation_fence_before);
    c.u64(value.leases.len() as u64);
    for (id, observed) in &value.leases {
        c.str(id.as_str());
        c.u64(observed.event_count);
        optional(c, observed.grant_active, |c, v| c.u64(u64::from(v)));
    }
    c.u64(value.source_event_counts.len() as u64);
    for (id, count) in &value.source_event_counts {
        c.str(id.as_str());
        c.u64(*count);
    }
}
pub(super) fn decode_context(
    r: &mut Reader<'_>,
) -> Result<SourceRevalidationContext, RestoreError> {
    let head = read_optional(r, |r| Ok(SnapshotId(r.digest()?)))?;
    let model_generation = read_optional(r, |r| r.number())?;
    let analysis = read_optional(r, |r| r.digest())?;
    let policy = read_optional(r, |r| r.digest())?;
    let reservation_fence_before = r.number()?;
    let mut leases = BTreeMap::new();
    for _ in 0..r.count()? {
        let id = TransactionId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
        let event_count = r.number()?;
        let grant_active = read_optional(r, |r| match r.number()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(RestoreError::Encoding),
        })?;
        if leases
            .insert(
                id,
                SourceLeaseObservation {
                    event_count,
                    grant_active,
                },
            )
            .is_some()
        {
            return Err(RestoreError::NonCanonical);
        }
    }
    let mut source_event_counts = BTreeMap::new();
    for _ in 0..r.count()? {
        let id = SourceAttemptId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
        let n = r.number()?;
        if source_event_counts.insert(id, n).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    Ok(SourceRevalidationContext {
        head,
        model_generation,
        analysis,
        policy,
        reservation_fence_before,
        leases,
        source_event_counts,
    })
}
