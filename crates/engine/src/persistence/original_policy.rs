//! Complete original policy input, including immutable registry dependencies.
//! Parsing does not bind it to today's snapshot or authorize installation.
use super::*;
use crate::ids::Canonical;
use crate::source_execution::{SourceCheckDefinition, SourceExecutionPolicy};

pub(crate) fn encode_original_policy(c: &mut Canonical, policy: &SourceExecutionPolicy) {
    c.u64(policy.input().generation);
    let patch = PatchPolicyRecord::new(policy.input().patch_policy.clone())
        .expect("SourceExecutionPolicy constructor validates its patch policy");
    c.digest(patch.digest());
    c.bytes(&patch.canonical_bytes());
    c.u64(policy.checks().len() as u64);
    for definition in policy.checks().values() {
        let check = CheckSpecRecord::new(definition.input().check.clone());
        c.digest(check.digest());
        c.bytes(&check.canonical_bytes());
        c.digest(definition.digest());
        c.bytes(&definition.canonical_bytes());
    }
    c.digest(policy.digest());
    c.bytes(&policy.canonical_bytes());
}

pub(super) fn decode_original_policy(
    r: &mut Reader<'_>,
) -> Result<SourceExecutionPolicy, RestoreError> {
    let generation = r.number()?;
    let patch_id = r.digest()?;
    let patch_bytes = r.field()?;
    let patch = restore_patch_policy(patch_id, patch_bytes, r.limits)?;
    let mut definitions: BTreeMap<Digest, SourceCheckDefinition> = BTreeMap::new();
    for _ in 0..r.count()? {
        let expected = r.digest()?;
        let bytes = r.field()?;
        let mut nested = Reader::new(expected, CheckSpecRecord::FORMAT, bytes, r.limits)?;
        nested.entries = r.entries;
        let check = lifecycle::decode_check(&mut nested)?;
        nested.end()?;
        r.entries = nested.entries;
        let checked = CheckSpecRecord::new(check);
        lifecycle::canonical(
            checked.digest(),
            expected,
            &checked.canonical_bytes(),
            bytes,
        )?;
        let expected = r.digest()?;
        let bytes = r.field()?;
        let mut nested = Reader::new(
            expected,
            SourceCheckDefinitionRecord::FORMAT,
            bytes,
            r.limits,
        )?;
        nested.entries = r.entries;
        let definition =
            source_execution::decode_definition_fields(&mut nested, checked.check(), bytes.len())?;
        nested.end()?;
        r.entries = nested.entries;
        lifecycle::canonical(
            definition.digest(),
            expected,
            &definition.canonical_bytes(),
            bytes,
        )?;
        if definitions.insert(expected, definition).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    let expected = r.digest()?;
    let bytes = r.field()?;
    let mut nested = Reader::new(
        expected,
        SourceExecutionPolicyRecord::FORMAT,
        bytes,
        r.limits,
    )?;
    nested.entries = r.entries;
    let policy = source_execution::decode_policy_fields(
        &mut nested,
        None,
        generation,
        patch.policy(),
        &definitions,
        bytes.len(),
    )?;
    nested.end()?;
    r.entries = nested.entries;
    lifecycle::canonical(policy.digest(), expected, &policy.canonical_bytes(), bytes)?;
    Ok(policy)
}
