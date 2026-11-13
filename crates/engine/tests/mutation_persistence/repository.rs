use super::*;
use celetixo_engine::access::Completeness;
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::snapshot::{InclusionPolicy, Snapshot, SnapshotManifest};
use std::collections::BTreeMap;

#[test]
fn original_source_analysis_requests_preserve_their_own_unregistered_bindings() {
    use celetixo_engine::repository::{
        RepositoryAnalysis, RepositoryAnalysisError, RepositoryAnalysisInput,
    };
    let base = Snapshot::new(
        scope(),
        Revision(0),
        SnapshotManifest {
            base_commit: "reference".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "source-reference/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();
    let analysis = RepositoryAnalysis::new(
        &base,
        RepositoryAnalysisInput {
            index_generation: Digest::of("test/v1", b"index"),
            analyzer_version: "source/v1".into(),
            configuration: Digest::of("test/v1", b"configuration"),
            elements: [ElementId::new("x").unwrap()].into(),
            dependencies: BTreeMap::new(),
            checks: BTreeMap::new(),
            gaps: Default::default(),
        },
    )
    .unwrap();
    let source_model = EngineeringModel {
        generation: 1,
        analyzer: "celetixo-repository".into(),
        version: analysis.analyzer_version().into(),
        configuration: analysis.digest(),
        dependencies: analysis.dependencies().clone(),
        completeness: Completeness::Complete,
    };
    let mut other_scope = scope();
    other_scope.repository = RepositoryId::new("unregistered").unwrap();
    let foreign = Snapshot::new(
        other_scope,
        Revision(0),
        base.manifest().clone(),
        BTreeMap::new(),
    )
    .unwrap();
    let foreign_analysis = RepositoryAnalysis::new(
        &foreign,
        RepositoryAnalysisInput {
            index_generation: analysis.index_generation(),
            analyzer_version: analysis.analyzer_version().into(),
            configuration: analysis.configuration(),
            elements: analysis.elements().clone(),
            dependencies: analysis.dependencies().clone(),
            checks: analysis.checks().clone(),
            gaps: analysis.gaps().clone(),
        },
    )
    .unwrap();
    let requests = [
        (
            RepositoryMutation::RegisterRepository {
                snapshot: Box::new(base.clone()),
                analysis: Box::new(analysis.clone()),
            },
            Ok(MutationValue::Repository {
                snapshot: base.id(),
                model: Box::new(source_model),
            }),
        ),
        (
            RepositoryMutation::RegisterRepository {
                snapshot: Box::new(base.clone()),
                analysis: Box::new(foreign_analysis.clone()),
            },
            Err(EngineError::RepositoryAnalysis(
                RepositoryAnalysisError::SnapshotMismatch,
            )),
        ),
        (
            RepositoryMutation::InstallAnalysis {
                expected: analysis.digest(),
                analysis: Box::new(foreign_analysis),
            },
            Err(EngineError::RepositoryAnalysis(
                RepositoryAnalysisError::SnapshotMismatch,
            )),
        ),
    ];
    for (request, outcome) in requests {
        let request = MutationRequest::Repository(Box::new(request));
        let bytes = request.canonical_bytes(&scope(), &actor());
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &actor(),
                request.digest(&scope(), &actor()),
                &bytes,
                Default::default()
            )
            .unwrap(),
            request
        );
        for length in 0..bytes.len() {
            let prefix = &bytes[..length];
            assert!(
                restore_mutation_request(
                    &scope(),
                    &actor(),
                    Digest::of(MutationRequest::FORMAT, prefix),
                    prefix,
                    Default::default()
                )
                .is_err()
            );
        }
        let record = MutationOutcomeRecord::new(
            scope(),
            key(),
            MutationRecord {
                request: request.clone(),
                recorded_at_ms: 9,
                resolved_attempt: None,
                state_before: None,
                lease_events_before: 0,
                source_execution: None,
                administration: None,
                outcome,
            },
        )
        .unwrap();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                &request,
                record.digest(),
                &record.canonical_bytes(),
                Default::default()
            )
            .unwrap(),
            record
        );
        assert!(
            record.recovery_size().unwrap().bytes >= bytes.len() + record.canonical_bytes().len()
        );
    }
    assert_eq!(
        restore_repository_analysis(
            &base,
            analysis.index_generation(),
            analysis.digest(),
            &analysis.canonical_bytes(),
            Default::default()
        )
        .unwrap(),
        analysis
    );
    assert_eq!(
        restore_repository_analysis(
            &foreign,
            analysis.index_generation(),
            analysis.digest(),
            &analysis.canonical_bytes(),
            Default::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
}

#[test]
fn original_repository_codecs_preserve_invalid_models_and_require_no_transaction() {
    let snapshot = Snapshot::new(
        scope(),
        Revision(0),
        SnapshotManifest {
            base_commit: "reference".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "reference/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();
    let model = EngineeringModel {
        generation: 1,
        analyzer: "reference".into(),
        version: "v1".into(),
        configuration: Digest::of("test/v1", b"configuration"),
        dependencies: [(
            ElementId::new("x").unwrap(),
            [ElementId::new("y").unwrap()].into(),
        )]
        .into(),
        completeness: Completeness::Incomplete {
            reasons: vec!["reference reason λ\0".into()],
        },
    };
    let mut invalid = model.clone();
    invalid.generation = 0;
    invalid.analyzer.clear();
    for request in [
        RepositoryMutation::Register {
            snapshot: Box::new(snapshot.clone()),
            model: Box::new(model.clone()),
        },
        RepositoryMutation::Register {
            snapshot: Box::new(snapshot.clone()),
            model: Box::new(invalid.clone()),
        },
        RepositoryMutation::UpdateModel {
            model: Box::new(model.clone()),
        },
        RepositoryMutation::UpdateModel {
            model: Box::new(invalid),
        },
    ] {
        let input_model = match &request {
            RepositoryMutation::Register { model, .. }
            | RepositoryMutation::UpdateModel { model } => *model.clone(),
            _ => unreachable!(),
        };
        let request = MutationRequest::Repository(Box::new(request));
        assert!(request.transaction().is_none());
        let bytes = request.canonical_bytes(&scope(), &actor());
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &actor(),
                request.digest(&scope(), &actor()),
                &bytes,
                Default::default()
            )
            .unwrap(),
            request
        );
        for end in 0..bytes.len() {
            let prefix = &bytes[..end];
            assert!(
                restore_mutation_request(
                    &scope(),
                    &actor(),
                    Digest::of(MutationRequest::FORMAT, prefix),
                    prefix,
                    Default::default()
                )
                .is_err()
            );
        }
        for limits in [
            DomainDecodeLimits {
                collection_entries: 1,
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
                    &bytes,
                    limits
                )
                .is_err()
            );
        }
        let outcome = if input_model.validate().is_ok() {
            Ok(MutationValue::Repository {
                snapshot: snapshot.id(),
                model: Box::new(input_model),
            })
        } else {
            Err(EngineError::IncompleteAnalysis)
        };
        let record = MutationRecord {
            request: request.clone(),
            recorded_at_ms: 9,
            resolved_attempt: None,
            state_before: None,
            lease_events_before: 0,
            source_execution: None,
            administration: None,
            outcome,
        };
        let value = MutationOutcomeRecord::new(scope(), key(), record.clone()).unwrap();
        let encoded = value.canonical_bytes();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key(),
                &request,
                value.digest(),
                &encoded,
                Default::default()
            )
            .unwrap(),
            value
        );
        assert!(value.recovery_size().unwrap().bytes >= bytes.len() + encoded.len());
        for end in 0..encoded.len() {
            let prefix = &encoded[..end];
            assert!(
                restore_mutation_outcome(
                    &scope(),
                    &key(),
                    &request,
                    Digest::of(MutationOutcomeRecord::FORMAT, prefix),
                    prefix,
                    Default::default()
                )
                .is_err()
            );
        }
        let mut wrong = record;
        wrong.state_before = Some(TransactionState::Proposed);
        wrong.resolved_attempt = Some(0);
        assert!(MutationOutcomeRecord::new(scope(), key(), wrong).is_err());
    }
}
