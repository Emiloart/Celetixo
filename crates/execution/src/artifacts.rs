//! Scope-bound local immutable evidence objects with confirmed local persistence.
//!
//! This Linux adapter synchronizes object data and directory publication before
//! returning a reference. It is not a distributed object store, retention daemon,
//! access-control service or protection against an administrator with host UID.

use celetixo_engine::ids::{Digest, Scope};
use celetixo_engine::source_evidence::{SourceArtifactError, SourceArtifactReference};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactStoreLimits {
    pub max_object_bytes: u64,
    pub max_total_bytes: u64,
    /// Published and pending objects both count. The fixed store header is extra.
    pub max_objects: usize,
}
impl Default for ArtifactStoreLimits {
    fn default() -> Self {
        Self {
            max_object_bytes: 16 * 1024 * 1024,
            max_total_bytes: 128 * 1024 * 1024,
            max_objects: 4096,
        }
    }
}
impl ArtifactStoreLimits {
    fn validate(self) -> Result<(), SourceArtifactError> {
        if self.max_object_bytes == 0
            || self.max_object_bytes > 64 * 1024 * 1024
            || self.max_total_bytes < self.max_object_bytes
            || self.max_total_bytes > 512 * 1024 * 1024
            || self.max_objects == 0
            || self.max_objects > 100_000
        {
            return Err(SourceArtifactError::Limit);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactStoreEvent {
    BeforeWrite,
    BeforeFileSync,
    BeforePublish,
    BeforeDirectorySync,
    BeforeReadback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactStoreInitializationEvent {
    BeforeHeaderSync,
    BeforeRootSync,
    BeforeParentSync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub removed_pending_objects: usize,
    pub removed_bytes: u64,
}

#[cfg(target_os = "linux")]
pub use linux::LocalSourceArtifactStore;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use celetixo_engine::source_evidence::{SOURCE_ARTIFACT_DOMAIN, SourceArtifactReader};
    use rustix::fs::{AtFlags, Dir, FileType, FlockOperation, Mode, OFlags, RenameFlags};
    use std::collections::BTreeSet;
    use std::fs::{File, Metadata};
    use std::io::{Read, Write};
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;
    use std::sync::Mutex;
    use tempfile::TempDir;

    pub struct LocalSourceArtifactStore {
        scope: Scope,
        retention: Digest,
        limits: ArtifactStoreLimits,
        root: File,
        path: PathBuf,
        header: Vec<u8>,
        temporary: Option<TempDir>,
        counter: Mutex<u64>,
    }

    impl LocalSourceArtifactStore {
        /// Ephemeral fixture storage, deleted when its final owner is dropped.
        /// Use create/open for artifacts that must survive normal shutdown.
        pub fn temporary(
            scope: Scope,
            retention: Digest,
            limits: ArtifactStoreLimits,
        ) -> Result<Self, SourceArtifactError> {
            limits.validate()?;
            let temporary = tempfile::Builder::new()
                .prefix("celetixo-evidence-")
                .tempdir()
                .map_err(persistence)?;
            let path = temporary.path().to_path_buf();
            let root = open_absolute(&path)?;
            rustix::fs::fchmod(&root, Mode::from_raw_mode(0o700)).map_err(persistence)?;
            let mut result = Self::initialize(root, path, scope, retention, limits, true)?;
            result.confirm_initialization(&mut |_| Ok(()))?;
            result.temporary = Some(temporary);
            Ok(result)
        }

        /// Create one new private scope-specific directory. Existing directories
        /// are never adopted or emptied; use open with the exact original contract.
        pub fn create(
            path: impl AsRef<Path>,
            scope: Scope,
            retention: Digest,
            limits: ArtifactStoreLimits,
        ) -> Result<Self, SourceArtifactError> {
            Self::create_with_hook(path, scope, retention, limits, |_| Ok(()))
        }

        /// Trusted initialization fault injection. A failed creation can leave
        /// an incomplete directory, but never returns a ready storage adapter.
        pub fn create_with_hook(
            path: impl AsRef<Path>,
            scope: Scope,
            retention: Digest,
            limits: ArtifactStoreLimits,
            mut hook: impl FnMut(ArtifactStoreInitializationEvent) -> io::Result<()>,
        ) -> Result<Self, SourceArtifactError> {
            limits.validate()?;
            let path = absolute(path.as_ref())?;
            let parent = path
                .parent()
                .ok_or_else(|| persistence("invalid store root"))?;
            let leaf = path
                .file_name()
                .ok_or_else(|| persistence("invalid store root"))?;
            let parent_fd = open_absolute(parent)?;
            rustix::fs::mkdirat(&parent_fd, leaf, Mode::from_raw_mode(0o700))
                .map_err(persistence)?;
            let root = File::from(
                rustix::fs::openat(&parent_fd, leaf, directory_flags(), Mode::empty())
                    .map_err(persistence)?,
            );
            rustix::fs::fchmod(&root, Mode::from_raw_mode(0o700)).map_err(persistence)?;
            let result = Self::initialize(root, path, scope, retention, limits, true)?;
            result.confirm_initialization(&mut hook)?;
            Ok(result)
        }

        pub fn open(
            path: impl AsRef<Path>,
            scope: Scope,
            retention: Digest,
            limits: ArtifactStoreLimits,
        ) -> Result<Self, SourceArtifactError> {
            Self::open_with_hook(path, scope, retention, limits, |_| Ok(()))
        }

        /// Recovery confirms the exact header, root and parent publication
        /// independently; an earlier constructor may have lost its response
        /// before any of those synchronization operations completed.
        pub fn open_with_hook(
            path: impl AsRef<Path>,
            scope: Scope,
            retention: Digest,
            limits: ArtifactStoreLimits,
            mut hook: impl FnMut(ArtifactStoreInitializationEvent) -> io::Result<()>,
        ) -> Result<Self, SourceArtifactError> {
            limits.validate()?;
            let path = absolute(path.as_ref())?;
            let root = open_absolute(&path)?;
            let result = Self::initialize(root, path, scope, retention, limits, false)?;
            result.confirm_initialization(&mut hook)?;
            Ok(result)
        }

        fn initialize(
            root: File,
            path: PathBuf,
            scope: Scope,
            retention: Digest,
            limits: ArtifactStoreLimits,
            create: bool,
        ) -> Result<Self, SourceArtifactError> {
            // This lock coordinates trusted local adapter instances and budget
            // updates. It has no role in transaction admission or commit authority.
            rustix::fs::flock(&root, FlockOperation::NonBlockingLockExclusive)
                .map_err(persistence)?;
            let header = header_bytes(&scope, retention, limits);
            let result = Self {
                scope,
                retention,
                limits,
                root,
                path,
                header,
                temporary: None,
                counter: Mutex::new(0),
            };
            result.validate_root_identity()?;
            if create {
                let mut file = File::from(
                    rustix::fs::openat(
                        &result.root,
                        "store.meta",
                        OFlags::WRONLY
                            | OFlags::CREATE
                            | OFlags::EXCL
                            | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC,
                        Mode::from_raw_mode(0o600),
                    )
                    .map_err(persistence)?,
                );
                file.write_all(&result.header).map_err(persistence)?;
                rustix::fs::fchmod(&file, Mode::from_raw_mode(0o444)).map_err(persistence)?;
            }
            result.validate_root()?;
            result.census()?;
            Ok(result)
        }

        fn confirm_initialization(
            &self,
            hook: &mut impl FnMut(ArtifactStoreInitializationEvent) -> io::Result<()>,
        ) -> Result<(), SourceArtifactError> {
            self.validate_root()?;
            let header = self.open_regular("store.meta", 0o444)?;
            if read_bounded(&header, self.header.len() as u64)? != self.header {
                return Err(SourceArtifactError::Scope);
            }
            let header_identity = header.metadata().map_err(persistence)?;
            let parent = self
                .path
                .parent()
                .ok_or_else(|| persistence("artifact store has no parent"))?;
            let parent = open_absolute(parent)?;
            hook(ArtifactStoreInitializationEvent::BeforeHeaderSync).map_err(persistence)?;
            header.sync_all().map_err(persistence)?;
            hook(ArtifactStoreInitializationEvent::BeforeRootSync).map_err(persistence)?;
            self.root.sync_all().map_err(persistence)?;
            hook(ArtifactStoreInitializationEvent::BeforeParentSync).map_err(persistence)?;
            parent.sync_all().map_err(persistence)?;
            let current_header = self.open_regular("store.meta", 0o444)?;
            if fingerprint(&header.metadata().map_err(persistence)?)
                != fingerprint(&header_identity)
                || fingerprint(&current_header.metadata().map_err(persistence)?)
                    != fingerprint(&header_identity)
            {
                return Err(SourceArtifactError::Integrity);
            }
            self.validate_root()?;
            Ok(())
        }

        pub fn root_path(&self) -> &Path {
            &self.path
        }
        pub fn scope(&self) -> &Scope {
            &self.scope
        }
        pub fn limits(&self) -> ArtifactStoreLimits {
            self.limits
        }

        pub fn put_bytes(
            &self,
            name: &str,
            media_type: &str,
            bytes: &[u8],
        ) -> Result<SourceArtifactReference, SourceArtifactError> {
            self.put_bytes_with_hook(name, media_type, bytes, |_| Ok(()))
        }

        /// Trusted deterministic persistence fault injection; no repository code.
        pub fn put_bytes_with_hook(
            &self,
            name: &str,
            media_type: &str,
            bytes: &[u8],
            mut hook: impl FnMut(ArtifactStoreEvent) -> io::Result<()>,
        ) -> Result<SourceArtifactReference, SourceArtifactError> {
            if !valid_text(name, 4096) || !valid_text(media_type, 1024) {
                return Err(persistence("invalid artifact metadata"));
            }
            if bytes.len() as u64 > self.limits.max_object_bytes {
                return Err(SourceArtifactError::Limit);
            }
            let mut counter = self
                .counter
                .lock()
                .map_err(|_| persistence("artifact store mutex poisoned"))?;
            self.validate_root()?;
            let address = Digest::of(SOURCE_ARTIFACT_DOMAIN, bytes);
            let final_name = object_name(address);
            if self.object_exists(&final_name)? {
                let existing = self.read_object(address, self.limits.max_object_bytes)?;
                if existing != bytes {
                    return Err(SourceArtifactError::Integrity);
                }
                return Ok(self.reference(name, media_type, address, bytes.len() as u64));
            }
            let census = self.census()?;
            if census.objects >= self.limits.max_objects
                || census
                    .bytes
                    .checked_add(bytes.len() as u64)
                    .is_none_or(|sum| sum > self.limits.max_total_bytes)
            {
                return Err(SourceArtifactError::Limit);
            }
            let (pending, mut file) = loop {
                *counter = counter.checked_add(1).ok_or(SourceArtifactError::Limit)?;
                let name = format!(".pending-{:016x}", *counter);
                match rustix::fs::openat(
                    &self.root,
                    name.as_str(),
                    OFlags::WRONLY
                        | OFlags::CREATE
                        | OFlags::EXCL
                        | OFlags::NOFOLLOW
                        | OFlags::CLOEXEC,
                    Mode::from_raw_mode(0o600),
                ) {
                    Ok(fd) => break (name, File::from(fd)),
                    Err(rustix::io::Errno::EXIST)
                        if *counter <= self.limits.max_objects as u64 + 1 =>
                    {
                        continue;
                    }
                    Err(error) => return Err(persistence(error)),
                }
            };
            let mut published = false;
            let result = (|| {
                hook(ArtifactStoreEvent::BeforeWrite).map_err(persistence)?;
                file.write_all(bytes).map_err(persistence)?;
                rustix::fs::fchmod(&file, Mode::from_raw_mode(0o444)).map_err(persistence)?;
                hook(ArtifactStoreEvent::BeforeFileSync).map_err(persistence)?;
                file.sync_all().map_err(persistence)?;
                hook(ArtifactStoreEvent::BeforePublish).map_err(persistence)?;
                self.validate_root_identity()?;
                rustix::fs::renameat_with(
                    &self.root,
                    pending.as_str(),
                    &self.root,
                    final_name.as_str(),
                    RenameFlags::NOREPLACE,
                )
                .map_err(persistence)?;
                published = true;
                hook(ArtifactStoreEvent::BeforeDirectorySync).map_err(persistence)?;
                self.root.sync_all().map_err(persistence)?;
                hook(ArtifactStoreEvent::BeforeReadback).map_err(persistence)?;
                if self.read_object(address, self.limits.max_object_bytes)? != bytes {
                    return Err(SourceArtifactError::Integrity);
                }
                Ok(self.reference(name, media_type, address, bytes.len() as u64))
            })();
            if result.is_err() && !published {
                match rustix::fs::unlinkat(&self.root, pending.as_str(), AtFlags::empty()) {
                    Ok(()) => self.root.sync_all().map_err(persistence)?,
                    Err(rustix::io::Errno::NOENT) => {}
                    Err(error) => {
                        return Err(persistence(format!(
                            "unpublished artifact cleanup failed: {error}"
                        )));
                    }
                }
            }
            result
        }

        pub fn reconcile_pending(&self) -> Result<ReconciliationReport, SourceArtifactError> {
            let _guard = self
                .counter
                .lock()
                .map_err(|_| persistence("artifact store mutex poisoned"))?;
            self.validate_root()?;
            let census = self.census()?;
            let mut report = ReconciliationReport {
                removed_pending_objects: 0,
                removed_bytes: 0,
            };
            for (name, bytes) in census.pending {
                rustix::fs::unlinkat(&self.root, name.as_str(), AtFlags::empty())
                    .map_err(persistence)?;
                report.removed_pending_objects += 1;
                report.removed_bytes += bytes;
            }
            self.root.sync_all().map_err(persistence)?;
            self.validate_root()?;
            Ok(report)
        }

        fn reference(
            &self,
            name: &str,
            media_type: &str,
            content: Digest,
            bytes: u64,
        ) -> SourceArtifactReference {
            SourceArtifactReference {
                scope: self.scope.clone(),
                name: name.into(),
                media_type: media_type.into(),
                content,
                bytes,
                retention_policy: self.retention,
            }
        }

        fn object_exists(&self, name: &str) -> Result<bool, SourceArtifactError> {
            match rustix::fs::statat(&self.root, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => Ok(true),
                Err(rustix::io::Errno::NOENT) => Ok(false),
                Err(error) => Err(persistence(error)),
            }
        }

        fn validate_root_identity(&self) -> Result<(), SourceArtifactError> {
            let path = std::fs::symlink_metadata(&self.path).map_err(persistence)?;
            let held = self.root.metadata().map_err(persistence)?;
            if !path.is_dir()
                || path.dev() != held.dev()
                || path.ino() != held.ino()
                || held.mode() & 0o7777 != 0o700
                || held.uid() != rustix::process::geteuid().as_raw()
            {
                return Err(SourceArtifactError::Integrity);
            }
            Ok(())
        }

        fn validate_root(&self) -> Result<(), SourceArtifactError> {
            self.validate_root_identity()?;
            let file = self.open_regular("store.meta", 0o444)?;
            if read_bounded(&file, self.header.len() as u64)? != self.header {
                return Err(SourceArtifactError::Scope);
            }
            Ok(())
        }

        fn open_regular(&self, name: &str, mode: u32) -> Result<File, SourceArtifactError> {
            let initial =
                rustix::fs::statat(&self.root, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| {
                    if e == rustix::io::Errno::NOENT {
                        SourceArtifactError::Unavailable(name.into())
                    } else {
                        persistence(e)
                    }
                })?;
            if FileType::from_raw_mode(initial.st_mode) != FileType::RegularFile
                || initial.st_mode & 0o7777 != mode
                || initial.st_nlink != 1
                || initial.st_uid != rustix::process::geteuid().as_raw()
            {
                return Err(SourceArtifactError::Integrity);
            }
            let file = File::from(
                rustix::fs::openat(
                    &self.root,
                    name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(persistence)?,
            );
            let opened = file.metadata().map_err(persistence)?;
            if !opened.is_file()
                || opened.dev() != initial.st_dev
                || opened.ino() != initial.st_ino
                || opened.mode() & 0o7777 != mode
                || opened.nlink() != 1
                || opened.uid() != initial.st_uid
            {
                return Err(SourceArtifactError::Integrity);
            }
            Ok(file)
        }

        fn read_object(
            &self,
            address: Digest,
            maximum: u64,
        ) -> Result<Vec<u8>, SourceArtifactError> {
            self.validate_root()?;
            let name = object_name(address);
            let file = self.open_regular(&name, 0o444)?;
            let before = file.metadata().map_err(persistence)?;
            let bytes = read_bounded(&file, maximum.min(self.limits.max_object_bytes))?;
            if Digest::of(SOURCE_ARTIFACT_DOMAIN, &bytes) != address {
                return Err(SourceArtifactError::Integrity);
            }
            // Reads establish local persistence independently, including recovery
            // of a publication whose successful directory sync response was lost.
            file.sync_all().map_err(persistence)?;
            self.root.sync_all().map_err(persistence)?;
            let current = rustix::fs::statat(&self.root, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                .map_err(persistence)?;
            let after = file.metadata().map_err(persistence)?;
            if fingerprint(&before) != fingerprint(&after)
                || current.st_dev != after.dev()
                || current.st_ino != after.ino()
                || current.st_nlink != 1
                || current.st_mode & 0o7777 != 0o444
            {
                return Err(SourceArtifactError::Integrity);
            }
            self.validate_root()?;
            Ok(bytes)
        }

        fn census(&self) -> Result<Census, SourceArtifactError> {
            let mut census = Census {
                objects: 0,
                bytes: 0,
                pending: Vec::new(),
            };
            let mut names = BTreeSet::new();
            for entry in Dir::read_from(&self.root).map_err(persistence)? {
                let entry = entry.map_err(persistence)?;
                let raw = entry.file_name().to_bytes();
                if matches!(raw, b"." | b".." | b"store.meta") {
                    continue;
                }
                let name = std::str::from_utf8(raw)
                    .map_err(|_| SourceArtifactError::Integrity)?
                    .to_owned();
                if !names.insert(name.clone()) {
                    return Err(SourceArtifactError::Integrity);
                }
                census.objects += 1;
                if census.objects > self.limits.max_objects {
                    return Err(SourceArtifactError::Limit);
                }
                let stat = rustix::fs::statat(&self.root, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(persistence)?;
                if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
                    || stat.st_nlink != 1
                    || stat.st_uid != rustix::process::geteuid().as_raw()
                    || stat.st_size < 0
                    || stat.st_size as u64 > self.limits.max_object_bytes
                {
                    return Err(SourceArtifactError::Integrity);
                }
                census.bytes = census
                    .bytes
                    .checked_add(stat.st_size as u64)
                    .ok_or(SourceArtifactError::Limit)?;
                if census.bytes > self.limits.max_total_bytes {
                    return Err(SourceArtifactError::Limit);
                }
                if is_pending(&name) && matches!(stat.st_mode & 0o7777, 0o600 | 0o444) {
                    census.pending.push((name, stat.st_size as u64));
                } else if let Some(address) = parse_object_name(&name) {
                    self.read_object(address, self.limits.max_object_bytes)?;
                } else {
                    return Err(SourceArtifactError::Integrity);
                }
            }
            Ok(census)
        }
    }

    impl SourceArtifactReader for LocalSourceArtifactStore {
        fn read(
            &self,
            scope: &Scope,
            address: Digest,
            max_bytes: u64,
        ) -> Result<Vec<u8>, SourceArtifactError> {
            if scope != &self.scope {
                return Err(SourceArtifactError::Scope);
            }
            self.read_object(address, max_bytes)
        }
        fn retention_policy(&self) -> Digest {
            self.retention
        }
    }

    impl Drop for LocalSourceArtifactStore {
        fn drop(&mut self) {
            if let Some(temporary) = self.temporary.take() {
                if self.validate_root_identity().is_ok() {
                    let _ = temporary.close();
                } else {
                    let _ = temporary.keep();
                }
            }
        }
    }

    struct Census {
        objects: usize,
        bytes: u64,
        pending: Vec<(String, u64)>,
    }
    fn object_name(address: Digest) -> String {
        format!("{address}.artifact")
    }
    fn parse_object_name(name: &str) -> Option<Digest> {
        let hex = name.strip_suffix(".artifact")?;
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            return None;
        }
        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
        }
        Some(Digest::from_bytes(bytes))
    }
    fn is_pending(name: &str) -> bool {
        name.strip_prefix(".pending-").is_some_and(|suffix| {
            suffix.len() == 16
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        })
    }
    fn read_bounded(file: &File, limit: u64) -> Result<Vec<u8>, SourceArtifactError> {
        let before = file.metadata().map_err(persistence)?;
        if before.len() > limit {
            return Err(SourceArtifactError::Limit);
        }
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(persistence)?;
        if bytes.len() as u64 > limit {
            return Err(SourceArtifactError::Limit);
        }
        if fingerprint(&before) != fingerprint(&file.metadata().map_err(persistence)?) {
            return Err(SourceArtifactError::Integrity);
        }
        Ok(bytes)
    }
    fn fingerprint(m: &Metadata) -> (u64, u64, u32, u64, u64, i64, i64, i64, i64) {
        (
            m.dev(),
            m.ino(),
            m.mode(),
            m.nlink(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    }
    fn header_bytes(scope: &Scope, retention: Digest, limits: ArtifactStoreLimits) -> Vec<u8> {
        let mut bytes = b"celetixo/local-source-artifact-store/v1".to_vec();
        for field in [
            scope.organization.as_str(),
            scope.workspace.as_str(),
            scope.repository.as_str(),
        ] {
            bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes.extend_from_slice(retention.as_bytes());
        for n in [
            limits.max_object_bytes,
            limits.max_total_bytes,
            limits.max_objects as u64,
        ] {
            bytes.extend_from_slice(&n.to_be_bytes());
        }
        bytes
    }
    fn valid_text(value: &str, maximum: usize) -> bool {
        !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
    }
    fn absolute(path: &Path) -> Result<PathBuf, SourceArtifactError> {
        if !path.is_absolute() {
            return Err(persistence("artifact root must be absolute"));
        }
        for component in path.components() {
            if !matches!(component, Component::RootDir | Component::Normal(_)) {
                return Err(persistence("artifact root must be canonical"));
            }
        }
        Ok(path.to_path_buf())
    }
    fn directory_flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
    }
    fn open_absolute(path: &Path) -> Result<File, SourceArtifactError> {
        let path = absolute(path)?;
        let mut file = File::from(
            rustix::fs::open("/", directory_flags(), Mode::empty()).map_err(persistence)?,
        );
        for component in path.components() {
            if let Component::Normal(name) = component {
                file = File::from(
                    rustix::fs::openat(&file, name, directory_flags(), Mode::empty())
                        .map_err(persistence)?,
                );
            }
        }
        Ok(file)
    }
    fn persistence(error: impl std::fmt::Display) -> SourceArtifactError {
        SourceArtifactError::Persistence(error.to_string())
    }
}
