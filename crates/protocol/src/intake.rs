use crate::celetixo::{orchestration::v1::TaskIntent, platform::v1::TaskIntentEnvelope};
use prost::Message;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

pub const CONTRACT_VERSION: u32 = 1;
pub const MAX_WIRE_BYTES: usize = 1_048_576;
const MAX_SET_ENTRIES: usize = 4_096;
const MAX_FIELDS: usize = 32_768;
const SUPPORTED_CAPABILITY: &str = "task-intent.v1";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolError {
    #[error("protocol payload exceeds its bounded wire budget")]
    PayloadTooLarge,
    #[error("malformed protocol wire encoding")]
    MalformedWire,
    #[error("unknown protocol field {0}; this intake cannot establish its semantics")]
    UnknownField(u32),
    #[error("duplicate singular protocol field {0}")]
    DuplicateField(u32),
    #[error("unsupported protocol contract version {0}")]
    UnsupportedVersion(u32),
    #[error("unknown required protocol capability")]
    UnsupportedCapability,
    #[error("invalid or missing protocol field: {0}")]
    InvalidField(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestDigest([u8; 32]);
impl RequestDigest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A complete, syntactically validated request. Still untrusted and unauthorized.
/// No lossy projection to the smaller native Engine admission record occurs here.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedTaskIntent {
    envelope: TaskIntentEnvelope,
    digest: RequestDigest,
}
impl ValidatedTaskIntent {
    pub fn envelope(&self) -> &TaskIntentEnvelope {
        &self.envelope
    }
    pub fn request_digest(&self) -> RequestDigest {
        self.digest
    }
}

/// Decode every known field, rejecting unknown safety semantics before prost can
/// discard them. Relays must preserve original binary bytes rather than decode
/// through this consumer and reconstruct an unknown future request.
pub fn decode_task_intent(bytes: &[u8]) -> Result<ValidatedTaskIntent, ProtocolError> {
    if bytes.len() > MAX_WIRE_BYTES {
        return Err(ProtocolError::PayloadTooLarge);
    }
    scan(bytes, ENVELOPE, &mut 0)?;
    let envelope = TaskIntentEnvelope::decode(bytes).map_err(|_| ProtocolError::MalformedWire)?;
    validate(&envelope)?;
    let digest = canonical_digest(&envelope);
    Ok(ValidatedTaskIntent { envelope, digest })
}

#[derive(Clone, Copy)]
enum Kind {
    String,
    Uint32,
    Uint64,
    Bool,
    Message(&'static [Field]),
}
type Field = (u32, Kind, bool);
use Kind::{Bool, Message as Nested, String as Text, Uint32, Uint64};
include!("generated/strict_fields.rs");

fn varint(bytes: &[u8], offset: &mut usize) -> Result<u64, ProtocolError> {
    let mut value = 0;
    for index in 0..10 {
        let byte = *bytes.get(*offset).ok_or(ProtocolError::MalformedWire)?;
        *offset += 1;
        if index == 9 && byte > 1 {
            return Err(ProtocolError::MalformedWire);
        }
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            if index > 0 && byte == 0 {
                return Err(ProtocolError::MalformedWire);
            }
            return Ok(value);
        }
    }
    Err(ProtocolError::MalformedWire)
}

fn scan(bytes: &[u8], schema: &[Field], fields: &mut usize) -> Result<(), ProtocolError> {
    let mut offset = 0;
    let mut seen = BTreeSet::new();
    while offset < bytes.len() {
        *fields += 1;
        if *fields > MAX_FIELDS {
            return Err(ProtocolError::PayloadTooLarge);
        }
        let key = varint(bytes, &mut offset)?;
        let number = u32::try_from(key >> 3).map_err(|_| ProtocolError::MalformedWire)?;
        if number == 0 || number > 536_870_911 {
            return Err(ProtocolError::MalformedWire);
        }
        let &(_, kind, repeated) = schema
            .iter()
            .find(|field| field.0 == number)
            .ok_or(ProtocolError::UnknownField(number))?;
        if !repeated && !seen.insert(number) {
            return Err(ProtocolError::DuplicateField(number));
        }
        match kind {
            Text | Nested(_) => {
                if key & 7 != 2 {
                    return Err(ProtocolError::MalformedWire);
                }
                let length = usize::try_from(varint(bytes, &mut offset)?)
                    .map_err(|_| ProtocolError::MalformedWire)?;
                let end = offset
                    .checked_add(length)
                    .ok_or(ProtocolError::MalformedWire)?;
                let content = bytes.get(offset..end).ok_or(ProtocolError::MalformedWire)?;
                if let Nested(nested) = kind {
                    scan(content, nested, fields)?;
                }
                offset = end;
            }
            Uint32 | Uint64 | Bool => {
                if key & 7 != 0 {
                    return Err(ProtocolError::MalformedWire);
                }
                let value = varint(bytes, &mut offset)?;
                if (matches!(kind, Uint32) && value > u64::from(u32::MAX))
                    || (matches!(kind, Bool) && value > 1)
                {
                    return Err(ProtocolError::MalformedWire);
                }
            }
        }
    }
    Ok(())
}

fn id(value: &str, field: &'static str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > 256
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(ProtocolError::InvalidField(field));
    }
    Ok(())
}
fn text(value: &str, limit: usize, field: &'static str) -> Result<(), ProtocolError> {
    if value.trim().is_empty() || value.len() > limit || value.contains('\0') {
        return Err(ProtocolError::InvalidField(field));
    }
    Ok(())
}
fn set(values: &[String], limit: usize, field: &'static str) -> Result<(), ProtocolError> {
    if values.len() > MAX_SET_ENTRIES {
        return Err(ProtocolError::InvalidField(field));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        text(value, limit, field)?;
        if !seen.insert(value) {
            return Err(ProtocolError::InvalidField(field));
        }
    }
    Ok(())
}

fn validate(envelope: &TaskIntentEnvelope) -> Result<(), ProtocolError> {
    if envelope.contract_version != CONTRACT_VERSION {
        return Err(ProtocolError::UnsupportedVersion(envelope.contract_version));
    }
    let scope = envelope
        .scope
        .as_ref()
        .ok_or(ProtocolError::InvalidField("scope"))?;
    id(&scope.organization_id, "scope.organization_id")?;
    id(&scope.workspace_id, "scope.workspace_id")?;
    id(&scope.repository_id, "scope.repository_id")?;
    id(&envelope.idempotency_key, "idempotency_key")?;
    id(&envelope.trace_id, "trace_id")?;
    set(
        &envelope.required_capabilities,
        256,
        "required_capabilities",
    )?;
    if envelope
        .required_capabilities
        .iter()
        .any(|value| value != SUPPORTED_CAPABILITY)
    {
        return Err(ProtocolError::UnsupportedCapability);
    }
    let intent = envelope
        .intent
        .as_ref()
        .ok_or(ProtocolError::InvalidField("intent"))?;
    id(&intent.task_id, "task_id")?;
    if !intent.parent_task_id.is_empty() {
        id(&intent.parent_task_id, "parent_task_id")?;
        if intent.parent_task_id == intent.task_id {
            return Err(ProtocolError::InvalidField("parent_task_id"));
        }
    }
    id(&intent.repository_id, "repository_id")?;
    if intent.repository_id != scope.repository_id {
        return Err(ProtocolError::InvalidField("scope.repository_id"));
    }
    id(&intent.base_revision, "base_revision")?;
    text(
        &intent.objective_description,
        16_384,
        "objective_description",
    )?;
    set(&intent.explicit_reads, 4_096, "explicit_reads")?;
    set(&intent.explicit_writes, 4_096, "explicit_writes")?;
    set(&intent.invariant_contracts, 16_384, "invariant_contracts")?;
    set(&intent.depends_on_task_ids, 256, "depends_on_task_ids")?;
    for dependency in &intent.depends_on_task_ids {
        id(dependency, "depends_on_task_ids")?;
        if dependency == &intent.task_id {
            return Err(ProtocolError::InvalidField("depends_on_task_ids"));
        }
    }
    let impact = intent
        .impact_policy
        .as_ref()
        .ok_or(ProtocolError::InvalidField("impact_policy"))?;
    set(
        &impact.allowed_domains,
        4_096,
        "impact_policy.allowed_domains",
    )?;
    set(
        &impact.forbidden_domains,
        4_096,
        "impact_policy.forbidden_domains",
    )?;
    if impact
        .allowed_domains
        .iter()
        .any(|value| impact.forbidden_domains.contains(value))
    {
        return Err(ProtocolError::InvalidField("impact_policy.domains"));
    }
    let verification = intent
        .verification
        .as_ref()
        .ok_or(ProtocolError::InvalidField("verification"))?;
    if verification.minimum_sandbox_tier > 3 {
        return Err(ProtocolError::InvalidField(
            "verification.minimum_sandbox_tier",
        ));
    }
    set(
        &verification.required_test_targets,
        4_096,
        "verification.required_test_targets",
    )?;
    let budget = intent
        .budget
        .as_ref()
        .ok_or(ProtocolError::InvalidField("budget"))?;
    if budget.timeout_seconds == 0 {
        return Err(ProtocolError::InvalidField("budget.timeout_seconds"));
    }
    let retry = intent
        .retry_policy
        .as_ref()
        .ok_or(ProtocolError::InvalidField("retry_policy"))?;
    if retry.max_attempts == 0 {
        return Err(ProtocolError::InvalidField("retry_policy.max_attempts"));
    }
    intent
        .rollback_policy
        .as_ref()
        .ok_or(ProtocolError::InvalidField("rollback_policy"))?;
    let escalation = intent
        .escalation_policy
        .as_ref()
        .ok_or(ProtocolError::InvalidField("escalation_policy"))?;
    if escalation.max_escalations > 0 {
        id(
            &escalation.fallback_target,
            "escalation_policy.fallback_target",
        )?;
    } else if !escalation.fallback_target.is_empty() {
        return Err(ProtocolError::InvalidField(
            "escalation_policy.fallback_target",
        ));
    }
    let provenance = intent
        .provenance
        .as_ref()
        .ok_or(ProtocolError::InvalidField("provenance"))?;
    text(&provenance.model_name, 256, "provenance.model_name")?;
    text(&provenance.model_version, 256, "provenance.model_version")?;
    text(
        &provenance.prompt_template_hash,
        256,
        "provenance.prompt_template_hash",
    )?;
    text(&provenance.agent_version, 256, "provenance.agent_version")?;
    Ok(())
}

/// Independent canonical semantic encoding. Protobuf field order, default-value
/// presence, implementation-specific serialization and trace IDs are not identity.
fn canonical_digest(envelope: &TaskIntentEnvelope) -> RequestDigest {
    struct Canonical(Vec<u8>);
    impl Canonical {
        fn bytes(&mut self, bytes: &[u8]) {
            self.0
                .extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            self.0.extend_from_slice(bytes);
        }
        fn string(&mut self, value: &str) {
            self.bytes(value.as_bytes());
        }
        fn number(&mut self, value: u64) {
            self.bytes(&value.to_be_bytes());
        }
        fn set(&mut self, values: &[String]) {
            self.number(values.len() as u64);
            for value in values.iter().collect::<BTreeSet<_>>() {
                self.string(value);
            }
        }
    }
    // All message presence and fields have already been validated above.
    let scope = envelope.scope.as_ref().expect("validated scope");
    let intent: &TaskIntent = envelope.intent.as_ref().expect("validated intent");
    let mut out = Canonical(Vec::new());
    out.string("celetixo/task-intent-request/v1");
    out.number(u64::from(envelope.contract_version));
    out.string(&scope.organization_id);
    out.string(&scope.workspace_id);
    out.string(&scope.repository_id);
    out.set(&envelope.required_capabilities);
    for value in [
        &intent.task_id,
        &intent.parent_task_id,
        &intent.repository_id,
        &intent.base_revision,
        &intent.objective_description,
    ] {
        out.string(value);
    }
    for values in [
        &intent.explicit_reads,
        &intent.explicit_writes,
        &intent.invariant_contracts,
        &intent.depends_on_task_ids,
    ] {
        out.set(values);
    }
    let impact = intent.impact_policy.as_ref().expect("validated impact");
    out.number(u64::from(impact.max_reference_depth));
    out.set(&impact.allowed_domains);
    out.set(&impact.forbidden_domains);
    let verification = intent
        .verification
        .as_ref()
        .expect("validated verification");
    out.number(u64::from(verification.minimum_sandbox_tier));
    out.set(&verification.required_test_targets);
    out.number(u64::from(verification.require_strict_typecheck));
    out.number(u64::from(verification.allow_adaptive_escalation));
    let budget = intent.budget.as_ref().expect("validated budget");
    out.number(budget.max_token_limit);
    out.number(budget.timeout_seconds);
    out.number(budget.max_cost_microusd);
    let retry = intent.retry_policy.as_ref().expect("validated retry");
    out.number(u64::from(retry.max_attempts));
    out.number(u64::from(retry.allow_rebase));
    let rollback = intent.rollback_policy.as_ref().expect("validated rollback");
    out.number(u64::from(rollback.preserve_failed_artifacts));
    out.number(u64::from(rollback.preserve_failed_patch));
    let escalation = intent
        .escalation_policy
        .as_ref()
        .expect("validated escalation");
    out.string(&escalation.fallback_target);
    out.number(u64::from(escalation.max_escalations));
    let provenance = intent.provenance.as_ref().expect("validated provenance");
    for value in [
        &provenance.model_name,
        &provenance.model_version,
        &provenance.prompt_template_hash,
        &provenance.agent_version,
    ] {
        out.string(value);
    }
    RequestDigest(Sha256::digest(out.0).into())
}
