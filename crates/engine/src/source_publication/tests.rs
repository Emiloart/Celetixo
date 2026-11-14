use super::*;
use crate::ids::{
    Digest, ElementId, OrganizationId, PrincipalId, RepositoryId, Scope, SourceAttemptId,
    TransactionId, WorkspaceId,
};
use crate::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, SnapshotManifest, WorkspaceInput,
    effective_workspace,
};
use crate::source_patch::{PatchBinding, PatchEntry, PatchOperation, PatchPolicy, Preimage};
use std::collections::BTreeMap;

mod analysis;

fn fixture(revision: u64, replacement: &[u8]) -> (Snapshot, SourcePatch) {
    let scope = Scope {
        organization: OrganizationId::new("publication-org").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    };
    let base = Snapshot::new(
        scope.clone(),
        Revision(revision),
        SnapshotManifest {
            base_commit: "immutable-base".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "source-publication-identity-test/v1".into(),
            inputs: vec![WorkspaceInput {
                path: "src/main".into(),
                layer: InputLayer::Base,
                kind: InputKind::File,
                mode: FileMode::Executable,
                bytes: Some(b"original\r\n".to_vec()),
            }],
            environment: BTreeMap::from([("LANG".into(), "C".into())]),
        },
        BTreeMap::from([(ElementId::new("symbol:main").unwrap(), 17)]),
    )
    .unwrap();
    let digest = Digest::of("publication-test/v1", b"fixture");
    let patch = SourcePatch::new(
        PatchBinding {
            scope,
            principal: PrincipalId::new("owner").unwrap(),
            transaction: TransactionId::new("transaction").unwrap(),
            transaction_attempt: 2,
            source_attempt: SourceAttemptId::new("source-attempt").unwrap(),
            base_snapshot: base.id(),
            base_revision: base.revision(),
            analysis_artifact: digest,
            index_generation: digest,
            impact_request: digest,
            impact_result: digest,
            created_at_ms: 1,
            expires_at_ms: 100,
            producer: "publication identity fixture".into(),
        },
        PatchPolicy::default(),
        vec![PatchOperation::Replace {
            path: "src/main".into(),
            expected: Preimage::of(&base.manifest().inputs[0]).unwrap(),
            entry: PatchEntry::file(replacement.to_vec(), FileMode::Executable).unwrap(),
        }],
    )
    .unwrap();
    (base, patch)
}

#[test]
fn publication_preserves_verified_manifest_bytes_modes_state_and_original_identities() {
    let (base, patch) = fixture(41, b"// source trivia\r\n\0\xff\n");
    let original_base = base.clone();
    let original_patch = patch.canonical_bytes();
    let application = patch.apply(&base).unwrap();
    let original_report = application.report().canonical_bytes();
    let candidate = SourcePublicationCandidate::new(&base, &patch).unwrap();
    candidate.validate(&base, &patch).unwrap();
    assert_eq!(candidate.application(), &application);
    assert_eq!(candidate.application().snapshot().revision(), Revision(41));
    assert_eq!(candidate.snapshot().revision(), Revision(42));
    assert_ne!(candidate.snapshot().id(), application.snapshot().id());
    assert_ne!(candidate.snapshot().id(), base.id());
    assert_eq!(candidate.snapshot().scope(), base.scope());
    assert_eq!(
        candidate.snapshot().manifest(),
        application.snapshot().manifest()
    );
    assert_eq!(candidate.snapshot().state(), application.snapshot().state());
    assert_eq!(candidate.snapshot().state(), base.state());
    let files = effective_workspace(candidate.snapshot()).unwrap();
    assert_eq!(
        files["src/main"].bytes.as_deref(),
        Some(b"// source trivia\r\n\0\xff\n".as_slice())
    );
    assert_eq!(files["src/main"].mode, FileMode::Executable);
    assert_eq!(
        candidate.application().report().canonical_bytes(),
        original_report
    );
    assert_eq!(patch.canonical_bytes(), original_patch);
    assert_eq!(base, original_base);
}

#[test]
fn revision_exhaustion_rejects_publication_without_changing_the_application_contract() {
    let (base, patch) = fixture(u64::MAX, b"new");
    let application = patch.apply(&base).unwrap();
    assert_eq!(application.snapshot().revision(), Revision(u64::MAX));
    assert_eq!(
        SourcePublicationCandidate::new(&base, &patch),
        Err(SourcePublicationError::RevisionExhausted)
    );
    assert_eq!(patch.apply(&base).unwrap(), application);
    let (base, patch) = fixture(u64::MAX - 1, b"new");
    assert_eq!(
        SourcePublicationCandidate::new(&base, &patch)
            .unwrap()
            .snapshot()
            .revision(),
        Revision(u64::MAX)
    );
}

#[test]
fn wrong_immutable_base_scope_revision_and_content_cannot_select_a_publication_candidate() {
    let (base, patch) = fixture(4, b"new");
    let candidate = SourcePublicationCandidate::new(&base, &patch).unwrap();
    for variant in 0..3 {
        let mut scope = base.scope().clone();
        let mut revision = base.revision();
        let mut manifest = base.manifest().clone();
        match variant {
            0 => scope.organization = OrganizationId::new("foreign-org").unwrap(),
            1 => revision = Revision(5),
            2 => manifest.inputs[0].bytes = Some(b"different base".to_vec()),
            _ => unreachable!(),
        }
        let wrong = Snapshot::new(scope, revision, manifest, base.state().clone()).unwrap();
        assert_eq!(
            SourcePublicationCandidate::new(&wrong, &patch),
            Err(SourcePatchError::WrongBase.into())
        );
        assert_eq!(
            candidate.validate(&wrong, &patch),
            Err(SourcePatchError::WrongBase.into())
        );
    }
}

#[test]
fn valid_but_substituted_application_or_successor_metadata_fails_independent_replay() {
    let (base, patch) = fixture(9, b"verified");
    let candidate = SourcePublicationCandidate::new(&base, &patch).unwrap();
    let (_, different_patch) = fixture(9, b"unverified");
    let alternate = SourcePublicationCandidate::new(&base, &different_patch).unwrap();
    for variant in 0..5 {
        let mut forged = candidate.clone();
        match variant {
            0 => forged.snapshot = base.clone(),
            1 => forged.snapshot = alternate.snapshot.clone(),
            2 => forged.application = alternate.application.clone(),
            3 => {
                let mut state = forged.snapshot.state().clone();
                state.insert(ElementId::new("symbol:main").unwrap(), 18);
                forged.snapshot = Snapshot::new(
                    base.scope().clone(),
                    Revision(10),
                    forged.snapshot.manifest().clone(),
                    state,
                )
                .unwrap();
            }
            4 => forged.snapshot = forged.application.snapshot().clone(),
            _ => unreachable!(),
        }
        assert_eq!(
            forged.validate(&base, &patch),
            Err(SourcePublicationError::BindingMismatch),
            "variant {variant}"
        );
    }
    assert_eq!(
        candidate.validate(&base, &different_patch),
        Err(SourcePublicationError::BindingMismatch)
    );
}
