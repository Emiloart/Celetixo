#![cfg(unix)]

use celetixo_engine::ids::{OrganizationId, RepositoryId, Revision, Scope, WorkspaceId};
use celetixo_engine::snapshot::{FileMode, InputKind, InputLayer, WorkspaceInput};
use celetixo_repository::capture::{
    CaptureConsistency, CaptureError, CaptureIssue, CaptureRequest, ExternalInput, Overlay,
    capture, capture_git_tree, capture_with_hook,
};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("capture-test-org").unwrap(),
        workspace: WorkspaceId::new("capture-test-workspace").unwrap(),
        repository: RepositoryId::new("capture-test-repository").unwrap(),
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8_lossy(&result.stdout).trim().into()
}

fn repository() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.name", "Capture Fixture"]);
    git(
        dir.path(),
        &["config", "user.email", "capture@example.invalid"],
    );
    fs::write(
        dir.path().join("source.py"),
        b"def answer():\n    return 42\n",
    )
    .unwrap();
    fs::write(dir.path().join(".gitignore"), b"ignored.txt\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "fixture"]);
    dir
}

fn request(root: &Path) -> CaptureRequest {
    CaptureRequest::new(root, scope(), Revision(0))
}

fn input<'a>(
    result: &'a celetixo_repository::capture::CaptureResult,
    path: &str,
    layer: InputLayer,
) -> &'a WorkspaceInput {
    result
        .snapshot
        .manifest()
        .inputs
        .iter()
        .find(|i| i.path == path && i.layer == layer)
        .unwrap()
}

#[test]
fn clean_capture_is_reproducible_and_live_atomicity_is_explicit() {
    let dir = repository();
    let first = capture(&request(dir.path())).unwrap();
    let second = capture(&request(dir.path())).unwrap();
    assert_eq!(first.snapshot.id(), second.snapshot.id());
    assert_eq!(first.report, second.report);
    assert!(first.snapshot.state().is_empty());
    assert!(first.report.census_complete());
    assert!(!first.report.complete());
    assert_eq!(
        first.report.consistency,
        CaptureConsistency::ObservedTwoPass
    );
    assert_eq!(first.report.issues.len(), 1);
    assert!(
        first
            .report
            .issues
            .contains(&CaptureIssue::LiveSourceNotAtomic)
    );
    assert!(first.report.excluded_paths.contains_key(".git"));
    assert_eq!(
        input(&first, "source.py", InputLayer::Base).bytes,
        input(&first, "source.py", InputLayer::WorkingTree).bytes
    );
    assert_eq!(
        first.report.content_digests.len(),
        first.snapshot.manifest().inputs.len()
    );
}

#[test]
fn staged_unstaged_untracked_ignored_and_overlay_bytes_are_distinct() {
    let dir = repository();
    let clean = capture(&request(dir.path())).unwrap();
    fs::write(dir.path().join("source.py"), b"staged\n").unwrap();
    git(dir.path(), &["add", "source.py"]);
    fs::write(dir.path().join("source.py"), b"unstaged\n").unwrap();
    fs::write(dir.path().join("new.py"), b"untracked\n").unwrap();
    fs::write(
        dir.path().join("ignored.txt"),
        b"relevant despite Git ignore\n",
    )
    .unwrap();
    let mut req = request(dir.path());
    req.overlays.insert(
        "source.py".into(),
        Overlay {
            version: 7,
            bytes: Some(b"unsaved\n".to_vec()),
            mode: FileMode::Regular,
        },
    );
    let dirty = capture(&req).unwrap();
    assert_ne!(clean.snapshot.id(), dirty.snapshot.id());
    assert_eq!(
        input(&dirty, "source.py", InputLayer::Index)
            .bytes
            .as_deref(),
        Some(b"staged\n".as_slice())
    );
    assert_eq!(
        input(&dirty, "source.py", InputLayer::WorkingTree)
            .bytes
            .as_deref(),
        Some(b"unstaged\n".as_slice())
    );
    assert_eq!(
        dirty.effective_files().unwrap()["source.py"].bytes,
        b"unsaved\n"
    );
    assert_eq!(
        dirty.effective_files().unwrap()["new.py"].bytes,
        b"untracked\n"
    );
    assert!(dirty.effective_files().unwrap().contains_key("ignored.txt"));
    req.overlays.get_mut("source.py").unwrap().version = 8;
    assert_ne!(dirty.snapshot.id(), capture(&req).unwrap().snapshot.id());
}

#[test]
fn deletions_renames_and_overlay_tombstones_do_not_resurrect_base_content() {
    let dir = repository();
    git(dir.path(), &["mv", "source.py", "moved.py"]);
    let moved = capture(&request(dir.path())).unwrap();
    assert!(moved.effective_files().unwrap().contains_key("moved.py"));
    assert!(!moved.effective_files().unwrap().contains_key("source.py"));
    assert!(
        input(&moved, "source.py", InputLayer::WorkingTree)
            .bytes
            .is_none()
    );
    fs::remove_file(dir.path().join("moved.py")).unwrap();
    let absent = capture(&request(dir.path())).unwrap();
    assert!(!absent.effective_files().unwrap().contains_key("moved.py"));
    let mut req = request(dir.path());
    req.overlays.insert(
        ".gitignore".into(),
        Overlay {
            version: 1,
            bytes: None,
            mode: FileMode::Regular,
        },
    );
    let overlaid = capture(&req).unwrap();
    assert!(
        !overlaid
            .effective_files()
            .unwrap()
            .contains_key(".gitignore")
    );
}

#[test]
fn concurrent_file_change_aborts_and_next_capture_recovers() {
    let dir = repository();
    let result = capture_with_hook(&request(dir.path()), |_| {
        fs::write(dir.path().join("source.py"), b"changed\n").unwrap()
    });
    assert!(matches!(result, Err(CaptureError::ChangedDuringCapture)));
    let recovered = capture(&request(dir.path())).unwrap();
    assert_eq!(
        recovered.effective_files().unwrap()["source.py"].bytes,
        b"changed\n"
    );
}

#[test]
fn observed_change_and_restore_is_detected_by_metadata() {
    let dir = repository();
    let original = fs::read(dir.path().join("source.py")).unwrap();
    let result = capture_with_hook(&request(dir.path()), |_| {
        fs::write(dir.path().join("source.py"), b"temporary\n").unwrap();
        fs::write(dir.path().join("source.py"), &original).unwrap();
    });
    assert!(matches!(result, Err(CaptureError::ChangedDuringCapture)));
}

#[test]
fn concurrent_staging_and_head_changes_are_rejected() {
    let dir = repository();
    let result = capture_with_hook(&request(dir.path()), |_| {
        fs::write(dir.path().join("new.py"), b"new\n").unwrap();
        git(dir.path(), &["add", "new.py"]);
    });
    assert!(matches!(result, Err(CaptureError::ChangedDuringCapture)));
    let result = capture_with_hook(&request(dir.path()), |_| {
        git(dir.path(), &["commit", "-qm", "advance"]);
    });
    assert!(matches!(result, Err(CaptureError::ChangedDuringCapture)));
}

#[test]
fn executable_modes_and_symlinks_are_preserved_without_following() {
    let dir = repository();
    fs::write(dir.path().join("run"), b"#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(dir.path().join("run"), fs::Permissions::from_mode(0o755)).unwrap();
    symlink("/outside/never/read", dir.path().join("link")).unwrap();
    let result = capture(&request(dir.path())).unwrap();
    assert_eq!(
        result.effective_files().unwrap()["run"].mode,
        FileMode::Executable
    );
    assert_eq!(
        result.effective_files().unwrap()["link"].kind,
        InputKind::Symlink
    );
    assert_eq!(
        result.effective_files().unwrap()["link"].bytes,
        b"/outside/never/read"
    );
    assert!(!result.effective_files().unwrap().contains_key("link/child"));
    fs::set_permissions(dir.path().join("run"), fs::Permissions::from_mode(0o644)).unwrap();
    assert_ne!(
        result.snapshot.id(),
        capture(&request(dir.path())).unwrap().snapshot.id()
    );
}

#[test]
fn required_inputs_exclusions_and_finite_budgets_fail_explicitly() {
    let dir = repository();
    let mut req = request(dir.path());
    req.policy
        .required_paths
        .insert("generated/missing.py".into());
    req.policy
        .excluded_paths
        .insert("source.py".into(), "test intentional omission".into());
    let result = capture(&req).unwrap();
    assert!(!result.report.census_complete());
    assert!(
        result
            .report
            .issues
            .contains(&CaptureIssue::MissingRequiredInput(
                "generated/missing.py".into()
            ))
    );
    assert!(
        result
            .report
            .issues
            .contains(&CaptureIssue::ExcludedInput("source.py".into()))
    );
    assert!(!result.effective_files().unwrap().contains_key("source.py"));
    assert_eq!(
        result.snapshot.manifest().inclusion_policy.excluded_paths["source.py"],
        "test intentional omission"
    );
    let mut req = request(dir.path());
    req.policy.max_file_bytes = 2;
    assert!(matches!(capture(&req), Err(CaptureError::Limit(_))));
    req = request(dir.path());
    req.policy.max_files = 1;
    assert!(matches!(capture(&req), Err(CaptureError::Limit(_))));
}

#[test]
fn generated_and_mounted_inputs_are_bound_and_missing_inputs_remain_unknown() {
    let dir = repository();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("schema.json"), b"{\"v\":1}").unwrap();
    let mut req = request(dir.path());
    req.external_inputs.push(ExternalInput {
        path: "generated/schema.json".into(),
        source: external.path().join("schema.json"),
        layer: InputLayer::Generated,
        required: true,
    });
    req.external_inputs.push(ExternalInput {
        path: "mounted/missing".into(),
        source: external.path().join("missing"),
        layer: InputLayer::Mounted,
        required: true,
    });
    req.policy
        .required_paths
        .insert("generated/schema.json".into());
    let first = capture(&req).unwrap();
    assert!(
        first
            .effective_files()
            .unwrap()
            .contains_key("generated/schema.json")
    );
    assert!(
        !first
            .report
            .issues
            .contains(&CaptureIssue::MissingRequiredInput(
                "generated/schema.json".into()
            ))
    );
    assert!(
        first
            .report
            .issues
            .contains(&CaptureIssue::MissingRequiredInput(
                "mounted/missing".into()
            ))
    );
    fs::write(external.path().join("schema.json"), b"{\"v\":2}").unwrap();
    assert_ne!(first.snapshot.id(), capture(&req).unwrap().snapshot.id());
}

#[test]
fn symlink_ancestor_external_inputs_are_rejected() {
    let dir = repository();
    let external = tempfile::tempdir().unwrap();
    fs::create_dir(external.path().join("real")).unwrap();
    fs::write(external.path().join("real/secret"), b"never traverse alias").unwrap();
    symlink(external.path().join("real"), external.path().join("alias")).unwrap();
    let mut req = request(dir.path());
    req.external_inputs.push(ExternalInput {
        path: "mounted/input".into(),
        source: external.path().join("alias/secret"),
        layer: InputLayer::Mounted,
        required: true,
    });
    assert!(matches!(capture(&req), Err(CaptureError::Io { .. })));
}

#[test]
fn non_utf8_paths_are_rejected_without_lossy_collision() {
    use std::os::unix::ffi::OsStringExt;
    let dir = repository();
    fs::write(
        dir.path()
            .join(std::ffi::OsString::from_vec(vec![b'x', 0xff])),
        b"unsupported name",
    )
    .unwrap();
    assert!(matches!(
        capture(&request(dir.path())),
        Err(CaptureError::InvalidPath)
    ));
}

#[test]
fn unborn_repository_and_binary_content_are_captured() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    fs::write(dir.path().join("data.bin"), [0, 255, 0, 1]).unwrap();
    let result = capture(&request(dir.path())).unwrap();
    assert_eq!(result.snapshot.manifest().base_commit, "unborn");
    assert_eq!(
        result.effective_files().unwrap()["data.bin"].bytes,
        [0, 255, 0, 1]
    );
}

#[test]
fn pinned_tree_is_complete_and_never_claims_to_capture_dirty_workspace() {
    let dir = repository();
    let commit = git(dir.path(), &["rev-parse", "HEAD"]);
    let pinned = capture_git_tree(&request(dir.path()), &commit).unwrap();
    assert_eq!(pinned.report.consistency, CaptureConsistency::GitTree);
    assert!(pinned.report.complete());
    fs::write(
        dir.path().join("source.py"),
        b"dirty workspace ignored only by explicit tree API\n",
    )
    .unwrap();
    fs::write(dir.path().join("new.py"), b"untracked\n").unwrap();
    let same_tree = capture_git_tree(&request(dir.path()), &commit).unwrap();
    assert_eq!(pinned.snapshot.id(), same_tree.snapshot.id());
    assert!(!same_tree.effective_files().unwrap().contains_key("new.py"));
    let live = capture(&request(dir.path())).unwrap();
    assert_ne!(live.snapshot.id(), pinned.snapshot.id());
    assert!(live.effective_files().unwrap().contains_key("new.py"));
    assert_ne!(
        live.effective_files().unwrap()["source.py"].bytes,
        pinned.effective_files().unwrap()["source.py"].bytes
    );
}

#[test]
fn pinned_tree_accepts_immutable_overlay_generated_and_environment_inputs() {
    let dir = repository();
    let commit = git(dir.path(), &["rev-parse", "HEAD"]);
    let mut req = request(dir.path());
    req.overlays.insert(
        "source.py".into(),
        Overlay {
            version: 1,
            bytes: Some(b"buffer\n".to_vec()),
            mode: FileMode::Regular,
        },
    );
    req.immutable_inputs.push(WorkspaceInput {
        path: "generated/schema.json".into(),
        layer: InputLayer::Generated,
        kind: InputKind::File,
        mode: FileMode::Regular,
        bytes: Some(b"{}".to_vec()),
    });
    req.policy
        .required_paths
        .insert("generated/schema.json".into());
    req.environment.insert("PYTHONPATH".into(), "src".into());
    let result = capture_git_tree(&req, &commit).unwrap();
    assert!(result.report.complete());
    assert_eq!(
        result.effective_files().unwrap()["source.py"].bytes,
        b"buffer\n"
    );
    assert_eq!(
        result.effective_files().unwrap()["generated/schema.json"].bytes,
        b"{}"
    );
    req.environment.insert("PYTHONPATH".into(), "lib".into());
    assert_ne!(
        result.snapshot.id(),
        capture_git_tree(&req, &commit).unwrap().snapshot.id()
    );
    req.external_inputs.push(ExternalInput {
        path: "mounted/a".into(),
        source: dir.path().join("source.py"),
        layer: InputLayer::Mounted,
        required: true,
    });
    assert!(matches!(
        capture_git_tree(&req, &commit),
        Err(CaptureError::InvalidPolicy(_))
    ));
}

#[test]
fn full_commit_oid_and_valid_policy_are_required_for_pinned_capture() {
    let dir = repository();
    assert!(matches!(
        capture_git_tree(&request(dir.path()), "HEAD"),
        Err(CaptureError::InvalidPolicy(_))
    ));
    let mut req = request(dir.path());
    req.policy
        .excluded_paths
        .insert("../outside".into(), "invalid".into());
    assert!(matches!(capture(&req), Err(CaptureError::InvalidPath)));
    let mut req = request(dir.path());
    req.policy
        .excluded_paths
        .insert("source.py".into(), "".into());
    assert!(matches!(capture(&req), Err(CaptureError::InvalidPolicy(_))));
}

#[test]
fn git_filters_hooks_and_fsmonitor_are_not_executed_during_capture() {
    let dir = repository();
    let sentinel = dir.path().join("executed");
    let command = format!("touch {}", sentinel.display());
    git(dir.path(), &["config", "core.fsmonitor", &command]);
    git(dir.path(), &["config", "filter.evil.clean", &command]);
    git(dir.path(), &["config", "filter.evil.smudge", &command]);
    fs::write(
        dir.path().join(".gitattributes"),
        b"source.py filter=evil\n",
    )
    .unwrap();
    let result = capture(&request(dir.path())).unwrap();
    assert!(result.effective_files().unwrap().contains_key("source.py"));
    assert!(!sentinel.exists());
}

#[test]
fn initialized_submodule_dirty_content_is_captured_and_uninitialized_is_unknown() {
    let dependency = repository();
    let dir = repository();
    git(
        dir.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            dependency.path().to_str().unwrap(),
            "vendor/library",
        ],
    );
    git(dir.path(), &["commit", "-qm", "submodule"]);
    fs::write(
        dir.path().join("vendor/library/source.py"),
        b"dirty submodule\n",
    )
    .unwrap();
    let result = capture(&request(dir.path())).unwrap();
    assert_eq!(
        result.effective_files().unwrap()["vendor/library/source.py"].bytes,
        b"dirty submodule\n"
    );
    assert!(result.report.census_complete());
    assert_eq!(
        input(&result, "vendor/library", InputLayer::Submodule).kind,
        InputKind::Submodule
    );
    fs::remove_dir_all(dir.path().join("vendor/library")).unwrap();
    let absent = capture(&request(dir.path())).unwrap();
    assert!(
        absent
            .report
            .issues
            .contains(&CaptureIssue::UninitializedSubmodule(
                "vendor/library".into()
            ))
    );
    assert!(!absent.report.census_complete());
}

#[test]
fn pinned_tree_submodule_reference_does_not_claim_nested_content_complete() {
    let dependency = repository();
    let dir = repository();
    git(
        dir.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            dependency.path().to_str().unwrap(),
            "vendor/library",
        ],
    );
    git(dir.path(), &["commit", "-qm", "submodule"]);
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let result = capture_git_tree(&request(dir.path()), &head).unwrap();
    assert!(!result.report.complete());
    assert!(
        result
            .report
            .issues
            .contains(&CaptureIssue::UninitializedSubmodule(
                "vendor/library".into()
            ))
    );
}

#[test]
fn sha256_git_object_format_is_verified_and_supported() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "--object-format=sha256"]);
    git(dir.path(), &["config", "user.name", "Capture Fixture"]);
    git(
        dir.path(),
        &["config", "user.email", "capture@example.invalid"],
    );
    fs::write(dir.path().join("a.py"), b"a=1\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "sha256"]);
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    assert_eq!(head.len(), 64);
    let result = capture_git_tree(&request(dir.path()), &head).unwrap();
    assert!(result.report.complete());
    assert_eq!(result.effective_files().unwrap()["a.py"].bytes, b"a=1\n");
}

#[test]
fn object_database_content_substitution_is_rejected() {
    let dir = repository();
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let original = git(dir.path(), &["rev-parse", "HEAD:source.py"]);
    fs::write(dir.path().join("other"), b"substituted blob\n").unwrap();
    let other = git(dir.path(), &["hash-object", "-w", "other"]);
    let object = |oid: &str| {
        dir.path()
            .join(".git/objects")
            .join(&oid[..2])
            .join(&oid[2..])
    };
    fs::set_permissions(object(&original), fs::Permissions::from_mode(0o644)).unwrap();
    fs::copy(object(&other), object(&original)).unwrap();
    assert!(matches!(
        capture_git_tree(&request(dir.path()), &head),
        Err(CaptureError::Git { .. })
    ));
}

#[test]
fn index_intent_to_add_and_full_staging_have_different_snapshot_identities() {
    let dir = repository();
    fs::write(dir.path().join("empty.py"), b"").unwrap();
    git(dir.path(), &["add", "-N", "empty.py"]);
    let intent = capture(&request(dir.path())).unwrap();
    git(dir.path(), &["add", "empty.py"]);
    let staged = capture(&request(dir.path())).unwrap();
    assert_ne!(intent.snapshot.id(), staged.snapshot.id());
    assert_eq!(
        intent.effective_files().unwrap(),
        staged.effective_files().unwrap()
    );
}

#[test]
fn raw_unix_permissions_affect_live_capture_identity() {
    let dir = repository();
    let first = capture(&request(dir.path())).unwrap();
    fs::set_permissions(
        dir.path().join("source.py"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let second = capture(&request(dir.path())).unwrap();
    assert_ne!(first.snapshot.id(), second.snapshot.id());
    assert_eq!(
        first.effective_files().unwrap(),
        second.effective_files().unwrap()
    );
}

#[test]
fn capture_root_must_be_the_actual_workspace_root() {
    let dir = repository();
    fs::create_dir(dir.path().join("nested")).unwrap();
    assert!(matches!(
        capture(&request(&dir.path().join("nested"))),
        Err(CaptureError::InvalidPolicy(_))
    ));
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    assert!(matches!(
        capture_git_tree(&request(&dir.path().join("nested")), &head),
        Err(CaptureError::InvalidPolicy(_))
    ));
}

#[test]
fn nested_repository_metadata_is_explicitly_incomplete() {
    let dir = repository();
    fs::create_dir(dir.path().join("nested")).unwrap();
    git(&dir.path().join("nested"), &["init", "-q"]);
    fs::write(dir.path().join("nested/source.py"), b"nested=1\n").unwrap();
    let result = capture(&request(dir.path())).unwrap();
    assert!(
        result
            .effective_files()
            .unwrap()
            .contains_key("nested/source.py")
    );
    assert!(
        result
            .report
            .issues
            .contains(&CaptureIssue::NestedRepository("nested".into()))
    );
    assert!(!result.report.census_complete());
}

#[test]
fn unmerged_index_is_not_reported_as_complete() {
    let dir = repository();
    let main = git(dir.path(), &["branch", "--show-current"]);
    git(dir.path(), &["checkout", "-qb", "other"]);
    fs::write(dir.path().join("source.py"), b"other\n").unwrap();
    git(dir.path(), &["commit", "-qam", "other"]);
    git(dir.path(), &["checkout", "-q", &main]);
    fs::write(dir.path().join("source.py"), b"main\n").unwrap();
    git(dir.path(), &["commit", "-qam", "main"]);
    let merge = Command::new("git")
        .current_dir(dir.path())
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!merge.status.success());
    let result = capture(&request(dir.path())).unwrap();
    assert!(
        result
            .report
            .issues
            .contains(&CaptureIssue::UnmergedIndex("source.py".into()))
    );
    assert!(!result.report.census_complete());
}

#[test]
fn overlay_cannot_create_a_child_beneath_a_captured_symlink() {
    let dir = repository();
    symlink("/outside", dir.path().join("linked")).unwrap();
    let mut req = request(dir.path());
    req.overlays.insert(
        "linked/child.py".into(),
        Overlay {
            version: 1,
            bytes: Some(b"child=1\n".to_vec()),
            mode: FileMode::Regular,
        },
    );
    assert!(matches!(capture(&req), Err(CaptureError::InvalidPolicy(_))));
}

#[test]
fn replacing_a_directory_with_an_external_symlink_between_passes_rejects() {
    let dir = repository();
    fs::create_dir(dir.path().join("local")).unwrap();
    fs::write(dir.path().join("local/source.py"), b"local=1\n").unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.py"), b"must not be followed\n").unwrap();
    let result = capture_with_hook(&request(dir.path()), |_| {
        fs::remove_dir_all(dir.path().join("local")).unwrap();
        symlink(outside.path(), dir.path().join("local")).unwrap();
    });
    assert!(matches!(result, Err(CaptureError::ChangedDuringCapture)));
    let next = capture(&request(dir.path())).unwrap();
    assert_eq!(
        next.effective_files().unwrap()["local"].kind,
        InputKind::Symlink
    );
    assert!(
        !next
            .effective_files()
            .unwrap()
            .contains_key("local/secret.py")
    );
}

#[test]
fn capture_report_tampering_cannot_remove_uncertainty_or_change_bindings() {
    let dir = repository();
    let original = capture(&request(dir.path())).unwrap();
    original.validate().unwrap();
    let mut changed_policy = original.clone();
    changed_policy.report.policy.max_file_bytes += 1;
    assert!(matches!(
        changed_policy.validate(),
        Err(CaptureError::InvalidReport("policy contents"))
    ));
    let mut removed = original.clone();
    removed.report.issues.clear();
    assert!(removed.report.complete());
    assert!(matches!(
        removed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
    let mut changed = original.clone();
    changed.report.consistency = CaptureConsistency::GitTree;
    assert!(matches!(
        changed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
    let mut changed = original.clone();
    changed.report.content_digests.clear();
    assert!(matches!(
        changed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
    let mut changed = original.clone();
    changed.report.content_bytes += 1;
    assert!(matches!(
        changed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
    let mut changed = original.clone();
    changed.report.input_count += 1;
    assert!(matches!(
        changed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
    let mut changed = original.clone();
    changed.report.excluded_paths.clear();
    assert!(matches!(
        changed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
    let mut changed = original.clone();
    changed.report.policy_digest = celetixo_engine::ids::Digest::of("tampered", b"policy");
    assert!(matches!(
        changed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let mut changed = capture_git_tree(&request(dir.path()), &head).unwrap();
    changed.validate().unwrap();
    changed
        .report
        .issues
        .insert(CaptureIssue::MissingRequiredInput("fabricated".into()));
    assert!(matches!(
        changed.validate(),
        Err(CaptureError::InvalidReport(_))
    ));
}

#[test]
fn immutable_generated_documents_allow_explicit_editor_overlays() {
    let dir = repository();
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let mut req = request(dir.path());
    req.immutable_inputs.push(WorkspaceInput {
        path: "generated/a.py".into(),
        layer: InputLayer::Generated,
        kind: InputKind::File,
        mode: FileMode::Regular,
        bytes: Some(b"generated=1\n".to_vec()),
    });
    req.overlays.insert(
        "generated/a.py".into(),
        Overlay {
            version: 1,
            bytes: Some(b"edited=1\n".to_vec()),
            mode: FileMode::Regular,
        },
    );
    let result = capture_git_tree(&req, &head).unwrap();
    assert!(result.report.complete());
    assert_eq!(
        result.effective_files().unwrap()["generated/a.py"].bytes,
        b"edited=1\n"
    );
}

#[test]
fn empty_directories_and_directory_permissions_are_snapshot_inputs() {
    let dir = repository();
    let first = capture(&request(dir.path())).unwrap();
    fs::create_dir(dir.path().join("empty")).unwrap();
    let second = capture(&request(dir.path())).unwrap();
    assert_ne!(first.snapshot.id(), second.snapshot.id());
    assert_eq!(
        first.effective_files().unwrap(),
        second.effective_files().unwrap()
    );
    fs::set_permissions(dir.path().join("empty"), fs::Permissions::from_mode(0o700)).unwrap();
    let third = capture(&request(dir.path())).unwrap();
    assert_ne!(second.snapshot.id(), third.snapshot.id());
    assert!(
        third
            .snapshot
            .manifest()
            .environment
            .values()
            .any(|value| value == "40700:empty")
    );
}

#[test]
fn external_modes_and_observed_change_and_restore_are_bound() {
    let dir = repository();
    let external_dir = tempfile::tempdir().unwrap();
    let source = external_dir.path().join("schema.json");
    fs::write(&source, b"original").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
    let mut req = request(dir.path());
    req.external_inputs.push(ExternalInput {
        path: "schema.json".into(),
        source: source.clone(),
        layer: InputLayer::Mounted,
        required: true,
    });
    let first = capture(&req).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).unwrap();
    let changed_mode = capture(&req).unwrap();
    assert_ne!(first.snapshot.id(), changed_mode.snapshot.id());
    assert_eq!(
        first.effective_files().unwrap()["schema.json"].bytes,
        changed_mode.effective_files().unwrap()["schema.json"].bytes
    );
    let result = capture_with_hook(&req, |_| {
        fs::write(&source, b"modified").unwrap();
        fs::write(&source, b"original").unwrap();
    });
    assert!(matches!(result, Err(CaptureError::ChangedDuringCapture)));
    let recovered = capture(&req).unwrap();
    assert_eq!(changed_mode.snapshot.id(), recovered.snapshot.id());
}

#[test]
fn per_worktree_git_configuration_changes_snapshot_identity() {
    let dir = repository();
    git(dir.path(), &["config", "extensions.worktreeConfig", "true"]);
    git(
        dir.path(),
        &["config", "--worktree", "core.autocrlf", "false"],
    );
    let first = capture(&request(dir.path())).unwrap();
    git(
        dir.path(),
        &["config", "--worktree", "core.autocrlf", "true"],
    );
    let second = capture(&request(dir.path())).unwrap();
    assert_eq!(
        first.effective_files().unwrap(),
        second.effective_files().unwrap()
    );
    assert_ne!(first.snapshot.id(), second.snapshot.id());
    assert!(matches!(
        capture_with_hook(&request(dir.path()), |_| {
            git(
                dir.path(),
                &["config", "--worktree", "core.autocrlf", "input"],
            );
        }),
        Err(CaptureError::ChangedDuringCapture)
    ));
}

#[test]
fn git_attributes_inputs_are_preserved_and_missing_configuration_is_explicit() {
    let dir = repository();
    let attrs = dir.path().join(".git/info/attributes");
    fs::write(&attrs, b"*.py text eol=lf\n").unwrap();
    let first = capture(&request(dir.path())).unwrap();
    assert_eq!(
        first.effective_files().unwrap()[".git/info/attributes"].bytes,
        b"*.py text eol=lf\n"
    );
    fs::write(&attrs, b"*.py -text\n").unwrap();
    let second = capture(&request(dir.path())).unwrap();
    assert_ne!(first.snapshot.id(), second.snapshot.id());
    assert!(matches!(
        capture_with_hook(&request(dir.path()), |_| {
            fs::write(&attrs, b"*.py text\n").unwrap();
        }),
        Err(CaptureError::ChangedDuringCapture)
    ));
    let external = tempfile::tempdir().unwrap();
    let source = external.path().join("attributes");
    git(
        dir.path(),
        &["config", "core.attributesFile", source.to_str().unwrap()],
    );
    let missing = capture(&request(dir.path())).unwrap();
    assert!(
        missing
            .report
            .issues
            .contains(&CaptureIssue::MissingRequiredInput(
                ".git/configured-attributes".into()
            ))
    );
    assert!(!missing.report.census_complete());
    fs::write(&source, b"*.py diff=python\n").unwrap();
    let complete_census = capture(&request(dir.path())).unwrap();
    assert!(complete_census.report.census_complete());
    assert_eq!(
        complete_census.effective_files().unwrap()[".git/configured-attributes"].bytes,
        b"*.py diff=python\n"
    );
    assert_ne!(missing.snapshot.id(), complete_census.snapshot.id());
}

#[test]
fn credential_admission_rejects_all_content_layers_before_publication_and_recovers() {
    let dir = repository();
    let secret = format!("ghp_{}", "q".repeat(36)).into_bytes();
    let mut cases = Vec::new();
    let mut req = request(dir.path());
    req.overlays.insert(
        "buffer.py".into(),
        Overlay {
            version: 1,
            bytes: Some(secret.clone()),
            mode: FileMode::Regular,
        },
    );
    cases.push(req);
    for layer in [InputLayer::Generated, InputLayer::Mounted] {
        let mut req = request(dir.path());
        req.immutable_inputs.push(WorkspaceInput {
            path: "generated.py".into(),
            layer,
            kind: InputKind::File,
            mode: FileMode::Regular,
            bytes: Some(secret.clone()),
        });
        cases.push(req);
    }
    let outside = tempfile::tempdir().unwrap();
    let external = outside.path().join("data");
    fs::write(&external, &secret).unwrap();
    let mut req = request(dir.path());
    req.external_inputs.push(ExternalInput {
        path: "mounted.txt".into(),
        source: external,
        layer: InputLayer::Mounted,
        required: true,
    });
    cases.push(req);
    for req in cases {
        let mut second_pass_reached = false;
        let error = capture_with_hook(&req, |_| second_pass_reached = true).unwrap_err();
        assert!(matches!(error, CaptureError::SensitiveInput(_)));
        assert!(!second_pass_reached);
        assert!(
            !error
                .to_string()
                .contains(std::str::from_utf8(&secret).unwrap())
        );
    }
    fs::write(dir.path().join("data.txt"), &secret).unwrap();
    assert!(matches!(
        capture(&request(dir.path())),
        Err(CaptureError::SensitiveInput(_))
    ));
    git(dir.path(), &["add", "data.txt"]);
    fs::write(dir.path().join("data.txt"), b"public replacement").unwrap();
    assert!(matches!(
        capture(&request(dir.path())),
        Err(CaptureError::SensitiveInput(_))
    ));
    git(dir.path(), &["commit", "-qm", "sensitive staged fixture"]);
    let commit = git(dir.path(), &["rev-parse", "HEAD"]);
    assert!(matches!(
        capture_git_tree(&request(dir.path()), &commit),
        Err(CaptureError::SensitiveInput(_))
    ));
    assert!(matches!(
        capture(&request(dir.path())),
        Err(CaptureError::SensitiveInput(_))
    ));
    let mut excluded = request(dir.path());
    excluded.policy.excluded_paths.insert(
        "data.txt".into(),
        "credential-bearing fixture excluded by governance policy".into(),
    );
    let recovered = capture(&excluded).unwrap();
    assert!(
        recovered
            .report
            .issues
            .contains(&CaptureIssue::ExcludedInput("data.txt".into()))
    );
    assert!(
        !recovered
            .effective_files()
            .unwrap()
            .contains_key("data.txt")
    );
}

#[test]
fn credential_paths_require_exact_reviewed_public_bytes_and_policy_is_bound() {
    let dir = repository();
    let bytes = b"PUBLIC_API_BASE=https://example.invalid\n";
    fs::write(dir.path().join(".env"), bytes).unwrap();
    assert!(
        matches!(capture(&request(dir.path())), Err(CaptureError::SensitiveInput(path)) if path == ".env")
    );
    let mut approved = request(dir.path());
    let digest = celetixo_engine::ids::Digest::of("celetixo/source-content/v1", bytes);
    approved
        .policy
        .credentials
        .reviewed_public_inputs
        .insert(".env".into(), [digest].into());
    let first = capture(&approved).unwrap();
    assert_eq!(first.effective_files().unwrap()[".env"].bytes, bytes);
    fs::write(
        dir.path().join(".env"),
        b"PUBLIC_API_BASE=https://changed.invalid\n",
    )
    .unwrap();
    assert!(matches!(
        capture(&approved),
        Err(CaptureError::SensitiveInput(_))
    ));
    fs::write(dir.path().join(".env"), bytes).unwrap();
    approved
        .policy
        .credentials
        .reviewed_public_inputs
        .insert("other.example".into(), [digest].into());
    let changed_policy = capture(&approved).unwrap();
    assert_ne!(
        first.report.policy_digest,
        changed_policy.report.policy_digest
    );
    assert_ne!(first.snapshot.id(), changed_policy.snapshot.id());
    approved.policy.credentials.version = 2;
    assert!(matches!(
        capture(&approved),
        Err(CaptureError::InvalidPolicy(_))
    ));
    fs::remove_file(dir.path().join(".env")).unwrap();
    fs::write(dir.path().join(".env.example"), bytes).unwrap();
    assert!(capture(&request(dir.path())).is_ok());
}

#[test]
fn private_keys_and_environment_credentials_are_not_snapshot_inputs() {
    let dir = repository();
    let private_key = format!(
        "-----BEGIN {}-----\n{}\n-----END {}-----\n",
        "PRIVATE KEY",
        "A".repeat(64),
        "PRIVATE KEY"
    );
    fs::write(dir.path().join("unexpected.data"), &private_key).unwrap();
    assert!(matches!(
        capture(&request(dir.path())),
        Err(CaptureError::SensitiveInput(_))
    ));
    fs::remove_file(dir.path().join("unexpected.data")).unwrap();
    let provider_key = format!("AKIA{}", "0".repeat(16));
    let database_url = ["postgres://user:", "realpassword", "@localhost/database"].concat();
    for (key, value) in [
        ("PASSWORD", "short"),
        ("GITHUB_TOKEN", "opaque-credential-value"),
        ("BUILD_OPTION", provider_key.as_str()),
        ("DATABASE_URL", database_url.as_str()),
    ] {
        let mut req = request(dir.path());
        req.environment.insert(key.into(), value.into());
        let error = capture(&req).unwrap_err();
        assert!(matches!(error, CaptureError::SensitiveInput(_)), "{key}");
        assert!(!error.to_string().contains(value));
        let commit = git(dir.path(), &["rev-parse", "HEAD"]);
        assert!(matches!(
            capture_git_tree(&req, &commit),
            Err(CaptureError::SensitiveInput(_))
        ));
    }
    let mut req = request(dir.path());
    req.environment.insert("BUILD_MODE".into(), "test".into());
    req.environment.insert("GITHUB_TOKEN".into(), "credential-ref://capture-test-org/capture-test-workspace/capture-test-repository/read-only-github".into());
    assert!(capture(&req).is_ok());
    req.environment.insert("GITHUB_TOKEN".into(), "credential-ref://other-org/capture-test-workspace/capture-test-repository/read-only-github".into());
    assert!(matches!(
        capture(&req),
        Err(CaptureError::SensitiveInput(_))
    ));
    req.environment.remove("GITHUB_TOKEN");
    req.environment.insert("BUILD_OPTION".into(), "credential-ref://other-org/capture-test-workspace/capture-test-repository/read-only-github".into());
    assert!(matches!(
        capture(&req),
        Err(CaptureError::SensitiveInput(_))
    ));
    req.environment.insert(
        "BUILD_OPTION".into(),
        "credential-ref://capture-test-org/capture-test-workspace/capture-test-repository/.."
            .into(),
    );
    assert!(matches!(
        capture(&req),
        Err(CaptureError::SensitiveInput(_))
    ));
}

#[test]
fn public_source_literals_and_reviewed_grant_assignments_require_exact_admission() {
    let dir = repository();
    for (path, bytes) in [
        (
            "detector.rs",
            include_bytes!("../src/capture.rs").as_slice(),
        ),
        ("capture_tests.rs", include_bytes!("capture.rs").as_slice()),
        (
            "language_service.rs",
            include_bytes!("../src/lsp.rs").as_slice(),
        ),
    ] {
        fs::write(dir.path().join(path), bytes).unwrap();
    }
    capture(&request(dir.path())).unwrap();
    // These public Rust assignments now contain credential-shaped numeric grant
    // selectors. Keep the conservative default rejection and an explicit frozen
    // review; do not discover/approve arbitrary source bytes in the test.
    for (path, bytes) in [
        (
            "engine.rs",
            include_bytes!("../../engine/src/engine.rs").as_slice(),
        ),
        (
            "authority_records.rs",
            include_bytes!("../../engine/src/engine/authority/records.rs").as_slice(),
        ),
    ] {
        fs::write(dir.path().join(path), bytes).unwrap();
    }
    assert!(matches!(
        capture(&request(dir.path())),
        Err(CaptureError::SensitiveInput(_))
    ));
    let mut approved = request(dir.path());
    for argument in [
        "engine.rs=9f896e748937467d9713ca5a9c323e34f8e9b413de1aa8fd3b4a3a1864379811",
        "authority_records.rs=99330c01b6fe6fabe1834ac343d8c9ae76c7685d3750eab34ced9118235eb98f",
    ] {
        celetixo_repository::reviewed_inputs::add_exact_review(
            &mut approved.policy.credentials,
            argument,
        )
        .unwrap();
    }
    capture(&approved).unwrap();
    for path in ["engine.rs", "authority_records.rs"] {
        let original = fs::read(dir.path().join(path)).unwrap();
        let mut changed = original.clone();
        changed.extend_from_slice(b"\n// changed public bytes\n");
        fs::write(dir.path().join(path), changed).unwrap();
        assert!(matches!(
            capture(&approved),
            Err(CaptureError::SensitiveInput(_))
        ));
        fs::write(dir.path().join(path), original).unwrap();
    }
}

#[test]
fn replaced_and_staged_deleted_gitlinks_do_not_hide_current_file_bytes() {
    let dir = repository();
    let commit = git(dir.path(), &["rev-parse", "HEAD"]);
    git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &commit,
            "vendor",
        ],
    );
    git(dir.path(), &["commit", "-qm", "gitlink fixture"]);
    fs::write(dir.path().join("vendor"), b"replacement one").unwrap();
    let first = capture(&request(dir.path())).unwrap();
    assert_eq!(
        first.effective_files().unwrap()["vendor"].bytes,
        b"replacement one"
    );
    assert_eq!(
        first.effective_files().unwrap()["vendor"].kind,
        InputKind::File
    );
    assert!(
        first
            .report
            .issues
            .contains(&CaptureIssue::UninitializedSubmodule("vendor".into()))
    );
    fs::write(dir.path().join("vendor"), b"replacement two").unwrap();
    let second = capture(&request(dir.path())).unwrap();
    assert_ne!(first.snapshot.id(), second.snapshot.id());
    git(dir.path(), &["rm", "--cached", "-q", "vendor"]);
    let unstaged_replacement = capture(&request(dir.path())).unwrap();
    assert_eq!(
        input(&unstaged_replacement, "vendor", InputLayer::Index).bytes,
        None
    );
    assert_eq!(
        input(&unstaged_replacement, "vendor", InputLayer::Untracked)
            .bytes
            .as_deref(),
        Some(b"replacement two".as_slice())
    );
    assert!(unstaged_replacement.report.census_complete());
    fs::remove_file(dir.path().join("vendor")).unwrap();
    let removed = capture(&request(dir.path())).unwrap();
    assert!(!removed.effective_files().unwrap().contains_key("vendor"));
    assert!(removed.report.census_complete());
}
