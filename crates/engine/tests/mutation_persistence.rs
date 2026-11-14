use celetixo_engine::transaction::TransactionState;
use celetixo_engine::{engine::*, ids::*, persistence::*};

#[path = "mutation_persistence/administrative.rs"]
mod administrative;
#[path = "mutation_persistence/native_effects.rs"]
mod native_effects;
#[path = "mutation_persistence/repository.rs"]
mod repository;

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("o").unwrap(),
        workspace: WorkspaceId::new("w").unwrap(),
        repository: RepositoryId::new("r").unwrap(),
    }
}
fn actor() -> MutationActor {
    MutationActor::Session {
        principal: PrincipalId::new("p").unwrap(),
        grant: 7,
    }
}
fn key() -> MutationKey {
    MutationKey {
        actor: actor(),
        key: IdempotencyKey::new("operation").unwrap(),
    }
}
fn request() -> MutationRequest {
    MutationRequest::Reserve {
        transaction: TransactionId::new("tx").unwrap(),
        ttl_ms: 0,
    }
}
fn rejection() -> MutationRecord {
    MutationRecord {
        request: request(),
        recorded_at_ms: 9,
        resolved_attempt: None,
        state_before: None,
        lease_events_before: 0,
        source_execution: None,
        source_commit: None,
        source_revalidation: None,
        administration: None,
        outcome: Err(EngineError::NotFound),
    }
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn number(out: &mut Vec<u8>, n: u64) {
    field(out, &n.to_be_bytes());
}
fn header() -> Vec<u8> {
    let mut out = vec![];
    for s in ["o", "w", "r", "Session", "p"] {
        field(&mut out, s.as_bytes());
    }
    number(&mut out, 7);
    out
}

#[test]
fn independent_frozen_layout_preserves_a_rejected_request_without_a_snapshot_or_transaction() {
    let request = request();
    let mut bytes = header();
    field(&mut bytes, b"Reserve");
    field(&mut bytes, b"tx");
    number(&mut bytes, 0);
    assert_eq!(bytes, request.canonical_bytes(&scope(), &actor()));
    let digest = request.digest(&scope(), &actor());
    assert_eq!(
        digest.to_string(),
        "13b9af4ed09beff0a3d6b7d50095c1f95c386a3e473da7030c215f00294bb03c"
    );
    assert_eq!(
        restore_mutation_request(&scope(), &actor(), digest, &bytes, Default::default()).unwrap(),
        request
    );
    let mut outcome_bytes = header();
    field(&mut outcome_bytes, b"operation");
    field(&mut outcome_bytes, digest.as_bytes());
    for n in [9, 0, 0, 0] {
        number(&mut outcome_bytes, n);
    }
    for s in ["Rejected", "Native", "NotFound"] {
        field(&mut outcome_bytes, s.as_bytes());
    }
    let outcome = MutationOutcomeRecord::new(scope(), key(), rejection()).unwrap();
    assert_eq!(outcome.canonical_bytes(), outcome_bytes);
    assert_eq!(
        outcome.digest().to_string(),
        "db92601fb1436d4f21f435eecf76a978fa8aec2faec351c11af7bb484341d70e"
    );
    assert_eq!(
        restore_mutation_outcome(
            &scope(),
            &key(),
            &request,
            outcome.digest(),
            &outcome_bytes,
            Default::default()
        )
        .unwrap(),
        outcome
    );
}

#[test]
fn complete_success_and_typed_rejections_roundtrip_without_infrastructure_outcomes() {
    let req = MutationRequest::Reserve {
        transaction: TransactionId::new("tx").unwrap(),
        ttl_ms: 10,
    };
    let accepted = MutationRecord {
        request: req,
        recorded_at_ms: 5,
        resolved_attempt: Some(0),
        state_before: Some(TransactionState::Analyzed),
        lease_events_before: 0,
        source_execution: None,
        source_commit: None,
        source_revalidation: None,
        administration: None,
        outcome: Ok(MutationValue::Lease {
            expires_at_ms: 15,
            fence: 8,
            acquisition_attempt: 0,
        }),
    };
    let mut records = vec![accepted];
    for error in [
        EngineError::Unauthorized,
        EngineError::InvalidTime,
        EngineError::IncompleteAnalysis,
        EngineError::ReservationConflict,
        EngineError::CounterExhausted,
        EngineError::IllegalTransition {
            from: TransactionState::Proposed,
            to: TransactionState::Reserved,
        },
    ] {
        records.push(MutationRecord {
            request: if error == EngineError::InvalidTime {
                request()
            } else {
                MutationRequest::Reserve {
                    transaction: TransactionId::new("tx").unwrap(),
                    ttl_ms: 10,
                }
            },
            recorded_at_ms: 5,
            resolved_attempt: Some(0),
            state_before: Some(if matches!(error, EngineError::IllegalTransition { .. }) {
                TransactionState::Proposed
            } else {
                TransactionState::Analyzed
            }),
            lease_events_before: 0,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: None,
            outcome: Err(error),
        });
    }
    records.push(MutationRecord {
        request: MutationRequest::Renew {
            transaction: TransactionId::new("tx").unwrap(),
            ttl_ms: 10,
        },
        recorded_at_ms: 5,
        resolved_attempt: Some(0),
        state_before: Some(TransactionState::Reserved),
        lease_events_before: 1,
        source_execution: None,
        source_commit: None,
        source_revalidation: None,
        administration: None,
        outcome: Err(EngineError::InvalidLease),
    });
    for record in records {
        let value = MutationOutcomeRecord::new(scope(), key(), record.clone()).unwrap();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                &record.request,
                value.digest(),
                &value.canonical_bytes(),
                Default::default()
            )
            .unwrap(),
            value
        );
    }
    for error in [
        EngineError::ResponseLost,
        EngineError::StoreUnavailable,
        EngineError::Authority(celetixo_engine::engine::authority::AuthorityFailure::Unavailable),
    ] {
        let mut record = rejection();
        record.outcome = Err(error);
        assert!(MutationOutcomeRecord::new(scope(), key(), record).is_err());
    }
}

#[test]
fn state_mutation_codecs_preserve_original_reasons_results_and_rejections() {
    let transaction = TransactionId::new("tx").unwrap();
    let execute = MutationRequest::Execute {
        transaction: transaction.clone(),
    };
    let cancel = MutationRequest::Abort {
        transaction: transaction.clone(),
        reason: "cancel λ\0with provenance".into(),
    };
    let bad = MutationRequest::Abort {
        transaction,
        reason: " \n\t".into(),
    };
    for record in [
        MutationRecord {
            request: execute.clone(),
            recorded_at_ms: 5,
            resolved_attempt: Some(2),
            state_before: Some(TransactionState::Reserved),
            lease_events_before: 4,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: None,
            outcome: Ok(MutationValue::State {
                state: TransactionState::Executing,
                attempt: 2,
            }),
        },
        MutationRecord {
            request: execute,
            recorded_at_ms: 5,
            resolved_attempt: Some(2),
            state_before: Some(TransactionState::Executing),
            lease_events_before: 4,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: None,
            outcome: Err(EngineError::IllegalTransition {
                from: TransactionState::Executing,
                to: TransactionState::Executing,
            }),
        },
        MutationRecord {
            request: cancel,
            recorded_at_ms: 5,
            resolved_attempt: Some(2),
            state_before: Some(TransactionState::Executing),
            lease_events_before: 4,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: None,
            outcome: Ok(MutationValue::State {
                state: TransactionState::Aborted,
                attempt: 2,
            }),
        },
        MutationRecord {
            request: bad,
            recorded_at_ms: 5,
            resolved_attempt: None,
            state_before: None,
            lease_events_before: 0,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: None,
            outcome: Err(EngineError::InvalidIntent("abort needs reason".into())),
        },
    ] {
        let bytes = record.request.canonical_bytes(&scope(), &actor());
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &actor(),
                record.request.digest(&scope(), &actor()),
                &bytes,
                Default::default()
            )
            .unwrap(),
            record.request
        );
        let outcome = MutationOutcomeRecord::new(scope(), key(), record.clone()).unwrap();
        let bytes = outcome.canonical_bytes();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                &record.request,
                outcome.digest(),
                &bytes,
                Default::default()
            )
            .unwrap(),
            outcome
        );
        for length in 0..bytes.len() {
            let truncated = &bytes[..length];
            assert!(
                restore_mutation_outcome(
                    &scope(),
                    &key(),
                    &record.request,
                    Digest::of(MutationOutcomeRecord::FORMAT, truncated),
                    truncated,
                    Default::default()
                )
                .is_err()
            );
        }
        let mut wrong = record;
        wrong.outcome = Ok(MutationValue::State {
            state: TransactionState::Committed,
            attempt: 2,
        });
        assert!(MutationOutcomeRecord::new(scope(), key(), wrong).is_err());
    }
}

#[test]
fn analysis_result_and_typed_original_errors_roundtrip_with_bounded_accesses() {
    use celetixo_engine::access::{AccessSet, Predicate};
    use celetixo_engine::repository::RepositoryAnalysisError;
    let request = MutationRequest::Analyze {
        transaction: TransactionId::new("tx").unwrap(),
    };
    let accesses = AccessSet {
        reads: [ElementId::new("dependency").unwrap()].into(),
        writes: [ElementId::new("symbol").unwrap()].into(),
        predicates: [Predicate::Prefix("generated:".into())].into(),
        ..Default::default()
    };
    let mut outcomes = vec![Ok(MutationValue::Analysis {
        attempt: 0,
        accesses,
    })];
    outcomes.extend(
        [
            EngineError::ModelChanged,
            EngineError::IncompleteAnalysis,
            EngineError::RepositoryAnalysis(RepositoryAnalysisError::UnknownElement(
                ElementId::new("unknown").unwrap(),
            )),
            EngineError::SourceAttempt(SourceAttemptError::PolicyChanged),
        ]
        .map(Err),
    );
    for outcome in outcomes {
        let record = MutationRecord {
            request: request.clone(),
            recorded_at_ms: 5,
            resolved_attempt: Some(0),
            state_before: Some(TransactionState::Proposed),
            lease_events_before: 0,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: None,
            outcome,
        };
        let value = MutationOutcomeRecord::new(scope(), key(), record.clone()).unwrap();
        let bytes = value.canonical_bytes();
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &actor(),
                request.digest(&scope(), &actor()),
                &request.canonical_bytes(&scope(), &actor()),
                Default::default()
            )
            .unwrap(),
            request
        );
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                &request,
                value.digest(),
                &bytes,
                Default::default()
            )
            .unwrap(),
            value
        );
        assert!(value.recovery_size().unwrap().bytes >= bytes.len());
        for length in 0..bytes.len() {
            let truncated = &bytes[..length];
            assert!(
                restore_mutation_outcome(
                    &scope(),
                    &key(),
                    &request,
                    Digest::of(MutationOutcomeRecord::FORMAT, truncated),
                    truncated,
                    Default::default()
                )
                .is_err()
            );
        }
        let mut wrong = record;
        wrong.state_before = Some(TransactionState::Reserved);
        assert!(MutationOutcomeRecord::new(scope(), key(), wrong).is_err());
    }
}

#[test]
fn original_proposal_codec_keeps_rejected_intent_fields_without_snapshot_lookup() {
    use celetixo_engine::{
        access::{AccessSet, Condition, Invariant},
        evidence::CheckSpec,
        transaction::TransactionIntent,
    };
    let check = CheckSpec::new(
        CheckId::new("check").unwrap(),
        vec!["not-executed".into()],
        "reference/v1".into(),
        Digest::of("check/v1", b"environment"),
        Digest::of("check/v1", b"config"),
    )
    .unwrap();
    let invariant = Invariant {
        id: InvariantId::new("invariant").unwrap(),
        condition: Condition::Present(ElementId::new("symbol").unwrap()),
    };
    for (intent, reason) in [
        (
            TransactionIntent {
                objective: " \n".into(),
                accesses: AccessSet::default(),
                invariants: vec![],
                required_checks: vec![],
                depends_on: [TransactionId::new("unknown-dependency").unwrap()].into(),
            },
            "objective and required checks must be nonempty",
        ),
        (
            TransactionIntent {
                objective: "invalid duplicate contracts".into(),
                accesses: AccessSet::default(),
                invariants: vec![invariant.clone(), invariant],
                required_checks: vec![check.clone(), check],
                depends_on: Default::default(),
            },
            "duplicate required check identity",
        ),
    ] {
        let request = MutationRequest::Propose {
            transaction: TransactionId::new("tx").unwrap(),
            base: SnapshotId(Digest::of("unknown-snapshot/v1", b"original")),
            intent: Box::new(intent),
        };
        let bytes = request.canonical_bytes(&scope(), &actor());
        let expected = request.digest(&scope(), &actor());
        assert_eq!(
            restore_mutation_request(&scope(), &actor(), expected, &bytes, Default::default())
                .unwrap(),
            request
        );
        for length in 0..bytes.len() {
            let truncated = &bytes[..length];
            assert!(
                restore_mutation_request(
                    &scope(),
                    &actor(),
                    Digest::of(MutationRequest::FORMAT, truncated),
                    truncated,
                    Default::default()
                )
                .is_err()
            );
        }
        let record = MutationRecord {
            request,
            recorded_at_ms: 5,
            resolved_attempt: None,
            state_before: None,
            lease_events_before: 0,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: None,
            outcome: Err(EngineError::InvalidIntent(reason.into())),
        };
        let value = MutationOutcomeRecord::new(scope(), key(), record.clone()).unwrap();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                &record.request,
                value.digest(),
                &value.canonical_bytes(),
                Default::default()
            )
            .unwrap(),
            value
        );
        assert!(value.recovery_size().unwrap().bytes >= bytes.len());
    }
}

#[test]
fn request_and_outcome_context_substitution_rehashed_corruption_and_trailing_fields_reject() {
    let req = request();
    let bytes = req.canonical_bytes(&scope(), &actor());
    for length in 0..bytes.len() {
        let truncated = &bytes[..length];
        let digest = Digest::of(MutationRequest::FORMAT, truncated);
        assert!(
            restore_mutation_request(&scope(), &actor(), digest, truncated, Default::default())
                .is_err(),
            "accepted request prefix {length}"
        );
    }
    let mut other_scope = scope();
    other_scope.repository = RepositoryId::new("other").unwrap();
    assert!(
        restore_mutation_request(
            &other_scope,
            &actor(),
            req.digest(&scope(), &actor()),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    let other_actor = MutationActor::Session {
        principal: PrincipalId::new("p").unwrap(),
        grant: 8,
    };
    assert!(
        restore_mutation_request(
            &scope(),
            &other_actor,
            req.digest(&scope(), &actor()),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    let outcome = MutationOutcomeRecord::new(scope(), key(), rejection()).unwrap();
    let bytes = outcome.canonical_bytes();
    for length in 0..bytes.len() {
        let truncated = &bytes[..length];
        let digest = Digest::of(MutationOutcomeRecord::FORMAT, truncated);
        assert!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                &req,
                digest,
                truncated,
                Default::default()
            )
            .is_err(),
            "accepted outcome prefix {length}"
        );
    }
    let mut changed_key = key();
    changed_key.key = IdempotencyKey::new("other").unwrap();
    assert!(
        restore_mutation_outcome(
            &scope(),
            &changed_key,
            &req,
            outcome.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    let changed_body = MutationRequest::Reserve {
        transaction: TransactionId::new("tx").unwrap(),
        ttl_ms: 1,
    };
    assert!(
        restore_mutation_outcome(
            &scope(),
            &key(),
            &changed_body,
            outcome.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    for tag in [b"Native".as_slice(), b"NotFound".as_slice()] {
        let mut changed = bytes.clone();
        let start = changed.windows(tag.len()).position(|s| s == tag).unwrap();
        changed[start] = b'!';
        let digest = Digest::of(MutationOutcomeRecord::FORMAT, &changed);
        assert!(
            restore_mutation_outcome(&scope(), &key(), &req, digest, &changed, Default::default())
                .is_err()
        );
    }
    let mut trailing = bytes;
    number(&mut trailing, 1);
    assert!(
        restore_mutation_outcome(
            &scope(),
            &key(),
            &req,
            Digest::of(MutationOutcomeRecord::FORMAT, &trailing),
            &trailing,
            Default::default()
        )
        .is_err()
    );
}

#[test]
fn zero_grants_inconsistent_outcome_contexts_and_decode_limits_fail_closed() {
    let request = request();
    let bytes = request.canonical_bytes(&scope(), &actor());
    let digest = request.digest(&scope(), &actor());
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 0,
            ..Default::default()
        },
    ] {
        assert!(restore_mutation_request(&scope(), &actor(), digest, &bytes, limits).is_err());
    }
    let invalid_actor = MutationActor::Session {
        principal: PrincipalId::new("p").unwrap(),
        grant: 0,
    };
    let bytes = request.canonical_bytes(&scope(), &invalid_actor);
    assert!(
        restore_mutation_request(
            &scope(),
            &invalid_actor,
            request.digest(&scope(), &invalid_actor),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    for corrupt in 0..3 {
        let mut record = rejection();
        match corrupt {
            0 => record.state_before = Some(TransactionState::Proposed),
            1 => record.lease_events_before = 1,
            _ => {
                record.outcome = Ok(MutationValue::Lease {
                    expires_at_ms: 0,
                    fence: 0,
                    acquisition_attempt: 0,
                })
            }
        }
        assert!(MutationOutcomeRecord::new(scope(), key(), record).is_err());
    }
}
