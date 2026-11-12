use celetixo_engine::access::{AccessSet, Completeness};
use celetixo_engine::ids::*;
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::persistence::*;
use celetixo_engine::repository::{AnalysisGap, RepositoryAnalysis, RepositoryAnalysisInput};
use celetixo_engine::snapshot::{InclusionPolicy, Snapshot, SnapshotManifest};
use std::collections::{BTreeMap, BTreeSet};

const ANALYSIS_FORMAT: &str = "celetixo/repository-analysis/v1";
fn d(s: &str) -> Digest {
    Digest::of("semantic-persistence-test/v1", s.as_bytes())
}
fn e(s: &str) -> ElementId {
    ElementId::new(s).unwrap()
}
fn snapshot() -> Snapshot {
    Snapshot::new(
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("ws").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        },
        Revision(0),
        SnapshotManifest {
            base_commit: "fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "semantic-persistence-test/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap()
}
fn dependencies() -> BTreeMap<ElementId, BTreeSet<ElementId>> {
    BTreeMap::from([
        (e("a"), BTreeSet::from([e("b"), e("c")])),
        (e("b"), BTreeSet::from([e("a")])),
    ])
}
fn model() -> EngineeringModel {
    EngineeringModel {
        generation: u64::MAX,
        analyzer: "finite-reference".into(),
        version: "v1".into(),
        configuration: d("config"),
        dependencies: dependencies(),
        completeness: Completeness::Incomplete {
            reasons: vec!["gap-b".into(), "gap-a".into()],
        },
    }
}
fn analysis_input() -> RepositoryAnalysisInput {
    RepositoryAnalysisInput {
        index_generation: d("index"),
        analyzer_version: "analyzer-v1".into(),
        configuration: d("config"),
        elements: BTreeSet::from([e("a"), e("b"), e("c")]),
        dependencies: dependencies(),
        checks: BTreeMap::from([
            (
                e("a"),
                BTreeSet::from([CheckId::new("lint").unwrap(), CheckId::new("unit").unwrap()]),
            ),
            (e("c"), BTreeSet::new()),
        ]),
        gaps: BTreeSet::from([
            AnalysisGap {
                stage: "index".into(),
                path: None,
                detail: "unresolved".into(),
            },
            AnalysisGap {
                stage: "parser".into(),
                path: Some("src/main.rs".into()),
                detail: "unsupported".into(),
            },
        ]),
    }
}

// Independently split/pack the length-delimited fields. Mutations below supply
// their own correct hash, so content addressing cannot mask decoder weaknesses.
fn fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut fields = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let length = u64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize;
        fields.push(bytes[offset + 8..offset + 8 + length].to_vec());
        offset += 8 + length;
    }
    fields
}
fn packed(fields: &[Vec<u8>]) -> Vec<u8> {
    fields
        .iter()
        .flat_map(|field| [(field.len() as u64).to_be_bytes().to_vec(), field.clone()].concat())
        .collect()
}
fn unhex(hex: &str) -> Vec<u8> {
    let hex = hex.trim();
    assert!(hex.len().is_multiple_of(2));
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn decode_model(bytes: &[u8]) -> Result<EngineeringModelRecord, RestoreError> {
    restore_model_record(
        &snapshot(),
        u64::MAX,
        Digest::of(EngineeringModelRecord::FORMAT, bytes),
        bytes,
        DomainDecodeLimits::default(),
    )
}
fn decode_analysis(bytes: &[u8]) -> Result<RepositoryAnalysis, RestoreError> {
    restore_repository_analysis(
        &snapshot(),
        d("index"),
        Digest::of(ANALYSIS_FORMAT, bytes),
        bytes,
        DomainDecodeLimits::default(),
    )
}

#[test]
fn frozen_formats_preserve_previous_analysis_identity_and_new_model_context() {
    let snapshot = snapshot();
    let analysis = RepositoryAnalysis::new(&snapshot, analysis_input()).unwrap();
    let expected = unhex(include_str!("fixtures/repository-analysis-v1.hex"));
    assert_eq!(analysis.canonical_bytes(), expected);
    assert_eq!(analysis.digest(), Digest::of(ANALYSIS_FORMAT, &expected));
    assert_eq!(decode_analysis(&expected).unwrap(), analysis);
    let model = EngineeringModelRecord::new(&snapshot, model()).unwrap();
    let expected = unhex(include_str!("fixtures/engineering-model-record-v1.hex"));
    assert_eq!(model.canonical_bytes(), expected);
    assert_eq!(decode_model(&expected).unwrap(), model);
}

#[test]
fn cycles_transitive_impact_and_every_completeness_form_survive_model_reconstruction() {
    let snapshot = snapshot();
    let accesses = AccessSet {
        writes: BTreeSet::from([e("c")]),
        ..Default::default()
    };
    for completeness in [
        Completeness::Complete,
        Completeness::Incomplete { reasons: vec![] },
        model().completeness,
    ] {
        let mut original = model();
        original.completeness = completeness;
        let record = EngineeringModelRecord::new(&snapshot, original.clone()).unwrap();
        let restored = decode_model(&record.canonical_bytes()).unwrap();
        assert_eq!(restored.model(), &original);
        assert_eq!(
            restored.model().analyze(snapshot.id(), &accesses),
            original.analyze(snapshot.id(), &accesses)
        );
        let impact = restored.model().analyze(snapshot.id(), &accesses);
        assert!(
            impact
                .derived()
                .reads
                .is_superset(&BTreeSet::from([e("a"), e("b")]))
        );
    }
}

#[test]
fn complete_and_incomplete_analysis_reconstruction_preserves_all_facts() {
    for complete in [false, true] {
        let mut input = analysis_input();
        if complete {
            input.gaps.clear();
        }
        let analysis = RepositoryAnalysis::new(&snapshot(), input).unwrap();
        let restored = decode_analysis(&analysis.canonical_bytes()).unwrap();
        assert_eq!(restored, analysis);
        assert_eq!(restored.is_complete(), complete);
    }
}

#[test]
fn snapshot_scope_and_generation_are_external_requirements_not_payload_choices() {
    let snapshot = snapshot();
    let model = EngineeringModelRecord::new(&snapshot, model()).unwrap();
    let analysis = RepositoryAnalysis::new(&snapshot, analysis_input()).unwrap();
    let changed = snapshot.successor(Revision(1), BTreeMap::new()).unwrap();
    for bad in [
        changed,
        Snapshot::new(
            Scope {
                repository: RepositoryId::new("foreign").unwrap(),
                ..snapshot.scope().clone()
            },
            snapshot.revision(),
            snapshot.manifest().clone(),
            snapshot.state().clone(),
        )
        .unwrap(),
    ] {
        assert_eq!(
            restore_model_record(
                &bad,
                u64::MAX,
                model.digest(),
                &model.canonical_bytes(),
                DomainDecodeLimits::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
        assert_eq!(
            restore_repository_analysis(
                &bad,
                d("index"),
                analysis.digest(),
                &analysis.canonical_bytes(),
                DomainDecodeLimits::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
    }
    assert_eq!(
        restore_model_record(
            &snapshot,
            1,
            model.digest(),
            &model.canonical_bytes(),
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
    assert_eq!(
        restore_repository_analysis(
            &snapshot,
            d("stale-index"),
            analysis.digest(),
            &analysis.canonical_bytes(),
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
}

#[test]
fn truncation_digest_substitution_and_trailing_data_reject_for_both_domains() {
    let model = EngineeringModelRecord::new(&snapshot(), model())
        .unwrap()
        .canonical_bytes();
    let analysis = RepositoryAnalysis::new(&snapshot(), analysis_input())
        .unwrap()
        .canonical_bytes();
    for bytes in [&model, &analysis] {
        for end in 0..bytes.len() {
            let result = if bytes == &model {
                decode_model(&bytes[..end]).map(|_| ())
            } else {
                decode_analysis(&bytes[..end]).map(|_| ())
            };
            assert!(result.is_err(), "accepted truncated prefix {end}");
        }
    }
    assert_eq!(
        restore_model_record(
            &snapshot(),
            u64::MAX,
            d("forged"),
            &model,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::DigestMismatch)
    );
    assert_eq!(
        restore_repository_analysis(
            &snapshot(),
            d("index"),
            d("forged"),
            &analysis,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::DigestMismatch)
    );
    for bytes in [model, analysis] {
        let mut appended = bytes;
        appended.push(0);
        assert!(decode_model(&appended).is_err());
        assert!(decode_analysis(&appended).is_err());
    }
}

#[test]
fn model_rejects_invalid_provenance_tags_identities_and_noncanonical_dependency_sets() {
    let original = fields(
        &EngineeringModelRecord::new(&snapshot(), model())
            .unwrap()
            .canonical_bytes(),
    );
    for (field, value) in [
        (4, 0_u64.to_be_bytes().to_vec()),
        (5, b" ".to_vec()),
        (6, vec![]),
        (9, b"bad identity".to_vec()),
        (16, 2_u64.to_be_bytes().to_vec()),
    ] {
        let mut modified = original.clone();
        modified[field] = value;
        assert!(
            decode_model(&packed(&modified)).is_err(),
            "accepted invalid model field {field}"
        );
    }
    for (a, b) in [(11, 12), (9, 13)] {
        let mut modified = original.clone();
        modified[a] = modified[b].clone();
        assert_eq!(
            decode_model(&packed(&modified)),
            Err(RestoreError::NonCanonical)
        );
    }
    let mut modified = original;
    modified.swap(11, 12);
    assert_eq!(
        decode_model(&packed(&modified)),
        Err(RestoreError::NonCanonical)
    );
}

#[test]
fn analysis_rejects_unknown_elements_invalid_gaps_and_duplicate_or_reordered_sets() {
    let original = fields(
        &RepositoryAnalysis::new(&snapshot(), analysis_input())
            .unwrap()
            .canonical_bytes(),
    );
    for (field, value) in [
        (5, b" ".to_vec()),
        (12, b"unknown".to_vec()),
        (14, b"unknown".to_vec()),
        (20, b"unknown".to_vec()),
        (22, b"bad check".to_vec()),
        (27, vec![]),
        (31, 2_u64.to_be_bytes().to_vec()),
        (33, vec![]),
    ] {
        let mut modified = original.clone();
        modified[field] = value;
        assert!(
            decode_analysis(&packed(&modified)).is_err(),
            "accepted invalid analysis field {field}"
        );
    }
    for (a, b) in [(8, 9), (14, 15), (12, 16), (20, 24), (22, 23)] {
        let mut modified = original.clone();
        modified[a] = modified[b].clone();
        assert_eq!(
            decode_analysis(&packed(&modified)),
            Err(RestoreError::NonCanonical)
        );
    }
    for (a, b) in [(8, 9), (14, 15), (22, 23)] {
        let mut modified = original.clone();
        modified.swap(a, b);
        assert_eq!(
            decode_analysis(&packed(&modified)),
            Err(RestoreError::NonCanonical)
        );
    }
    let mut duplicate_gap = original.clone();
    duplicate_gap[26] = 3_u64.to_be_bytes().to_vec();
    duplicate_gap.extend_from_slice(&original[27..30]);
    assert_eq!(
        decode_analysis(&packed(&duplicate_gap)),
        Err(RestoreError::NonCanonical)
    );
}

#[test]
fn decoding_budgets_cover_nested_collections_fields_and_length_amplification() {
    let model = EngineeringModelRecord::new(&snapshot(), model()).unwrap();
    let analysis = RepositoryAnalysis::new(&snapshot(), analysis_input()).unwrap();
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 1,
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
        assert_eq!(
            restore_model_record(
                &snapshot(),
                u64::MAX,
                model.digest(),
                &model.canonical_bytes(),
                limits
            ),
            Err(RestoreError::Limit)
        );
        assert_eq!(
            restore_repository_analysis(
                &snapshot(),
                d("index"),
                analysis.digest(),
                &analysis.canonical_bytes(),
                limits
            ),
            Err(RestoreError::Limit)
        );
    }
    for mut bytes in [model.canonical_bytes(), analysis.canonical_bytes()] {
        bytes[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(matches!(decode_model(&bytes), Err(RestoreError::Limit)));
        assert!(matches!(decode_analysis(&bytes), Err(RestoreError::Limit)));
    }
    // Counts accumulate across nested sets/maps, rather than resetting per field.
    for (entries, accepted) in [(6, false), (7, true)] {
        let limits = DomainDecodeLimits {
            collection_entries: entries,
            ..Default::default()
        };
        assert_eq!(
            restore_model_record(
                &snapshot(),
                u64::MAX,
                model.digest(),
                &model.canonical_bytes(),
                limits
            )
            .is_ok(),
            accepted
        );
    }
    for (entries, accepted) in [(13, false), (14, true)] {
        let limits = DomainDecodeLimits {
            collection_entries: entries,
            ..Default::default()
        };
        assert_eq!(
            restore_repository_analysis(
                &snapshot(),
                d("index"),
                analysis.digest(),
                &analysis.canonical_bytes(),
                limits
            )
            .is_ok(),
            accepted
        );
    }
}
