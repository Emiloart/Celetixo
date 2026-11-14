//! Live preview tests use finite analyzer and in-memory artifact fixtures only.
use super::*;

#[path = "publication/request.rs"]
mod request;

fn sealed() -> (Harness, Attempt) {
    let h = Harness::new();
    let attempt = h.start();
    let bundle = h.seal(&attempt, h.input(&attempt));
    assert_eq!(
        h.deliver(&attempt, &bundle).unwrap().assessment,
        SourceEvidenceAssessment::Passed
    );
    (h, attempt)
}

#[test]
fn scoped_reader_can_analyze_exact_successor_without_commit_or_model_permission() {
    let (h, attempt) = sealed();
    let reads = h.reader.reads.load(Ordering::SeqCst);
    let before = h
        .engine
        .transaction(&h.observer, &scope(), &attempt.transaction)
        .unwrap();
    let candidate = h
        .engine
        .source_publication_candidate(&h.observer, &scope(), attempt.admission.id())
        .unwrap();
    assert_eq!(candidate.application(), &attempt.applied);
    assert_eq!(candidate.snapshot().revision(), Revision(1));
    assert_eq!(
        candidate.snapshot().manifest(),
        attempt.applied.snapshot().manifest()
    );
    assert_ne!(
        candidate.snapshot().id(),
        attempt.admission.resulting_snapshot()
    );
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    assert_eq!(
        h.engine
            .transaction(&h.observer, &scope(), &attempt.transaction)
            .unwrap(),
        before
    );
    assert_eq!(h.reader.reads.load(Ordering::SeqCst), reads);
    // A proposed successor is usable for fresh analyzer input, without being
    // selected as the repository head or accepted as authoritative analysis.
    let facts = RepositoryAnalysis::new(
        candidate.snapshot(),
        RepositoryAnalysisInput {
            index_generation: d("publication-index"),
            analyzer_version: "finite-publication-fixture/v1".into(),
            configuration: d("publication-config"),
            elements: [element()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element(), [cid()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    let proof = celetixo_engine::source_publication::SourcePublicationAnalysis::new(
        &h.base,
        &attempt.patch,
        facts,
    )
    .unwrap();
    assert_eq!(proof.candidate(), &candidate);
    assert_eq!(
        h.engine.commit(
            &h.owner,
            &scope(),
            &attempt.transaction,
            IdempotencyKey::new("still-unpublished").unwrap()
        ),
        Err(EngineError::SourceMutationUnsupported)
    );
}

#[test]
fn unsealed_failed_and_unknown_admissions_cannot_supply_preview() {
    let h = Harness::new();
    let attempt = h.start();
    assert_eq!(
        h.engine
            .source_publication_candidate(&h.observer, &scope(), attempt.admission.id()),
        Err(SourceAttemptError::InvalidState.into())
    );
    let mut input = h.input(&attempt);
    input.outcome = SourceCheckOutcome::Exited { code: 1 };
    let bundle = h.seal(&attempt, input);
    assert!(matches!(
        h.deliver(&attempt, &bundle).unwrap().assessment,
        SourceEvidenceAssessment::Failed { .. }
    ));
    assert_eq!(
        h.engine
            .source_publication_candidate(&h.observer, &scope(), attempt.admission.id()),
        Err(SourceAttemptError::InvalidState.into())
    );
    assert_eq!(
        h.engine.source_publication_candidate(
            &h.observer,
            &scope(),
            SourceAdmissionId(d("unknown-admission"))
        ),
        Err(EngineError::NotFound)
    );
}

#[test]
fn fresh_opaque_reader_and_current_owner_authorization_are_required() {
    for mode in 0..4 {
        let (h, attempt) = sealed();
        let reader = match mode {
            0 => {
                h.admin.revoke(&h.observer).unwrap();
                h.observer.clone()
            }
            1 => {
                let (_, foreign) = InMemoryStore::new();
                foreign
                    .grant(
                        h.observer.principal().clone(),
                        scope(),
                        [Permission::Read].into(),
                    )
                    .unwrap()
            }
            2 => {
                let mut foreign = scope();
                foreign.repository = RepositoryId::new("other-repository").unwrap();
                h.admin
                    .grant(
                        h.observer.principal().clone(),
                        foreign,
                        [Permission::Read].into(),
                    )
                    .unwrap()
            }
            3 => {
                h.admin.revoke(&h.owner).unwrap();
                h.observer.clone()
            }
            _ => unreachable!(),
        };
        assert_eq!(
            h.engine
                .source_publication_candidate(&reader, &scope(), attempt.admission.id()),
            Err(EngineError::Unauthorized),
            "mode {mode}"
        );
    }
}

#[test]
fn expiry_cancellation_and_changed_model_require_new_work() {
    for mode in 0..4 {
        let (h, attempt) = sealed();
        match mode {
            0 => {
                h.admin.advance_time(501).unwrap();
            }
            1 => {
                h.admin.advance_time(1001).unwrap();
            }
            2 => {
                h.engine
                    .abort(
                        &h.owner,
                        &scope(),
                        &attempt.transaction,
                        "cancel publication".into(),
                    )
                    .unwrap();
            }
            3 => {
                let mut input = h.policy.input().clone();
                input.generation += 1;
                h.engine
                    .install_source_policy(
                        &h.owner,
                        &scope(),
                        Some(h.policy.digest()),
                        SourceExecutionPolicy::new(input).unwrap(),
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let result =
            h.engine
                .source_publication_candidate(&h.observer, &scope(), attempt.admission.id());
        match mode {
            0 => assert_eq!(result, Err(SourceAttemptError::Expired.into())),
            1 => assert_eq!(result, Err(EngineError::InvalidLease)),
            2 => assert_eq!(result, Err(SourceAttemptError::InvalidState.into())),
            3 => assert_eq!(result, Err(EngineError::ModelChanged)),
            _ => unreachable!(),
        }
        assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
    }
}

#[test]
fn preview_never_substitutes_for_current_artifact_availability() {
    let (h, attempt) = sealed();
    h.reader.unavailable.store(true, Ordering::SeqCst);
    h.admin.revoke_source_artifact_store(&h.storage).unwrap();
    // This is a source-byte proposal for analysis, not commit preparation. The
    // commit gate must separately resolve and reread the required artifacts.
    let candidate = h
        .engine
        .source_publication_candidate(&h.observer, &scope(), attempt.admission.id())
        .unwrap();
    assert_eq!(candidate.application(), &attempt.applied);
    assert_eq!(h.engine.current(&h.observer, &scope()).unwrap(), h.base);
}
