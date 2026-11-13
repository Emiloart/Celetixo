//! Independently assembled wire fields, without the production canonical writer.
use super::*;
use celetixo_engine::persistence::{DomainDecodeLimits, RestoreError};
fn field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}
fn text(bytes: &mut Vec<u8>, value: &str) {
    field(bytes, value.as_bytes());
}
fn number(bytes: &mut Vec<u8>, value: u64) {
    field(bytes, &value.to_be_bytes());
}
fn digest(bytes: &mut Vec<u8>, value: Digest) {
    field(bytes, value.as_bytes());
}
fn binding(bytes: &mut Vec<u8>, b: &celetixo_engine::source_patch::PatchBinding) {
    for value in [
        b.scope.organization.as_str(),
        b.scope.workspace.as_str(),
        b.scope.repository.as_str(),
        b.principal.as_str(),
        b.transaction.as_str(),
    ] {
        text(bytes, value);
    }
    number(bytes, b.transaction_attempt);
    text(bytes, b.source_attempt.as_str());
    digest(bytes, b.base_snapshot.0);
    number(bytes, b.base_revision.0);
    for value in [
        b.analysis_artifact,
        b.index_generation,
        b.impact_request,
        b.impact_result,
    ] {
        digest(bytes, value);
    }
    number(bytes, b.created_at_ms);
    number(bytes, b.expires_at_ms);
    text(bytes, &b.producer);
}
fn header(bytes: &mut Vec<u8>, principal: &PrincipalId) {
    for value in [
        scope().organization.as_str(),
        scope().workspace.as_str(),
        scope().repository.as_str(),
        "SourceExecutor",
        principal.as_str(),
    ] {
        text(bytes, value);
    }
    number(bytes, 1);
}

#[test]
fn original_source_request_and_reply_layouts_bind_every_field_and_enforce_decode_limits() {
    let (h, a, backend) = fixture();
    for (name, action, label, state) in [
        (
            "claim",
            SourceExecutorAction::Claim,
            "ClaimSourceAttempt",
            "Patching",
        ),
        (
            "patch",
            SourceExecutorAction::PatchApplied,
            "SourcePatchApplied",
            "PatchApplied",
        ),
        (
            "verify",
            SourceExecutorAction::BeginVerification,
            "BeginSourceVerification",
            "Verifying",
        ),
        ("finish", finish(), "FinishSourceAttempt", "ExecutionFailed"),
    ] {
        let reply = call(&h, &a, name, action.clone());
        let snapshot = records(&backend);
        let key = MutationKey {
            actor: MutationActor::SourceExecutor {
                principal: h.executor.principal().clone(),
                grant: 1,
            },
            key: key(name),
        };
        let record = &snapshot.base.mutations[&key];
        let mut expected = Vec::new();
        header(&mut expected, h.executor.principal());
        text(&mut expected, label);
        digest(&mut expected, a.id().0);
        binding(&mut expected, a.binding());
        if name == "finish" {
            text(&mut expected, "ExecutionFailed");
            text(&mut expected, "original worker failure");
        }
        assert_eq!(
            record.request.canonical_bytes(&scope(), &key.actor),
            expected
        );
        let request_id = Digest::of(MutationRequest::FORMAT, &expected);
        assert_eq!(request_id, reply.request);
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &key.actor,
                request_id,
                &expected,
                Default::default()
            )
            .unwrap(),
            record.request
        );
        let input = expected;
        let mut expected = Vec::new();
        header(&mut expected, h.executor.principal());
        text(&mut expected, name);
        digest(&mut expected, request_id);
        number(&mut expected, record.recorded_at_ms);
        number(&mut expected, 1);
        number(&mut expected, 0); // Real resolved native attempt.
        number(&mut expected, 1);
        text(&mut expected, "Executing");
        number(&mut expected, record.lease_events_before);
        let context = record.source_execution.as_ref().unwrap();
        number(&mut expected, 1);
        digest(&mut expected, a.id().0);
        number(&mut expected, 1);
        text(
            &mut expected,
            if name == "claim" {
                "Admitted"
            } else if name == "patch" {
                "Patching"
            } else if name == "verify" {
                "PatchApplied"
            } else {
                "Verifying"
            },
        );
        if name == "claim" {
            number(&mut expected, 0);
        } else {
            number(&mut expected, 1);
            number(&mut expected, 1);
        }
        number(&mut expected, context.events_before);
        number(&mut expected, context.events_after);
        text(&mut expected, "Accepted");
        text(&mut expected, "SourceAttempt");
        digest(&mut expected, a.id().0);
        binding(&mut expected, a.binding());
        text(&mut expected, state);
        number(&mut expected, 1);
        text(&mut expected, h.executor.principal().as_str());
        number(&mut expected, 0);
        let Ok(MutationValue::SourceAttempt(value)) = &reply.outcome else {
            panic!()
        };
        number(&mut expected, value.events.len() as u64);
        for event in &value.events {
            number(&mut expected, event.sequence);
            number(&mut expected, event.at_ms);
            text(&mut expected, event.from.name());
            text(&mut expected, event.to.name());
            text(&mut expected, &event.reason);
        }
        let encoded = MutationOutcomeRecord::new(scope(), key.clone(), record.clone()).unwrap();
        assert_eq!(encoded.canonical_bytes(), expected);
        for limits in [
            DomainDecodeLimits {
                encoded_bytes: expected.len() - 1,
                ..Default::default()
            },
            DomainDecodeLimits {
                collection_entries: 1,
                ..Default::default()
            },
        ] {
            assert_eq!(
                restore_mutation_outcome(
                    &scope(),
                    &key,
                    &record.request,
                    encoded.digest(),
                    &expected,
                    limits
                ),
                Err(RestoreError::Limit)
            );
        }
        let mut changed = record.request.clone();
        let MutationRequest::SourceExecutor(request) = &mut changed else {
            panic!()
        };
        request.binding.impact_result = d("another-impact-result");
        assert_ne!(changed.canonical_bytes(&scope(), &key.actor), input);
        assert!(
            restore_mutation_outcome(
                &scope(),
                &key,
                &changed,
                encoded.digest(),
                &expected,
                Default::default()
            )
            .is_err()
        );
    }
    let mut forged = records(&backend);
    let (key, record) = forged
        .base
        .mutations
        .iter()
        .find(|(k, _)| k.key.as_str() == "claim")
        .map(|(k, r)| (k.clone(), r.clone()))
        .unwrap();
    forged.base.mutations.insert(
        MutationKey {
            key: super::key("another-effect-key"),
            ..key
        },
        record,
    );
    assert!(AuthorityState::from_source_records(forged, Default::default()).is_err());
}
