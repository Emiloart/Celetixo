use celetixo_engine::ids::{ElementId, OrganizationId, RepositoryId, Revision, Scope, WorkspaceId};
use celetixo_engine::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotError, SnapshotManifest,
    State, WorkspaceInput,
};
use std::collections::BTreeMap;

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("organization").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    }
}

fn input(layer: InputLayer, path: &str, bytes: &[u8]) -> WorkspaceInput {
    WorkspaceInput {
        path: path.into(),
        layer,
        kind: InputKind::File,
        mode: FileMode::Regular,
        bytes: Some(bytes.into()),
    }
}

fn manifest() -> SnapshotManifest {
    SnapshotManifest {
        base_commit: "git:base-commit".into(),
        canonicalization_version: 1,
        inclusion_policy: InclusionPolicy::complete(),
        provenance: "reference-fixture/capture-v1".into(),
        inputs: vec![input(InputLayer::Base, "src/main.rs", b"base")],
        environment: BTreeMap::from([("RUST_VERSION".into(), "1.94.0".into())]),
    }
}

fn snapshot(manifest: SnapshotManifest) -> Result<Snapshot, SnapshotError> {
    Snapshot::new(scope(), Revision(0), manifest, State::new())
}

#[test]
fn git_base_does_not_hide_dirty_layers_and_absence_is_not_empty_content() {
    let base = manifest();
    let base_id = snapshot(base.clone()).unwrap().id();
    let mut ids = std::collections::BTreeSet::from([base_id]);
    for layer in InputLayer::ALL {
        let mut dirty = base.clone();
        dirty.inputs.push(input(layer, "added.txt", b"exact input"));
        assert!(ids.insert(snapshot(dirty).unwrap().id()));
    }
    let mut staged_delete = base.clone();
    let mut deletion = input(InputLayer::Index, "src/main.rs", b"");
    deletion.bytes = None;
    staged_delete.inputs.push(deletion);
    let deleted = snapshot(staged_delete.clone()).unwrap();
    staged_delete.inputs.last_mut().unwrap().bytes = Some(vec![]);
    let empty = snapshot(staged_delete).unwrap();
    assert_ne!(deleted.id(), empty.id());
    assert_ne!(base_id, deleted.id());
    assert_ne!(base_id, empty.id());
    assert_eq!(deleted.manifest().base_commit, empty.manifest().base_commit);
}

#[test]
fn order_independent_capture_preserves_staged_unstaged_and_editor_views() {
    let mut original = manifest();
    original.inputs.extend([
        input(InputLayer::Index, "src/main.rs", b"staged"),
        input(InputLayer::WorkingTree, "src/main.rs", b"unstaged"),
        input(InputLayer::Untracked, "new.txt", b"new"),
        input(InputLayer::Overlay, "src/main.rs", b"editor buffer"),
    ]);
    let expected = snapshot(original.clone()).unwrap();
    for turn in 0..original.inputs.len() {
        let mut shuffled = original.clone();
        shuffled.inputs.rotate_left(turn);
        shuffled.inputs.reverse();
        let captured = snapshot(shuffled).unwrap();
        assert_eq!(captured, expected);
    }
    for index in 1..original.inputs.len() {
        let mut changed = original.clone();
        changed.inputs[index].bytes.as_mut().unwrap().push(0xff);
        assert_ne!(snapshot(changed).unwrap().id(), expected.id());
    }
}

#[test]
fn binary_modes_symlinks_submodules_generated_and_mounted_inputs_are_bound() {
    let mut base = manifest();
    let mut binary = input(
        InputLayer::Untracked,
        "binary.dat",
        &[0, 0xff, 0x80, 10, 13],
    );
    let mut link = input(InputLayer::WorkingTree, "link", b"../../external-target");
    link.kind = InputKind::Symlink;
    link.mode = FileMode::Symlink;
    let mut module = input(
        InputLayer::Submodule,
        "vendor/lib",
        b"git:submodule-revision",
    );
    module.kind = InputKind::Submodule;
    module.mode = FileMode::Gitlink;
    base.inputs.extend([
        binary.clone(),
        link.clone(),
        module.clone(),
        input(InputLayer::Submodule, "vendor/lib/dirty.rs", b"dirty"),
        input(InputLayer::Generated, "build/schema.rs", b"generated"),
        input(InputLayer::Mounted, "external/config", b"mounted"),
    ]);
    let original = snapshot(base.clone()).unwrap();
    for index in 1..base.inputs.len() {
        let mut changed = base.clone();
        changed.inputs[index].bytes.as_mut().unwrap().push(1);
        assert_ne!(snapshot(changed).unwrap().id(), original.id());
    }
    binary.mode = FileMode::Executable;
    let mut changed = base.clone();
    changed.inputs[1] = binary;
    assert_ne!(snapshot(changed).unwrap().id(), original.id());
    link.bytes = Some(vec![0xff, b'/', b'.', b'.']);
    let mut changed = base;
    changed.inputs[2] = link;
    assert_ne!(snapshot(changed).unwrap().id(), original.id());
}

#[test]
fn every_valid_manifest_metadata_and_state_input_affects_identity() {
    let original = snapshot(manifest()).unwrap();
    let expected = original.id();
    let mut variants = Vec::new();
    let mut changed = manifest();
    changed.base_commit.push('2');
    variants.push(changed);
    let mut changed = manifest();
    changed.provenance.push('2');
    variants.push(changed);
    let mut changed = manifest();
    changed
        .environment
        .insert("RUST_VERSION".into(), "new".into());
    variants.push(changed);
    let mut changed = manifest();
    changed.environment.insert("TARGET".into(), "target".into());
    variants.push(changed);
    let mut changed = manifest();
    changed.inputs[0].path = "src/lib.rs".into();
    variants.push(changed);
    let mut changed = manifest();
    changed.inputs[0].layer = InputLayer::Index;
    variants.push(changed);
    let mut changed = manifest();
    changed.inputs[0].kind = InputKind::Symlink;
    changed.inputs[0].mode = FileMode::Symlink;
    variants.push(changed);
    let mut changed = manifest();
    changed.inclusion_policy.excluded_paths.insert(
        "unrelated-output.txt".into(),
        "not used by analysis or execution".into(),
    );
    variants.push(changed.clone());
    changed.inclusion_policy.excluded_paths.insert(
        "unrelated-output.txt".into(),
        "different recorded rationale".into(),
    );
    variants.push(changed);
    for variant in variants {
        assert_ne!(snapshot(variant).unwrap().id(), expected);
    }
    let mut state = State::new();
    state.insert(ElementId::new("symbol:one").unwrap(), 5);
    let five = Snapshot::new(scope(), Revision(0), manifest(), state.clone())
        .unwrap()
        .id();
    assert_ne!(five, expected);
    state.insert(ElementId::new("symbol:one").unwrap(), 6);
    let six = Snapshot::new(scope(), Revision(0), manifest(), state)
        .unwrap()
        .id();
    assert_ne!(six, expected);
    assert_ne!(five, six);
    assert_ne!(
        Snapshot::new(scope(), Revision(1), manifest(), State::new())
            .unwrap()
            .id(),
        expected
    );
    let mut scopes = vec![scope(); 3];
    scopes[0].organization = OrganizationId::new("other").unwrap();
    scopes[1].workspace = WorkspaceId::new("other").unwrap();
    scopes[2].repository = RepositoryId::new("other").unwrap();
    for scope in scopes {
        assert_ne!(
            Snapshot::new(scope, Revision(0), manifest(), State::new())
                .unwrap()
                .id(),
            expected
        );
    }
}

#[test]
fn canonical_paths_reject_traversal_absolute_and_ambiguous_representations() {
    for path in [
        "",
        "/absolute",
        "//absolute",
        "a//b",
        "a/",
        ".",
        "..",
        "a/./b",
        "a/../b",
        "a\\b",
        "C:/drive",
        "a:b",
        "a\0b",
        "a\nb",
    ] {
        let mut invalid = manifest();
        invalid.inputs[0].path = path.into();
        assert_eq!(
            snapshot(invalid),
            Err(SnapshotError::InvalidPath(path.into())),
            "accepted ambiguous path {path:?}"
        );
    }
    let mut exact_utf8 = manifest();
    exact_utf8.inputs[0].path = "src/é.rs".into();
    let composed = snapshot(exact_utf8.clone()).unwrap().id();
    exact_utf8.inputs[0].path = "src/e\u{301}.rs".into();
    assert_ne!(snapshot(exact_utf8).unwrap().id(), composed);
}

#[test]
fn duplicate_or_structurally_impossible_inputs_reject_in_either_order() {
    let mut duplicate = manifest();
    duplicate.inputs.push(duplicate.inputs[0].clone());
    assert!(matches!(
        snapshot(duplicate),
        Err(SnapshotError::DuplicateInput { .. })
    ));
    for reverse in [false, true] {
        let mut impossible = manifest();
        impossible
            .inputs
            .push(input(InputLayer::Base, "src", b"file"));
        if reverse {
            impossible.inputs.reverse();
        }
        assert!(matches!(
            snapshot(impossible),
            Err(SnapshotError::AmbiguousHierarchy { .. })
        ));
    }
}

#[test]
fn tombstones_allow_file_directory_replacement_without_ambiguous_live_inputs() {
    let mut capture = manifest();
    let mut removed = input(InputLayer::Index, "replaced", b"");
    removed.bytes = None;
    capture.inputs.extend([
        removed,
        input(InputLayer::Index, "replaced/child", b"now a directory"),
    ]);
    assert!(snapshot(capture.clone()).is_ok());
    capture.inputs.last_mut().unwrap().bytes = None;
    let index = capture.inputs.len() - 2;
    capture.inputs[index].bytes = Some(b"now a file".to_vec());
    assert!(snapshot(capture).is_ok());
}

#[test]
fn unsupported_or_incomplete_capture_cannot_construct_a_snapshot() {
    let mut invalid = manifest();
    invalid.canonicalization_version = 2;
    assert_eq!(
        snapshot(invalid),
        Err(SnapshotError::UnsupportedCanonicalization(2))
    );
    let mut invalid = manifest();
    invalid.inclusion_policy.version = 2;
    assert_eq!(
        snapshot(invalid),
        Err(SnapshotError::UnsupportedInclusionPolicy(2))
    );
    for layer in InputLayer::ALL {
        let mut invalid = manifest();
        invalid.inclusion_policy.captured_layers.remove(&layer);
        assert_eq!(
            snapshot(invalid),
            Err(SnapshotError::IncompleteCapture(layer))
        );
    }
    let mut invalid = manifest();
    invalid
        .inclusion_policy
        .excluded_paths
        .insert("src/main.rs".into(), "excluded".into());
    assert_eq!(
        snapshot(invalid),
        Err(SnapshotError::ExcludedInput("src/main.rs".into()))
    );
    let mut invalid = manifest();
    invalid
        .inclusion_policy
        .excluded_paths
        .insert("irrelevant".into(), "".into());
    assert_eq!(
        snapshot(invalid),
        Err(SnapshotError::MissingExclusionReason("irrelevant".into()))
    );
}

#[test]
fn invalid_modes_references_base_deletions_and_environment_reject() {
    let mut invalid = manifest();
    invalid.inputs[0].mode = FileMode::Symlink;
    assert!(matches!(
        snapshot(invalid),
        Err(SnapshotError::InvalidMode(_))
    ));
    let mut invalid = manifest();
    invalid.inputs[0].bytes = None;
    assert!(matches!(
        snapshot(invalid),
        Err(SnapshotError::DeletedBaseInput(_))
    ));
    for kind in [InputKind::Symlink, InputKind::Submodule] {
        for bytes in [vec![], vec![0], vec![b'a', 0, b'b']] {
            let mut invalid = manifest();
            invalid.inputs[0].kind = kind;
            invalid.inputs[0].mode = match kind {
                InputKind::Symlink => FileMode::Symlink,
                _ => FileMode::Gitlink,
            };
            invalid.inputs[0].bytes = Some(bytes);
            assert!(matches!(
                snapshot(invalid),
                Err(SnapshotError::InvalidReference(_))
            ));
        }
    }
    for (key, value) in [("", "value"), ("a=b", "value"), ("A", "a\0b")] {
        let mut invalid = manifest();
        invalid.environment.insert(key.into(), value.into());
        assert_eq!(snapshot(invalid), Err(SnapshotError::InvalidEnvironment));
    }
    for field in ["base_commit", "provenance"] {
        let mut invalid = manifest();
        match field {
            "base_commit" => invalid.base_commit = " ".into(),
            _ => invalid.provenance = "\n".into(),
        }
        assert_eq!(
            snapshot(invalid),
            Err(SnapshotError::MissingProvenance(field))
        );
    }
}

#[test]
fn successor_is_a_new_immutable_identity_and_requires_one_revision_step() {
    let original_manifest = manifest();
    let captured = snapshot(original_manifest.clone()).unwrap();
    let mut changed_input = original_manifest;
    changed_input.inputs[0].bytes = Some(b"external mutation".to_vec());
    assert_eq!(
        captured.manifest().inputs[0].bytes.as_deref(),
        Some(b"base".as_slice())
    );
    let state = State::from([(ElementId::new("reference:value").unwrap(), 11)]);
    let next = captured.successor(Revision(1), state.clone()).unwrap();
    assert_eq!(next.scope(), captured.scope());
    assert_eq!(next.revision(), Revision(1));
    assert_eq!(next.manifest(), captured.manifest());
    assert_eq!(next.state(), &state);
    assert_eq!(captured.revision(), Revision(0));
    assert!(captured.state().is_empty());
    assert_ne!(next.id(), captured.id());
    for revision in [Revision(0), Revision(2), Revision(u64::MAX)] {
        assert_eq!(
            captured.successor(revision, state.clone()),
            Err(SnapshotError::InvalidSuccessorRevision)
        );
    }
    let max = Snapshot::new(scope(), Revision(u64::MAX), manifest(), State::new()).unwrap();
    assert_eq!(
        max.successor(Revision(0), state),
        Err(SnapshotError::InvalidSuccessorRevision)
    );
}

#[test]
fn length_delimited_encoding_distinguishes_field_boundaries() {
    let mut first = manifest();
    first.base_commit = "ab".into();
    first.provenance = "c".into();
    let mut second = manifest();
    second.base_commit = "a".into();
    second.provenance = "bc".into();
    assert_ne!(
        snapshot(first).unwrap().id(),
        snapshot(second).unwrap().id()
    );
}

#[test]
fn manifest_address_preserves_capture_provenance_without_reference_state() {
    let mut source = manifest();
    source.inputs.extend([
        input(InputLayer::Index, "src/main.rs", b"staged"),
        input(InputLayer::Overlay, "src/main.rs", b"editor"),
    ]);
    let expected = source.digest();
    source.inputs.reverse();
    assert_eq!(source.digest(), expected);
    let mut changed = source.clone();
    changed.inputs[0].bytes = Some(b"different editor buffer".to_vec());
    assert_ne!(changed.digest(), expected);
    changed = source.clone();
    changed.inputs[0].bytes = None;
    assert_ne!(changed.digest(), expected);
    changed.inputs[0].bytes = Some(vec![]);
    assert_ne!(changed.digest(), source.digest());
    changed = source.clone();
    changed
        .inclusion_policy
        .excluded_paths
        .insert("ignored".into(), "not an input".into());
    assert_ne!(changed.digest(), expected);
    changed = source.clone();
    changed.provenance.push_str("/different-capture");
    assert_ne!(changed.digest(), expected);
    let first = snapshot(source.clone()).unwrap();
    let second = Snapshot::new(scope(), Revision(1), source, State::new()).unwrap();
    assert_ne!(first.id(), second.id());
    assert_eq!(first.manifest().digest(), second.manifest().digest());
}

#[test]
fn effective_view_keeps_layer_precedence_and_rejects_external_aliases() {
    use celetixo_engine::snapshot::{EffectiveWorkspaceError, effective_workspace};
    let mut source = manifest();
    source.inputs.extend([
        input(InputLayer::WorkingTree, "src/main.rs", b"disk"),
        input(InputLayer::Overlay, "src/main.rs", b"editor"),
    ]);
    let view = effective_workspace(&snapshot(source.clone()).unwrap()).unwrap();
    assert_eq!(
        view["src/main.rs"].bytes.as_deref(),
        Some(b"editor".as_slice())
    );
    source
        .inputs
        .push(input(InputLayer::Mounted, "src/main.rs", b"external"));
    assert_eq!(
        effective_workspace(&snapshot(source).unwrap()),
        Err(EffectiveWorkspaceError::ExternalCollision(
            "src/main.rs".into()
        ))
    );
    let mut crossed = manifest();
    crossed
        .inputs
        .push(input(InputLayer::WorkingTree, "src", b"file"));
    assert_eq!(
        effective_workspace(&snapshot(crossed).unwrap()),
        Err(EffectiveWorkspaceError::NonDirectoryAncestor(
            "src/main.rs".into()
        ))
    );
}
