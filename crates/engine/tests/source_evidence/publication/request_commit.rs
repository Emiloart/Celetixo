//! Actual source head advancement and failure semantics under finite fixtures.
use super::*;
use celetixo_engine::engine::authority::AuthorityState;

fn commit(
    h: &Harness,
    analyst: &Session,
    request: SourceCommitRequest,
) -> Result<SourceCommitReceipt, EngineError> {
    h.engine
        .commit_source_prepared(&h.owner, analyst, &scope(), request)
}

#[test]
fn source_commit_publishes_one_revision_analysis_history_and_original_reply() {
    let (h, attempt, analyst, request) = fixture();
    let before = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    let reads = h.reader.reads.load(Ordering::SeqCst);
    let receipt = commit(&h, &analyst, request.clone()).unwrap();
    assert!(h.reader.reads.load(Ordering::SeqCst) > reads);
    assert_eq!(receipt.publication_snapshot, request.publication_snapshot);
    assert_ne!(receipt.publication_snapshot, receipt.verified_snapshot);
    let after = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    assert_eq!(after.history.len(), 1);
    assert_eq!(after.history[0].receipt, receipt);
    assert_eq!(
        after.base.repository.as_ref().unwrap().current,
        request.publication_snapshot
    );
    assert!(
        after.base.repository.as_ref().unwrap().history.is_empty(),
        "source history must not fabricate native patch IDs"
    );
    assert_eq!(after.analysis_head, Some(request.analysis.digest()));
    assert_eq!(
        after.policy_head, before.policy_head,
        "old policy remains explicitly snapshot bound"
    );
    assert_eq!(
        after.base.transactions[&attempt.transaction].state,
        TransactionState::Committed
    );
    assert!(
        after.base.transactions[&attempt.transaction]
            .lease
            .is_none()
    );
    assert_eq!(
        after.attempts[&request.binding.source_attempt].state,
        SourceAttemptState::EvidenceSealed
    );
    assert_eq!(
        after.attempts[&request.binding.source_attempt].events,
        before.attempts[&request.binding.source_attempt].events
    );
    assert_eq!(after.base.mutations.len(), before.base.mutations.len() + 1);
    let checked = AuthorityState::from_source_records(after.clone(), Default::default()).unwrap();
    assert_eq!(checked.source_records(Default::default()).unwrap(), after);
    let reads = h.reader.reads.load(Ordering::SeqCst);
    // Original recovery is a historical answer under current owner Commit.
    h.admin.revoke(&analyst).unwrap();
    h.admin.revoke_source_executor(&h.executor).unwrap();
    h.admin.revoke_source_artifact_store(&h.storage).unwrap();
    h.admin.advance_time(10_001).unwrap();
    h.reader.unavailable.store(true, Ordering::SeqCst);
    assert_eq!(commit(&h, &analyst, request.clone()).unwrap(), receipt);
    assert_eq!(h.reader.reads.load(Ordering::SeqCst), reads);
    let mut changed = request.clone();
    changed.fence += 1;
    assert_eq!(
        commit(&h, &analyst, changed),
        Err(EngineError::IdempotencyConflict)
    );
    h.admin.revoke(&h.owner).unwrap();
    assert_eq!(
        commit(&h, &analyst, request),
        Err(EngineError::Unauthorized)
    );
    AuthorityState::from_source_records(
        h.admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap(),
        Default::default(),
    )
    .unwrap();
}

#[test]
fn prepublication_faults_leave_no_effect_and_lost_success_is_recovered_once() {
    for fault in [
        CommitFault::BeforeValidation,
        CommitFault::DuringValidation,
        CommitFault::BeforePublication,
        CommitFault::DuringPublication,
        CommitFault::AfterPublication,
    ] {
        let (h, _, analyst, request) = fixture();
        let before = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        let result = h.engine.commit_source_prepared_with_fault(
            &h.owner,
            &analyst,
            &scope(),
            request.clone(),
            fault,
        );
        let after = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        if fault == CommitFault::AfterPublication {
            assert_eq!(result, Err(EngineError::ResponseLost));
            assert_eq!(after.history.len(), 1);
        } else {
            assert_eq!(result, Err(EngineError::InjectedFailure(fault)));
            assert_eq!(after, before, "{fault:?}");
        }
        let receipt = commit(&h, &analyst, request.clone()).unwrap();
        assert_eq!(commit(&h, &analyst, request).unwrap(), receipt);
        assert_eq!(
            h.admin
                .inspect_source_records(&scope(), Default::default())
                .unwrap()
                .history
                .len(),
            1
        );
    }
}

#[test]
fn missing_or_corrupt_objects_cannot_publish_or_memoize_a_commit_decision() {
    for mode in 0..3 {
        let (h, _, analyst, request) = fixture();
        let before = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        let original = h.reader.objects.lock().unwrap().clone();
        match mode {
            0 => h.reader.unavailable.store(true, Ordering::SeqCst),
            1 => h.reader.objects.lock().unwrap().clear(),
            _ => {
                for bytes in h.reader.objects.lock().unwrap().values_mut() {
                    bytes.push(42);
                }
            }
        }
        assert!(
            matches!(
                commit(&h, &analyst, request.clone()),
                Err(EngineError::SourceEvidence(_))
            ),
            "mode {mode}"
        );
        assert_eq!(
            h.admin
                .inspect_source_records(&scope(), Default::default())
                .unwrap(),
            before
        );
        h.reader.unavailable.store(false, Ordering::SeqCst);
        *h.reader.objects.lock().unwrap() = original;
        assert!(
            commit(&h, &analyst, request).is_ok(),
            "same key can retry restored storage"
        );
    }
}

#[test]
fn opaque_sessions_and_current_roles_are_required_for_each_new_source_commit() {
    for mode in 0..5 {
        let (h, _, analyst, request) = fixture();
        let before = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        let (_, foreign_admin) = InMemoryStore::new();
        let foreign = foreign_admin
            .grant(
                analyst.principal().clone(),
                scope(),
                [Permission::ManageModel].into(),
            )
            .unwrap();
        let result = match mode {
            0 => {
                h.admin.revoke(&analyst).unwrap();
                commit(&h, &analyst, request)
            }
            1 => {
                h.admin.revoke(&h.owner).unwrap();
                commit(&h, &analyst, request)
            }
            2 => commit(&h, &foreign, request),
            3 => commit(&h, &h.observer, request),
            _ => h
                .engine
                .commit_source_prepared(&h.observer, &analyst, &scope(), request),
        };
        assert_eq!(result, Err(EngineError::Unauthorized), "mode {mode}");
        let after = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        assert_eq!(after.history, before.history);
        assert_eq!(after.base.mutations, before.base.mutations);
    }
}

#[test]
fn final_gate_rechecks_revocations_expiry_and_cancellation_after_unlocked_object_io() {
    for mode in 0..6 {
        let (h, attempt, analyst, request) = fixture();
        let admin = h.admin.clone();
        let owner = h.owner.clone();
        let analysis_session = analyst.clone();
        let executor = h.executor.clone();
        let storage = h.storage.clone();
        let engine = h.engine.clone();
        let transaction = attempt.transaction.clone();
        h.reader.on_read(move || match mode {
            0 => admin.revoke(&owner).unwrap(),
            1 => admin.revoke(&analysis_session).unwrap(),
            2 => admin.revoke_source_executor(&executor).unwrap(),
            3 => admin.revoke_source_artifact_store(&storage).unwrap(),
            4 => {
                admin.advance_time(1001).unwrap();
            }
            _ => {
                let canceller = admin
                    .grant(
                        owner.principal().clone(),
                        scope(),
                        [Permission::Cancel].into(),
                    )
                    .unwrap();
                engine
                    .abort(
                        &canceller,
                        &scope(),
                        &transaction,
                        "cancel during source commit artifact read".into(),
                    )
                    .unwrap();
            }
        });
        let expected = match mode {
            0 | 1 | 3 => EngineError::Unauthorized,
            2 => SourceAttemptError::UntrustedExecutor.into(),
            4 => EngineError::InvalidLease,
            _ => SourceAttemptError::InvalidState.into(),
        };
        assert_eq!(commit(&h, &analyst, request), Err(expected), "mode {mode}");
        let after = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        assert!(after.history.is_empty());
        assert_eq!(after.base.repository.as_ref().unwrap().current, h.base.id());
        assert_ne!(
            after.base.transactions[&attempt.transaction].state,
            TransactionState::Committed
        );
        AuthorityState::from_source_records(after, Default::default()).unwrap();
    }
}

#[test]
fn a_rejected_raw_request_keeps_its_original_result_and_does_not_poison_a_new_key() {
    let (h, _, analyst, request) = fixture();
    let mut rejected = request.clone();
    rejected.publication_proof = d("wrong-publication-proof");
    assert_eq!(
        commit(&h, &analyst, rejected.clone()),
        Err(EngineError::CommitRequestMismatch)
    );
    let after = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    AuthorityState::from_source_records(after, Default::default()).unwrap();
    assert_eq!(
        commit(&h, &analyst, request.clone()),
        Err(EngineError::IdempotencyConflict)
    );
    let mut fresh = request;
    fresh.key = IdempotencyKey::new("fresh-source-commit").unwrap();
    let receipt = commit(&h, &analyst, fresh).unwrap();
    let reads = h.reader.reads.load(Ordering::SeqCst);
    assert_eq!(
        commit(&h, &analyst, rejected),
        Err(EngineError::CommitRequestMismatch)
    );
    assert_eq!(h.reader.reads.load(Ordering::SeqCst), reads);
    assert_eq!(
        h.engine.current(&h.observer, &scope()).unwrap().id(),
        receipt.publication_snapshot
    );
}

#[test]
fn a_competing_commit_during_unlocked_reads_has_one_publication_and_original_reply() {
    for same_key in [true, false] {
        let (h, _, analyst, request) = fixture();
        let engine = h.engine.clone();
        let owner = h.owner.clone();
        let approver = analyst.clone();
        let mut competing = request.clone();
        if !same_key {
            competing.key = IdempotencyKey::new("competing-source-commit").unwrap();
        }
        h.reader.on_read(move || {
            engine
                .commit_source_prepared(&owner, &approver, &scope(), competing)
                .unwrap();
        });
        let result = commit(&h, &analyst, request.clone());
        let after = h
            .admin
            .inspect_source_records(&scope(), Default::default())
            .unwrap();
        assert_eq!(after.history.len(), 1);
        if same_key {
            assert_eq!(result.unwrap(), after.history[0].receipt);
        } else {
            assert_eq!(result, Err(SourceAttemptError::InvalidState.into()));
        }
        AuthorityState::from_source_records(after, Default::default()).unwrap();
    }
}

#[test]
fn checked_reopen_rejects_missing_or_substituted_source_publication_effects() {
    let (h, _, analyst, request) = fixture();
    commit(&h, &analyst, request.clone()).unwrap();
    let original = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    for mode in 0..8 {
        let mut rows = original.clone();
        match mode {
            0 => rows.history.clear(),
            1 => {
                rows.base
                    .mutations
                    .retain(|_, m| !matches!(m.outcome, Ok(MutationValue::SourceCommitted(_))));
            }
            2 => rows.history[0].receipt.model_generation += 1,
            3 => rows.history[0].receipt.verified_snapshot = request.publication_snapshot,
            4 => rows.base.repository.as_mut().unwrap().current = h.base.id(),
            5 => {
                rows.base
                    .transactions
                    .get_mut(&request.binding.transaction)
                    .unwrap()
                    .state = TransactionState::Validating
            }
            6 => {
                rows.base
                    .transactions
                    .get_mut(&request.binding.transaction)
                    .unwrap()
                    .lease_history
                    .pop();
            }
            _ => rows.history[0].accesses.writes.clear(),
        }
        assert!(
            AuthorityState::from_source_records(rows, Default::default()).is_err(),
            "mode {mode}"
        );
    }
}
