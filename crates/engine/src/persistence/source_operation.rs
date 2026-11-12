//! Exact request bodies and decisions, independent of today's mutable heads.
use super::*;
use crate::engine::{EngineError, SourceAttemptOptions};
use crate::ids::{Canonical, IdempotencyKey, SourceAdmissionId, TransactionId};
use crate::source_patch::{DecodeLimits, SourcePatch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRequestRecord {
    patch: SourcePatch,
    options: SourceAttemptOptions,
}
impl SourceRequestRecord {
    pub const FORMAT: &str = "celetixo/source-request-record/v1";
    pub fn new(patch: SourcePatch, options: SourceAttemptOptions) -> Result<Self, RestoreError> {
        options.validate()?;
        Ok(Self { patch, options })
    }
    pub fn patch(&self) -> &SourcePatch {
        &self.patch
    }
    pub fn options(&self) -> &SourceAttemptOptions {
        &self.options
    }
    /// The existing operation identity; empty options still use patch identity.
    pub fn request_digest(&self) -> Digest {
        self.options.request_digest(self.patch.id())
    }
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.bytes(&self.patch.canonical_bytes());
        c.u64(self.options.optional_checks.len() as u64);
        for check in &self.options.optional_checks {
            c.str(check.as_str());
        }
        c.into_bytes()
    }
}
pub fn restore_source_request(
    scope: &Scope,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceRequestRecord, RestoreError> {
    let mut r = Reader::new(expected, SourceRequestRecord::FORMAT, bytes, limits)?;
    let patch = SourcePatch::from_canonical_bytes(
        r.field()?,
        DecodeLimits {
            max_envelope_bytes: limits.encoded_bytes,
            max_operations: limits.collection_entries,
            max_payload_bytes: limits.field_bytes,
            max_total_payload_bytes: limits.encoded_bytes,
        },
    )?;
    if &patch.binding().scope != scope {
        return Err(RestoreError::BindingMismatch);
    }
    // Patch payloads are already in parent bytes; share the nested entry budget.
    r.entries = r
        .entries
        .checked_add(patch.operations().len())
        .ok_or(RestoreError::Limit)?;
    if r.entries > limits.collection_entries {
        return Err(RestoreError::Limit);
    }
    let mut optional_checks = BTreeSet::new();
    for _ in 0..r.count()? {
        let check = crate::ids::CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
        if !optional_checks.insert(check) {
            return Err(RestoreError::NonCanonical);
        }
    }
    r.end()?;
    let record = SourceRequestRecord::new(patch, SourceAttemptOptions { optional_checks })?;
    lifecycle::canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

/// Resolved target context is deliberately separate from the possibly invalid
/// submitted attempt in SourcePatch. A rejected request may name attempt 999
/// without creating such an authoritative attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOutcomeContext {
    pub scope: Scope,
    pub transaction: TransactionId,
    pub target_attempt: u64,
    pub key: IdempotencyKey,
    pub request: Digest,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOutcomeRecord {
    context: SourceOutcomeContext,
    outcome: Result<SourceAdmissionId, EngineError>,
}
impl SourceOutcomeRecord {
    pub const FORMAT: &str = "celetixo/source-outcome-record/v1";
    pub fn new(
        context: SourceOutcomeContext,
        outcome: Result<SourceAdmissionId, EngineError>,
    ) -> Result<Self, RestoreError> {
        if let Err(error) = &outcome {
            match error {
                EngineError::SourceAttempt(_)
                | EngineError::SourceEvidence(_)
                | EngineError::RepositoryAnalysis(_) => {
                    super::source_errors::encode_engine_error(&mut Canonical::default(), error)?;
                }
                _ => {
                    CommitOutcomeRecord::new(
                        context.scope.clone(),
                        context.transaction.clone(),
                        context.target_attempt,
                        Err(error.clone()),
                    )?;
                }
            }
        }
        Ok(Self { context, outcome })
    }
    pub fn context(&self) -> &SourceOutcomeContext {
        &self.context
    }
    pub fn outcome(&self) -> &Result<SourceAdmissionId, EngineError> {
        &self.outcome
    }
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = outcome::context(
            &self.context.scope,
            &self.context.transaction,
            self.context.target_attempt,
        );
        c.str(self.context.key.as_str());
        c.digest(self.context.request);
        match &self.outcome {
            Ok(id) => {
                c.str("Accepted");
                c.digest(id.0);
            }
            Err(error) => {
                c.str("Rejected");
                super::source_errors::encode_engine_error(&mut c, error)
                    .expect("validated supported domain error");
            }
        }
        c.into_bytes()
    }
}
pub fn restore_source_outcome(
    context: &SourceOutcomeContext,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceOutcomeRecord, RestoreError> {
    let mut r = Reader::new(expected, SourceOutcomeRecord::FORMAT, bytes, limits)?;
    outcome::validate_context(
        &mut r,
        &context.scope,
        &context.transaction,
        context.target_attempt,
    )?;
    if r.text()? != context.key.as_str() || r.digest()? != context.request {
        return Err(RestoreError::BindingMismatch);
    }
    let result = match r.text()?.as_str() {
        "Accepted" => Ok(SourceAdmissionId(r.digest()?)),
        "Rejected" => Err(super::source_errors::decode_engine_error(&mut r)?),
        _ => return Err(RestoreError::Encoding),
    };
    r.end()?;
    let record = SourceOutcomeRecord::new(context.clone(), result)?;
    lifecycle::canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}
