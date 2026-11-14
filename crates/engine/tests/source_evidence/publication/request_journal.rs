//! Metadata fixtures cannot publish, restore a capability or establish I/O.
use super::*;
use celetixo_engine::engine::authority::{AuthorityState, NativeRecordLimits};
use celetixo_engine::persistence::{
    MutationOutcomeRecord, restore_mutation_outcome, restore_mutation_request,
    restore_mutation_request_for_key, restore_source_commit_receipt,
};

fn metadata() -> (Harness, MutationKey, MutationRecord) {
    let (h, attempt, _, request) = fixture();
    let rows = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    let old = &rows.attempts[&request.binding.source_attempt];
    let receipt = SourceCommitReceipt {
        scope: scope(),
        transaction: request.binding.transaction.clone(),
        attempt: request.binding.transaction_attempt,
        source_attempt: request.binding.source_attempt.clone(),
        admission: request.admission,
        patch: request.patch,
        evidence: request.evidence,
        verified_snapshot: request.verified_snapshot,
        publication_snapshot: request.publication_snapshot,
        revision: request.publication_revision,
        model_generation: rows.base.repository.as_ref().unwrap().model.generation + 1,
        analysis: request.analysis.digest(),
        request: request.digest(&scope(), h.owner.principal()),
        committed_at_ms: rows.base.clock_ms,
    };
    let context = SourceCommitContext {
        analysis_principal: request.analysis_principal.clone(),
        analysis_grant: request.analysis_grant,
        head: Some(h.base.id()),
        model_generation: Some(receipt.model_generation - 1),
        analysis: rows.analysis_head,
        policy: rows.policy_head,
        source: Some(SourceCommitSourceContext {
            admission: request.admission,
            source_attempt: request.binding.source_attempt.clone(),
            state: old.state,
            event_count: old.events.len() as u64,
            evidence: old.evidence,
            executor_grant: old.executor_grant,
            executor_active: old.executor_grant.map(|id| rows.executors[&id].active),
            store_registration: Some(request.store_registration),
            store_active: Some(rows.artifact_stores[&request.store_registration].active),
        }),
        artifacts: Some(SourceCommitArtifactRead {
            evidence: request.evidence,
            registration: request.store_registration,
            provenance: request.storage_provenance,
            observed_at_ms: rows.base.clock_ms,
        }),
    };
    let key = MutationKey {
        actor: MutationActor::Session {
            principal: h.owner.principal().clone(),
            grant: request.authorization_grant,
        },
        key: request.key.clone(),
    };
    let record = MutationRecord {
        request: MutationRequest::SourceCommit(Box::new(request)),
        recorded_at_ms: rows.base.clock_ms,
        resolved_attempt: Some(attempt.admission.binding().transaction_attempt),
        state_before: Some(TransactionState::Validating),
        lease_events_before: rows.base.transactions[&attempt.transaction]
            .lease_history
            .len() as u64,
        administration: None,
        source_execution: None,
        source_revalidation: None,
        source_commit: Some(Box::new(context)),
        outcome: Ok(MutationValue::SourceCommitted(Box::new(receipt))),
    };
    (h, key, record)
}

fn receipt_fields(r: &SourceCommitReceipt) -> Vec<Vec<u8>> {
    vec![
        r.scope.organization.as_str().as_bytes().to_vec(),
        r.scope.workspace.as_str().as_bytes().to_vec(),
        r.scope.repository.as_str().as_bytes().to_vec(),
        r.transaction.as_str().as_bytes().to_vec(),
        r.attempt.to_be_bytes().to_vec(),
        r.source_attempt.as_str().as_bytes().to_vec(),
        r.admission.0.as_bytes().to_vec(),
        r.patch.0.as_bytes().to_vec(),
        r.evidence.0.as_bytes().to_vec(),
        r.verified_snapshot.0.as_bytes().to_vec(),
        r.publication_snapshot.0.as_bytes().to_vec(),
        r.revision.0.to_be_bytes().to_vec(),
        r.model_generation.to_be_bytes().to_vec(),
        r.analysis.as_bytes().to_vec(),
        r.request.as_bytes().to_vec(),
        r.committed_at_ms.to_be_bytes().to_vec(),
    ]
}

fn request(record: &MutationRecord) -> &SourceCommitRequest {
    let MutationRequest::SourceCommit(request) = &record.request else {
        panic!("source request fixture");
    };
    request
}
fn receipt(record: &MutationRecord) -> &SourceCommitReceipt {
    let Ok(MutationValue::SourceCommitted(receipt)) = &record.outcome else {
        panic!("source result fixture");
    };
    receipt
}

#[test]
fn exact_source_receipt_retains_both_snapshots_and_model_generation() {
    let (h, _, record) = metadata();
    let receipt = receipt(&record);
    let bytes = encode(&receipt_fields(receipt));
    assert_eq!(receipt.canonical_bytes(), bytes);
    assert_eq!(
        restore_source_commit_receipt(
            &scope(),
            h.owner.principal(),
            request(&record),
            receipt.digest(),
            &bytes,
            Default::default()
        )
        .unwrap(),
        *receipt
    );
    assert_eq!(receipt.patch, request(&record).patch);
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    for index in 0..16 {
        let mut fields = receipt_fields(receipt);
        fields[index] = match index {
            0..=3 | 5 => b"another-valid-identity".to_vec(),
            4 | 11 => 999_u64.to_be_bytes().to_vec(),
            // The journal's independently observed old generation proves the
            // exact successor; a standalone receipt only validates this bound.
            12 => 0_u64.to_be_bytes().to_vec(),
            // Expiry is an exclusive bound, not a current authorization result.
            15 => request(&record)
                .binding
                .expires_at_ms
                .to_be_bytes()
                .to_vec(),
            _ => vec![42; 32],
        };
        let changed = encode(&fields);
        assert!(
            restore_source_commit_receipt(
                &scope(),
                h.owner.principal(),
                request(&record),
                Digest::of(SourceCommitReceipt::FORMAT, &changed),
                &changed,
                Default::default()
            )
            .is_err(),
            "field {index}"
        );
    }
}

#[test]
fn shared_journal_restores_request_with_its_actual_outer_operation_key() {
    let (h, key, record) = metadata();
    let r = request(&record);
    let MutationActor::Session { principal, grant } = &key.actor else {
        unreachable!()
    };
    let mut independent = vec![
        scope().organization.as_str().as_bytes().to_vec(),
        scope().workspace.as_str().as_bytes().to_vec(),
        scope().repository.as_str().as_bytes().to_vec(),
        b"Session".to_vec(),
        principal.as_str().as_bytes().to_vec(),
        grant.to_be_bytes().to_vec(),
        b"CommitSource".to_vec(),
    ];
    independent.extend(fields(r).into_iter().skip(4));
    let bytes = encode(&independent);
    assert_eq!(bytes, record.request.canonical_bytes(&scope(), &key.actor));
    let id = Digest::of(MutationRequest::FORMAT, &bytes);
    assert_eq!(
        restore_mutation_request_for_key(&scope(), &key, id, &bytes, Default::default()).unwrap(),
        record.request
    );
    assert_eq!(
        restore_mutation_request(&scope(), &key.actor, id, &bytes, Default::default()),
        Err(RestoreError::UnsupportedDomainValue)
    );
    let mut another = key.clone();
    another.key = IdempotencyKey::new("other-operation").unwrap();
    let parsed =
        restore_mutation_request_for_key(&scope(), &another, id, &bytes, Default::default())
            .unwrap();
    let MutationRequest::SourceCommit(parsed) = parsed else {
        unreachable!()
    };
    assert_eq!(parsed.key, another.key);
    assert_eq!(
        parsed.digest(&scope(), h.owner.principal()),
        r.digest(&scope(), h.owner.principal())
    );
}

#[test]
fn source_outcome_has_an_independent_complete_canonical_layout() {
    let (_, key, record) = metadata();
    let c = record.source_commit.as_ref().unwrap();
    let s = c.source.as_ref().unwrap();
    let a = c.artifacts.as_ref().unwrap();
    let MutationActor::Session { principal, grant } = &key.actor else {
        unreachable!()
    };
    let n = |n: u64| n.to_be_bytes().to_vec();
    let mut fields = vec![
        scope().organization.as_str().as_bytes().to_vec(),
        scope().workspace.as_str().as_bytes().to_vec(),
        scope().repository.as_str().as_bytes().to_vec(),
        b"Session".to_vec(),
        principal.as_str().as_bytes().to_vec(),
        n(*grant),
        key.key.as_str().as_bytes().to_vec(),
        record
            .request
            .digest(&scope(), &key.actor)
            .as_bytes()
            .to_vec(),
        n(record.recorded_at_ms),
        n(1),
        n(record.resolved_attempt.unwrap()),
        n(1),
        b"Validating".to_vec(),
        n(record.lease_events_before),
        c.analysis_principal.as_str().as_bytes().to_vec(),
        n(c.analysis_grant),
        n(1),
        c.head.unwrap().0.as_bytes().to_vec(),
        n(1),
        n(c.model_generation.unwrap()),
        n(1),
        c.analysis.unwrap().as_bytes().to_vec(),
        n(1),
        c.policy.unwrap().as_bytes().to_vec(),
        n(1),
        s.admission.0.as_bytes().to_vec(),
        s.source_attempt.as_str().as_bytes().to_vec(),
        b"EvidenceSealed".to_vec(),
        n(s.event_count),
        n(1),
        s.evidence.unwrap().0.as_bytes().to_vec(),
        n(1),
        n(s.executor_grant.unwrap()),
        n(1),
        n(u64::from(s.executor_active.unwrap())),
        n(1),
        n(s.store_registration.unwrap()),
        n(1),
        n(u64::from(s.store_active.unwrap())),
        n(1),
        a.evidence.0.as_bytes().to_vec(),
        n(a.registration),
        a.provenance.as_bytes().to_vec(),
        n(a.observed_at_ms),
        b"Accepted".to_vec(),
        b"SourceCommitted".to_vec(),
    ];
    fields.extend(receipt_fields(receipt(&record)));
    let bytes = encode(&fields);
    let value = MutationOutcomeRecord::new(scope(), key.clone(), record.clone()).unwrap();
    assert_eq!(value.canonical_bytes(), bytes);
    assert_eq!(
        restore_mutation_outcome(
            &scope(),
            &key,
            &record.request,
            value.digest(),
            &bytes,
            Default::default()
        )
        .unwrap(),
        value
    );
    // A storage value round-trip never publishes this fabricated history.
    let mut wrong = key.clone();
    wrong.key = IdempotencyKey::new("wrong-selector").unwrap();
    assert!(
        restore_mutation_outcome(
            &scope(),
            &wrong,
            &record.request,
            value.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
}

#[test]
fn unissued_metadata_neither_bypasses_live_roles_nor_reconstructs_a_commit() {
    let (h, key, record) = metadata();
    assert_eq!(
        h.engine
            .mutate(&h.owner, &scope(), key.key.clone(), record.request.clone()),
        Err(EngineError::Unauthorized)
    );
    let mut rows = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    rows.base.mutations.insert(key, record);
    // Head/history and terminal/lease effects are absent. Metadata alone fails.
    assert!(AuthorityState::from_source_records(rows, NativeRecordLimits::default()).is_err());
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

#[test]
fn missing_selector_rejection_reconstructs_without_fake_transaction_or_admission() {
    let (h, key, mut record) = metadata();
    let MutationRequest::SourceCommit(raw) = &mut record.request else {
        unreachable!()
    };
    raw.admission = SourceAdmissionId(d("not-yet-created-admission"));
    raw.binding.transaction = TransactionId::new("missing-transaction").unwrap();
    raw.binding.transaction_attempt = 999;
    raw.binding.source_attempt = SourceAttemptId::new("unissued-source-selector").unwrap();
    record.resolved_attempt = None;
    record.state_before = None;
    record.lease_events_before = 0;
    record.source_commit.as_mut().unwrap().source = None;
    record.source_commit.as_mut().unwrap().artifacts = None;
    record.outcome = Err(EngineError::NotFound);
    let value = MutationOutcomeRecord::new(scope(), key.clone(), record.clone()).unwrap();
    let restored = restore_mutation_outcome(
        &scope(),
        &key,
        &record.request,
        value.digest(),
        &value.canonical_bytes(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(restored.record(), &record);
    let mut rows = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    let original_transactions = rows.base.transactions.len();
    let original_admissions = rows.attempts.len();
    rows.base.mutations.insert(key, record);
    let state = AuthorityState::from_source_records(rows.clone(), Default::default()).unwrap();
    assert_eq!(state.source_records(Default::default()).unwrap(), rows);
    assert_eq!(rows.base.transactions.len(), original_transactions);
    assert_eq!(rows.attempts.len(), original_admissions);
}

#[test]
fn malformed_contexts_results_and_receipts_are_rejected_after_rehashing() {
    let (_, key, record) = metadata();
    for mode in 0..10 {
        let mut changed = record.clone();
        match mode {
            0 => changed.source_commit = None,
            1 => changed.source_commit.as_mut().unwrap().analysis_grant = 0,
            2 => changed.source_commit.as_mut().unwrap().artifacts = None,
            3 => {
                changed
                    .source_commit
                    .as_mut()
                    .unwrap()
                    .source
                    .as_mut()
                    .unwrap()
                    .event_count = 0
            }
            4 => {
                changed
                    .source_commit
                    .as_mut()
                    .unwrap()
                    .artifacts
                    .as_mut()
                    .unwrap()
                    .observed_at_ms += 1
            }
            5 => changed.source_commit.as_mut().unwrap().model_generation = Some(u64::MAX),
            6 => changed.source_commit.as_mut().unwrap().head = None,
            7 => {
                let Ok(MutationValue::SourceCommitted(r)) = &mut changed.outcome else {
                    unreachable!()
                };
                r.model_generation += 1;
            }
            8 => {
                let Ok(MutationValue::SourceCommitted(r)) = &mut changed.outcome else {
                    unreachable!()
                };
                r.scope.repository = RepositoryId::new("wrong-scope").unwrap();
            }
            9 => changed.outcome = Err(EngineError::ResponseLost),
            _ => unreachable!(),
        }
        assert!(
            MutationOutcomeRecord::new(scope(), key.clone(), changed).is_err(),
            "mode {mode}"
        );
    }
    let value = MutationOutcomeRecord::new(scope(), key.clone(), record.clone()).unwrap();
    let bytes = value.canonical_bytes();
    for end in 0..bytes.len() {
        let part = &bytes[..end];
        assert!(
            restore_mutation_outcome(
                &scope(),
                &key,
                &record.request,
                Digest::of(MutationOutcomeRecord::FORMAT, part),
                part,
                Default::default()
            )
            .is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(
        restore_mutation_outcome(
            &scope(),
            &key,
            &record.request,
            Digest::of(MutationOutcomeRecord::FORMAT, &trailing),
            &trailing,
            Default::default()
        )
        .is_err()
    );
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 8,
            ..Default::default()
        },
    ] {
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key,
                &record.request,
                value.digest(),
                &bytes,
                limits
            ),
            Err(RestoreError::Limit)
        );
    }
}
