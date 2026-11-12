//! Deterministic private materialization from immutable, exact-base source bytes.
//!
//! Linux directory-relative no-follow operations never read a live repository or
//! execute its programs. This component preserves source modes; a validated
//! executor must separately enforce a read-only source mount and other isolation.
//! Private directories exclude other users, not an adversary with the embedding
//! process's UID/privileges. Observational checks do not prove absence of ABA edits.

use celetixo_engine::ids::{Digest, SnapshotId};
use celetixo_engine::snapshot::Snapshot;
use celetixo_engine::source_patch::{AppliedSourcePatch, SourcePatch, SourcePatchError};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupOutcome {
    Removed,
    /// Identity no longer matches, or deletion failed. Path is a reconciliation
    /// hint, never deletion authority. Never follow a replacement symlink or
    /// recursively remove an unrelated replacement directory.
    Quarantined {
        path: PathBuf,
        reason: String,
    },
}

#[derive(Debug, Error)]
pub enum MaterializationError {
    #[error("private no-follow materialization is supported only on Linux")]
    UnsupportedPlatform,
    #[error(transparent)]
    Patch(#[from] SourcePatchError),
    #[error("opaque Gitlink cannot be reproduced as an execution file: {0:?}")]
    UnsupportedGitlink(String),
    #[error("captured directory/Unix metadata is not implemented by this materializer: {0:?}")]
    UnsupportedCapturedMetadata(String),
    #[error("materialization budget exceeded: {0}")]
    Limit(&'static str),
    #[error("private materialization I/O failed at {path:?}: {source}")]
    Io { path: String, source: io::Error },
    #[error("private materialization changed or was replaced: {0:?}")]
    Changed(String),
    #[error("materialized source differs at {path:?}: {reason}")]
    Mismatch { path: String, reason: &'static str },
    #[error("materialization failed: {cause}; cleanup: {cleanup:?}")]
    Failed {
        cause: Box<MaterializationError>,
        cleanup: CleanupOutcome,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationReport {
    snapshot: SnapshotId,
    manifest: Digest,
    application: Digest,
    files: usize,
    symlinks: usize,
    directories: usize,
    content_bytes: u64,
}

impl MaterializationReport {
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn manifest(&self) -> Digest {
        self.manifest
    }
    pub fn application(&self) -> Digest {
        self.application
    }
    pub fn files(&self) -> usize {
        self.files
    }
    pub fn symlinks(&self) -> usize {
        self.symlinks
    }
    pub fn directories(&self) -> usize {
        self.directories
    }
    pub fn content_bytes(&self) -> u64 {
        self.content_bytes
    }

    /// Stable receipt identity, excluding random paths, inode numbers and timings.
    pub fn digest(&self) -> Digest {
        let mut bytes = Vec::new();
        for digest in [self.snapshot.0, self.manifest, self.application] {
            bytes.extend_from_slice(digest.as_bytes());
        }
        for count in [
            self.files as u64,
            self.symlinks as u64,
            self.directories as u64,
            self.content_bytes,
        ] {
            bytes.extend_from_slice(&count.to_be_bytes());
        }
        Digest::of("celetixo/private-materialization/v1", &bytes)
    }
}

#[derive(Debug, Clone)]
pub enum MaterializationEvent {
    /// The registry owns this empty allocation but has not published its record.
    RegistryRootCreated(PathBuf),
    BeforeOwnershipPublication(PathBuf),
    PrivateRootCreated(PathBuf),
    BeforeEntry {
        source: PathBuf,
        logical_path: String,
    },
    BeforeValidation(PathBuf),
}

#[cfg(target_os = "linux")]
pub use linux::PrivateWorkspace;

#[cfg(not(target_os = "linux"))]
pub struct PrivateWorkspace;

#[cfg(not(target_os = "linux"))]
impl PrivateWorkspace {
    pub fn materialize(_: &Snapshot, _: &SourcePatch) -> Result<Self, MaterializationError> {
        Err(MaterializationError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use celetixo_engine::snapshot::{FileMode, InputKind, WorkspaceInput};
    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs::{File, Metadata};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    use tempfile::TempDir;

    pub struct PrivateWorkspace {
        root: Option<TempDir>,
        root_fd: File,
        source_fd: File,
        source_path: PathBuf,
        applied: AppliedSourcePatch,
        report: MaterializationReport,
        directories: BTreeSet<String>,
        quarantined: Option<CleanupOutcome>,
        execution_retention: Option<String>,
        registered: Option<crate::workspaces::WorkspaceLease>,
    }

    impl PrivateWorkspace {
        pub fn materialize(
            base: &Snapshot,
            patch: &SourcePatch,
        ) -> Result<Self, MaterializationError> {
            Self::materialize_with_hook(base, patch, |_| Ok(()))
        }

        /// Fault-injection callback supplied by a trusted embedding/test harness,
        /// never selected or executed from repository content.
        pub fn materialize_with_hook(
            base: &Snapshot,
            patch: &SourcePatch,
            hook: impl FnMut(MaterializationEvent) -> io::Result<()>,
        ) -> Result<Self, MaterializationError> {
            Self::materialize_root(base, patch, None, hook)
        }

        pub(crate) fn materialize_registered(
            base: &Snapshot,
            patch: &SourcePatch,
            allocation: crate::workspaces::RegisteredRoot,
            hook: impl FnMut(MaterializationEvent) -> io::Result<()>,
        ) -> Result<Self, MaterializationError> {
            Self::materialize_root(base, patch, Some(allocation), hook)
        }

        fn materialize_root(
            base: &Snapshot,
            patch: &SourcePatch,
            allocation: Option<crate::workspaces::RegisteredRoot>,
            mut hook: impl FnMut(MaterializationEvent) -> io::Result<()>,
        ) -> Result<Self, MaterializationError> {
            let applied = patch.apply(base)?;
            applied.validate(base, patch)?;
            for key in applied.report().base_manifest().environment.keys() {
                if key.starts_with("celetixo.capture.directory.")
                    || key.starts_with("celetixo.capture.unix-mode.")
                    || key.starts_with(
                        "celetixo.source_application.origin.celetixo.capture.directory.",
                    )
                    || key.starts_with(
                        "celetixo.source_application.origin.celetixo.capture.unix-mode.",
                    )
                {
                    return Err(MaterializationError::UnsupportedCapturedMetadata(
                        key.clone(),
                    ));
                }
            }
            let mut directories = BTreeSet::from([String::new()]);
            let mut files = 0_usize;
            let mut symlinks = 0_usize;
            let mut bytes = 0_u64;
            let inputs = &applied.snapshot().manifest().inputs;
            for input in inputs {
                match input.kind {
                    InputKind::Submodule => {
                        return Err(MaterializationError::UnsupportedGitlink(input.path.clone()));
                    }
                    InputKind::File => files += 1,
                    InputKind::Symlink => symlinks += 1,
                }
                bytes = bytes
                    .checked_add(
                        input
                            .bytes
                            .as_ref()
                            .ok_or_else(|| mismatch(&input.path, "tombstone in normalized source"))?
                            .len() as u64,
                    )
                    .ok_or(MaterializationError::Limit("content byte overflow"))?;
                for (index, _) in input.path.match_indices('/') {
                    directories.insert(input.path[..index].to_owned());
                    if directories.len().saturating_add(inputs.len())
                        > patch.policy().max_result_files
                    {
                        return Err(MaterializationError::Limit(
                            "files and inferred directories",
                        ));
                    }
                }
            }
            if directories.len().saturating_add(inputs.len()) > patch.policy().max_result_files
                || bytes > patch.policy().max_result_bytes
            {
                return Err(MaterializationError::Limit("disk input census"));
            }
            let report = MaterializationReport {
                snapshot: applied.snapshot().id(),
                manifest: applied.snapshot().manifest().digest(),
                application: applied.report().digest(),
                files,
                symlinks,
                directories: directories.len(),
                content_bytes: bytes,
            };
            let (root, registered) = match allocation {
                Some(allocation) => (allocation.root, Some(allocation.lease)),
                None => (
                    tempfile::Builder::new()
                        .prefix("celetixo-source-")
                        .tempdir()
                        .map_err(|source| io_at("<private-parent>", source))?,
                    None,
                ),
            };
            let root_fd = open_absolute_directory(root.path())?;
            rustix::fs::fchmod(&root_fd, Mode::from_raw_mode(0o700))
                .map_err(|e| io_at("<private-parent>", e))?;
            rustix::fs::mkdirat(&root_fd, "source", Mode::from_raw_mode(0o700))
                .map_err(|e| io_at("<source-root>", e))?;
            let source_fd = open_directory(&root_fd, "source")?;
            let source_path = root.path().join("source");
            let mut workspace = Self {
                root: Some(root),
                root_fd,
                source_fd,
                source_path,
                applied,
                report,
                directories,
                quarantined: None,
                execution_retention: None,
                registered,
            };
            let result = (|| {
                hook(MaterializationEvent::PrivateRootCreated(
                    workspace.source_path.clone(),
                ))
                .map_err(|e| io_at("<trusted-fault-hook>", e))?;
                workspace.populate(&mut hook)?;
                hook(MaterializationEvent::BeforeValidation(
                    workspace.source_path.clone(),
                ))
                .map_err(|e| io_at("<trusted-fault-hook>", e))?;
                workspace.validate_contents()
            })();
            match result {
                Ok(()) => Ok(workspace),
                Err(cause) => Err(MaterializationError::Failed {
                    cause: Box::new(cause),
                    cleanup: workspace.cleanup(),
                }),
            }
        }

        pub fn source_path(&self) -> &Path {
            &self.source_path
        }

        #[cfg(test)]
        pub(crate) fn duplicate_owner_for_test(&self) -> std::io::Result<File> {
            self.registered
                .as_ref()
                .expect("registered workspace fixture")
                .duplicate_owner_for_test()
        }
        pub fn applied(&self) -> &AppliedSourcePatch {
            &self.applied
        }
        pub fn report(&self) -> &MaterializationReport {
            &self.report
        }

        /// Internal subordinate-runtime latch: validation remains available,
        /// while Drop must retain a source tree possibly referenced by a live
        /// process. The durable hint is outside the read-only source mount.
        pub(crate) fn retain_for_execution(
            &mut self,
            journal: &Path,
        ) -> Result<(), MaterializationError> {
            self.validate_contents()?;
            if self.execution_retention.is_some() {
                return Err(MaterializationError::Changed(
                    "execution retention is already active".into(),
                ));
            }
            let reason =
                format!("execution termination/cleanup not yet confirmed; reconcile {journal:?}");
            self.execution_retention = Some(reason.clone());
            let fd = rustix::fs::openat(
                &self.root_fd,
                "execution-retention.json",
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|e| io_at("<execution-retention>", e))?;
            let mut file = File::from(fd);
            file.write_all(&serde_json::to_vec(&serde_json::json!({"version":1,"journal":journal.to_string_lossy(),"reason":reason})).expect("retention JSON"))
                .map_err(|e| io_at("<execution-retention>", e))?;
            file.sync_all()
                .map_err(|e| io_at("<execution-retention>", e))?;
            self.root_fd
                .sync_all()
                .map_err(|e| io_at("<execution-retention>", e))
        }

        /// Called only by the trusted runtime after whole-process termination
        /// and runtime resource cleanup have both been confirmed.
        pub(crate) fn release_after_execution(&mut self) -> Result<(), MaterializationError> {
            if self.execution_retention.is_none() {
                return Err(MaterializationError::Changed(
                    "execution retention was not active".into(),
                ));
            }
            rustix::fs::unlinkat(&self.root_fd, "execution-retention.json", AtFlags::empty())
                .map_err(|e| io_at("<execution-retention>", e))?;
            self.root_fd
                .sync_all()
                .map_err(|e| io_at("<execution-retention>", e))?;
            self.execution_retention = None;
            Ok(())
        }

        fn populate(
            &mut self,
            hook: &mut impl FnMut(MaterializationEvent) -> io::Result<()>,
        ) -> Result<(), MaterializationError> {
            for directory in &self.directories {
                if directory.is_empty() {
                    continue;
                }
                self.validate_identity()?;
                let (parent, leaf) = parent_and_leaf(directory);
                let fd = self.directory_fd(parent)?;
                rustix::fs::mkdirat(&fd, leaf, Mode::from_raw_mode(0o700))
                    .map_err(|e| io_at(directory, e))?;
            }
            // Regular files first. Links are never traversed and are installed
            // only after every non-link input and its directories exist.
            for kind in [InputKind::File, InputKind::Symlink] {
                for input in self
                    .applied
                    .snapshot()
                    .manifest()
                    .inputs
                    .iter()
                    .filter(|i| i.kind == kind)
                {
                    hook(MaterializationEvent::BeforeEntry {
                        source: self.source_path.clone(),
                        logical_path: input.path.clone(),
                    })
                    .map_err(|e| io_at("<trusted-fault-hook>", e))?;
                    self.validate_identity()?;
                    let (parent, leaf) = parent_and_leaf(&input.path);
                    let fd = self.directory_fd(parent)?;
                    let bytes = input
                        .bytes
                        .as_deref()
                        .ok_or_else(|| mismatch(&input.path, "missing normalized bytes"))?;
                    if kind == InputKind::Symlink {
                        let target = std::str::from_utf8(bytes)
                            .map_err(|_| mismatch(&input.path, "unsupported link encoding"))?;
                        rustix::fs::symlinkat(target, &fd, leaf)
                            .map_err(|e| io_at(&input.path, e))?;
                    } else {
                        let mut file = File::from(
                            rustix::fs::openat(
                                &fd,
                                leaf,
                                OFlags::WRONLY
                                    | OFlags::CREATE
                                    | OFlags::EXCL
                                    | OFlags::NOFOLLOW
                                    | OFlags::CLOEXEC,
                                Mode::from_raw_mode(0o600),
                            )
                            .map_err(|e| io_at(&input.path, e))?,
                        );
                        file.write_all(bytes).map_err(|e| io_at(&input.path, e))?;
                        rustix::fs::fchmod(&file, Mode::from_raw_mode(expected_mode(input.mode)?))
                            .map_err(|e| io_at(&input.path, e))?;
                    }
                }
            }
            for directory in self.directories.iter().rev() {
                let fd = self.directory_fd(directory)?;
                rustix::fs::fchmod(&fd, Mode::from_raw_mode(0o755))
                    .map_err(|e| io_at(directory, e))?;
            }
            Ok(())
        }

        fn directory_fd(&self, path: &str) -> Result<File, MaterializationError> {
            let mut fd = open_directory(&self.source_fd, ".")?;
            if !path.is_empty() {
                for component in path.split('/') {
                    fd = open_directory(&fd, component)?;
                }
            }
            Ok(fd)
        }

        fn validate_identity(&self) -> Result<(), MaterializationError> {
            if let Some(lease) = &self.registered {
                lease
                    .validate()
                    .map_err(|e| io_at("<workspace-ownership>", io::Error::other(e.to_string())))?;
            }
            let root = self
                .root
                .as_ref()
                .ok_or_else(|| MaterializationError::Changed("closed private root".into()))?;
            let metadata =
                std::fs::symlink_metadata(root.path()).map_err(|e| io_at("<private-parent>", e))?;
            let held = self
                .root_fd
                .metadata()
                .map_err(|e| io_at("<private-parent>", e))?;
            if !metadata.is_dir()
                || metadata.dev() != held.dev()
                || metadata.ino() != held.ino()
                || metadata.mode() & 0o7777 != 0o700
            {
                return Err(MaterializationError::Changed(
                    "private parent identity/mode".into(),
                ));
            }
            let source = rustix::fs::statat(&self.root_fd, "source", AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|e| io_at("<source-root>", e))?;
            let held = self
                .source_fd
                .metadata()
                .map_err(|e| io_at("<source-root>", e))?;
            if FileType::from_raw_mode(source.st_mode) != FileType::Directory
                || source.st_dev != held.dev()
                || source.st_ino != held.ino()
            {
                return Err(MaterializationError::Changed("source root identity".into()));
            }
            Ok(())
        }

        /// Execution requires durable scoped ownership established before copy.
        /// Plain T0 copies remain available for deterministic materializer users.
        pub(crate) fn validate_registered(&self) -> Result<(), MaterializationError> {
            if self.registered.is_none() {
                return Err(MaterializationError::Changed(
                    "execution requires a registered workspace".into(),
                ));
            }
            self.validate_identity()
        }

        /// Exact no-follow census, including directories, bytes, modes and link
        /// targets. This observes mutations; it is not a substitute for a RO mount.
        pub fn validate_contents(&self) -> Result<(), MaterializationError> {
            self.validate_identity()?;
            let expected: BTreeMap<_, _> = self
                .applied
                .snapshot()
                .manifest()
                .inputs
                .iter()
                .map(|i| (i.path.as_str(), i))
                .collect();
            let mut observed_files = BTreeSet::new();
            let mut observed_dirs = BTreeSet::new();
            self.census(
                &self.source_fd,
                "",
                &expected,
                &mut observed_files,
                &mut observed_dirs,
            )?;
            if observed_files.len() != expected.len() || observed_dirs != self.directories {
                return Err(mismatch("<source-root>", "file/directory census"));
            }
            self.validate_identity()?;
            Ok(())
        }

        fn census(
            &self,
            fd: &File,
            relative: &str,
            expected: &BTreeMap<&str, &WorkspaceInput>,
            files: &mut BTreeSet<String>,
            directories: &mut BTreeSet<String>,
        ) -> Result<(), MaterializationError> {
            let before = fd.metadata().map_err(|e| io_at(relative, e))?;
            if before.mode() & 0o7777 != 0o755 {
                return Err(mismatch(relative, "directory mode"));
            }
            if !self.directories.contains(relative) {
                return Err(mismatch(relative, "unexpected directory"));
            }
            directories.insert(relative.into());
            let mut names = Vec::new();
            for entry in Dir::read_from(fd).map_err(|e| io_at(relative, e))? {
                let entry = entry.map_err(|e| io_at(relative, e))?;
                let name = entry.file_name().to_bytes();
                if matches!(name, b"." | b"..") {
                    continue;
                }
                let name = std::str::from_utf8(name)
                    .map_err(|_| mismatch(relative, "unexpected non-UTF8 name"))?
                    .to_owned();
                if names.len() >= expected.len().saturating_add(self.directories.len()) {
                    return Err(MaterializationError::Limit("unexpected disk census"));
                }
                names.push(name);
            }
            names.sort();
            for name in names {
                let path = if relative.is_empty() {
                    name.clone()
                } else {
                    format!("{relative}/{name}")
                };
                let initial = rustix::fs::statat(fd, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|e| io_at(&path, e))?;
                match FileType::from_raw_mode(initial.st_mode) {
                    FileType::Directory => {
                        let child = open_directory(fd, &name)?;
                        let child_metadata = child.metadata().map_err(|e| io_at(&path, e))?;
                        if child_metadata.ino() != initial.st_ino
                            || child_metadata.dev() != initial.st_dev
                        {
                            return Err(MaterializationError::Changed(path));
                        }
                        self.census(&child, &path, expected, files, directories)?;
                    }
                    FileType::RegularFile | FileType::Symlink => {
                        let input = expected
                            .get(path.as_str())
                            .ok_or_else(|| mismatch(&path, "unexpected file or link"))?;
                        let bytes = input
                            .bytes
                            .as_deref()
                            .ok_or_else(|| mismatch(&path, "absent manifest bytes"))?;
                        if FileType::from_raw_mode(initial.st_mode) == FileType::Symlink {
                            if input.kind != InputKind::Symlink {
                                return Err(mismatch(&path, "unexpected symlink"));
                            }
                            let actual = rustix::fs::readlinkat(fd, name.as_str(), Vec::new())
                                .map_err(|e| io_at(&path, e))?;
                            if actual.to_bytes() != bytes {
                                return Err(mismatch(&path, "symlink target"));
                            }
                        } else {
                            if input.kind != InputKind::File
                                || initial.st_mode & 0o7777 != expected_mode(input.mode)?
                                || initial.st_nlink != 1
                            {
                                return Err(mismatch(&path, "file kind/mode/link count"));
                            }
                            let file = File::from(
                                rustix::fs::openat(
                                    fd,
                                    name.as_str(),
                                    OFlags::RDONLY
                                        | OFlags::NOFOLLOW
                                        | OFlags::NONBLOCK
                                        | OFlags::CLOEXEC,
                                    Mode::empty(),
                                )
                                .map_err(|e| io_at(&path, e))?,
                            );
                            let opened = file.metadata().map_err(|e| io_at(&path, e))?;
                            if opened.ino() != initial.st_ino
                                || opened.dev() != initial.st_dev
                                || !opened.is_file()
                            {
                                return Err(MaterializationError::Changed(path));
                            }
                            let mut actual = Vec::new();
                            (&file)
                                .take(bytes.len() as u64 + 1)
                                .read_to_end(&mut actual)
                                .map_err(|e| io_at(&path, e))?;
                            if actual != bytes {
                                return Err(mismatch(&path, "file bytes"));
                            }
                            if fingerprint(&opened)
                                != fingerprint(&file.metadata().map_err(|e| io_at(&path, e))?)
                            {
                                return Err(MaterializationError::Changed(path));
                            }
                        }
                        files.insert(path.clone());
                    }
                    _ => return Err(mismatch(&path, "unsupported filesystem object")),
                }
                let after = rustix::fs::statat(fd, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|e| io_at(&path, e))?;
                if initial.st_dev != after.st_dev
                    || initial.st_ino != after.st_ino
                    || initial.st_mode != after.st_mode
                    || initial.st_size != after.st_size
                    || initial.st_mtime != after.st_mtime
                    || initial.st_mtime_nsec != after.st_mtime_nsec
                    || initial.st_ctime != after.st_ctime
                    || initial.st_ctime_nsec != after.st_ctime_nsec
                {
                    return Err(MaterializationError::Changed(path));
                }
            }
            if fingerprint(&before) != fingerprint(&fd.metadata().map_err(|e| io_at(relative, e))?)
            {
                return Err(MaterializationError::Changed(relative.into()));
            }
            Ok(())
        }

        pub fn cleanup(mut self) -> CleanupOutcome {
            self.cleanup_inner()
        }

        /// Retain a workspace when an executor cannot prove process termination
        /// or safe reuse. The caller owns reconciliation; no future materializer
        /// attempt reuses this random private root.
        pub fn quarantine(mut self, reason: impl Into<String>) -> CleanupOutcome {
            self.quarantine_in_place(reason)
        }

        /// Borrowed executors must quarantine before returning an uncertain
        /// termination outcome. Validation closes immediately and Drop retains
        /// the directory even if the caller forgets to consume this workspace.
        pub fn quarantine_in_place(&mut self, reason: impl Into<String>) -> CleanupOutcome {
            if let Some(outcome) = &self.quarantined {
                return outcome.clone();
            }
            let Some(root) = self.root.take() else {
                return CleanupOutcome::Removed;
            };
            let path = self.owned_path_hint(root.path());
            let _ = root.keep();
            // An inactive/quarantined controller no longer owns a live copy.
            // Runtime retention remains a separate durable deletion barrier.
            self.registered = None;
            let outcome = CleanupOutcome::Quarantined {
                path,
                reason: reason.into(),
            };
            self.quarantined = Some(outcome.clone());
            outcome
        }

        fn cleanup_inner(&mut self) -> CleanupOutcome {
            if let Some(reason) = self.execution_retention.clone() {
                return self.quarantine_in_place(reason);
            }
            let Some(root) = self.root.take() else {
                return self.quarantined.clone().unwrap_or(CleanupOutcome::Removed);
            };
            let path = root.path().to_path_buf();
            let valid = self
                .registered
                .as_ref()
                .is_none_or(|lease| lease.validate().is_ok())
                && std::fs::symlink_metadata(&path)
                    .ok()
                    .is_some_and(|metadata| {
                        self.root_fd.metadata().ok().is_some_and(|held| {
                            metadata.is_dir()
                                && metadata.dev() == held.dev()
                                && metadata.ino() == held.ino()
                        })
                    });
            if !valid {
                let _ = root.keep();
                return CleanupOutcome::Quarantined {
                    path: self.owned_path_hint(&path),
                    reason: "private root identity changed; replacement left untouched".into(),
                };
            }
            match root.close() {
                Ok(()) => match self
                    .registered
                    .as_ref()
                    .map(|lease| lease.confirm_removal())
                    .transpose()
                {
                    Ok(_) => CleanupOutcome::Removed,
                    Err(error) => CleanupOutcome::Quarantined {
                        path,
                        reason: format!("workspace removal durability: {error}"),
                    },
                },
                Err(error) => CleanupOutcome::Quarantined {
                    path,
                    reason: error.to_string(),
                },
            }
        }

        fn owned_path_hint(&self, fallback: &Path) -> PathBuf {
            std::fs::read_link(format!("/proc/self/fd/{}", self.root_fd.as_raw_fd()))
                .unwrap_or_else(|_| fallback.to_path_buf())
        }
    }

    impl Drop for PrivateWorkspace {
        fn drop(&mut self) {
            let _ = self.cleanup_inner();
        }
    }

    fn expected_mode(mode: FileMode) -> Result<u32, MaterializationError> {
        match mode {
            FileMode::Regular => Ok(0o644),
            FileMode::Executable => Ok(0o755),
            _ => Err(mismatch("<mode>", "not a regular file mode")),
        }
    }
    fn parent_and_leaf(path: &str) -> (&str, &str) {
        path.rsplit_once('/').unwrap_or(("", path))
    }
    fn open_directory(parent: &File, name: &str) -> Result<File, MaterializationError> {
        rustix::fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|e| io_at(name, e))
    }
    fn open_absolute_directory(path: &Path) -> Result<File, MaterializationError> {
        if !path.is_absolute() {
            return Err(MaterializationError::Changed(
                "temporary root is not absolute".into(),
            ));
        }
        let mut fd = File::from(
            rustix::fs::open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| io_at("<private-parent>", e))?,
        );
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    fd = File::from(
                        rustix::fs::openat(
                            &fd,
                            name,
                            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                            Mode::empty(),
                        )
                        .map_err(|e| io_at("<private-parent>", e))?,
                    );
                }
                _ => {
                    return Err(MaterializationError::Changed(
                        "noncanonical temporary root".into(),
                    ));
                }
            }
        }
        Ok(fd)
    }
    fn fingerprint(metadata: &Metadata) -> (u64, u64, u32, u64, i64, i64, i64, i64) {
        (
            metadata.dev(),
            metadata.ino(),
            metadata.mode(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    }
    fn io_at(path: &str, error: impl Into<io::Error>) -> MaterializationError {
        MaterializationError::Io {
            path: path.into(),
            source: error.into(),
        }
    }
}

fn mismatch(path: &str, reason: &'static str) -> MaterializationError {
    MaterializationError::Mismatch {
        path: path.into(),
        reason,
    }
}
