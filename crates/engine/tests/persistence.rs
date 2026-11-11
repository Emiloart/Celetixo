use celetixo_engine::ids::{
    Digest, ElementId, OrganizationId, RepositoryId, Revision, Scope, SnapshotId, WorkspaceId,
};
use celetixo_engine::persistence::{RestoreError, SnapshotDecodeLimits, restore_snapshot};
use celetixo_engine::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotManifest, WorkspaceInput,
};
use std::collections::BTreeMap;

fn snapshot() -> Snapshot {
    Snapshot::new(
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("ws").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        },
        Revision(u64::MAX),
        SnapshotManifest {
            base_commit: "public-base".into(),
            canonicalization_version: 1,
            provenance: "durable-codec-fixture".into(),
            inclusion_policy: InclusionPolicy::complete(),
            inputs: [
                ("source.rs", InputLayer::Base, Some(&b"old\0\xff"[..])),
                ("source.rs", InputLayer::Index, Some(&b"staged"[..])),
                ("source.rs", InputLayer::WorkingTree, None),
                ("source.rs", InputLayer::Overlay, Some(&b"overlay"[..])),
                ("untracked", InputLayer::Untracked, Some(&b""[..])),
                ("generated", InputLayer::Generated, Some(&b"generated"[..])),
                ("mounted", InputLayer::Mounted, Some(&b"mounted"[..])),
            ]
            .into_iter()
            .map(|(path, layer, bytes)| WorkspaceInput {
                path: path.into(),
                layer,
                kind: InputKind::File,
                mode: FileMode::Regular,
                bytes: bytes.map(ToOwned::to_owned),
            })
            .collect(),
            environment: BTreeMap::from([("fixture_profile".into(), "public".into())]),
        },
        BTreeMap::from([
            (ElementId::new("min").unwrap(), i64::MIN),
            (ElementId::new("max").unwrap(), i64::MAX),
        ]),
    )
    .unwrap()
}

fn id(bytes: &[u8]) -> SnapshotId {
    SnapshotId(Digest::of("celetixo/snapshot/v1", bytes))
}

#[test]
fn layered_binary_snapshot_restores_exact_identity_and_full_integer_range() {
    let original = snapshot();
    assert_eq!(
        restore_snapshot(
            original.id(),
            &original.canonical_bytes(),
            SnapshotDecodeLimits::default()
        )
        .unwrap(),
        original
    );
}

#[test]
fn corrupted_truncated_appended_and_invalid_identity_bytes_reject_even_when_rehashed() {
    let original = snapshot();
    let bytes = original.canonical_bytes();
    for end in 0..bytes.len() {
        assert!(
            restore_snapshot(
                id(&bytes[..end]),
                &bytes[..end],
                SnapshotDecodeLimits::default()
            )
            .is_err(),
            "accepted truncated prefix {end}"
        );
    }
    let mut appended = bytes.clone();
    appended.push(0);
    assert_eq!(
        restore_snapshot(id(&appended), &appended, SnapshotDecodeLimits::default()),
        Err(RestoreError::NonCanonical)
    );
    let mut corrupted = bytes.clone();
    corrupted[8] = b' ';
    assert_eq!(
        restore_snapshot(id(&corrupted), &corrupted, SnapshotDecodeLimits::default()),
        Err(RestoreError::Identity)
    );
    assert_eq!(
        restore_snapshot(original.id(), &corrupted, SnapshotDecodeLimits::default()),
        Err(RestoreError::DigestMismatch)
    );
}

#[test]
fn resource_limits_prevent_oversized_and_collection_amplification_decoding() {
    let original = snapshot();
    let bytes = original.canonical_bytes();
    for limits in [
        SnapshotDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..Default::default()
        },
        SnapshotDecodeLimits {
            collection_entries: 1,
            ..Default::default()
        },
        SnapshotDecodeLimits {
            field_bytes: 2,
            ..Default::default()
        },
    ] {
        assert_eq!(
            restore_snapshot(original.id(), &bytes, limits),
            Err(RestoreError::Limit)
        );
    }
    let mut length = bytes;
    length[..8].copy_from_slice(&u64::MAX.to_be_bytes());
    assert_eq!(
        restore_snapshot(id(&length), &length, SnapshotDecodeLimits::default()),
        Err(RestoreError::Limit)
    );
}

#[test]
fn decoded_values_still_require_constructor_invariants_and_canonical_order() {
    let original = snapshot();
    let bytes = original.canonical_bytes();
    // Schema version is the fifth length-delimited field, after scope/revision.
    let mut modified = bytes.clone();
    let mut cursor = 0;
    for _ in 0..4 {
        let len = u64::from_be_bytes(modified[cursor..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8 + len;
    }
    modified[cursor + 8..cursor + 16].copy_from_slice(&2_u64.to_be_bytes());
    assert!(matches!(
        restore_snapshot(id(&modified), &modified, SnapshotDecodeLimits::default()),
        Err(RestoreError::Snapshot(_))
    ));
    // The original decoder must also preserve canonical set ordering.
    let mut modified = bytes;
    let first = modified
        .windows(16)
        .position(|x| x == [8_u64.to_be_bytes(), 0_u64.to_be_bytes()].concat())
        .unwrap();
    modified[first + 8..first + 16].copy_from_slice(&7_u64.to_be_bytes());
    assert!(restore_snapshot(id(&modified), &modified, SnapshotDecodeLimits::default()).is_err());
}
