use celetixo_protocol::{
    MAX_WIRE_BYTES, ProtocolError, celetixo::platform::v1::TaskIntentEnvelope, decode_task_intent,
};
use prost::Message;

const GOLDEN: &[u8] = include_bytes!("../../../proto/fixtures/task_intent_envelope.bin");

fn fixture() -> TaskIntentEnvelope {
    TaskIntentEnvelope::decode(GOLDEN).unwrap()
}

fn strict(
    envelope: &TaskIntentEnvelope,
) -> Result<celetixo_protocol::ValidatedTaskIntent, ProtocolError> {
    decode_task_intent(&envelope.encode_to_vec())
}

fn varint(mut value: u64, bytes: &mut Vec<u8>) {
    while value > 127 {
        bytes.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn message_field(tag: u32, content: &[u8], bytes: &mut Vec<u8>) {
    varint(u64::from(tag) << 3 | 2, bytes);
    varint(content.len() as u64, bytes);
    bytes.extend_from_slice(content);
}

#[test]
fn full_adr_contract_golden_wire_is_retained() {
    let validated = decode_task_intent(GOLDEN).unwrap();
    assert_eq!(validated.envelope().encode_to_vec(), GOLDEN);
    let intent = validated.envelope().intent.as_ref().unwrap();
    assert_eq!(intent.depends_on_task_ids, ["task:baseline"]);
    assert_eq!(intent.explicit_reads.len(), 2);
    assert_eq!(intent.explicit_writes, ["symbol:parse"]);
    assert_eq!(
        intent.invariant_contracts,
        ["parser-preserves-valid-inputs:v1"]
    );
    assert_eq!(
        intent.impact_policy.as_ref().unwrap().max_reference_depth,
        8
    );
    assert!(
        intent
            .verification
            .as_ref()
            .unwrap()
            .require_strict_typecheck
    );
    assert_eq!(intent.budget.as_ref().unwrap().max_cost_microusd, 2_000_000);
    assert!(intent.retry_policy.as_ref().unwrap().allow_rebase);
    assert!(
        intent
            .rollback_policy
            .as_ref()
            .unwrap()
            .preserve_failed_patch
    );
    assert_eq!(
        intent.escalation_policy.as_ref().unwrap().max_escalations,
        1
    );
    assert_eq!(
        intent.provenance.as_ref().unwrap().agent_version,
        "fixture-agent-v1"
    );
}

#[test]
fn request_identity_preserves_all_adr_fields_and_scope() {
    type Mutation = Box<dyn Fn(&mut TaskIntentEnvelope)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(|e| e.scope.as_mut().unwrap().organization_id.push('2')),
        Box::new(|e| e.scope.as_mut().unwrap().workspace_id.push('2')),
        Box::new(|e| {
            e.scope.as_mut().unwrap().repository_id.push('2');
            e.intent.as_mut().unwrap().repository_id.push('2');
        }),
        Box::new(|e| e.intent.as_mut().unwrap().task_id.push('2')),
        Box::new(|e| e.intent.as_mut().unwrap().parent_task_id.push('2')),
        Box::new(|e| e.intent.as_mut().unwrap().base_revision.push('2')),
        Box::new(|e| e.intent.as_mut().unwrap().objective_description.push('2')),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .explicit_reads
                .push("symbol:extra".into())
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .explicit_writes
                .push("symbol:extra".into())
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .invariant_contracts
                .push("another-invariant:v1".into())
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .depends_on_task_ids
                .push("task:other".into())
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .impact_policy
                .as_mut()
                .unwrap()
                .max_reference_depth += 1
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .impact_policy
                .as_mut()
                .unwrap()
                .allowed_domains
                .push("another".into())
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .impact_policy
                .as_mut()
                .unwrap()
                .forbidden_domains
                .push("another".into())
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .verification
                .as_mut()
                .unwrap()
                .minimum_sandbox_tier = 2
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .verification
                .as_mut()
                .unwrap()
                .required_test_targets
                .push("extra-tests".into())
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .verification
                .as_mut()
                .unwrap()
                .require_strict_typecheck = false
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .verification
                .as_mut()
                .unwrap()
                .allow_adaptive_escalation = false
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .budget
                .as_mut()
                .unwrap()
                .max_token_limit += 1
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .budget
                .as_mut()
                .unwrap()
                .timeout_seconds += 1
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .budget
                .as_mut()
                .unwrap()
                .max_cost_microusd += 1
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .retry_policy
                .as_mut()
                .unwrap()
                .max_attempts += 1
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .retry_policy
                .as_mut()
                .unwrap()
                .allow_rebase = false
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .rollback_policy
                .as_mut()
                .unwrap()
                .preserve_failed_artifacts = false
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .rollback_policy
                .as_mut()
                .unwrap()
                .preserve_failed_patch = false
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .escalation_policy
                .as_mut()
                .unwrap()
                .fallback_target
                .push('2')
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .escalation_policy
                .as_mut()
                .unwrap()
                .max_escalations += 1
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .provenance
                .as_mut()
                .unwrap()
                .model_name
                .push('2')
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .provenance
                .as_mut()
                .unwrap()
                .model_version
                .push('2')
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .provenance
                .as_mut()
                .unwrap()
                .prompt_template_hash
                .push('2')
        }),
        Box::new(|e| {
            e.intent
                .as_mut()
                .unwrap()
                .provenance
                .as_mut()
                .unwrap()
                .agent_version
                .push('2')
        }),
        Box::new(|e| e.required_capabilities.clear()),
    ];
    let original = strict(&fixture()).unwrap().request_digest();
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut changed = fixture();
        mutate(&mut changed);
        assert_ne!(
            original,
            strict(&changed).unwrap().request_digest(),
            "mutation {index}"
        );
    }
}

#[test]
fn semantic_identity_is_independent_of_wire_order_sets_trace_and_retry_key() {
    let original = strict(&fixture()).unwrap().request_digest();
    let mut equivalent = fixture();
    equivalent.intent.as_mut().unwrap().explicit_reads.reverse();
    equivalent.trace_id = "trace:retry".into();
    equivalent.idempotency_key = "request:new-operation-identity".into();
    assert_eq!(original, strict(&equivalent).unwrap().request_digest());
    let intent = equivalent.intent.take().unwrap().encode_to_vec();
    let mut reordered = equivalent.encode_to_vec();
    message_field(3, &intent, &mut reordered);
    assert_eq!(
        original,
        decode_task_intent(&reordered).unwrap().request_digest()
    );
}

#[test]
fn unknown_fields_cannot_disappear_during_prost_decoding() {
    let mut unknown_top = GOLDEN.to_vec();
    varint(99 << 3, &mut unknown_top);
    varint(1, &mut unknown_top);
    assert!(TaskIntentEnvelope::decode(unknown_top.as_slice()).is_ok());
    assert_eq!(
        decode_task_intent(&unknown_top),
        Err(ProtocolError::UnknownField(99))
    );

    let mut envelope = fixture();
    let mut intent = envelope.intent.take().unwrap();
    let mut budget = intent.budget.take().unwrap().encode_to_vec();
    varint(99 << 3, &mut budget);
    varint(1, &mut budget);
    let mut intent_bytes = intent.encode_to_vec();
    message_field(12, &budget, &mut intent_bytes);
    let mut bytes = envelope.encode_to_vec();
    message_field(3, &intent_bytes, &mut bytes);
    assert_eq!(
        decode_task_intent(&bytes),
        Err(ProtocolError::UnknownField(99))
    );
}

#[test]
fn duplicate_singular_fields_do_not_merge_policy_or_override_scope() {
    let mut bytes = GOLDEN.to_vec();
    bytes.extend_from_slice(&[8, 1]);
    assert_eq!(
        decode_task_intent(&bytes),
        Err(ProtocolError::DuplicateField(1))
    );
    let mut envelope = fixture();
    let mut intent = envelope.intent.take().unwrap().encode_to_vec();
    message_field(10, &[], &mut intent);
    let mut bytes = envelope.encode_to_vec();
    message_field(3, &intent, &mut bytes);
    assert_eq!(
        decode_task_intent(&bytes),
        Err(ProtocolError::DuplicateField(10))
    );
}

#[test]
fn unknown_versions_capabilities_and_tiers_fail_closed_but_t0_is_valid() {
    let mut envelope = fixture();
    envelope.contract_version = 2;
    assert_eq!(strict(&envelope), Err(ProtocolError::UnsupportedVersion(2)));
    envelope.contract_version = 0;
    assert_eq!(strict(&envelope), Err(ProtocolError::UnsupportedVersion(0)));
    envelope = fixture();
    envelope
        .required_capabilities
        .push("caller-grants-commit.v1".into());
    assert_eq!(strict(&envelope), Err(ProtocolError::UnsupportedCapability));
    envelope = fixture();
    envelope
        .intent
        .as_mut()
        .unwrap()
        .verification
        .as_mut()
        .unwrap()
        .minimum_sandbox_tier = 4;
    assert_eq!(
        strict(&envelope),
        Err(ProtocolError::InvalidField(
            "verification.minimum_sandbox_tier"
        ))
    );
    envelope
        .intent
        .as_mut()
        .unwrap()
        .verification
        .as_mut()
        .unwrap()
        .minimum_sandbox_tier = 0;
    assert!(strict(&envelope).is_ok());
}

#[test]
fn missing_policy_messages_do_not_receive_permissive_implicit_defaults() {
    for field in 0..7 {
        let mut envelope = fixture();
        let intent = envelope.intent.as_mut().unwrap();
        match field {
            0 => intent.impact_policy = None,
            1 => intent.verification = None,
            2 => intent.budget = None,
            3 => intent.retry_policy = None,
            4 => intent.rollback_policy = None,
            5 => intent.escalation_policy = None,
            _ => intent.provenance = None,
        }
        assert!(matches!(
            strict(&envelope),
            Err(ProtocolError::InvalidField(_))
        ));
    }
    let mut envelope = fixture();
    envelope.intent.as_mut().unwrap().rollback_policy = Some(Default::default());
    assert!(
        strict(&envelope).is_ok(),
        "explicit present false-valued rollback policy is retained"
    );
    envelope
        .intent
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .max_cost_microusd = 0;
    envelope
        .intent
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .max_token_limit = 0;
    assert!(
        strict(&envelope).is_ok(),
        "zero token/cost budgets prohibit those resources"
    );
}

#[test]
fn malformed_wire_and_uint32_bool_overflows_reject() {
    for bytes in [
        &[0][..],
        &[8, 0x80][..],
        &[10, 0][..],
        &[8, 0x81, 0][..],
        &[15][..],
    ] {
        assert!(decode_task_intent(bytes).is_err());
    }
    let mut envelope = fixture();
    let mut intent = envelope.intent.take().unwrap();
    intent.verification = None;
    for verification in [vec![24, 2], vec![8, 0xff, 0xff, 0xff, 0xff, 0x1f]] {
        let mut intent_bytes = intent.encode_to_vec();
        message_field(11, &verification, &mut intent_bytes);
        let mut bytes = envelope.encode_to_vec();
        message_field(3, &intent_bytes, &mut bytes);
        assert_eq!(
            decode_task_intent(&bytes),
            Err(ProtocolError::MalformedWire)
        );
    }
    for prefix in 0..GOLDEN.len() {
        // A prefix ending inside a field must reject. A prefix ending between
        // optional envelope fields can be a complete protobuf request; it must
        // retain the entire required intent, scope and idempotency identity.
        if let Ok(accepted) = decode_task_intent(&GOLDEN[..prefix]) {
            let expected = fixture();
            assert_eq!(
                accepted.envelope().intent,
                expected.intent,
                "prefix {prefix}"
            );
            assert_eq!(accepted.envelope().scope, expected.scope, "prefix {prefix}");
            assert_eq!(
                accepted.envelope().idempotency_key,
                expected.idempotency_key,
                "prefix {prefix}"
            );
        }
    }
}

#[test]
fn identifiers_scope_self_dependencies_duplicates_and_budgets_are_checked() {
    let mut envelope = fixture();
    envelope.scope.as_mut().unwrap().repository_id.push('2');
    assert!(strict(&envelope).is_err());
    envelope = fixture();
    envelope
        .intent
        .as_mut()
        .unwrap()
        .depends_on_task_ids
        .push("task:refactor".into());
    assert!(strict(&envelope).is_err());
    envelope = fixture();
    envelope
        .intent
        .as_mut()
        .unwrap()
        .explicit_reads
        .push("symbol:parser".into());
    assert!(strict(&envelope).is_err());
    envelope = fixture();
    envelope.trace_id = "a\nb".into();
    assert!(strict(&envelope).is_err());
    envelope = fixture();
    envelope
        .intent
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .timeout_seconds = 0;
    assert!(strict(&envelope).is_err());
    envelope = fixture();
    envelope
        .intent
        .as_mut()
        .unwrap()
        .retry_policy
        .as_mut()
        .unwrap()
        .max_attempts = 0;
    assert!(strict(&envelope).is_err());
    assert_eq!(
        decode_task_intent(&vec![0; MAX_WIRE_BYTES + 1]),
        Err(ProtocolError::PayloadTooLarge)
    );
}
