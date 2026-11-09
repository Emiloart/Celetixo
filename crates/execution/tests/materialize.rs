#![cfg(target_os = "linux")]

use celetixo_engine::ids::{
    Digest, OrganizationId, PrincipalId, RepositoryId, Revision, Scope, SourceAttemptId,
    TransactionId, WorkspaceId,
};
use celetixo_engine::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotManifest, State,
    WorkspaceInput,
};
use celetixo_engine::source_patch::{
    PatchBinding, PatchEntry, PatchOperation, PatchPolicy, Preimage, SourcePatch, SymlinkPolicy,
};
use celetixo_execution::materialize::{
    CleanupOutcome, MaterializationError, MaterializationEvent, PrivateWorkspace,
};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

fn input(path: &str, bytes: &[u8], mode: FileMode) -> WorkspaceInput {
    WorkspaceInput {
        path: path.into(),
        layer: InputLayer::Base,
        kind: InputKind::File,
        mode,
        bytes: Some(bytes.into()),
    }
}
fn snapshot(inputs: Vec<WorkspaceInput>, environment: BTreeMap<String, String>) -> Snapshot {
    Snapshot::new(
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("ws").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        },
        Revision(3),
        SnapshotManifest {
            base_commit: "immutable Git-tree fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "materialization test".into(),
            inputs,
            environment,
        },
        State::new(),
    )
    .unwrap()
}
fn base() -> Snapshot {
    snapshot(
        vec![
            input("README.md", b"frozen original", FileMode::Regular),
            input("dir/original", b"untouched", FileMode::Regular),
            input("run", b"#!/bin/sh\nexit 0\n", FileMode::Executable),
        ],
        BTreeMap::new(),
    )
}
fn binding(base: &Snapshot) -> PatchBinding {
    let digest = Digest::of("test/materialize", b"analysis");
    PatchBinding {
        scope: base.scope().clone(),
        principal: PrincipalId::new("human").unwrap(),
        transaction: TransactionId::new("tx").unwrap(),
        transaction_attempt: 1,
        source_attempt: SourceAttemptId::new("materialize").unwrap(),
        base_snapshot: base.id(),
        base_revision: base.revision(),
        analysis_artifact: digest,
        index_generation: digest,
        impact_request: digest,
        impact_result: digest,
        created_at_ms: 0,
        expires_at_ms: 100,
        producer: "controlled materialization fixture".into(),
    }
}
fn patch(base: &Snapshot) -> SourcePatch {
    SourcePatch::new(
        binding(base),
        PatchPolicy::default(),
        vec![PatchOperation::Create {
            path: "dir/new.bin".into(),
            entry: PatchEntry::file(vec![0, 255, 10, 13, 42], FileMode::Regular).unwrap(),
        }],
    )
    .unwrap()
}
fn root_of(source: &std::path::Path) -> PathBuf {
    source.parent().unwrap().to_path_buf()
}

#[test]
fn exact_immutable_bytes_and_modes_are_materialized_without_reading_or_mutating_live_source() {
    let live = tempfile::tempdir().unwrap();
    fs::write(live.path().join("README.md"), b"live differs after capture").unwrap();
    let base = base();
    let original = base.clone();
    let patch = patch(&base);
    let workspace = PrivateWorkspace::materialize(&base, &patch).unwrap();
    assert_eq!(
        fs::read(workspace.source_path().join("README.md")).unwrap(),
        b"frozen original"
    );
    assert_eq!(
        fs::read(workspace.source_path().join("dir/new.bin")).unwrap(),
        vec![0, 255, 10, 13, 42]
    );
    assert_eq!(
        fs::metadata(workspace.source_path().join("run"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
    assert_eq!(
        fs::metadata(workspace.source_path().join("README.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o644
    );
    assert_eq!(
        fs::metadata(workspace.source_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
    assert_eq!(
        fs::metadata(root_of(workspace.source_path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o700
    );
    assert_eq!(workspace.applied().snapshot().revision(), Revision(3));
    assert_eq!(
        workspace.report().snapshot(),
        workspace.applied().snapshot().id()
    );
    assert_eq!(
        workspace.report().manifest(),
        workspace.applied().snapshot().manifest().digest()
    );
    assert_eq!(
        workspace.report().application(),
        workspace.applied().report().digest()
    );
    assert_eq!(workspace.report().files(), 4);
    assert_eq!(workspace.report().directories(), 2);
    assert_eq!(workspace.report().symlinks(), 0);
    workspace.validate_contents().unwrap();
    workspace.validate_contents().unwrap();
    assert_eq!(base, original);
    assert_eq!(
        fs::read(live.path().join("README.md")).unwrap(),
        b"live differs after capture"
    );
    let owned = root_of(workspace.source_path());
    assert_eq!(workspace.cleanup(), CleanupOutcome::Removed);
    assert!(!owned.exists());
}

#[test]
fn private_instances_share_receipt_identity_but_never_writable_files_or_cleanup_scope() {
    let base = base();
    let patch = patch(&base);
    let first = PrivateWorkspace::materialize(&base, &patch).unwrap();
    let second = PrivateWorkspace::materialize(&base, &patch).unwrap();
    assert_ne!(first.source_path(), second.source_path());
    assert_eq!(first.report(), second.report());
    assert_eq!(first.report().digest(), second.report().digest());
    fs::write(
        first.source_path().join("README.md"),
        b"changed private source",
    )
    .unwrap();
    assert!(first.validate_contents().is_err());
    second.validate_contents().unwrap();
    assert_eq!(first.cleanup(), CleanupOutcome::Removed);
    assert!(second.source_path().exists());
    assert_eq!(second.cleanup(), CleanupOutcome::Removed);
}

#[test]
fn explicit_quarantine_retains_only_the_owned_workspace_for_executor_reconciliation() {
    let base = base();
    let patch = patch(&base);
    let workspace = PrivateWorkspace::materialize(&base, &patch).unwrap();
    let root = root_of(workspace.source_path());
    let outcome = workspace.quarantine("termination could not be established");
    assert_eq!(
        outcome,
        CleanupOutcome::Quarantined {
            path: root.clone(),
            reason: "termination could not be established".into()
        }
    );
    assert!(root.join("source/README.md").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn borrowed_executor_quarantine_closes_validation_and_drop_cannot_remove_the_retained_source() {
    let base = base();
    let patch = patch(&base);
    let mut workspace = PrivateWorkspace::materialize(&base, &patch).unwrap();
    let root = root_of(workspace.source_path());
    let outcome = workspace.quarantine_in_place("process tree termination uncertain");
    assert!(matches!(outcome, CleanupOutcome::Quarantined { .. }));
    assert!(workspace.validate_contents().is_err());
    assert_eq!(
        workspace.quarantine_in_place("cannot overwrite prior reason"),
        outcome
    );
    assert_eq!(workspace.cleanup(), outcome);
    assert!(root.join("source/README.md").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn symlinks_are_reproduced_as_links_and_cleanup_never_follows_outside_targets() {
    let base = base();
    let patch = SourcePatch::new(
        binding(&base),
        PatchPolicy {
            symlinks: SymlinkPolicy::InternalRelativeOnly,
            ..PatchPolicy::default()
        },
        vec![PatchOperation::Create {
            path: "dir/link".into(),
            entry: PatchEntry::symlink(b"../README.md".to_vec()).unwrap(),
        }],
    )
    .unwrap();
    let workspace = PrivateWorkspace::materialize(&base, &patch).unwrap();
    assert!(
        fs::symlink_metadata(workspace.source_path().join("dir/link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(workspace.source_path().join("dir/link")).unwrap(),
        PathBuf::from("../README.md")
    );
    assert_eq!(workspace.report().symlinks(), 1);
    workspace.validate_contents().unwrap();
    let sentinel = tempfile::tempdir().unwrap();
    fs::write(sentinel.path().join("sentinel"), b"keep").unwrap();
    symlink(
        sentinel.path(),
        workspace.source_path().join("attacker-link"),
    )
    .unwrap();
    assert!(workspace.validate_contents().is_err());
    assert_eq!(workspace.cleanup(), CleanupOutcome::Removed);
    assert_eq!(fs::read(sentinel.path().join("sentinel")).unwrap(), b"keep");
}

#[test]
fn exact_census_detects_changed_bytes_modes_links_missing_inputs_and_unexpected_directories() {
    let base = base();
    let patch = patch(&base);
    for kind in 0..6 {
        let workspace = PrivateWorkspace::materialize(&base, &patch).unwrap();
        let path = workspace.source_path().join("README.md");
        match kind {
            0 => fs::write(&path, b"tampered").unwrap(),
            1 => fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap(),
            2 => fs::remove_file(&path).unwrap(),
            3 => fs::create_dir(workspace.source_path().join("unexpected-empty")).unwrap(),
            4 => {
                fs::remove_file(&path).unwrap();
                symlink("dir/original", &path).unwrap();
            }
            _ => fs::hard_link(&path, workspace.source_path().join("alias")).unwrap(),
        }
        assert!(
            workspace.validate_contents().is_err(),
            "missed mutation {kind}"
        );
        assert_eq!(workspace.cleanup(), CleanupOutcome::Removed);
    }
}

#[test]
fn a_directory_replaced_with_an_escape_symlink_is_never_traversed() {
    let base = base();
    let patch = patch(&base);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"keep").unwrap();
    let mut root = None;
    let error = PrivateWorkspace::materialize_with_hook(&base, &patch, |event| {
        if let MaterializationEvent::BeforeEntry {
            source,
            logical_path,
        } = event
        {
            root = Some(root_of(&source));
            if logical_path == "README.md" {
                fs::remove_dir(source.join("dir"))?;
                symlink(outside.path(), source.join("dir"))?;
            }
        }
        Ok(())
    })
    .err()
    .unwrap();
    assert!(matches!(
        error,
        MaterializationError::Failed {
            cleanup: CleanupOutcome::Removed,
            ..
        }
    ));
    assert!(!root.unwrap().exists());
    assert!(!outside.path().join("new.bin").exists());
    assert_eq!(fs::read(outside.path().join("sentinel")).unwrap(), b"keep");
}

#[test]
fn substituted_source_root_cannot_redirect_writes_and_partial_resources_are_cleaned() {
    let base = base();
    let patch = patch(&base);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"keep").unwrap();
    let error = PrivateWorkspace::materialize_with_hook(&base, &patch, |event| {
        if let MaterializationEvent::PrivateRootCreated(source) = event {
            fs::remove_dir(&source)?;
            symlink(outside.path(), source)?;
        }
        Ok(())
    })
    .err()
    .unwrap();
    assert!(matches!(
        error,
        MaterializationError::Failed {
            cleanup: CleanupOutcome::Removed,
            ..
        }
    ));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}

#[test]
fn replaced_private_parent_is_quarantined_without_deleting_the_replacement() {
    let base = base();
    let patch = patch(&base);
    let moved = tempfile::tempdir().unwrap();
    let mut replacement = None;
    let error = PrivateWorkspace::materialize_with_hook(&base, &patch, |event| {
        if let MaterializationEvent::PrivateRootCreated(source) = event {
            let root = root_of(&source);
            fs::rename(&root, moved.path().join("old-owned-root"))?;
            fs::create_dir(&root)?;
            fs::write(root.join("replacement-sentinel"), b"keep replacement")?;
            replacement = Some(root);
        }
        Ok(())
    })
    .err()
    .unwrap();
    assert!(matches!(
        error,
        MaterializationError::Failed {
            cleanup: CleanupOutcome::Quarantined { .. },
            ..
        }
    ));
    let replacement = replacement.unwrap();
    assert_eq!(
        fs::read(replacement.join("replacement-sentinel")).unwrap(),
        b"keep replacement"
    );
    fs::remove_dir_all(replacement).unwrap();
}

#[test]
fn fault_during_copy_and_final_content_mutation_publish_no_workspace_and_report_cleanup() {
    let base = base();
    let original = base.clone();
    let patch = patch(&base);
    for stage in 0..2 {
        let mut owned = None;
        let error = PrivateWorkspace::materialize_with_hook(&base, &patch, |event| {
            match event {
                MaterializationEvent::PrivateRootCreated(source) => owned = Some(root_of(&source)),
                MaterializationEvent::BeforeEntry { logical_path, .. }
                    if stage == 0 && logical_path == "dir/new.bin" =>
                {
                    return Err(io::Error::other("injected write/disk failure"));
                }
                MaterializationEvent::BeforeValidation(source) if stage == 1 => {
                    fs::write(source.join("README.md"), b"mutation after copy")?
                }
                _ => {}
            }
            Ok(())
        })
        .err()
        .unwrap();
        assert!(matches!(
            error,
            MaterializationError::Failed {
                cleanup: CleanupOutcome::Removed,
                ..
            }
        ));
        assert!(!owned.unwrap().exists());
        assert_eq!(base, original);
    }
}

#[test]
fn gitlinks_and_unimplemented_capture_metadata_reject_before_any_private_disk_side_effect() {
    let mut gitlink = input("submodule", b"pinned-object", FileMode::Regular);
    gitlink.kind = InputKind::Submodule;
    gitlink.mode = FileMode::Gitlink;
    let base = snapshot(vec![gitlink], BTreeMap::new());
    let error = PrivateWorkspace::materialize_with_hook(&base, &patch(&base), |_| {
        panic!("must reject before creating workspace")
    })
    .err()
    .unwrap();
    assert!(matches!(error, MaterializationError::UnsupportedGitlink(_)));
    for prefix in [
        "celetixo.capture.directory.",
        "celetixo.capture.unix-mode.",
        "celetixo.source_application.origin.celetixo.capture.directory.",
    ] {
        let key = format!("{prefix}{}", "0".repeat(64));
        let base = snapshot(
            vec![input("a", b"x", FileMode::Regular)],
            BTreeMap::from([(key, "retained-but-unsupported".into())]),
        );
        let error = PrivateWorkspace::materialize_with_hook(&base, &patch(&base), |_| {
            panic!("must reject before creating workspace")
        })
        .err()
        .unwrap();
        assert!(
            matches!(error, MaterializationError::UnsupportedCapturedMetadata(_)),
            "{error:?}"
        );
    }
}

#[test]
fn declared_resource_bounds_include_derived_directory_census_before_creating_disk_inputs() {
    let base = snapshot(vec![], BTreeMap::new());
    let patch = SourcePatch::new(
        binding(&base),
        PatchPolicy {
            max_result_files: 2,
            ..PatchPolicy::default()
        },
        vec![PatchOperation::Create {
            path: "a/b/c".into(),
            entry: PatchEntry::file(b"x".to_vec(), FileMode::Regular).unwrap(),
        }],
    )
    .unwrap();
    let error = PrivateWorkspace::materialize_with_hook(&base, &patch, |_| {
        panic!("must reject before creating workspace")
    })
    .err()
    .unwrap();
    assert!(matches!(error, MaterializationError::Limit(_)));
}

#[test]
fn immutable_report_uncertainty_is_retained_and_non_operational_git_configuration_is_not_a_git_directory()
 {
    let mut metadata = input(".git/config", b"[core]\n", FileMode::Regular);
    metadata.layer = InputLayer::Mounted;
    let base = snapshot(
        vec![input("a", b"old", FileMode::Regular), metadata],
        BTreeMap::from([(
            "celetixo.capture.uncertainty".into(),
            "LiveSourceNotAtomic".into(),
        )]),
    );
    let before = Preimage::of(
        base.manifest()
            .inputs
            .iter()
            .find(|i| i.path == "a")
            .unwrap(),
    )
    .unwrap();
    let patch = SourcePatch::new(
        binding(&base),
        PatchPolicy::default(),
        vec![PatchOperation::Replace {
            path: "a".into(),
            expected: before,
            entry: PatchEntry::file(b"new".to_vec(), FileMode::Regular).unwrap(),
        }],
    )
    .unwrap();
    let workspace = PrivateWorkspace::materialize(&base, &patch).unwrap();
    assert!(!workspace.source_path().join(".git").exists());
    assert_eq!(
        workspace.applied().report().base_manifest(),
        base.manifest()
    );
    assert_eq!(
        workspace.applied().snapshot().manifest().environment["celetixo.source_application.origin.celetixo.capture.uncertainty"],
        "LiveSourceNotAtomic"
    );
    assert_eq!(
        workspace.applied().report().non_operational_inputs().len(),
        1
    );
    workspace.validate_contents().unwrap();
    assert_eq!(workspace.cleanup(), CleanupOutcome::Removed);
}
