//! Pure metadata formats do not admit evidence or retrieve any physical object.
use celetixo_engine::engine::SourceEvidenceReceipt;
use celetixo_engine::ids::*;
use celetixo_engine::persistence::*;
use celetixo_engine::source_evidence::{SourceArtifactReference, SourceEvidenceAssessment};

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("o").unwrap(),
        workspace: WorkspaceId::new("w").unwrap(),
        repository: RepositoryId::new("r").unwrap(),
    }
}
fn reference() -> SourceArtifactReference {
    SourceArtifactReference {
        scope: scope(),
        name: "sealed-evidence".into(),
        media_type: "application/vnd.celetixo.source-evidence-v2".into(),
        content: Digest::from_bytes([4; 32]),
        bytes: 37,
        retention_policy: Digest::from_bytes([3; 32]),
    }
}
fn receipt(assessment: SourceEvidenceAssessment) -> SourceEvidenceReceipt {
    SourceEvidenceReceipt {
        admission: SourceAdmissionId(Digest::from_bytes([1; 32])),
        evidence: SourceEvidenceId(Digest::from_bytes([2; 32])),
        assessment,
        recorded_at_ms: 12,
        envelope: reference(),
        storage_provenance: Digest::from_bytes([5; 32]),
    }
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn independent_reference_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    for id in [b"o", b"w", b"r"] {
        field(&mut bytes, id);
    }
    field(&mut bytes, &7_u64.to_be_bytes());
    field(&mut bytes, b"sealed-evidence");
    field(&mut bytes, b"application/vnd.celetixo.source-evidence-v2");
    field(&mut bytes, &[4; 32]);
    field(&mut bytes, &37_u64.to_be_bytes());
    field(&mut bytes, &[3; 32]);
    bytes
}
fn independent_receipt_bytes(tag: &str, items: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for id in [b"o", b"w", b"r"] {
        field(&mut bytes, id);
    }
    field(&mut bytes, &7_u64.to_be_bytes());
    field(&mut bytes, &[1; 32]);
    field(&mut bytes, &[2; 32]);
    field(&mut bytes, tag.as_bytes());
    if tag == "Failed" || tag == "Incomplete" {
        field(&mut bytes, &(items.len() as u64).to_be_bytes());
        for item in items {
            field(&mut bytes, item.as_bytes());
        }
    }
    field(&mut bytes, &12_u64.to_be_bytes());
    field(&mut bytes, &independent_reference_bytes());
    field(&mut bytes, &[5; 32]);
    bytes
}
#[test]
fn logical_reference_addresses_separate_store_name_and_media_from_physical_bytes() {
    let value = SourceArtifactReferenceRecord::new(scope(), 7, reference()).unwrap();
    assert_eq!(value.canonical_bytes(), independent_reference_bytes());
    assert_eq!(
        value.digest().to_string(),
        "845d9de8de230de1c0c2822ff3e9f1d34bde0577aabf480d79c6de969a0e09e3"
    );
    let other_store = SourceArtifactReferenceRecord::new(scope(), 8, reference()).unwrap();
    let mut named = reference();
    named.name = "another-name".into();
    let other_name = SourceArtifactReferenceRecord::new(scope(), 7, named).unwrap();
    let mut media = reference();
    media.media_type = "application/octet-stream".into();
    let other_media = SourceArtifactReferenceRecord::new(scope(), 7, media).unwrap();
    for other in [&other_store, &other_name, &other_media] {
        assert_eq!(value.reference().content, other.reference().content);
        assert_ne!(value.digest(), other.digest());
    }
    assert_eq!(
        restore_source_artifact_reference(
            &scope(),
            7,
            value.digest(),
            &value.canonical_bytes(),
            Default::default()
        )
        .unwrap(),
        value
    );
    assert!(
        restore_source_artifact_reference(
            &scope(),
            8,
            value.digest(),
            &value.canonical_bytes(),
            Default::default()
        )
        .is_err()
    );
}
#[test]
fn original_receipt_assessments_have_frozen_addresses_and_preserve_binary_reasons() {
    for (assessment, tag, items, expected) in [
        (
            SourceEvidenceAssessment::Passed,
            "Passed",
            vec![],
            "13e93f3c4d1094e68ea1c24e40891a5fd04e1f1f3f3ca3c4ebfbf909cca2dcb0",
        ),
        (
            SourceEvidenceAssessment::Failed {
                checks: [CheckId::new("a").unwrap(), CheckId::new("b").unwrap()].into(),
            },
            "Failed",
            vec!["a", "b"],
            "983548cac77c8796a2290fb89082541f77c3ffb099b3e6a89eb8f9fa708abe45",
        ),
        (
            SourceEvidenceAssessment::Incomplete {
                reasons: ["reason\0one".into(), "z".into()].into(),
            },
            "Incomplete",
            vec!["reason\0one", "z"],
            "ff4c13a49ba6bdfa8205094238804996ac90e6847c4759c6d655c10560a3291e",
        ),
    ] {
        let original = receipt(assessment);
        let value = SourceEvidenceReceiptRecord::new(scope(), 7, original.clone()).unwrap();
        assert_eq!(
            value.canonical_bytes(),
            independent_receipt_bytes(tag, &items)
        );
        assert_eq!(value.digest().to_string(), expected);
        let restored = restore_source_evidence_receipt(
            &scope(),
            7,
            value.digest(),
            &value.canonical_bytes(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(restored.receipt(), &original);
        assert_eq!(restored, value);
    }
}
#[test]
fn rehashed_receipts_reject_unknown_tags_duplicate_or_unsorted_sets_and_wrong_context() {
    for bytes in [
        independent_receipt_bytes("Unknown", &[]),
        independent_receipt_bytes("Failed", &["a", "a"]),
        independent_receipt_bytes("Failed", &["b", "a"]),
        independent_receipt_bytes("Incomplete", &["z", "a"]),
        independent_receipt_bytes("Failed", &[]),
    ] {
        let digest = Digest::of(SourceEvidenceReceiptRecord::FORMAT, &bytes);
        assert!(
            restore_source_evidence_receipt(&scope(), 7, digest, &bytes, Default::default())
                .is_err()
        );
    }
    let bytes = independent_receipt_bytes("Passed", &[]);
    let digest = Digest::of(SourceEvidenceReceiptRecord::FORMAT, &bytes);
    assert!(
        restore_source_evidence_receipt(&scope(), 8, digest, &bytes, Default::default()).is_err()
    );
    let mut other = scope();
    other.workspace = WorkspaceId::new("other").unwrap();
    assert!(
        restore_source_evidence_receipt(&other, 7, digest, &bytes, Default::default()).is_err()
    );
    for length in 0..bytes.len() {
        let shortened = &bytes[..length];
        let digest = Digest::of(SourceEvidenceReceiptRecord::FORMAT, shortened);
        assert!(
            restore_source_evidence_receipt(&scope(), 7, digest, shortened, Default::default())
                .is_err()
        );
    }
    let mut trailing = bytes;
    trailing.push(0);
    let digest = Digest::of(SourceEvidenceReceiptRecord::FORMAT, &trailing);
    assert!(
        restore_source_evidence_receipt(&scope(), 7, digest, &trailing, Default::default())
            .is_err()
    );
}
#[test]
fn malformed_reference_and_receipt_budgets_cannot_enter_the_metadata_graph() {
    let mut bad = reference();
    bad.name = "bad\0name".into();
    assert!(SourceArtifactReferenceRecord::new(scope(), 7, bad).is_err());
    assert!(SourceArtifactReferenceRecord::new(scope(), 0, reference()).is_err());
    assert!(
        SourceEvidenceReceiptRecord::new(
            scope(),
            7,
            receipt(SourceEvidenceAssessment::Incomplete {
                reasons: Default::default()
            })
        )
        .is_err()
    );
    let bytes = independent_receipt_bytes("Failed", &["a", "b"]);
    let digest = Digest::of(SourceEvidenceReceiptRecord::FORMAT, &bytes);
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 1,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            restore_source_evidence_receipt(&scope(), 7, digest, &bytes, limits),
            Err(RestoreError::Limit)
        ));
    }
}
