use super::*;
use celetixo_engine::persistence::{
    DomainDecodeLimits, RestoreError, restore_source_commit_request,
};
use celetixo_engine::source_publication::SourcePublicationCandidate;

#[path = "request_commit.rs"]
mod commit;
#[path = "request_journal.rs"]
mod journal;

fn analysis(h: &Harness, attempt: &Attempt) -> RepositoryAnalysis {
    let candidate = SourcePublicationCandidate::new(&h.base, &attempt.patch).unwrap();
    RepositoryAnalysis::new(
        candidate.snapshot(),
        RepositoryAnalysisInput {
            index_generation: d("approved-publication-index"),
            analyzer_version: "finite-request-fixture/v1".into(),
            configuration: d("approved-publication-config"),
            elements: [element()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element(), [cid()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap()
}

fn fixture() -> (Harness, Attempt, Session, SourceCommitRequest) {
    let mut h = Harness::new();
    let bootstrap = h.owner.clone();
    // The actual owner has neither Read nor ManageModel nor Verify. The model
    // approver has only ManageModel, under a different principal and grant.
    h.owner = h
        .admin
        .grant(
            bootstrap.principal().clone(),
            scope(),
            [
                Permission::Propose,
                Permission::Analyze,
                Permission::Reserve,
                Permission::Execute,
                Permission::Commit,
            ]
            .into(),
        )
        .unwrap();
    h.admin.revoke(&bootstrap).unwrap();
    let analyst = h
        .admin
        .grant(
            PrincipalId::new("independent-analysis-approver").unwrap(),
            scope(),
            [Permission::ManageModel].into(),
        )
        .unwrap();
    let attempt = h.start();
    let bundle = h.seal(&attempt, h.input(&attempt));
    h.deliver(&attempt, &bundle).unwrap();
    let request = prepare(&h, &attempt, &h.owner, &analyst).unwrap();
    (h, attempt, analyst, request)
}

fn prepare(
    h: &Harness,
    attempt: &Attempt,
    owner: &Session,
    analyst: &Session,
) -> Result<SourceCommitRequest, EngineError> {
    h.engine.prepare_source_commit_request(
        owner,
        analyst,
        &scope(),
        SourceCommitPreparation {
            admission: attempt.admission.id(),
            analysis: analysis(h, attempt),
            key: IdempotencyKey::new("source-commit-key").unwrap(),
        },
    )
}

#[test]
fn preparation_uses_distinct_least_privilege_sessions_and_preserves_exact_bindings() {
    let (h, attempt, analyst, request) = fixture();
    assert_eq!(
        h.engine.current(&h.owner, &scope()),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(
        h.engine.current(&analyst, &scope()),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(request.binding, *attempt.admission.binding());
    assert_eq!(request.analysis_principal, *analyst.principal());
    assert_ne!(request.analysis_grant, request.authorization_grant);
    assert_eq!(request.admission, attempt.admission.id());
    assert_eq!(request.patch, attempt.patch.id());
    assert_eq!(request.operations, attempt.patch.operation_digest());
    assert_eq!(request.policy, h.policy.digest());
    assert_eq!(request.policy_generation, h.policy.input().generation);
    assert_eq!(request.plan, attempt.admission.plan().id());
    assert_eq!(request.verified_snapshot, attempt.applied.snapshot().id());
    assert_eq!(
        request.verified_manifest,
        attempt.admission.resulting_manifest()
    );
    assert_eq!(
        request.application_report,
        attempt.applied.report().digest()
    );
    assert_ne!(request.publication_snapshot, request.verified_snapshot);
    assert_eq!(request.publication_revision, Revision(1));
    assert_eq!(request.analysis.snapshot(), request.publication_snapshot);
    let reads = h.reader.reads.load(Ordering::SeqCst);
    assert_eq!(prepare(&h, &attempt, &h.owner, &analyst).unwrap(), request);
    assert_eq!(h.reader.reads.load(Ordering::SeqCst), reads);
    let (evidence, receipt) = h
        .engine
        .source_evidence_record(&h.observer, &scope(), request.evidence)
        .unwrap();
    assert_eq!(receipt.assessment, SourceEvidenceAssessment::Passed);
    assert_eq!(evidence.binding().admission(), request.admission);
    assert_eq!(request.storage_provenance, receipt.storage_provenance);
    assert_eq!(
        request.evidence_receipt,
        celetixo_engine::persistence::SourceEvidenceReceiptRecord::new(
            scope(),
            request.store_registration,
            receipt
        )
        .unwrap()
        .digest()
    );
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    assert_eq!(
        h.engine
            .transaction(&h.observer, &scope(), &attempt.transaction)
            .unwrap()
            .state,
        TransactionState::Validating
    );
    assert_eq!(
        h.engine.commit(
            &h.owner,
            &scope(),
            &attempt.transaction,
            request.key.clone()
        ),
        Err(EngineError::SourceMutationUnsupported)
    );
}

#[test]
fn stale_and_incomplete_analysis_cannot_prepare_a_source_commit() {
    let (h, attempt, analyst, request) = fixture();
    for mode in 0..3 {
        let snapshot = match mode {
            0 => &h.base,
            1 => attempt.applied.snapshot(),
            _ => {
                // The same publication identity with an explicit gap is not
                // a complete-impact claim, even under an authorized approver.
                let candidate = SourcePublicationCandidate::new(&h.base, &attempt.patch).unwrap();
                let mut candidate_facts = request.analysis.clone();
                let input = RepositoryAnalysisInput {
                    index_generation: candidate_facts.index_generation(),
                    analyzer_version: candidate_facts.analyzer_version().into(),
                    configuration: candidate_facts.configuration(),
                    elements: candidate_facts.elements().clone(),
                    dependencies: candidate_facts.dependencies().clone(),
                    checks: candidate_facts.checks().clone(),
                    gaps: [celetixo_engine::repository::AnalysisGap {
                        stage: "semantic resolution".into(),
                        path: Some("src/main.rs".into()),
                        detail: "unresolved reference".into(),
                    }]
                    .into(),
                };
                candidate_facts = RepositoryAnalysis::new(candidate.snapshot(), input).unwrap();
                assert_eq!(
                    h.engine.prepare_source_commit_request(
                        &h.owner,
                        &analyst,
                        &scope(),
                        SourceCommitPreparation {
                            admission: attempt.admission.id(),
                            analysis: candidate_facts,
                            key: request.key.clone(),
                        }
                    ),
                    Err(EngineError::IncompleteAnalysis)
                );
                continue;
            }
        };
        let stale = RepositoryAnalysis::new(
            snapshot,
            RepositoryAnalysisInput {
                index_generation: request.analysis.index_generation(),
                analyzer_version: request.analysis.analyzer_version().into(),
                configuration: request.analysis.configuration(),
                elements: request.analysis.elements().clone(),
                dependencies: request.analysis.dependencies().clone(),
                checks: request.analysis.checks().clone(),
                gaps: BTreeSet::new(),
            },
        )
        .unwrap();
        assert_eq!(
            h.engine.prepare_source_commit_request(
                &h.owner,
                &analyst,
                &scope(),
                SourceCommitPreparation {
                    admission: attempt.admission.id(),
                    analysis: stale,
                    key: request.key.clone(),
                }
            ),
            Err(EngineError::RepositoryAnalysis(
                celetixo_engine::repository::RepositoryAnalysisError::SnapshotMismatch
            ))
        );
    }
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

#[test]
fn a_previously_prepared_body_does_not_cache_permission_lease_or_lifecycle_authority() {
    for mode in 0..5 {
        let (h, attempt, analyst, request) = fixture();
        match mode {
            0 => h.admin.revoke(&h.owner).unwrap(),
            1 => h.admin.revoke(&analyst).unwrap(),
            2 => {
                h.admin.advance_time(1001).unwrap();
            }
            3 => {
                h.admin.advance_time(501).unwrap();
            }
            4 => {
                // Cancel is deliberately a separate current scoped grant.
                let canceller = h
                    .admin
                    .grant(
                        h.owner.principal().clone(),
                        scope(),
                        [Permission::Cancel].into(),
                    )
                    .unwrap();
                h.engine
                    .abort(
                        &canceller,
                        &scope(),
                        &attempt.transaction,
                        "cancel prepared source commit".into(),
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let result = prepare(&h, &attempt, &h.owner, &analyst);
        let expected = match mode {
            0 | 1 => EngineError::Unauthorized,
            2 => EngineError::InvalidLease,
            3 => SourceAttemptError::Expired.into(),
            4 => SourceAttemptError::InvalidState.into(),
            _ => unreachable!(),
        };
        assert_eq!(result, Err(expected), "mode {mode}");
        // The old request remains merely historical metadata.
        assert_eq!(request.publication_revision, Revision(1));
        assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    }
}

#[test]
fn metadata_cannot_substitute_for_either_current_opaque_session() {
    let (h, attempt, analyst, _) = fixture();
    assert_eq!(
        prepare(&h, &attempt, &h.observer, &analyst),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(
        prepare(&h, &attempt, &h.owner, &h.observer),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(
        prepare(&h, &attempt, &h.owner, &h.owner),
        Err(EngineError::Unauthorized)
    );
    let (_, other_admin) = InMemoryStore::new();
    let foreign = other_admin
        .grant(
            analyst.principal().clone(),
            scope(),
            [Permission::ManageModel].into(),
        )
        .unwrap();
    assert_eq!(
        prepare(&h, &attempt, &h.owner, &foreign),
        Err(EngineError::Unauthorized)
    );
    let replacement = h
        .admin
        .grant(
            h.owner.principal().clone(),
            scope(),
            [Permission::Commit].into(),
        )
        .unwrap();
    assert_eq!(
        prepare(&h, &attempt, &replacement, &analyst),
        Err(EngineError::Unauthorized)
    );
    h.admin.revoke(&analyst).unwrap();
    assert_eq!(
        prepare(&h, &attempt, &h.owner, &analyst),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

#[test]
fn oversized_analyzer_input_is_bounded_before_preparation_replay() {
    let (h, attempt, analyst, request) = fixture();
    let candidate = SourcePublicationCandidate::new(&h.base, &attempt.patch).unwrap();
    let facts = RepositoryAnalysis::new(
        candidate.snapshot(),
        RepositoryAnalysisInput {
            index_generation: request.analysis.index_generation(),
            analyzer_version: request.analysis.analyzer_version().into(),
            configuration: request.analysis.configuration(),
            elements: (0..100_001)
                .map(|n| ElementId::new(format!("element:{n:06}")).unwrap())
                .collect(),
            dependencies: BTreeMap::new(),
            checks: BTreeMap::new(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    let reads = h.reader.reads.load(Ordering::SeqCst);
    assert_eq!(
        h.engine.prepare_source_commit_request(
            &h.owner,
            &analyst,
            &scope(),
            SourceCommitPreparation {
                admission: attempt.admission.id(),
                analysis: facts,
                key: request.key,
            }
        ),
        Err(EngineError::InvalidIntent(
            "source commit analysis exceeds recovery limits".into()
        ))
    );
    assert_eq!(h.reader.reads.load(Ordering::SeqCst), reads);
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

// Independent original-request encoder. No Canonical or request writer is used.
fn fields(request: &SourceCommitRequest) -> Vec<Vec<u8>> {
    let scope = scope();
    let b = &request.binding;
    vec![
        scope.organization.as_str().as_bytes().to_vec(),
        scope.workspace.as_str().as_bytes().to_vec(),
        scope.repository.as_str().as_bytes().to_vec(),
        b.principal.as_str().as_bytes().to_vec(),
        b.scope.organization.as_str().as_bytes().to_vec(),
        b.scope.workspace.as_str().as_bytes().to_vec(),
        b.scope.repository.as_str().as_bytes().to_vec(),
        b.principal.as_str().as_bytes().to_vec(),
        b.transaction.as_str().as_bytes().to_vec(),
        b.transaction_attempt.to_be_bytes().to_vec(),
        b.source_attempt.as_str().as_bytes().to_vec(),
        b.base_snapshot.0.as_bytes().to_vec(),
        b.base_revision.0.to_be_bytes().to_vec(),
        b.analysis_artifact.as_bytes().to_vec(),
        b.index_generation.as_bytes().to_vec(),
        b.impact_request.as_bytes().to_vec(),
        b.impact_result.as_bytes().to_vec(),
        b.created_at_ms.to_be_bytes().to_vec(),
        b.expires_at_ms.to_be_bytes().to_vec(),
        b.producer.as_bytes().to_vec(),
        request.authorization_grant.to_be_bytes().to_vec(),
        request.fence.to_be_bytes().to_vec(),
        request.analysis_principal.as_str().as_bytes().to_vec(),
        request.analysis_grant.to_be_bytes().to_vec(),
        request.admission.0.as_bytes().to_vec(),
        request.patch.0.as_bytes().to_vec(),
        request.operations.as_bytes().to_vec(),
        request.policy.as_bytes().to_vec(),
        request.policy_generation.to_be_bytes().to_vec(),
        request.plan.0.as_bytes().to_vec(),
        request.verified_snapshot.0.as_bytes().to_vec(),
        request.verified_manifest.as_bytes().to_vec(),
        request.application_report.as_bytes().to_vec(),
        request.publication_snapshot.0.as_bytes().to_vec(),
        request.publication_revision.0.to_be_bytes().to_vec(),
        request.publication_proof.as_bytes().to_vec(),
        request.evidence.0.as_bytes().to_vec(),
        request.evidence_receipt.as_bytes().to_vec(),
        request.store_registration.to_be_bytes().to_vec(),
        request.storage_provenance.as_bytes().to_vec(),
        request.analysis.digest().as_bytes().to_vec(),
        request.analysis.canonical_bytes(),
    ]
}
fn encode(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut result = Vec::new();
    for field in fields {
        result.extend_from_slice(&(field.len() as u64).to_be_bytes());
        result.extend_from_slice(field);
    }
    result
}
fn restore(
    owner: &PrincipalId,
    key: IdempotencyKey,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceCommitRequest, RestoreError> {
    restore_source_commit_request(
        &scope(),
        owner,
        key,
        Digest::of(SourceCommitRequest::FORMAT, bytes),
        bytes,
        limits,
    )
}

#[test]
fn canonical_original_request_round_trip_binds_every_field_and_both_actor_contexts() {
    let (h, _, _, request) = fixture();
    let bytes = encode(&fields(&request));
    assert_eq!(
        bytes,
        request.canonical_bytes(&scope(), h.owner.principal())
    );
    assert_eq!(
        restore(
            h.owner.principal(),
            request.key.clone(),
            &bytes,
            Default::default()
        )
        .unwrap(),
        request
    );
    let mut other_key = request.clone();
    other_key.key = IdempotencyKey::new("other-key").unwrap();
    assert_eq!(
        request.digest(&scope(), h.owner.principal()),
        other_key.digest(&scope(), h.owner.principal())
    );
    assert_eq!(
        restore(
            h.owner.principal(),
            other_key.key.clone(),
            &bytes,
            Default::default()
        )
        .unwrap(),
        other_key
    );
    let other_principal = PrincipalId::new("other-authenticated-owner").unwrap();
    assert_ne!(
        request.digest(&scope(), h.owner.principal()),
        request.digest(&scope(), &other_principal)
    );
    assert_eq!(
        restore(
            &other_principal,
            request.key.clone(),
            &bytes,
            Default::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
    let original = fields(&request);
    for index in 4..41 {
        let mut changed = original.clone();
        changed[index] = match index {
            4..=8 | 10 | 19 | 22 => b"different-selector".to_vec(),
            9 | 12 | 17 | 18 | 20 | 21 | 23 | 28 | 34 | 38 => 999_u64.to_be_bytes().to_vec(),
            _ => vec![42; 32],
        };
        let altered = encode(&changed);
        assert_ne!(
            Digest::of(SourceCommitRequest::FORMAT, &altered),
            request.digest(&scope(), h.owner.principal()),
            "field {index}"
        );
        // Selector parsing retains rejected bodies; corruption of the nested
        // analysis reference instead fails its independent content check.
        if index < 40 {
            let parsed = restore(
                h.owner.principal(),
                request.key.clone(),
                &altered,
                Default::default(),
            )
            .unwrap();
            assert_eq!(
                parsed.canonical_bytes(&scope(), h.owner.principal()),
                altered
            );
        } else {
            assert!(
                restore(
                    h.owner.principal(),
                    request.key.clone(),
                    &altered,
                    Default::default()
                )
                .is_err()
            );
        }
    }
}

#[test]
fn raw_rejected_scope_grants_fence_and_attempt_survive_under_independent_actor_context() {
    let (h, _, _, mut request) = fixture();
    request.binding.scope.repository = RepositoryId::new("wrong-raw-repository").unwrap();
    request.binding.principal = PrincipalId::new("wrong-raw-owner").unwrap();
    request.binding.transaction_attempt = 999;
    request.binding.created_at_ms = u64::MAX;
    request.binding.expires_at_ms = 0;
    request.authorization_grant = 0;
    request.analysis_grant = 0;
    request.fence = 0;
    request.store_registration = 0;
    let bytes = request.canonical_bytes(&scope(), h.owner.principal());
    let decoded = restore(
        h.owner.principal(),
        request.key.clone(),
        &bytes,
        Default::default(),
    )
    .unwrap();
    assert_eq!(decoded, request);
    assert_ne!(decoded.binding.principal, *h.owner.principal());
    assert_ne!(decoded.binding.scope, scope());
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}

#[test]
fn malformed_original_requests_and_analyzer_decode_limits_are_rejected() {
    let (h, _, _, request) = fixture();
    let bytes = request.canonical_bytes(&scope(), h.owner.principal());
    for end in 0..bytes.len() {
        assert!(
            restore(
                h.owner.principal(),
                request.key.clone(),
                &bytes[..end],
                Default::default()
            )
            .is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        restore(
            h.owner.principal(),
            request.key.clone(),
            &trailing,
            Default::default()
        ),
        Err(RestoreError::NonCanonical)
    );
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 32,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 1,
            ..Default::default()
        },
    ] {
        assert_eq!(
            restore(h.owner.principal(), request.key.clone(), &bytes, limits),
            Err(RestoreError::Limit)
        );
    }
}
