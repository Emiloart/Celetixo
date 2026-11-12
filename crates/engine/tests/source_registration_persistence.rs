use celetixo_engine::engine::SourceArtifactAdapter;
use celetixo_engine::ids::*;
use celetixo_engine::persistence::*;

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("o").unwrap(),
        workspace: WorkspaceId::new("w").unwrap(),
        repository: RepositoryId::new("r").unwrap(),
    }
}
fn record() -> SourceArtifactRegistrationRecord {
    SourceArtifactRegistrationRecord::new(
        scope(),
        7,
        SourceArtifactAdapter {
            identity: SourceArtifactAdapterId::new("filesystem-v1").unwrap(),
            configuration: Digest::from_bytes([0x11; 32]),
        },
        Digest::from_bytes([0x22; 32]),
    )
    .unwrap()
}
fn field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value);
}

#[test]
fn configured_registration_layout_and_provenance_have_independent_frozen_addresses() {
    let value = record();
    let mut provenance = Vec::new();
    field(&mut provenance, &7_u64.to_be_bytes());
    for id in [b"o", b"w", b"r"] {
        field(&mut provenance, id);
    }
    field(&mut provenance, &[0x22; 32]);
    field(&mut provenance, b"filesystem-v1");
    field(&mut provenance, &[0x11; 32]);
    let provenance = Digest::of("celetixo/source-artifact-store/v2", &provenance);
    assert_eq!(
        provenance.to_string(),
        "00f828168771c6bd4de69f8a58484ea5db24e365954d5d3f5fe6afd21bdaa1e9"
    );
    assert_eq!(value.provenance(), provenance);
    let mut bytes = Vec::new();
    for id in [b"o", b"w", b"r"] {
        field(&mut bytes, id);
    }
    field(&mut bytes, &7_u64.to_be_bytes());
    field(&mut bytes, b"filesystem-v1");
    field(&mut bytes, &[0x11; 32]);
    field(&mut bytes, &[0x22; 32]);
    field(&mut bytes, provenance.as_bytes());
    assert_eq!(value.canonical_bytes(), bytes);
    assert_eq!(
        value.digest().to_string(),
        "efc8f2def6256fd3f344d57a1209dc53adb88103962ed61d37e41d2fb3cad33f"
    );
    assert_eq!(
        restore_source_artifact_registration(
            &scope(),
            7,
            value.digest(),
            &bytes,
            Default::default()
        )
        .unwrap(),
        value
    );
}

#[test]
fn context_substitution_and_rehashed_malformed_registration_reject() {
    let value = record();
    let bytes = value.canonical_bytes();
    let mut other = scope();
    other.repository = RepositoryId::new("other").unwrap();
    assert!(
        restore_source_artifact_registration(&other, 7, value.digest(), &bytes, Default::default())
            .is_err()
    );
    assert!(
        restore_source_artifact_registration(
            &scope(),
            8,
            value.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    assert!(
        SourceArtifactRegistrationRecord::new(
            scope(),
            0,
            value.adapter().clone(),
            value.retention()
        )
        .is_err()
    );
    for length in 0..bytes.len() {
        let truncated = &bytes[..length];
        let digest = Digest::of(SourceArtifactRegistrationRecord::FORMAT, truncated);
        assert!(
            restore_source_artifact_registration(
                &scope(),
                7,
                digest,
                truncated,
                Default::default()
            )
            .is_err(),
            "accepted prefix {length}"
        );
    }
    let mut substituted = bytes.clone();
    *substituted.last_mut().unwrap() ^= 1;
    let digest = Digest::of(SourceArtifactRegistrationRecord::FORMAT, &substituted);
    assert!(matches!(
        restore_source_artifact_registration(&scope(), 7, digest, &substituted, Default::default()),
        Err(RestoreError::BindingMismatch)
    ));
    let mut trailing = bytes;
    trailing.push(0);
    let digest = Digest::of(SourceArtifactRegistrationRecord::FORMAT, &trailing);
    assert!(
        restore_source_artifact_registration(&scope(), 7, digest, &trailing, Default::default())
            .is_err()
    );
}

#[test]
fn registration_decode_limits_precede_materialization() {
    let value = record();
    let bytes = value.canonical_bytes();
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
            collection_entries: 0,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            restore_source_artifact_registration(&scope(), 7, value.digest(), &bytes, limits),
            Err(RestoreError::Limit)
        ));
    }
}
