use super::*;
use crate::ids::CheckId;
use crate::persistence::{DomainDecodeLimits, RestoreError, restore_source_publication_analysis};
use crate::repository::{AnalysisGap, RepositoryAnalysisInput};
use std::collections::BTreeSet;

fn facts(snapshot: &Snapshot, gaps: BTreeSet<AnalysisGap>) -> RepositoryAnalysis {
    let main = ElementId::new("symbol:main").unwrap();
    let helper = ElementId::new("symbol:helper").unwrap();
    RepositoryAnalysis::new(
        snapshot,
        RepositoryAnalysisInput {
            index_generation: Digest::of("publication-analyzer-test/v1", b"new index"),
            analyzer_version: "complete-finite-publication-fixture/v1".into(),
            configuration: Digest::of("publication-analyzer-test/v1", b"configuration"),
            elements: [main.clone(), helper.clone()].into(),
            dependencies: [(main.clone(), [helper].into())].into(),
            checks: [(main, [CheckId::new("check:main").unwrap()].into())].into(),
            gaps,
        },
    )
    .unwrap()
}

fn fixture() -> (Snapshot, SourcePatch, SourcePublicationAnalysis) {
    let (base, patch) = super::fixture(7, b"// newly analyzed source\r\n");
    let candidate = SourcePublicationCandidate::new(&base, &patch).unwrap();
    let record =
        SourcePublicationAnalysis::new(&base, &patch, facts(candidate.snapshot(), BTreeSet::new()))
            .unwrap();
    (base, patch, record)
}

// An independent field encoder, without calling Canonical or the record writer.
fn fields(record: &SourcePublicationAnalysis) -> Vec<Vec<u8>> {
    let candidate = record.candidate();
    let scope = candidate.snapshot().scope();
    vec![
        scope.organization.as_str().as_bytes().to_vec(),
        scope.workspace.as_str().as_bytes().to_vec(),
        scope.repository.as_str().as_bytes().to_vec(),
        record.base().0.as_bytes().to_vec(),
        candidate
            .application()
            .snapshot()
            .revision()
            .0
            .to_be_bytes()
            .to_vec(),
        record.patch().0.as_bytes().to_vec(),
        candidate
            .application()
            .snapshot()
            .id()
            .0
            .as_bytes()
            .to_vec(),
        candidate
            .application()
            .report()
            .digest()
            .as_bytes()
            .to_vec(),
        candidate.snapshot().id().0.as_bytes().to_vec(),
        candidate.snapshot().revision().0.to_be_bytes().to_vec(),
        record.analysis().digest().as_bytes().to_vec(),
        record.analysis().canonical_bytes(),
    ]
}

fn encode(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    bytes
}

fn restore(
    base: &Snapshot,
    patch: &SourcePatch,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourcePublicationAnalysis, RestoreError> {
    restore_source_publication_analysis(
        base,
        patch,
        Digest::of(SourcePublicationAnalysis::FORMAT, bytes),
        bytes,
        limits,
    )
}

#[test]
fn exact_publication_analysis_round_trips_with_an_independent_layout() {
    let (base, patch, record) = fixture();
    record.validate(&base, &patch).unwrap();
    let bytes = encode(&fields(&record));
    assert_eq!(record.canonical_bytes(), bytes);
    assert_eq!(
        restore(&base, &patch, &bytes, Default::default()).unwrap(),
        record
    );
    assert_eq!(
        record.analysis().snapshot(),
        record.candidate().snapshot().id()
    );
    assert_ne!(
        record.analysis().snapshot(),
        record.candidate().application().snapshot().id()
    );
    assert_eq!(record.candidate().snapshot().revision(), Revision(8));
    // No Engine, installation decision or runtime capability is reconstructed.
    assert_eq!(base.revision(), Revision(7));
    assert_eq!(patch.binding().base_snapshot, base.id());
}

#[test]
fn old_application_and_incomplete_analysis_cannot_describe_publication() {
    let (base, patch, record) = fixture();
    for wrong_snapshot in [&base, record.candidate().application().snapshot()] {
        assert_eq!(
            SourcePublicationAnalysis::new(&base, &patch, facts(wrong_snapshot, BTreeSet::new())),
            Err(SourcePublicationError::Analysis(
                RepositoryAnalysisError::SnapshotMismatch
            ))
        );
    }
    let incomplete = facts(
        record.candidate().snapshot(),
        [AnalysisGap {
            stage: "type resolution".into(),
            path: Some("src/main".into()),
            detail: "unresolved input".into(),
        }]
        .into(),
    );
    assert_eq!(
        SourcePublicationAnalysis::new(&base, &patch, incomplete),
        Err(SourcePublicationError::IncompleteAnalysis)
    );
    let (_, other_patch) = super::fixture(7, b"different source bytes");
    assert!(record.validate(&base, &other_patch).is_err());
    let mut forged = record.clone();
    forged.patch = other_patch.id();
    assert_eq!(
        forged.validate(&base, &patch),
        Err(SourcePublicationError::BindingMismatch)
    );
}

#[test]
fn every_publication_selector_is_checked_after_rehashing() {
    let (base, patch, record) = fixture();
    let original = fields(&record);
    for index in 0..11 {
        let mut changed = original.clone();
        changed[index] = match index {
            0..=2 => b"different-valid-scope".to_vec(),
            4 | 9 => 19_u64.to_be_bytes().to_vec(),
            _ => vec![42; 32],
        };
        assert!(
            restore(&base, &patch, &encode(&changed), Default::default()).is_err(),
            "field {index}"
        );
    }
    let (_, other_patch, other) = {
        let (base, patch) = super::fixture(7, b"another exact source result");
        let candidate = SourcePublicationCandidate::new(&base, &patch).unwrap();
        let other = SourcePublicationAnalysis::new(
            &base,
            &patch,
            facts(candidate.snapshot(), BTreeSet::new()),
        )
        .unwrap();
        (base, patch, other)
    };
    let mut changed = original;
    changed[10] = other.analysis().digest().as_bytes().to_vec();
    changed[11] = other.analysis().canonical_bytes();
    assert!(restore(&base, &patch, &encode(&changed), Default::default()).is_err());
    assert!(
        restore(
            &base,
            &other_patch,
            &record.canonical_bytes(),
            Default::default()
        )
        .is_err()
    );
}

#[test]
fn malformed_publication_records_and_combined_dependency_budgets_fail_closed() {
    let (base, patch, record) = fixture();
    let bytes = record.canonical_bytes();
    for end in 0..bytes.len() {
        assert!(
            restore(&base, &patch, &bytes[..end], Default::default()).is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        restore(&base, &patch, &trailing, Default::default()),
        Err(RestoreError::NonCanonical)
    );
    let mut oversized = bytes.clone();
    oversized[..8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(restore(&base, &patch, &oversized, Default::default()).is_err());
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..Default::default()
        },
        // The envelope alone fits: replay dependencies still consume the same budget.
        DomainDecodeLimits {
            encoded_bytes: bytes.len(),
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 32,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 32,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            restore(&base, &patch, &bytes, limits),
            Err(RestoreError::Limit)
        );
    }
    assert_eq!(
        restore_source_publication_analysis(
            &base,
            &patch,
            Digest::of("wrong-domain", &bytes),
            &bytes,
            Default::default()
        ),
        Err(RestoreError::DigestMismatch)
    );
    // The independently valid nested analysis cannot reset the aggregate limit.
    crate::persistence::restore_repository_analysis(
        record.candidate().snapshot(),
        record.analysis().index_generation(),
        record.analysis().digest(),
        &record.analysis().canonical_bytes(),
        DomainDecodeLimits {
            collection_entries: 32,
            ..Default::default()
        },
    )
    .unwrap();
}
