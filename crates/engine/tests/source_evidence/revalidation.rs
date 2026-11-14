//! Fresh semantic epochs over finite complete analyzer fixtures. These tests do
//! not establish language extraction completeness or sandbox isolation.
use super::*;
use celetixo_engine::engine::authority::{AuthorityState, SourceRecords};
use celetixo_engine::persistence::*;

fn key(value: &str) -> IdempotencyKey {
    IdempotencyKey::new(value).unwrap()
}
fn records(h: &Harness) -> SourceRecords {
    h.admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap()
}
fn checked(h: &Harness) {
    let records = records(h);
    let state = AuthorityState::from_source_records(records.clone(), Default::default()).unwrap();
    assert_eq!(state.source_records(Default::default()).unwrap(), records);
}
fn fresh_policy(h: &mut Harness) {
    let r = records(h);
    let facts = &r.analyses[&r.analysis_head.unwrap()];
    let mut input = h.policy.input().clone();
    input.generation += 1;
    input.snapshot = h.base.id();
    input.analysis_artifact = facts.digest();
    input.index_generation = facts.index_generation();
    input.capture.snapshot = h.base.id();
    input.capture.manifest = h.base.manifest().digest();
    if h.base.revision().0 > 0 {
        input.capture.consistency = SourceCaptureConsistency::EnginePublication;
        input.capture.capture_policy = r
            .history
            .iter()
            .find(|record| record.receipt.publication_snapshot == h.base.id())
            .unwrap()
            .receipt
            .digest();
    }
    for (path, rule) in &mut input.paths {
        rule.expected_base =
            Some(Preimage::of(&effective_workspace(&h.base).unwrap()[path]).unwrap());
    }
    let next = SourceExecutionPolicy::new(input).unwrap();
    h.engine
        .install_source_policy(&h.owner, &scope(), Some(h.policy.digest()), next.clone())
        .unwrap();
    h.policy = next;
}
fn start(h: &Harness, name: &str, path: &str, element: ElementId) -> Attempt {
    let transaction = TransactionId::new(name).unwrap();
    h.engine
        .propose(
            &h.owner,
            &scope(),
            transaction.clone(),
            h.base.id(),
            TransactionIntent {
                objective: "finite fresh revalidation".into(),
                accesses: AccessSet {
                    writes: [element].into(),
                    ..Default::default()
                },
                invariants: vec![],
                required_checks: vec![h.policy.checks()[&cid()].input().check.clone()],
                depends_on: BTreeSet::new(),
            },
        )
        .unwrap();
    h.engine.analyze(&h.owner, &scope(), &transaction).unwrap();
    h.engine
        .reserve(&h.owner, &scope(), &transaction, 1000)
        .unwrap();
    h.engine.execute(&h.owner, &scope(), &transaction).unwrap();
    run(h, &transaction, path, &format!("{name}-source-0"))
}
fn run(h: &Harness, transaction: &TransactionId, path: &str, id: &str) -> Attempt {
    let context = h
        .engine
        .source_patch_context(&h.owner, &scope(), transaction)
        .unwrap();
    let patch = SourcePatch::new(
        context
            .bind(
                SourceAttemptId::new(id).unwrap(),
                "finite fresh explicit patch".into(),
                500,
            )
            .unwrap(),
        h.policy.input().patch_policy.clone(),
        vec![PatchOperation::Replace {
            path: path.into(),
            expected: Preimage::of(
                h.base
                    .manifest()
                    .inputs
                    .iter()
                    .find(|input| input.path == path)
                    .unwrap(),
            )
            .unwrap(),
            entry: PatchEntry::file(
                if path == "src/main.rs" {
                    b"new".to_vec()
                } else {
                    b"new-z".to_vec()
                },
                FileMode::Regular,
            )
            .unwrap(),
        }],
    )
    .unwrap();
    let admission = h
        .engine
        .admit_source_attempt(&h.owner, &scope(), patch.clone(), key(id))
        .unwrap();
    h.engine
        .claim_source_attempt(&h.executor, &admission)
        .unwrap();
    let applied = h
        .engine
        .source_application(&h.executor, &admission)
        .unwrap();
    h.engine
        .mark_source_patch_applied(&h.executor, &admission)
        .unwrap();
    h.engine
        .begin_source_verification(&h.executor, &admission)
        .unwrap();
    Attempt {
        transaction: transaction.clone(),
        patch,
        admission,
        applied,
    }
}
fn seal(h: &Harness, attempt: &Attempt) -> Bundle {
    let bundle = h.seal(attempt, h.input(attempt));
    assert_eq!(
        h.deliver(attempt, &bundle).unwrap().assessment,
        SourceEvidenceAssessment::Passed
    );
    bundle
}
fn prepare_commit(h: &Harness, attempt: &Attempt, id: &str) -> SourceCommitRequest {
    let candidate = h
        .engine
        .source_publication_candidate(&h.observer, &scope(), attempt.admission.id())
        .unwrap();
    let old = records(h);
    let facts = &old.analyses[&old.analysis_head.unwrap()];
    let analysis = RepositoryAnalysis::new(
        candidate.snapshot(),
        RepositoryAnalysisInput {
            index_generation: d(id),
            analyzer_version: "finite-fresh-revalidation/v1".into(),
            configuration: d("finite-config"),
            elements: facts.elements().clone(),
            dependencies: facts.dependencies().clone(),
            checks: facts.checks().clone(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    h.engine
        .prepare_source_commit_request(
            &h.owner,
            &h.owner,
            &scope(),
            SourceCommitPreparation {
                admission: attempt.admission.id(),
                analysis,
                key: key(id),
            },
        )
        .unwrap()
}

#[test]
fn policy_change_creates_one_fresh_epoch_and_original_retry_does_not_renew_it() {
    let mut h = Harness::new();
    let old = h.start();
    let old_bundle = seal(&h, &old);
    assert!(matches!(
        h.engine.prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &old.transaction,
            900,
            key("no-change")
        ),
        Err(EngineError::InvalidIntent(_))
    ));
    fresh_policy(&mut h);
    let request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &old.transaction,
            900,
            key("fresh-policy"),
        )
        .unwrap();
    let before = records(&h);
    for fault in [
        CommitFault::BeforeValidation,
        CommitFault::DuringValidation,
        CommitFault::BeforePublication,
        CommitFault::DuringPublication,
    ] {
        assert_eq!(
            h.engine.revalidate_source_prepared_with_fault(
                &h.owner,
                &scope(),
                request.clone(),
                fault
            ),
            Err(EngineError::InjectedFailure(fault))
        );
        assert_eq!(records(&h), before);
    }
    assert_eq!(
        h.engine.revalidate_source_prepared_with_fault(
            &h.owner,
            &scope(),
            request.clone(),
            CommitFault::AfterPublication
        ),
        Err(EngineError::ResponseLost)
    );
    let receipt = h
        .engine
        .revalidate_source_prepared(&h.owner, &scope(), request.clone())
        .unwrap();
    assert_eq!(receipt.attempt, 1);
    assert_eq!(receipt.base, h.base.id());
    assert_ne!(receipt.previous_fence, receipt.fence);
    assert!(receipt.invalidated_source_attempts.is_empty());
    assert_eq!(
        records(&h).base.transactions[&old.transaction]
            .attempts
            .len(),
        2
    );
    checked(&h);
    h.admin.advance_time(10).unwrap();
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, &scope(), request.clone())
            .unwrap(),
        receipt
    );
    assert_eq!(
        records(&h).base.transactions[&old.transaction]
            .lease
            .as_ref()
            .unwrap()
            .expires_at,
        receipt.expires_at_ms
    );
    let bytes = request.canonical_bytes(&scope(), h.owner.principal());
    assert_eq!(
        restore_source_revalidation_request(
            &scope(),
            h.owner.principal(),
            request.key.clone(),
            request.digest(&scope(), h.owner.principal()),
            &bytes,
            Default::default()
        )
        .unwrap(),
        request
    );
    assert_eq!(
        restore_source_revalidation_receipt(
            &scope(),
            h.owner.principal(),
            &request,
            receipt.digest(),
            &receipt.canonical_bytes(),
            Default::default()
        )
        .unwrap(),
        receipt
    );
    let fresh = run(&h, &old.transaction, "src/main.rs", "fresh-policy-source-1");
    assert_ne!(fresh.admission.id(), old.admission.id());
    assert_eq!(fresh.admission.binding().transaction_attempt, 1);
    assert!(h.deliver(&fresh, &old_bundle).is_err());
    checked(&h);
    seal(&h, &fresh);
    let commit = prepare_commit(&h, &fresh, "fresh-policy-commit");
    h.engine
        .commit_source_prepared(&h.owner, &h.owner, &scope(), commit)
        .unwrap();
    checked(&h);
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, &scope(), request)
            .unwrap(),
        receipt
    );
}

#[test]
fn stale_parallel_source_work_reexecutes_at_current_head_then_commits_second_revision() {
    let mut h = Harness::configured(true);
    let a = start(&h, "first", "src/main.rs", element());
    let b = start(&h, "second", "src/z.rs", ElementId::new("file:z").unwrap());
    seal(&h, &a);
    let old_bundle = seal(&h, &b);
    let a_commit = prepare_commit(&h, &a, "first-commit");
    let b_stale_commit = prepare_commit(&h, &b, "second-stale-commit");
    h.engine
        .commit_source_prepared(&h.owner, &h.owner, &scope(), a_commit)
        .unwrap();
    assert!(
        h.engine
            .commit_source_prepared(&h.owner, &h.owner, &scope(), b_stale_commit)
            .is_err()
    );
    let unchanged = records(&h).base.transactions[&b.transaction].clone();
    let rejected = EngineError::IllegalTransition {
        from: TransactionState::RevalidationRequired,
        to: TransactionState::Analyzed,
    };
    assert_eq!(
        h.engine.analyze(&h.owner, &scope(), &b.transaction),
        Err(rejected.clone())
    );
    let original = h
        .engine
        .mutate(
            &h.owner,
            &scope(),
            key("generic-source-analyze-is-not-revalidation"),
            MutationRequest::Analyze {
                transaction: b.transaction.clone(),
            },
        )
        .unwrap();
    assert_eq!(original.outcome, Err(rejected));
    assert_eq!(records(&h).base.transactions[&b.transaction], unchanged);
    checked(&h);
    h.base = h.engine.current(&h.observer, &scope()).unwrap();
    fresh_policy(&mut h);
    let request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &b.transaction,
            800,
            key("second-fresh"),
        )
        .unwrap();
    let receipt = h
        .engine
        .revalidate_source_prepared(&h.owner, &scope(), request.clone())
        .unwrap();
    assert_eq!(receipt.base, h.base.id());
    assert_ne!(receipt.base, receipt.previous_base);
    assert_eq!(receipt.intervening_revisions, vec![Revision(1)]);
    checked(&h);
    assert!(
        h.engine
            .source_patch_context(&h.owner, &scope(), &b.transaction)
            .is_ok()
    );
    let fresh = run(&h, &b.transaction, "src/z.rs", "second-source-1");
    assert!(h.deliver(&fresh, &old_bundle).is_err());
    seal(&h, &fresh);
    let commit = prepare_commit(&h, &fresh, "second-commit");
    let committed = h
        .engine
        .commit_source_prepared(&h.owner, &h.owner, &scope(), commit)
        .unwrap();
    assert_eq!(committed.revision, Revision(2));
    checked(&h);
    let head = h.engine.current(&h.observer, &scope()).unwrap();
    assert_eq!(
        effective_workspace(&head).unwrap()["src/main.rs"].bytes,
        Some(b"new".to_vec())
    );
    assert_eq!(
        effective_workspace(&head).unwrap()["src/z.rs"].bytes,
        Some(b"new-z".to_vec())
    );
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, &scope(), request)
            .unwrap(),
        receipt
    );
}

#[test]
fn fresh_epoch_closes_unfinished_source_execution_and_fences_late_worker_results() {
    let mut h = Harness::new();
    let old = h.start();
    let late = h.seal(&old, h.input(&old));
    fresh_policy(&mut h);
    let request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &old.transaction,
            800,
            key("unfinished"),
        )
        .unwrap();
    let receipt = h
        .engine
        .revalidate_source_prepared(&h.owner, &scope(), request)
        .unwrap();
    assert_eq!(
        receipt.invalidated_source_attempts,
        [old.admission.binding().source_attempt.clone()].into()
    );
    let r = records(&h);
    assert_eq!(
        r.attempts[&old.admission.binding().source_attempt].state,
        SourceAttemptState::RevalidationRequired
    );
    assert!(h.deliver(&old, &late).is_err());
    assert!(
        h.engine
            .begin_source_verification(&h.executor, &old.admission)
            .is_err()
    );
    checked(&h);
    let fresh = run(&h, &old.transaction, "src/main.rs", "unfinished-source-1");
    seal(&h, &fresh);
    let commit = prepare_commit(&h, &fresh, "unfinished-commit");
    h.engine
        .commit_source_prepared(&h.owner, &h.owner, &scope(), commit)
        .unwrap();
    checked(&h);
}

#[test]
fn stale_selectors_and_original_domain_rejections_never_create_partial_epochs() {
    let mut h = Harness::new();
    let old = h.start();
    fresh_policy(&mut h);
    let request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &old.transaction,
            800,
            key("baseline"),
        )
        .unwrap();
    for case in 0..7 {
        let mut invalid = request.clone();
        invalid.key = key(&format!("bad-{case}"));
        let expected = match case {
            0 => {
                invalid.previous_impact = d("wrong-impact");
                EngineError::CommitRequestMismatch
            }
            1 => {
                invalid.head = SnapshotId(d("wrong-head"));
                EngineError::CommitRequestMismatch
            }
            2 => {
                invalid.fence += 1;
                EngineError::InvalidLease
            }
            3 => {
                invalid.ttl_ms = 0;
                EngineError::InvalidTime
            }
            4 => {
                invalid.authorization_grant += 1000;
                EngineError::Unauthorized
            }
            5 => {
                invalid.previous_attempt += 1;
                EngineError::CommitRequestMismatch
            }
            _ => {
                invalid.transaction = TransactionId::new("absent").unwrap();
                EngineError::NotFound
            }
        };
        let before = records(&h);
        assert_eq!(
            h.engine
                .revalidate_source_prepared(&h.owner, &scope(), invalid.clone()),
            Err(expected.clone())
        );
        let after = records(&h);
        assert_eq!(before.base.transactions, after.base.transactions);
        assert_eq!(before.attempts, after.attempts);
        assert_eq!(before.base.reservation_fence, after.base.reservation_fence);
        assert_eq!(before.base.mutations.len() + 1, after.base.mutations.len());
        assert_eq!(
            h.engine
                .revalidate_source_prepared(&h.owner, &scope(), invalid),
            Err(expected)
        );
        checked(&h);
    }
    h.engine
        .revalidate_source_prepared(&h.owner, &scope(), request)
        .unwrap();
    checked(&h);
}

#[test]
fn expiry_revocation_cancellation_and_foreign_sessions_cannot_revive_source_work() {
    for case in 0..4 {
        let mut h = Harness::new();
        let old = h.start();
        fresh_policy(&mut h);
        let request = h
            .engine
            .prepare_source_revalidation_request(
                &h.owner,
                &scope(),
                &old.transaction,
                800,
                key("cannot-revive"),
            )
            .unwrap();
        let caller = match case {
            0 => {
                h.admin.advance_time(1000).unwrap();
                h.owner.clone()
            }
            1 => {
                h.admin.revoke(&h.owner).unwrap();
                h.owner.clone()
            }
            2 => {
                h.engine
                    .abort(
                        &h.owner,
                        &scope(),
                        &old.transaction,
                        "human cancelled".into(),
                    )
                    .unwrap();
                h.owner.clone()
            }
            _ => {
                let (_, foreign) = InMemoryStore::new();
                foreign
                    .grant(h.owner.principal().clone(), scope(), Permission::ALL.into())
                    .unwrap()
            }
        };
        let before = records(&h);
        assert!(
            h.engine
                .revalidate_source_prepared(&caller, &scope(), request)
                .is_err()
        );
        let after = records(&h);
        assert_eq!(before.base.transactions, after.base.transactions);
        assert_eq!(before.base.reservation_fence, after.base.reservation_fence);
        assert_eq!(before.attempts, after.attempts);
        checked(&h);
    }
}

#[test]
fn checked_recovery_rejects_forged_epochs_reservations_and_original_observations() {
    let mut h = Harness::new();
    let old = h.start();
    fresh_policy(&mut h);
    let request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &old.transaction,
            800,
            key("tamper"),
        )
        .unwrap();
    h.engine
        .revalidate_source_prepared(&h.owner, &scope(), request)
        .unwrap();
    let valid = records(&h);
    checked(&h);
    for case in 0..9 {
        let mut invalid = valid.clone();
        let mutation_key = invalid
            .base
            .mutations
            .iter()
            .find(|(_, record)| matches!(record.request, MutationRequest::SourceRevalidate(_)))
            .unwrap()
            .0
            .clone();
        match case {
            0 => {
                invalid.base.mutations.remove(&mutation_key);
            }
            1 => {
                invalid
                    .base
                    .transactions
                    .get_mut(&old.transaction)
                    .unwrap()
                    .attempts[1]
                    .base = SnapshotId(d("invented-base"));
            }
            2 => {
                invalid
                    .base
                    .transactions
                    .get_mut(&old.transaction)
                    .unwrap()
                    .lease
                    .as_mut()
                    .unwrap()
                    .fence -= 1;
            }
            3 => {
                invalid
                    .base
                    .mutations
                    .get_mut(&mutation_key)
                    .unwrap()
                    .source_revalidation
                    .as_mut()
                    .unwrap()
                    .reservation_fence_before += 1;
            }
            4 => {
                invalid
                    .base
                    .mutations
                    .get_mut(&mutation_key)
                    .unwrap()
                    .source_revalidation
                    .as_mut()
                    .unwrap()
                    .leases
                    .clear();
            }
            5 => {
                invalid
                    .base
                    .mutations
                    .get_mut(&mutation_key)
                    .unwrap()
                    .source_revalidation
                    .as_mut()
                    .unwrap()
                    .source_event_counts
                    .clear();
            }
            6 => {
                let record = invalid.base.mutations.get_mut(&mutation_key).unwrap();
                let Ok(MutationValue::SourceRevalidated(receipt)) = &mut record.outcome else {
                    unreachable!()
                };
                receipt.accesses.writes.clear();
            }
            7 => {
                invalid
                    .base
                    .transactions
                    .get_mut(&old.transaction)
                    .unwrap()
                    .state = TransactionState::Analyzed;
                invalid
                    .base
                    .transactions
                    .get_mut(&old.transaction)
                    .unwrap()
                    .lease = None;
            }
            _ => {
                invalid
                    .base
                    .transactions
                    .get_mut(&old.transaction)
                    .unwrap()
                    .lease_history
                    .remove(1);
            }
        }
        assert!(
            AuthorityState::from_source_records(invalid, Default::default()).is_err(),
            "forgery {case}"
        );
    }
}

#[test]
fn publication_capture_cannot_be_invented_from_a_manifest_or_a_different_receipt() {
    let mut h = Harness::new();
    let old = h.start();
    seal(&h, &old);
    let mut forged = h.policy.input().clone();
    forged.generation += 1;
    forged.capture.consistency = SourceCaptureConsistency::EnginePublication;
    forged.capture.capture_policy = d("invented-publication");
    let before = records(&h);
    assert!(
        h.engine
            .install_source_policy(
                &h.owner,
                &scope(),
                Some(h.policy.digest()),
                SourceExecutionPolicy::new(forged).unwrap()
            )
            .is_err()
    );
    assert_eq!(records(&h), before);
    let commit = prepare_commit(&h, &old, "capture-commit");
    h.engine
        .commit_source_prepared(&h.owner, &h.owner, &scope(), commit)
        .unwrap();
    h.base = h.engine.current(&h.observer, &scope()).unwrap();
    fresh_policy(&mut h);
    checked(&h);
    let mut forged = h.policy.input().clone();
    forged.generation += 1;
    forged.capture.capture_policy = d("unrelated-receipt");
    let before = records(&h);
    assert!(
        h.engine
            .install_source_policy(
                &h.owner,
                &scope(),
                Some(h.policy.digest()),
                SourceExecutionPolicy::new(forged).unwrap()
            )
            .is_err()
    );
    assert_eq!(records(&h), before);
}

#[test]
fn fresh_semantic_dependencies_replace_old_impact_and_unknown_identities_fail_closed() {
    let mut h = Harness::configured(true);
    let old = start(
        &h,
        "semantic-change",
        "src/z.rs",
        ElementId::new("file:z").unwrap(),
    );
    let old_record = records(&h);
    let original = &old_record.analyses[&old_record.analysis_head.unwrap()];
    let facts = RepositoryAnalysis::new(
        &h.base,
        RepositoryAnalysisInput {
            index_generation: d("new-semantic-index"),
            analyzer_version: "finite-dependency-change/v1".into(),
            configuration: d("new-dependency-config"),
            elements: original.elements().clone(),
            dependencies: [(ElementId::new("file:z").unwrap(), [element()].into())].into(),
            checks: original.checks().clone(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    h.engine
        .install_repository_analysis(&h.owner, &scope(), original.digest(), facts.clone())
        .unwrap();
    fresh_policy(&mut h);
    let mut request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &old.transaction,
            800,
            key("new-semantics"),
        )
        .unwrap();
    let receipt = h
        .engine
        .revalidate_source_prepared(&h.owner, &scope(), request.clone())
        .unwrap();
    assert!(receipt.accesses.reads.contains(&element()));
    assert_ne!(
        receipt.impact,
        old_record.base.transactions[&old.transaction].attempts[0]
            .analysis
            .as_ref()
            .unwrap()
            .digest()
    );
    checked(&h);
    // The next complete graph no longer resolves an immutable declared identity.
    // This is a durable rejection, never an empty-impact epoch.
    let removed = RepositoryAnalysis::new(
        &h.base,
        RepositoryAnalysisInput {
            index_generation: d("removed-identity-index"),
            analyzer_version: "finite-removal/v1".into(),
            configuration: d("removed-config"),
            elements: [element()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element(), [cid()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    h.engine
        .install_repository_analysis(&h.owner, &scope(), facts.digest(), removed.clone())
        .unwrap();
    let mut policy = h.policy.input().clone();
    policy.generation += 1;
    policy.analysis_artifact = removed.digest();
    policy.index_generation = removed.index_generation();
    policy.paths.remove("src/z.rs");
    let policy = SourceExecutionPolicy::new(policy).unwrap();
    h.engine
        .install_source_policy(&h.owner, &scope(), Some(h.policy.digest()), policy.clone())
        .unwrap();
    let before = records(&h);
    let current = &before.base.transactions[&old.transaction].attempts[1];
    request.key = key("unresolved-intent");
    request.previous_attempt = 1;
    request.previous_base = current.base;
    request.previous_model_generation = current.model.generation;
    request.previous_impact = current.analysis.as_ref().unwrap().digest();
    request.fence = receipt.fence;
    request.model_generation = before.base.repository.as_ref().unwrap().model.generation;
    request.analysis = removed.digest();
    request.index_generation = removed.index_generation();
    request.policy = policy.digest();
    request.policy_generation = policy.input().generation;
    let expected = EngineError::RepositoryAnalysis(
        celetixo_engine::repository::RepositoryAnalysisError::UnknownElement(
            ElementId::new("file:z").unwrap(),
        ),
    );
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, &scope(), request.clone()),
        Err(expected.clone())
    );
    assert_eq!(records(&h).base.transactions, before.base.transactions);
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, &scope(), request),
        Err(expected)
    );
    checked(&h);
}

#[test]
fn original_transaction_mutations_recover_after_fresh_source_epoch_and_commit() {
    let mut h = Harness::new();
    let transaction = TransactionId::new("keyed-source").unwrap();
    let intent = TransactionIntent {
        objective: "original outcomes across source epochs".into(),
        accesses: AccessSet {
            writes: [element()].into(),
            ..Default::default()
        },
        invariants: vec![],
        required_checks: vec![h.policy.checks()[&cid()].input().check.clone()],
        depends_on: BTreeSet::new(),
    };
    let requests = [
        MutationRequest::Propose {
            transaction: transaction.clone(),
            base: h.base.id(),
            intent: Box::new(intent),
        },
        MutationRequest::Analyze {
            transaction: transaction.clone(),
        },
        MutationRequest::Reserve {
            transaction: transaction.clone(),
            ttl_ms: 1000,
        },
        MutationRequest::Execute {
            transaction: transaction.clone(),
        },
    ];
    let mut originals = Vec::new();
    for (i, request) in requests.iter().enumerate() {
        let result = h
            .engine
            .mutate(
                &h.owner,
                &scope(),
                key(&format!("initial-{i}")),
                request.clone(),
            )
            .unwrap();
        assert!(result.outcome.is_ok());
        originals.push(result);
    }
    let old = run(&h, &transaction, "src/main.rs", "keyed-source-0");
    fresh_policy(&mut h);
    let request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            &scope(),
            &transaction,
            800,
            key("keyed-fresh"),
        )
        .unwrap();
    h.engine
        .revalidate_source_prepared(&h.owner, &scope(), request)
        .unwrap();
    checked(&h);
    let fresh = run(&h, &transaction, "src/main.rs", "keyed-source-1");
    seal(&h, &fresh);
    let commit = prepare_commit(&h, &fresh, "keyed-commit");
    h.engine
        .commit_source_prepared(&h.owner, &h.owner, &scope(), commit)
        .unwrap();
    checked(&h);
    for (i, request) in requests.into_iter().enumerate() {
        let recovered = h
            .engine
            .mutate(&h.owner, &scope(), key(&format!("initial-{i}")), request)
            .unwrap();
        assert!(recovered.replayed);
        assert_eq!(recovered.outcome, originals[i].outcome);
        assert_eq!(recovered.recorded_at_ms, originals[i].recorded_at_ms);
    }
    assert_eq!(
        records(&h).attempts[&old.admission.binding().source_attempt].state,
        SourceAttemptState::RevalidationRequired
    );
    checked(&h);
}
