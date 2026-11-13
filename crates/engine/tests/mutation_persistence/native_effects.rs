//! Parsing original requests must not require today's snapshot or attempt.
use super::*;
use celetixo_engine::access::InvariantViolation;
use celetixo_engine::evidence::{
    Artifact, CheckResult, CheckSpec, EvidenceBindings, ExecutionProvenance, VerificationEvidence,
};
use celetixo_engine::patch::{Change, Patch, PatchError};

fn bindings() -> EvidenceBindings {
    EvidenceBindings {
        transaction: TransactionId::new("tx").unwrap(),
        attempt: 3,
        base_snapshot: SnapshotId(Digest::of("test/v1", b"unregistered-base")),
        candidate_snapshot: SnapshotId(Digest::of("test/v1", b"candidate")),
        patch: PatchId(Digest::of("test/v1", b"patch")),
        analysis: Digest::of("test/v1", b"analysis"),
    }
}
fn evidence() -> VerificationEvidence {
    VerificationEvidence::new(
        bindings(),
        CheckSpec::new(
            CheckId::new("check").unwrap(),
            vec!["reference-not-executed".into(), "λ argument".into()],
            "test-toolchain/v1".into(),
            Digest::of("test/v1", b"environment"),
            Digest::of("test/v1", b"check"),
        )
        .unwrap(),
        CheckResult::Incomplete {
            reason: "original reason λ".into(),
        },
        ExecutionProvenance {
            verifier: PrincipalId::new("p").unwrap(),
            runner: "reference/v1".into(),
            run_id: "run λ".into(),
            started_at: 1,
            finished_at: 4,
        },
        vec!["diagnostic λ\0".into()],
        vec![Artifact::new("stdout", "application/octet-stream", b"\0\xff\xfe".to_vec()).unwrap()],
    )
    .unwrap()
}

fn roundtrip(record: MutationRecord) {
    let request = &record.request;
    let input = request.canonical_bytes(&scope(), &actor());
    assert_eq!(
        restore_mutation_request(
            &scope(),
            &actor(),
            request.digest(&scope(), &actor()),
            &input,
            Default::default()
        )
        .unwrap(),
        *request
    );
    for end in 0..input.len() {
        let prefix = &input[..end];
        assert!(
            restore_mutation_request(
                &scope(),
                &actor(),
                Digest::of(MutationRequest::FORMAT, prefix),
                prefix,
                Default::default()
            )
            .is_err(),
            "accepted request prefix {end}"
        );
    }
    for limits in [
        DomainDecodeLimits {
            collection_entries: 0,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 1,
            ..Default::default()
        },
    ] {
        assert!(
            restore_mutation_request(
                &scope(),
                &actor(),
                request.digest(&scope(), &actor()),
                &input,
                limits
            )
            .is_err()
        );
    }
    let value = MutationOutcomeRecord::new(scope(), key(), record.clone()).unwrap();
    let bytes = value.canonical_bytes();
    assert_eq!(
        restore_mutation_outcome(
            &scope(),
            &key(),
            request,
            value.digest(),
            &bytes,
            Default::default()
        )
        .unwrap(),
        value
    );
    assert!(value.recovery_size().unwrap().bytes >= input.len() + bytes.len());
    for end in 0..bytes.len() {
        let prefix = &bytes[..end];
        assert!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                request,
                Digest::of(MutationOutcomeRecord::FORMAT, prefix),
                prefix,
                Default::default()
            )
            .is_err(),
            "accepted outcome prefix {end}"
        );
    }
    let mut changed = request.clone();
    match &mut changed {
        MutationRequest::SubmitPatch { transaction, .. }
        | MutationRequest::RecordEvidence { transaction, .. }
        | MutationRequest::Revalidate { transaction } => {
            *transaction = TransactionId::new("substituted-target").unwrap()
        }
        _ => unreachable!(),
    }
    assert!(
        restore_mutation_outcome(
            &scope(),
            &key(),
            &changed,
            value.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
}

#[test]
fn original_native_inputs_bind_raw_snapshot_patch_and_full_verification_fields() {
    let b = bindings();
    let patch = Patch::new(
        b.base_snapshot,
        [(
            ElementId::new("x").unwrap(),
            Change {
                expected: Some(-9),
                value: None,
            },
        )]
        .into(),
    )
    .unwrap();
    for (request, outcome, state, attempt) in [
        (
            MutationRequest::SubmitPatch {
                transaction: b.transaction.clone(),
                patch: patch.clone(),
            },
            Ok(MutationValue::Bindings {
                bindings: EvidenceBindings {
                    patch: patch.id(),
                    ..b.clone()
                },
            }),
            TransactionState::Executing,
            3,
        ),
        (
            MutationRequest::RecordEvidence {
                transaction: b.transaction.clone(),
                evidence: Box::new(evidence()),
            },
            Ok(MutationValue::Evidence {
                evidence: evidence().id(),
            }),
            TransactionState::Validating,
            3,
        ),
        (
            MutationRequest::Revalidate {
                transaction: b.transaction.clone(),
            },
            Ok(MutationValue::Bindings {
                bindings: b.clone(),
            }),
            TransactionState::RevalidationRequired,
            2,
        ),
    ] {
        let record = MutationRecord {
            request,
            recorded_at_ms: 5,
            resolved_attempt: Some(attempt),
            state_before: Some(state),
            lease_events_before: 1,
            administration: None,
            outcome,
        };
        roundtrip(record.clone());
        let mut invalid = record;
        invalid.resolved_attempt = Some(99);
        assert!(MutationOutcomeRecord::new(scope(), key(), invalid).is_err());
    }
}

#[test]
fn original_native_rejections_keep_before_images_invariants_and_mismatched_evidence() {
    let b = bindings();
    let patch = Patch::new(
        b.base_snapshot,
        [(
            ElementId::new("x").unwrap(),
            Change {
                expected: Some(-9),
                value: Some(12),
            },
        )]
        .into(),
    )
    .unwrap();
    let request = MutationRequest::SubmitPatch {
        transaction: b.transaction.clone(),
        patch,
    };
    for error in [
        EngineError::WrongPatchBase,
        EngineError::UndeclaredWrite,
        EngineError::Patch(PatchError::BeforeImage {
            element: ElementId::new("x").unwrap(),
            expected: Some(-9),
            actual: None,
        }),
        EngineError::Invariant(InvariantViolation::Unsatisfied {
            invariant: InvariantId::new("limit").unwrap(),
        }),
    ] {
        roundtrip(MutationRecord {
            request: request.clone(),
            recorded_at_ms: 5,
            resolved_attempt: Some(3),
            state_before: Some(TransactionState::Executing),
            lease_events_before: 1,
            administration: None,
            outcome: Err(error),
        });
    }
    let mut original = evidence();
    // Its selector deliberately names another transaction; parsing retains the
    // exact rejected body instead of rebinding it to the resolved attempt.
    let mismatched = MutationRequest::RecordEvidence {
        transaction: TransactionId::new("other").unwrap(),
        evidence: Box::new(original.clone()),
    };
    roundtrip(MutationRecord {
        request: mismatched,
        recorded_at_ms: 5,
        resolved_attempt: Some(8),
        state_before: Some(TransactionState::Validating),
        lease_events_before: 1,
        administration: None,
        outcome: Err(EngineError::EvidenceMismatch),
    });
    // A changed original evidence body cannot reuse the accepted outcome.
    let request = MutationRequest::RecordEvidence {
        transaction: b.transaction.clone(),
        evidence: Box::new(original.clone()),
    };
    let value = MutationOutcomeRecord::new(
        scope(),
        key(),
        MutationRecord {
            request: request.clone(),
            recorded_at_ms: 5,
            resolved_attempt: Some(3),
            state_before: Some(TransactionState::Validating),
            lease_events_before: 1,
            administration: None,
            outcome: Ok(MutationValue::Evidence {
                evidence: original.id(),
            }),
        },
    )
    .unwrap();
    original = VerificationEvidence::new(
        original.bindings().clone(),
        original.check().clone(),
        CheckResult::Passed { exit_code: 0 },
        original.provenance().clone(),
        original.diagnostics().to_vec(),
        original.artifacts().to_vec(),
    )
    .unwrap();
    let changed = MutationRequest::RecordEvidence {
        transaction: b.transaction,
        evidence: Box::new(original),
    };
    assert!(
        restore_mutation_outcome(
            &scope(),
            &key(),
            &changed,
            value.digest(),
            &value.canonical_bytes(),
            Default::default()
        )
        .is_err()
    );
}
