//! Patch, verification and revalidation replies share the atomic SQL journal.
use super::*;

fn start(h: &Harness, name: &str, write: &str, ttl: u64) -> Patch {
    h.engine
        .propose(&h.worker, h.scope(), id(name), h.base.id(), h.intent(write))
        .unwrap();
    h.engine.analyze(&h.worker, h.scope(), &id(name)).unwrap();
    h.engine
        .reserve(&h.worker, h.scope(), &id(name), ttl)
        .unwrap();
    h.engine.execute(&h.worker, h.scope(), &id(name)).unwrap();
    Patch::new(
        h.base.id(),
        [(
            e(write),
            Change {
                expected: h.base.state().get(&e(write)).copied(),
                value: Some(10),
            },
        )]
        .into(),
    )
    .unwrap()
}
fn record(h: &Harness, operation: &str, evidence: VerificationEvidence) -> MutationResponse {
    h.engine
        .mutate(
            &h.verifier,
            h.scope(),
            key(operation),
            MutationRequest::RecordEvidence {
                transaction: evidence.bindings().transaction.clone(),
                evidence: Box::new(evidence),
            },
        )
        .unwrap()
}
fn faulting_session(scope: &Scope, grant: u64, stage: AuthorityStage) -> (Engine, Session) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let armed = Arc::new(AtomicBool::new(false));
    let active = armed.clone();
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |point| {
        if active.load(Ordering::SeqCst) && point == stage {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let actor = admin
        .authenticate_existing_session(scope, grant, |_, _| Ok(()))
        .unwrap();
    armed.store(true, Ordering::SeqCst);
    (engine, actor)
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn original_native_replies_survive_later_evidence_selection_and_revalidation() {
    let mut h = Harness::new("native-mutation-effects");
    let patch = start(&h, "a", "x", 600_000);
    let request = MutationRequest::SubmitPatch {
        transaction: id("a"),
        patch,
    };
    let submitted = h
        .engine
        .mutate(&h.worker, h.scope(), key("patch"), request.clone())
        .unwrap();
    assert!(matches!(
        submitted.outcome,
        Ok(MutationValue::Bindings { .. })
    ));
    let failed = h.evidence_result("a", "original-failed", CheckResult::Failed { exit_code: 1 });
    let failed_reply = record(&h, "failed", failed.clone());
    let passed = h.evidence("a", "original-passed");
    let passed_reply = record(&h, "passed", passed.clone());
    h.reopen();
    let selected = h.engine.attempts(&h.worker, h.scope(), &id("a")).unwrap()[0]
        .evidence
        .clone();
    assert_eq!(
        selected.values().copied().collect::<Vec<_>>(),
        vec![passed.id()]
    );
    let replay = record(&h, "failed", failed);
    assert!(replay.replayed);
    assert_eq!(replay.outcome, failed_reply.outcome);
    assert_eq!(
        h.engine.attempts(&h.worker, h.scope(), &id("a")).unwrap()[0].evidence,
        selected
    );
    h.ready_at("b", &h.base, "y", 20, h.intent("y"));
    h.commit(h.request("b", "advance")).unwrap();
    assert!(matches!(
        h.commit(h.request("a", "stale")),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let attempts = h.engine.attempts(&h.worker, h.scope(), &id("a")).unwrap();
    let denied = h
        .engine
        .mutate(
            &h.worker,
            h.scope(),
            key("replace-old-attempt"),
            request.clone(),
        )
        .unwrap();
    assert_eq!(
        denied.outcome,
        Err(EngineError::IllegalTransition {
            from: TransactionState::RevalidationRequired,
            to: TransactionState::Validating
        })
    );
    assert_eq!(
        h.engine.attempts(&h.worker, h.scope(), &id("a")).unwrap(),
        attempts
    );
    let revalidate = MutationRequest::Revalidate {
        transaction: id("a"),
    };
    let revalidated = h
        .engine
        .mutate(&h.worker, h.scope(), key("revalidate"), revalidate.clone())
        .unwrap();
    assert!(
        matches!(&revalidated.outcome, Ok(MutationValue::Bindings { bindings }) if bindings.attempt == 1)
    );
    h.reopen();
    for (operation, input, expected) in [
        ("patch", request, submitted),
        ("revalidate", revalidate, revalidated),
    ] {
        let replay = h
            .engine
            .mutate(&h.worker, h.scope(), key(operation), input)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, expected.outcome);
        assert_eq!(replay.recorded_at_ms, expected.recorded_at_ms);
    }
    let reply = record(&h, "passed", passed.clone());
    assert!(reply.replayed);
    assert_eq!(reply.outcome, passed_reply.outcome);
    assert!(
        h.engine.attempts(&h.worker, h.scope(), &id("a")).unwrap()[1]
            .evidence
            .is_empty()
    );
    assert_eq!(
        record(&h, "fresh-key-stale-evidence", passed).outcome,
        Err(EngineError::EvidenceMismatch)
    );
    record(&h, "fresh-evidence", h.evidence("a", "attempt-one"));
    h.commit(h.request("a", "final")).unwrap();
    h.reopen();
    assert_eq!(
        h.engine.current(&h.worker, h.scope()).unwrap().revision(),
        Revision(2)
    );
}

fn ambiguous(h: &mut Harness, grant: u64, operation: &str, request: MutationRequest) {
    for stage in [AuthorityStage::StateLoaded, AuthorityStage::BeforeCommit] {
        let (engine, session) = faulting_session(h.scope(), grant, stage);
        let before = h.backend.inspect_registration(h.scope()).unwrap();
        assert_eq!(
            engine.mutate(&session, h.scope(), key(operation), request.clone()),
            Err(authority_error(AuthorityFailure::Unavailable))
        );
        assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    }
    let (engine, session) = faulting_session(h.scope(), grant, AuthorityStage::AfterCommit);
    assert_eq!(
        engine.mutate(&session, h.scope(), key(operation), request.clone()),
        Err(authority_error(AuthorityFailure::CommitIndeterminate))
    );
    h.reopen();
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    let session = if grant == 1 { &h.worker } else { &h.verifier };
    let reply = h
        .engine
        .mutate(session, h.scope(), key(operation), request)
        .unwrap();
    assert!(reply.replayed);
    assert!(reply.outcome.is_ok(), "{reply:?}");
    let after = h.backend.inspect_registration(h.scope()).unwrap();
    assert_eq!(before.records.mutations, after.records.mutations);
    assert_eq!(before.records.transactions, after.records.transactions);
    assert_eq!(before.records.evidence, after.records.evidence);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_mutation_faults_recover_each_original_effect_after_lost_response() {
    let mut h = Harness::new("native-mutation-faults");
    let patch = start(&h, "a", "x", 600_000);
    ambiguous(
        &mut h,
        1,
        "patch",
        MutationRequest::SubmitPatch {
            transaction: id("a"),
            patch,
        },
    );
    let evidence = h.evidence("a", "fault-evidence");
    ambiguous(
        &mut h,
        2,
        "evidence",
        MutationRequest::RecordEvidence {
            transaction: id("a"),
            evidence: Box::new(evidence),
        },
    );
    h.ready_at("b", &h.base, "y", 20, h.intent("y"));
    h.commit(h.request("b", "advance")).unwrap();
    assert!(matches!(
        h.commit(h.request("a", "stale")),
        Err(EngineError::RevalidationRequired { .. })
    ));
    ambiguous(
        &mut h,
        1,
        "revalidate",
        MutationRequest::Revalidate {
            transaction: id("a"),
        },
    );
    let records = h.backend.inspect_registration(h.scope()).unwrap().records;
    assert_eq!(records.mutations.len(), 3);
    assert_eq!(records.transactions[&id("a")].attempts.len(), 2);
    assert!(
        records.transactions[&id("a")].attempts[1]
            .evidence
            .is_empty()
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn delayed_patch_submission_cannot_publish_bindings_after_lease_expiry() {
    for journal in [false, true] {
        patch_expiry(journal);
    }
}
fn patch_expiry(journal: bool) {
    let h = Harness::new(if journal {
        "native-mutation-patch-expiry"
    } else {
        "native-direct-patch-expiry"
    });
    let patch = start(&h, "a", "x", 8_000);
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    let expiry = before.records.transactions[&id("a")]
        .lease
        .as_ref()
        .unwrap()
        .expires_at;
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            let now = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(expiry.saturating_sub(now) + 20));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let request = MutationRequest::SubmitPatch {
        transaction: id("a"),
        patch: patch.clone(),
    };
    let result = if journal {
        engine
            .mutate(&worker, h.scope(), key("patch"), request.clone())
            .map(|_| ())
    } else {
        engine
            .submit_patch(&worker, h.scope(), &id("a"), patch)
            .map(|_| ())
    };
    assert_eq!(result, Err(authority_error(AuthorityFailure::Contention)));
    assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    let retry = h
        .engine
        .mutate(&h.worker, h.scope(), key("patch"), request)
        .unwrap();
    assert!(!retry.replayed);
    assert_eq!(retry.outcome, Err(EngineError::InvalidLease));
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn delayed_revalidation_cannot_append_an_attempt_after_lease_expiry() {
    let mut h = Harness::new("native-mutation-revalidation-expiry");
    let patch = start(&h, "a", "x", 30_000);
    h.engine
        .submit_patch(&h.worker, h.scope(), &id("a"), patch)
        .unwrap();
    h.verify("a");
    h.ready_at("b", &h.base, "y", 20, h.intent("y"));
    h.commit(h.request("b", "advance")).unwrap();
    assert!(matches!(
        h.commit(h.request("a", "stale")),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let before = h.backend.inspect_registration(h.scope()).unwrap();
    let expiry = before.records.transactions[&id("a")]
        .lease
        .as_ref()
        .unwrap()
        .expires_at;
    // Wait outside a SQL transaction. The injected publication delay must cross
    // the lease deadline within the unchanged ten-second idle-transaction bound,
    // otherwise this probes connection termination instead of the final gate.
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(
        expiry.saturating_sub(now).saturating_sub(4_000),
    ));
    let backend = Arc::new(sql_authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            let now = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(expiry.saturating_sub(now) + 20));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let worker = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let request = MutationRequest::Revalidate {
        transaction: id("a"),
    };
    assert_eq!(
        engine.mutate(&worker, h.scope(), key("revalidate"), request.clone()),
        Err(authority_error(AuthorityFailure::Contention))
    );
    assert_eq!(h.backend.inspect_registration(h.scope()).unwrap(), before);
    let retry = h
        .engine
        .mutate(&h.worker, h.scope(), key("revalidate"), request)
        .unwrap();
    assert!(!retry.replayed);
    assert_eq!(retry.outcome, Err(EngineError::InvalidLease));
    h.reopen();
    assert_eq!(
        h.engine
            .attempts(&h.worker, h.scope(), &id("a"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn native_reply_constraints_bind_exact_attempt_evidence_and_admitting_verifier() {
    let h = Harness::new("native-mutation-result-constraints");
    let patch = start(&h, "a", "x", 600_000);
    h.engine
        .mutate(
            &h.worker,
            h.scope(),
            key("patch"),
            MutationRequest::SubmitPatch {
                transaction: id("a"),
                patch,
            },
        )
        .unwrap();
    record(&h, "evidence", h.evidence("a", "original"));
    // A different existing attempt without a keyed patch reply avoids the
    // duplicate-effect index when probing the exact candidate foreign key.
    h.engine
        .abort(
            &h.worker,
            h.scope(),
            &id("a"),
            "release fixture lease".into(),
        )
        .unwrap();
    let patch = start(&h, "b", "x", 600_000);
    h.engine
        .submit_patch(&h.worker, h.scope(), &id("b"), patch)
        .unwrap();
    let columns = "organization_id,workspace_id,repository_id,actor_kind,grant_kind,actor_principal_id,actor_grant_id,actor_digest,idempotency_key,request_kind,request_digest,request_payload_digest,transaction_id,recorded_at_ms,resolved_transaction_id,resolved_attempt,state_before,lease_events_before,outcome_kind,outcome_payload_digest,event_sequence,result_fence,result_expiry_ms,result_acquisition_attempt,result_kind,result_state,result_attempt,result_access_digest,result_base_snapshot_digest,result_candidate_snapshot_digest,result_patch_digest,result_analysis_digest,result_analysis_payload_digest,result_evidence_digest";
    for (variant, input, expected) in [
        (0, "patch", "23505"),
        (1, "patch", "23514"),
        (2, "patch", "23503"),
        (3, "evidence", "23503"),
    ] {
        let select = columns
            .split(',')
            .map(|column| match (variant, column) {
                (_, "idempotency_key") => "'forged-result'::text",
                (1, "result_patch_digest") => "NULL",
                (2, "result_candidate_snapshot_digest") => "decode(repeat('00',32),'hex')",
                (2, "transaction_id" | "resolved_transaction_id") => "'b'::text",
                (3, "actor_grant_id") => "1::numeric",
                (3, "actor_principal_id") => "'worker'::text",
                _ => column,
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut client = owner();
        let mut tx = client.transaction().unwrap();
        let error = tx.execute(&format!("INSERT INTO celetixo.engine_mutations ({columns}) SELECT {select} FROM celetixo.engine_mutations WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND idempotency_key=$4::text"), &[&h.scope().organization.as_str(), &h.scope().workspace.as_str(), &h.scope().repository.as_str(), &input]).unwrap_err();
        assert_eq!(error.code().unwrap().code(), expected, "variant {variant}");
        tx.rollback().unwrap();
    }
    assert_eq!(
        h.backend
            .inspect_registration(h.scope())
            .unwrap()
            .records
            .mutations
            .len(),
        2
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn rejected_revalidation_keeps_original_conflict_provenance_after_cancellation() {
    let mut h = Harness::new("native-mutation-conflict");
    let mut input = h.intent("y");
    input.accesses.reads.insert(e("x"));
    h.ready_at("reader", &h.base, "y", 20, input);
    h.ready_at("writer", &h.base, "x", 10, h.intent("x"));
    h.commit(h.request("writer", "advance")).unwrap();
    assert!(matches!(
        h.commit(h.request("reader", "stale")),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let request = MutationRequest::Revalidate {
        transaction: id("reader"),
    };
    let original = h
        .engine
        .mutate(&h.worker, h.scope(), key("revalidate"), request.clone())
        .unwrap();
    let Err(EngineError::Conflict(reports)) = &original.outcome else {
        panic!("{original:?}")
    };
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].contender, id("reader"));
    assert_eq!(reports[0].committed, id("writer"));
    assert_eq!(reports[0].committed_revision, Revision(1));
    assert!(
        reports[0]
            .contender_provenance
            .explicit
            .reads
            .contains(&e("x"))
    );
    h.engine
        .abort(
            &h.worker,
            h.scope(),
            &id("reader"),
            "invalidated read".into(),
        )
        .unwrap();
    h.reopen();
    let replay = h
        .engine
        .mutate(&h.worker, h.scope(), key("revalidate"), request)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, original.outcome);
    assert_eq!(replay.recorded_at_ms, original.recorded_at_ms);
    assert_eq!(
        h.engine.current(&h.worker, h.scope()).unwrap().revision(),
        Revision(1)
    );
}
