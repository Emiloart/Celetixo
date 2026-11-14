#![cfg(target_os = "linux")]

use celetixo_engine::ids::{Digest, OrganizationId, RepositoryId, Scope, WorkspaceId};
use celetixo_engine::source_evidence::{
    SOURCE_ARTIFACT_DOMAIN, SourceArtifactError, SourceArtifactReader,
};
use celetixo_execution::artifacts::{
    ArtifactStoreEvent, ArtifactStoreInitializationEvent, ArtifactStoreLimits,
    LocalSourceArtifactReader, LocalSourceArtifactStore,
};
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::Arc;

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("org").unwrap(),
        workspace: WorkspaceId::new("ws").unwrap(),
        repository: RepositoryId::new("repo").unwrap(),
    }
}
fn retention() -> Digest {
    Digest::of("test/retention", b"policy v1")
}
fn store() -> LocalSourceArtifactStore {
    LocalSourceArtifactStore::temporary(scope(), retention(), ArtifactStoreLimits::default())
        .unwrap()
}
fn object_path(store: &LocalSourceArtifactStore, address: Digest) -> PathBuf {
    store.root_path().join(format!("{address}.artifact"))
}

#[test]
fn immutable_binary_and_empty_objects_roundtrip_through_registered_reader_contract() {
    let store = Arc::new(store());
    for bytes in [b"".as_slice(), &[0, 255, 10, 13, 99], b"diagnostic\n"] {
        let reference = store
            .put_bytes("stdout", "application/octet-stream", bytes)
            .unwrap();
        assert_eq!(reference.scope, scope());
        assert_eq!(reference.bytes, bytes.len() as u64);
        assert_eq!(reference.retention_policy, retention());
        assert_eq!(reference.content, Digest::of(SOURCE_ARTIFACT_DOMAIN, bytes));
        let reader: Arc<dyn SourceArtifactReader> = store.clone();
        assert_eq!(
            reader
                .read(&scope(), reference.content, bytes.len() as u64)
                .unwrap(),
            bytes
        );
        assert_eq!(reader.retention_policy(), retention());
        assert_eq!(
            fs::metadata(object_path(&store, reference.content))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o444
        );
    }
}

#[test]
fn persistent_reopen_and_exact_duplicate_put_preserve_objects_and_scope_retention_contract() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("objects");
    let store = LocalSourceArtifactStore::create(
        &path,
        scope(),
        retention(),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    let reference = store
        .put_bytes(
            "execution-record",
            "application/json",
            b"{\"outcome\":\"failed\"}",
        )
        .unwrap();
    let count = fs::read_dir(&path).unwrap().count();
    assert_eq!(
        store
            .put_bytes(
                "execution-record",
                "application/json",
                b"{\"outcome\":\"failed\"}"
            )
            .unwrap(),
        reference
    );
    assert_eq!(fs::read_dir(&path).unwrap().count(), count);
    assert!(
        LocalSourceArtifactStore::open(&path, scope(), retention(), ArtifactStoreLimits::default())
            .is_err(),
        "an independent adapter must not bypass the local exclusive lock"
    );
    drop(store);
    let reopened =
        LocalSourceArtifactStore::open(&path, scope(), retention(), ArtifactStoreLimits::default())
            .unwrap();
    assert_eq!(
        reopened.read(&scope(), reference.content, 4096).unwrap(),
        b"{\"outcome\":\"failed\"}"
    );
    drop(reopened);
    let mut wrong = scope();
    wrong.workspace = WorkspaceId::new("other").unwrap();
    assert!(
        LocalSourceArtifactStore::open(&path, wrong, retention(), ArtifactStoreLimits::default())
            .is_err()
    );
    assert!(
        LocalSourceArtifactStore::open(
            &path,
            scope(),
            Digest::of("other", b"policy"),
            ArtifactStoreLimits::default()
        )
        .is_err()
    );
    assert!(
        LocalSourceArtifactStore::create(
            &path,
            scope(),
            retention(),
            ArtifactStoreLimits::default()
        )
        .is_err(),
        "create cannot adopt existing directory"
    );
}

#[test]
fn independent_readers_verify_new_objects_without_acquiring_or_releasing_writer_ownership() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("objects");
    let limits = ArtifactStoreLimits::default();
    let writer = LocalSourceArtifactStore::create(&path, scope(), retention(), limits).unwrap();
    let first = writer.put_bytes("first", "text/plain", b"first").unwrap();
    let readers = [
        LocalSourceArtifactReader::open(&path, scope(), retention(), limits).unwrap(),
        LocalSourceArtifactReader::open(&path, scope(), retention(), limits).unwrap(),
    ];
    let second = writer.put_bytes("second", "text/plain", b"second").unwrap();
    for reader in &readers {
        assert_eq!(reader.retention_policy(), retention());
        assert_eq!(reader.read(&scope(), first.content, 5).unwrap(), b"first");
        assert_eq!(reader.read(&scope(), second.content, 6).unwrap(), b"second");
        assert_eq!(
            reader.read(&scope(), second.content, 5),
            Err(SourceArtifactError::Limit)
        );
        let mut foreign = scope();
        foreign.repository = RepositoryId::new("foreign").unwrap();
        assert_eq!(
            reader.read(&foreign, first.content, 5),
            Err(SourceArtifactError::Scope)
        );
    }
    drop(readers);
    assert!(LocalSourceArtifactStore::open(&path, scope(), retention(), limits).is_err());
    let reader = LocalSourceArtifactReader::open(&path, scope(), retention(), limits).unwrap();
    drop(writer);
    let reopened_writer =
        LocalSourceArtifactStore::open(&path, scope(), retention(), limits).unwrap();
    let third = reopened_writer
        .put_bytes("third", "text/plain", b"third")
        .unwrap();
    assert_eq!(reader.read(&scope(), third.content, 5).unwrap(), b"third");
}

#[test]
fn independent_reader_checks_exact_header_limits_and_rejects_object_or_root_substitution() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("objects");
    let limits = ArtifactStoreLimits::default();
    let writer = LocalSourceArtifactStore::create(&path, scope(), retention(), limits).unwrap();
    let reference = writer
        .put_bytes("evidence", "text/plain", b"evidence")
        .unwrap();
    let mut foreign = scope();
    foreign.workspace = WorkspaceId::new("foreign").unwrap();
    assert!(LocalSourceArtifactReader::open(&path, foreign, retention(), limits).is_err());
    assert!(
        LocalSourceArtifactReader::open(&path, scope(), Digest::of("other", b"retention"), limits)
            .is_err()
    );
    assert!(
        LocalSourceArtifactReader::open(
            &path,
            scope(),
            retention(),
            ArtifactStoreLimits {
                max_objects: limits.max_objects + 1,
                ..limits
            }
        )
        .is_err()
    );
    let reader = LocalSourceArtifactReader::open(&path, scope(), retention(), limits).unwrap();
    let object = object_path(&writer, reference.content);
    fs::set_permissions(&object, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&object, b"tampered").unwrap();
    fs::set_permissions(&object, fs::Permissions::from_mode(0o444)).unwrap();
    assert_eq!(
        reader.read(&scope(), reference.content, 64),
        Err(SourceArtifactError::Integrity)
    );
    assert!(LocalSourceArtifactReader::open(&path, scope(), retention(), limits).is_err());
    let retained = parent.path().join("retained");
    fs::rename(&path, &retained).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(
        reader.read(&scope(), reference.content, 64),
        Err(SourceArtifactError::Integrity)
    );
    assert!(
        retained
            .join(format!("{}.artifact", reference.content))
            .exists()
    );
}

#[test]
fn interrupted_creation_is_not_ready_until_recovery_confirms_header_root_and_parent() {
    let phases = [
        ArtifactStoreInitializationEvent::BeforeHeaderSync,
        ArtifactStoreInitializationEvent::BeforeRootSync,
        ArtifactStoreInitializationEvent::BeforeParentSync,
    ];
    for interrupted_at in phases {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("objects");
        let creation = LocalSourceArtifactStore::create_with_hook(
            &path,
            scope(),
            retention(),
            ArtifactStoreLimits::default(),
            |phase| {
                if phase == interrupted_at {
                    Err(io::Error::other("interrupted store initialization"))
                } else {
                    Ok(())
                }
            },
        );
        assert!(creation.is_err());
        assert!(path.join("store.meta").is_file());

        // Readable metadata from an interrupted constructor is not itself a
        // persistence confirmation. Every recovery sync must still succeed.
        for interrupted_recovery in phases {
            let mut observed = Vec::new();
            let recovery = LocalSourceArtifactStore::open_with_hook(
                &path,
                scope(),
                retention(),
                ArtifactStoreLimits::default(),
                |phase| {
                    observed.push(phase);
                    if phase == interrupted_recovery {
                        Err(io::Error::other("interrupted store recovery"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(recovery.is_err());
            assert_eq!(observed.last(), Some(&interrupted_recovery));
        }

        let mut observed = Vec::new();
        let store = LocalSourceArtifactStore::open_with_hook(
            &path,
            scope(),
            retention(),
            ArtifactStoreLimits::default(),
            |phase| {
                observed.push(phase);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(observed, phases);
        let reference = store
            .put_bytes("stdout", "text/plain", b"recovered")
            .unwrap();
        drop(store);
        let reopened = LocalSourceArtifactStore::open(
            &path,
            scope(),
            retention(),
            ArtifactStoreLimits::default(),
        )
        .unwrap();
        assert_eq!(
            reopened.read(&scope(), reference.content, 4096).unwrap(),
            b"recovered"
        );
    }
}

#[test]
fn recovery_synchronizes_only_the_exact_validated_store_contract_and_header() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("objects");
    drop(
        LocalSourceArtifactStore::create(
            &path,
            scope(),
            retention(),
            ArtifactStoreLimits::default(),
        )
        .unwrap(),
    );
    let mut wrong_scope = scope();
    wrong_scope.workspace = WorkspaceId::new("other").unwrap();
    let mut synced = false;
    assert!(
        LocalSourceArtifactStore::open_with_hook(
            &path,
            wrong_scope,
            retention(),
            ArtifactStoreLimits::default(),
            |_| {
                synced = true;
                Ok(())
            },
        )
        .is_err()
    );
    assert!(
        !synced,
        "an unvalidated scope cannot reach persistence confirmation"
    );

    let header = path.join("store.meta");
    let header_bytes = fs::read(&header).unwrap();
    let recovery = LocalSourceArtifactStore::open_with_hook(
        &path,
        scope(),
        retention(),
        ArtifactStoreLimits::default(),
        |phase| {
            if phase == ArtifactStoreInitializationEvent::BeforeHeaderSync {
                fs::rename(&header, path.join("previous.meta"))?;
                fs::write(&header, &header_bytes)?;
                fs::set_permissions(&header, fs::Permissions::from_mode(0o444))?;
            }
            Ok(())
        },
    );
    assert!(matches!(recovery, Err(SourceArtifactError::Integrity)));
}

#[test]
fn cross_scope_reads_and_missing_or_oversized_objects_fail_explicitly() {
    let store = store();
    let reference = store.put_bytes("log", "text/plain", b"12345").unwrap();
    let mut wrong = scope();
    wrong.organization = OrganizationId::new("other").unwrap();
    assert_eq!(
        store.read(&wrong, reference.content, 100).unwrap_err(),
        SourceArtifactError::Scope
    );
    assert_eq!(
        store.read(&scope(), reference.content, 4).unwrap_err(),
        SourceArtifactError::Limit
    );
    assert!(matches!(
        store.read(&scope(), Digest::of("missing", b"object"), 100),
        Err(SourceArtifactError::Unavailable(_))
    ));
    assert!(store.put_bytes("", "text/plain", b"x").is_err());
    assert!(store.put_bytes("stdout", "bad\nmedia", b"x").is_err());
    // Logical artifact labels are never joined into filesystem paths.
    let labelled = store
        .put_bytes("../../logical-label", "text/plain", b"same address")
        .unwrap();
    assert_eq!(
        store.read(&scope(), labelled.content, 100).unwrap(),
        b"same address"
    );
}

#[test]
fn object_byte_inode_and_aggregate_limits_are_enforced_before_new_publication() {
    let limits = ArtifactStoreLimits {
        max_object_bytes: 4,
        max_total_bytes: 6,
        max_objects: 4,
    };
    let store = LocalSourceArtifactStore::temporary(scope(), retention(), limits).unwrap();
    assert_eq!(
        store
            .put_bytes("too big", "text/plain", b"12345")
            .unwrap_err(),
        SourceArtifactError::Limit
    );
    let first = store.put_bytes("first", "text/plain", b"aaaa").unwrap();
    store.put_bytes("second", "text/plain", b"bb").unwrap();
    assert_eq!(
        store
            .put_bytes("aggregate", "text/plain", b"c")
            .unwrap_err(),
        SourceArtifactError::Limit
    );
    assert_eq!(
        store.put_bytes("first", "text/plain", b"aaaa").unwrap(),
        first
    );
    let limits = ArtifactStoreLimits {
        max_object_bytes: 8,
        max_total_bytes: 32,
        max_objects: 2,
    };
    let store = LocalSourceArtifactStore::temporary(scope(), retention(), limits).unwrap();
    store.put_bytes("one", "text/plain", b"a").unwrap();
    store.put_bytes("two", "text/plain", b"b").unwrap();
    assert_eq!(
        store.put_bytes("three", "text/plain", b"c").unwrap_err(),
        SourceArtifactError::Limit
    );
}

#[test]
fn content_modes_symlink_substitution_and_hardlink_tampering_never_yield_verified_bytes() {
    for mutation in 0..4 {
        let store = store();
        let reference = store
            .put_bytes("stdout", "text/plain", b"original")
            .unwrap();
        let path = object_path(&store, reference.content);
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("sentinel"), b"keep").unwrap();
        match mutation {
            0 => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&path, b"tampered").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            }
            1 => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            2 => {
                fs::remove_file(&path).unwrap();
                symlink(outside.path().join("sentinel"), &path).unwrap();
            }
            _ => fs::hard_link(&path, outside.path().join("alias")).unwrap(),
        }
        assert!(
            store.read(&scope(), reference.content, 100).is_err(),
            "accepted tamper {mutation}"
        );
        assert!(
            store.put_bytes("retry", "text/plain", b"original").is_err(),
            "retry cannot silently repair/overwrite a corrupt object"
        );
        assert_eq!(fs::read(outside.path().join("sentinel")).unwrap(), b"keep");
    }
}

#[test]
fn root_symlink_replacement_and_scope_header_tampering_fail_without_touching_replacement() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("objects");
    let store = LocalSourceArtifactStore::create(
        &root,
        scope(),
        retention(),
        ArtifactStoreLimits::default(),
    )
    .unwrap();
    let reference = store.put_bytes("log", "text/plain", b"data").unwrap();
    let moved = outer.path().join("moved");
    fs::rename(&root, &moved).unwrap();
    let replacement = tempfile::tempdir().unwrap();
    fs::write(replacement.path().join("sentinel"), b"keep").unwrap();
    symlink(replacement.path(), &root).unwrap();
    assert!(store.read(&scope(), reference.content, 100).is_err());
    assert!(store.put_bytes("other", "text/plain", b"other").is_err());
    drop(store);
    assert_eq!(
        fs::read(replacement.path().join("sentinel")).unwrap(),
        b"keep"
    );
    assert!(
        LocalSourceArtifactStore::open(&root, scope(), retention(), ArtifactStoreLimits::default())
            .is_err()
    );
    fs::remove_file(&root).unwrap();
    fs::rename(&moved, &root).unwrap();
    let metadata = root.join("store.meta");
    fs::set_permissions(&metadata, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&metadata, b"forged metadata").unwrap();
    fs::set_permissions(&metadata, fs::Permissions::from_mode(0o444)).unwrap();
    assert!(
        LocalSourceArtifactStore::open(&root, scope(), retention(), ArtifactStoreLimits::default())
            .is_err()
    );
}

#[test]
fn persistence_faults_never_return_a_reference_and_published_objects_survive_for_recovery() {
    for event in [
        ArtifactStoreEvent::BeforeWrite,
        ArtifactStoreEvent::BeforeFileSync,
        ArtifactStoreEvent::BeforePublish,
        ArtifactStoreEvent::BeforeDirectorySync,
        ArtifactStoreEvent::BeforeReadback,
    ] {
        let store = store();
        let error = store
            .put_bytes_with_hook("stdout", "text/plain", b"bounded failed output", |point| {
                if point == event {
                    Err(io::Error::other("injected storage persistence failure"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert!(matches!(error, SourceArtifactError::Persistence(_)));
        let names: Vec<_> = fs::read_dir(store.root_path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            !names
                .iter()
                .any(|name| name.to_string_lossy().starts_with(".pending-"))
        );
        let retry = store
            .put_bytes("stdout", "text/plain", b"bounded failed output")
            .unwrap();
        assert_eq!(
            store.read(&scope(), retry.content, 100).unwrap(),
            b"bounded failed output"
        );
        assert_eq!(fs::read_dir(store.root_path()).unwrap().count(), 2);
    }
}

#[test]
fn published_readback_tampering_is_rejected_and_preserved_instead_of_overwritten() {
    let store = store();
    let expected = Digest::of(SOURCE_ARTIFACT_DOMAIN, b"expected");
    let path = object_path(&store, expected);
    let error = store
        .put_bytes_with_hook("record", "text/plain", b"expected", |point| {
            if point == ArtifactStoreEvent::BeforeReadback {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
                fs::write(&path, b"tampered")?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o444))?;
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error, SourceArtifactError::Integrity);
    assert_eq!(fs::read(&path).unwrap(), b"tampered");
    assert_eq!(
        store
            .put_bytes("record", "text/plain", b"expected")
            .unwrap_err(),
        SourceArtifactError::Integrity
    );
}

#[test]
fn explicit_pending_reconciliation_preserves_published_objects_and_unknown_files() {
    let store = store();
    let reference = store.put_bytes("log", "text/plain", b"kept").unwrap();
    let pending = store.root_path().join(".pending-00000000000000ff");
    fs::write(&pending, b"crash fragment").unwrap();
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600)).unwrap();
    let report = store.reconcile_pending().unwrap();
    assert_eq!(report.removed_pending_objects, 1);
    assert_eq!(report.removed_bytes, 14);
    assert!(!pending.exists());
    assert_eq!(
        store.read(&scope(), reference.content, 100).unwrap(),
        b"kept"
    );
    let unknown = store.root_path().join(".pending-user-data");
    fs::write(&unknown, b"must not delete").unwrap();
    assert_eq!(
        store.reconcile_pending().unwrap_err(),
        SourceArtifactError::Integrity
    );
    assert_eq!(fs::read(unknown).unwrap(), b"must not delete");
}

#[test]
fn reopened_store_accounts_for_pending_inodes_and_recovers_lost_publication_response() {
    let outer = tempfile::tempdir().unwrap();
    let root = outer.path().join("objects");
    let limits = ArtifactStoreLimits {
        max_object_bytes: 16,
        max_total_bytes: 32,
        max_objects: 2,
    };
    let store = LocalSourceArtifactStore::create(&root, scope(), retention(), limits).unwrap();
    assert!(
        store
            .put_bytes_with_hook("evidence", "text/plain", b"sealed", |event| {
                if event == ArtifactStoreEvent::BeforeDirectorySync {
                    Err(io::Error::other("lost persistence response"))
                } else {
                    Ok(())
                }
            })
            .is_err()
    );
    fs::write(root.join(".pending-0000000000000001"), b"crash").unwrap();
    fs::set_permissions(
        root.join(".pending-0000000000000001"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    drop(store);
    let reopened = LocalSourceArtifactStore::open(&root, scope(), retention(), limits).unwrap();
    assert_eq!(
        reopened.put_bytes("new", "text/plain", b"new").unwrap_err(),
        SourceArtifactError::Limit
    );
    assert_eq!(
        reopened
            .reconcile_pending()
            .unwrap()
            .removed_pending_objects,
        1
    );
    let recovered = reopened
        .put_bytes("evidence", "text/plain", b"sealed")
        .unwrap();
    assert_eq!(
        reopened.read(&scope(), recovered.content, 100).unwrap(),
        b"sealed"
    );
    reopened.put_bytes("new", "text/plain", b"new").unwrap();
}
