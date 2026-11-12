//! Scoped ownership and quota accounting for private source copies. Filesystem
//! locks protect allocation/recovery and owner liveness, never Engine semantics.
use crate::materialize::{MaterializationError, MaterializationEvent, PrivateWorkspace};
use celetixo_engine::engine::{
    Engine, EngineError, SourceAdmission, SourceCleanupCapability, SourceExecutorCapability,
};
use celetixo_engine::ids::{Digest, Scope};
use celetixo_engine::snapshot::Snapshot;
use celetixo_engine::source_patch::SourcePatch;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

const HEADER: &str = "registry.json";
const LOCK: &str = "registry.lock";
const OWNER: &str = "owner.lock";
const RECORD: &str = "workspace.json";
const PENDING: &str = "workspace.pending";
const RETENTION: &str = "execution-retention.json";
const PREFIX: &str = "workspace-";
const MAX_METADATA: u64 = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLimits {
    pub max_roots: usize,
    /// Reserved logical source bytes, including each copy still being written.
    pub max_total_input_bytes: u64,
    pub max_entries_per_root: usize,
}
impl Default for WorkspaceLimits {
    fn default() -> Self {
        Self {
            max_roots: 8,
            max_total_input_bytes: 512 * 1024 * 1024,
            max_entries_per_root: 100_016,
        }
    }
}
impl WorkspaceLimits {
    fn validate(self) -> Result<(), WorkspaceError> {
        if self.max_roots == 0
            || self.max_roots > 4096
            || self.max_total_input_bytes == 0
            || self.max_total_input_bytes > 4 * 1024 * 1024 * 1024
            || self.max_entries_per_root < 4
            || self.max_entries_per_root > 1_000_016
        {
            return Err(WorkspaceError::Limit);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace registry scope, identity or ownership differs")]
    Identity,
    #[error("workspace registry metadata is incomplete or altered: {0}")]
    Metadata(String),
    #[error("workspace registry capacity is busy or exhausted")]
    Limit,
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Materialization(#[from] MaterializationError),
    #[error("workspace registry I/O: {0}")]
    Io(#[from] io::Error),
}

impl From<rustix::io::Errno> for WorkspaceError {
    fn from(error: rustix::io::Errno) -> Self {
        Self::Io(error.into())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorkspaceRecovery {
    pub removed: Vec<PathBuf>,
    pub live: Vec<PathBuf>,
    pub quarantined: Vec<(PathBuf, String)>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    organization: String,
    workspace: String,
    repository: String,
    limits: WorkspaceLimits,
    root_dev: u64,
    root_ino: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    registry: String,
    admission: String,
    patch: String,
    base: String,
    candidate: String,
    manifest: String,
    application: String,
    root_dev: u64,
    root_ino: u64,
    input_bytes: u64,
    entries: usize,
}

struct RegistryState {
    path: PathBuf,
    root: File,
    lock: File,
    scope: Scope,
    limits: WorkspaceLimits,
    header: Vec<u8>,
    identity: Digest,
}

#[derive(Clone)]
pub struct WorkspaceRegistry {
    state: Arc<RegistryState>,
}

/// Kept by PrivateWorkspace. An owner lock prevents cleanup of an active copy;
/// an execution-retention marker independently prevents unsafe orphan deletion.
pub(crate) struct WorkspaceLease {
    registry: Arc<RegistryState>,
    root: File,
    owner: File,
    path: PathBuf,
    record: Vec<u8>,
}

pub(crate) struct RegisteredRoot {
    pub root: tempfile::TempDir,
    pub lease: WorkspaceLease,
}

/// Runtime cleanup may remove only its exact retained source. General orphan
/// cleanup instead requires the registry-specific Engine capability.
pub(crate) struct RetainedSource<'a> {
    pub path: &'a Path,
    pub scope: &'a Scope,
    pub admission: &'a str,
    pub candidate: &'a str,
    pub manifest: &'a str,
    pub latch: &'a [u8],
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use rustix::fs::{FlockOperation, Mode, OFlags};
    use std::os::unix::fs::MetadataExt;

    fn directory_flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
    }
    fn open_dir(path: &Path) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(io::Error::other("registry path must be absolute"));
        }
        let mut fd = File::from(rustix::fs::open("/", directory_flags(), Mode::empty())?);
        for component in path.components() {
            match component {
                Component::RootDir => (),
                Component::Normal(name) => {
                    fd = File::from(rustix::fs::openat(
                        &fd,
                        name,
                        directory_flags(),
                        Mode::empty(),
                    )?)
                }
                _ => return Err(io::Error::other("registry path is not canonical")),
            }
        }
        Ok(fd)
    }
    fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    fn private_dir(path: &Path, held: &File) -> Result<(), WorkspaceError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o7777 != 0o700
            || metadata.uid() != rustix::process::getuid().as_raw()
            || !same(&metadata, &held.metadata()?)
        {
            return Err(WorkspaceError::Identity);
        }
        Ok(())
    }
    fn regular(root: &File, name: &str, writable: bool) -> Result<File, WorkspaceError> {
        let mode = if writable {
            OFlags::RDWR
        } else {
            OFlags::RDONLY
        };
        let file = File::from(rustix::fs::openat(
            root,
            name,
            mode | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(WorkspaceError::Identity);
        }
        Ok(file)
    }
    fn read(root: &File, name: &str) -> Result<Vec<u8>, WorkspaceError> {
        let file = regular(root, name, false)?;
        let before = file.metadata()?;
        if before.len() > MAX_METADATA {
            return Err(WorkspaceError::Metadata(
                "oversized ownership record".into(),
            ));
        }
        let mut bytes = Vec::new();
        (&file).take(MAX_METADATA + 1).read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        if bytes.len() as u64 != before.len()
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.ctime() != after.ctime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime_nsec() != after.ctime_nsec()
            || !same(&before, &after)
        {
            return Err(WorkspaceError::Identity);
        }
        Ok(bytes)
    }
    fn write_new(root: &File, name: &str, bytes: &[u8]) -> Result<File, WorkspaceError> {
        let mut file = File::from(rustix::fs::openat(
            root,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )?);
        rustix::fs::fchmod(&file, Mode::from_raw_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(file)
    }
    fn try_lock(file: &File) -> Result<bool, WorkspaceError> {
        match rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(true),
            Err(rustix::io::Errno::WOULDBLOCK) => Ok(false),
            Err(error) => Err(io::Error::from(error).into()),
        }
    }

    impl RegistryState {
        fn validate(&self) -> Result<(), WorkspaceError> {
            private_dir(&self.path, &self.root)?;
            if read(&self.root, HEADER)? != self.header {
                return Err(WorkspaceError::Identity);
            }
            if !same(
                &regular(&self.root, LOCK, true)?.metadata()?,
                &self.lock.metadata()?,
            ) {
                return Err(WorkspaceError::Identity);
            }
            Ok(())
        }
        fn lock(&self) -> Result<File, WorkspaceError> {
            self.validate()?;
            let lock = regular(&self.root, LOCK, true)?;
            if !try_lock(&lock)? {
                return Err(WorkspaceError::Limit);
            }
            self.validate()?;
            Ok(lock)
        }
        fn roots(&self) -> Result<Vec<PathBuf>, WorkspaceError> {
            let mut paths = Vec::new();
            for entry in fs::read_dir(&self.path)? {
                let entry = entry?;
                let name = entry.file_name();
                if name == HEADER || name == LOCK {
                    continue;
                }
                if !name.to_str().is_some_and(|name| name.starts_with(PREFIX)) {
                    return Err(WorkspaceError::Metadata("unknown registry entry".into()));
                }
                paths.push(entry.path());
                if paths.len() > self.limits.max_roots {
                    return Err(WorkspaceError::Limit);
                }
            }
            paths.sort();
            Ok(paths)
        }
        fn record(&self, root: &File) -> Result<(Record, Vec<u8>), WorkspaceError> {
            let bytes = read(root, RECORD)?;
            let record: Record = serde_json::from_slice(&bytes)
                .map_err(|e| WorkspaceError::Metadata(e.to_string()))?;
            if record.version != 1
                || record.registry != self.identity.to_string()
                || record.input_bytes > self.limits.max_total_input_bytes
                || record.entries > self.limits.max_entries_per_root
                || serde_json::to_vec(&record)
                    .map_err(|e| WorkspaceError::Metadata(e.to_string()))?
                    != bytes
                || [
                    record.admission.as_str(),
                    record.patch.as_str(),
                    record.base.as_str(),
                    record.candidate.as_str(),
                    record.manifest.as_str(),
                    record.application.as_str(),
                ]
                .iter()
                .any(|value| {
                    value.len() != 64
                        || !value
                            .bytes()
                            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
                })
            {
                return Err(WorkspaceError::Identity);
            }
            let actual = root.metadata()?;
            if actual.dev() != record.root_dev || actual.ino() != record.root_ino {
                return Err(WorkspaceError::Identity);
            }
            Ok((record, bytes))
        }
    }

    impl WorkspaceRegistry {
        pub fn create(
            path: impl AsRef<Path>,
            scope: Scope,
            limits: WorkspaceLimits,
        ) -> Result<Self, WorkspaceError> {
            limits.validate()?;
            let path = path.as_ref().to_path_buf();
            let parent = path.parent().ok_or(WorkspaceError::Identity)?;
            let parent_fd = open_dir(parent)?;
            let leaf = path.file_name().ok_or(WorkspaceError::Identity)?;
            rustix::fs::mkdirat(&parent_fd, leaf, Mode::from_raw_mode(0o700))?;
            let root = open_dir(&path)?;
            rustix::fs::fchmod(&root, Mode::from_raw_mode(0o700))?;
            let header = header(&scope, limits, &root)?;
            write_new(&root, HEADER, &header)?;
            write_new(&root, LOCK, b"")?;
            root.sync_all()?;
            parent_fd.sync_all()?;
            Self::from_root(path, root, scope, limits, header)
        }
        pub fn open(
            path: impl AsRef<Path>,
            scope: Scope,
            limits: WorkspaceLimits,
        ) -> Result<Self, WorkspaceError> {
            limits.validate()?;
            let path = path.as_ref().to_path_buf();
            let root = open_dir(&path)?;
            let header = header(&scope, limits, &root)?;
            let result = Self::from_root(path, root, scope, limits, header)?;
            // Interrupted initialization does not become ready without confirming
            // the exact validated header/root/parent publication.
            regular(&result.state.root, HEADER, false)?.sync_all()?;
            regular(&result.state.root, LOCK, true)?.sync_all()?;
            result.state.root.sync_all()?;
            open_dir(result.state.path.parent().ok_or(WorkspaceError::Identity)?)?.sync_all()?;
            result.state.validate()?;
            Ok(result)
        }
        fn from_root(
            path: PathBuf,
            root: File,
            scope: Scope,
            limits: WorkspaceLimits,
            header: Vec<u8>,
        ) -> Result<Self, WorkspaceError> {
            let identity = Digest::of("celetixo/workspace-registry/v1", &header);
            let lock = regular(&root, LOCK, true)?;
            let result = Self {
                state: Arc::new(RegistryState {
                    path,
                    root,
                    lock,
                    scope,
                    limits,
                    header,
                    identity,
                }),
            };
            result.state.validate()?;
            regular(&result.state.root, LOCK, true)?;
            Ok(result)
        }
        pub fn path(&self) -> &Path {
            &self.state.path
        }
        pub fn scope(&self) -> &Scope {
            &self.state.scope
        }
        pub fn identity(&self) -> Digest {
            self.state.identity
        }

        pub(crate) fn cleanup_retained(
            source: RetainedSource<'_>,
            authorize: impl Fn() -> io::Result<()>,
        ) -> Result<crate::materialize::CleanupOutcome, WorkspaceError> {
            let parent = source.path.parent().ok_or(WorkspaceError::Identity)?;
            if !source
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(PREFIX)
                        && name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })
            {
                return Err(WorkspaceError::Identity);
            }
            let root = open_dir(parent)?;
            let header: Header = serde_json::from_slice(&read(&root, HEADER)?)
                .map_err(|e| WorkspaceError::Metadata(e.to_string()))?;
            let registry = Self::open(parent, source.scope.clone(), header.limits)?;
            let _lock = registry.state.lock()?;
            let root = open_dir(source.path)?;
            private_dir(source.path, &root)?;
            let owner = regular(&root, OWNER, true)?;
            if !try_lock(&owner)? {
                return Ok(crate::materialize::CleanupOutcome::Quarantined {
                    path: source.path.into(),
                    reason: "source owner is still active".into(),
                });
            }
            let (record, bytes) = registry.state.record(&root)?;
            if record.admission != source.admission
                || record.candidate != source.candidate
                || record.manifest != source.manifest
                || read(&root, RETENTION)? != source.latch
            {
                return Err(WorkspaceError::Identity);
            }
            bounded_census(source.path, registry.state.limits.max_entries_per_root)?;
            authorize()?;
            registry.state.validate()?;
            private_dir(source.path, &root)?;
            if read(&root, RECORD)? != bytes || read(&root, RETENTION)? != source.latch {
                return Err(WorkspaceError::Identity);
            }
            fs::remove_dir_all(source.path)?;
            registry.state.root.sync_all()?;
            Ok(crate::materialize::CleanupOutcome::Removed)
        }

        pub fn materialize(
            &self,
            engine: &Engine,
            executor: &SourceExecutorCapability,
            admission: &SourceAdmission,
            base: &Snapshot,
            patch: &SourcePatch,
            mut hook: impl FnMut(MaterializationEvent) -> io::Result<()>,
        ) -> Result<PrivateWorkspace, WorkspaceError> {
            engine.validate_source_attempt(executor, admission)?;
            if &admission.binding().scope != self.scope()
                || patch.id() != admission.patch()
                || base.id() != admission.binding().base_snapshot
            {
                return Err(WorkspaceError::Identity);
            }
            let application = engine.source_application(executor, admission)?;
            let mut bytes = 0_u64;
            let mut directories = std::collections::BTreeSet::new();
            for input in &application.snapshot().manifest().inputs {
                bytes = bytes
                    .checked_add(input.bytes.as_ref().map_or(0, |b| b.len() as u64))
                    .ok_or(WorkspaceError::Limit)?;
                for (i, _) in input.path.match_indices('/') {
                    directories.insert(input.path[..i].to_owned());
                }
            }
            let entries = application
                .snapshot()
                .manifest()
                .inputs
                .len()
                .checked_add(directories.len())
                .and_then(|n| n.checked_add(8))
                .ok_or(WorkspaceError::Limit)?;
            // The registry enforces current authority itself. A caller's hook
            // can narrow progress but cannot opt out of Engine revalidation.
            let mut checked = |event| {
                engine
                    .validate_source_attempt(executor, admission)
                    .map_err(io::Error::other)?;
                hook(event)?;
                engine
                    .validate_source_attempt(executor, admission)
                    .map(|_| ())
                    .map_err(io::Error::other)
            };
            let allocation = self.allocate(admission, bytes, entries, &mut checked)?;
            engine.validate_source_attempt(executor, admission)?;
            let workspace =
                PrivateWorkspace::materialize_registered(base, patch, allocation, checked)?;
            engine.validate_source_attempt(executor, admission)?;
            Ok(workspace)
        }
        fn allocate(
            &self,
            admission: &SourceAdmission,
            bytes: u64,
            entries: usize,
            hook: &mut impl FnMut(MaterializationEvent) -> io::Result<()>,
        ) -> Result<RegisteredRoot, WorkspaceError> {
            let _lock = self.state.lock()?;
            let roots = self.state.roots()?;
            if roots.len() >= self.state.limits.max_roots
                || entries > self.state.limits.max_entries_per_root
            {
                return Err(WorkspaceError::Limit);
            }
            let mut reserved = bytes;
            for path in roots {
                let root = open_dir(&path)?;
                private_dir(&path, &root)?;
                let (record, _) = self.state.record(&root)?;
                reserved = reserved
                    .checked_add(record.input_bytes)
                    .ok_or(WorkspaceError::Limit)?;
            }
            if reserved > self.state.limits.max_total_input_bytes {
                return Err(WorkspaceError::Limit);
            }
            let mut root = tempfile::Builder::new()
                .prefix(PREFIX)
                .tempdir_in(&self.state.path)?;
            // Failed or killed allocations remain owned by explicit recovery.
            root.disable_cleanup(true);
            let root_fd = open_dir(root.path())?;
            rustix::fs::fchmod(&root_fd, Mode::from_raw_mode(0o700))?;
            hook(MaterializationEvent::RegistryRootCreated(
                root.path().to_path_buf(),
            ))?;
            let owner = write_new(&root_fd, OWNER, b"")?;
            if !try_lock(&owner)? {
                return Err(WorkspaceError::Limit);
            }
            let metadata = root_fd.metadata()?;
            let record = Record {
                version: 1,
                registry: self.identity().to_string(),
                admission: admission.id().to_string(),
                patch: admission.patch().to_string(),
                base: admission.binding().base_snapshot.to_string(),
                candidate: admission.resulting_snapshot().to_string(),
                manifest: admission.resulting_manifest().to_string(),
                application: admission.application_report().to_string(),
                root_dev: metadata.dev(),
                root_ino: metadata.ino(),
                input_bytes: bytes,
                entries,
            };
            let record =
                serde_json::to_vec(&record).map_err(|e| WorkspaceError::Metadata(e.to_string()))?;
            write_new(&root_fd, PENDING, &record)?;
            hook(MaterializationEvent::BeforeOwnershipPublication(
                root.path().to_path_buf(),
            ))?;
            rustix::fs::renameat_with(
                &root_fd,
                PENDING,
                &root_fd,
                RECORD,
                rustix::fs::RenameFlags::NOREPLACE,
            )?;
            root_fd.sync_all()?;
            self.state.root.sync_all()?;
            let path = root.path().to_path_buf();
            Ok(RegisteredRoot {
                root,
                lease: WorkspaceLease {
                    registry: self.state.clone(),
                    root: root_fd,
                    owner,
                    path,
                    record,
                },
            })
        }

        pub fn recover(
            &self,
            engine: &Engine,
            authority: &SourceCleanupCapability,
        ) -> Result<WorkspaceRecovery, WorkspaceError> {
            self.recover_with_hook(engine, authority, |_| Ok(()))
        }
        pub fn recover_with_hook(
            &self,
            engine: &Engine,
            authority: &SourceCleanupCapability,
            mut before_delete: impl FnMut(&Path) -> io::Result<()>,
        ) -> Result<WorkspaceRecovery, WorkspaceError> {
            let authorize =
                || engine.validate_source_cleanup(authority, self.scope(), self.identity());
            authorize()?;
            let _lock = self.state.lock()?;
            let mut result = WorkspaceRecovery::default();
            for path in self.state.roots()? {
                authorize()?;
                self.state.validate()?;
                let inspected = (|| -> Result<Option<(File, Option<File>)>, WorkspaceError> {
                    let root = open_dir(&path)?;
                    private_dir(&path, &root)?;
                    // A durable retention marker is never overridden by a free
                    // owner lock. The runtime must first confirm termination.
                    match fs::symlink_metadata(path.join(RETENTION)) {
                        Ok(_) => {
                            return Err(WorkspaceError::Metadata(
                                "runtime-retained source requires runtime reconciliation".into(),
                            ));
                        }
                        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                        Err(e) => return Err(e.into()),
                    }
                    match regular(&root, OWNER, true) {
                        Ok(owner) if !try_lock(&owner)? => Ok(None),
                        Ok(owner) => {
                            // Keep the lock until deletion has been confirmed.
                            match fs::symlink_metadata(path.join(RECORD)) {
                                Ok(_) => {
                                    self.state.record(&root)?;
                                }
                                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                                    unpublished(&path, &root)?
                                }
                                Err(e) => return Err(e.into()),
                            }
                            Ok(Some((root, Some(owner))))
                        }
                        Err(WorkspaceError::Io(error))
                            if error.kind() == io::ErrorKind::NotFound =>
                        {
                            // No owner record can have launched: allocation holds
                            // the namespace lock through owner-record publication.
                            unpublished(&path, &root)?;
                            Ok(Some((root, None)))
                        }
                        Err(error) => Err(error),
                    }
                })();
                let (root, owner) = match inspected {
                    Ok(None) => {
                        result.live.push(path);
                        continue;
                    }
                    Ok(Some(held)) => held,
                    Err(error) => {
                        result.quarantined.push((path, error.to_string()));
                        continue;
                    }
                };
                let identity = root.metadata()?;
                before_delete(&path)?;
                authorize()?;
                self.state.validate()?;
                let actual = fs::symlink_metadata(&path)?;
                if !actual.is_dir() || !same(&actual, &identity) {
                    result
                        .quarantined
                        .push((path, "workspace identity changed before recovery".into()));
                    continue;
                }
                if fs::symlink_metadata(path.join(RETENTION)).is_ok() {
                    result
                        .quarantined
                        .push((path, "runtime retention appeared before recovery".into()));
                    continue;
                }
                bounded_census(&path, self.state.limits.max_entries_per_root)?;
                fs::remove_dir_all(&path)?;
                self.state.root.sync_all()?;
                drop(owner);
                result.removed.push(path);
            }
            Ok(result)
        }
    }
    // Allocation holds the namespace lock until canonical ownership is durable.
    // Before that publication no source may exist and no runtime may start. A
    // killed allocation can leave only these bounded, private metadata files.
    fn unpublished(path: &Path, root: &File) -> Result<(), WorkspaceError> {
        let mut count = 0;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name();
            if name != OWNER && name != PENDING {
                return Err(WorkspaceError::Identity);
            }
            read(root, name.to_str().ok_or(WorkspaceError::Identity)?)?;
            count += 1;
            if count > 2 {
                return Err(WorkspaceError::Identity);
            }
        }
        Ok(())
    }
    impl WorkspaceLease {
        #[cfg(test)]
        pub(crate) fn duplicate_owner_for_test(&self) -> io::Result<File> {
            self.owner.try_clone()
        }

        pub(crate) fn validate(&self) -> Result<(), WorkspaceError> {
            self.registry.validate()?;
            private_dir(&self.path, &self.root)?;
            let owner = regular(&self.root, OWNER, true)?;
            if !same(&owner.metadata()?, &self.owner.metadata()?)
                || read(&self.root, RECORD)? != self.record
            {
                return Err(WorkspaceError::Identity);
            }
            self.registry.record(&self.root)?;
            Ok(())
        }
        pub(crate) fn confirm_removal(&self) -> Result<(), WorkspaceError> {
            self.registry.validate()?;
            self.registry.root.sync_all()?;
            Ok(())
        }
    }
    fn header(
        scope: &Scope,
        limits: WorkspaceLimits,
        root: &File,
    ) -> Result<Vec<u8>, WorkspaceError> {
        let metadata = root.metadata()?;
        serde_json::to_vec(&Header {
            version: 1,
            organization: scope.organization.as_str().into(),
            workspace: scope.workspace.as_str().into(),
            repository: scope.repository.as_str().into(),
            limits,
            root_dev: metadata.dev(),
            root_ino: metadata.ino(),
        })
        .map_err(|e| WorkspaceError::Metadata(e.to_string()))
    }
    fn bounded_census(root: &Path, maximum: usize) -> Result<(), WorkspaceError> {
        let mut pending = vec![root.to_path_buf()];
        let mut count = 0_usize;
        while let Some(path) = pending.pop() {
            count = count.checked_add(1).ok_or(WorkspaceError::Limit)?;
            if count > maximum {
                return Err(WorkspaceError::Limit);
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() {
                for child in fs::read_dir(&path)? {
                    if pending.len().saturating_add(count) >= maximum {
                        return Err(WorkspaceError::Limit);
                    }
                    pending.push(child?.path());
                }
            }
        }
        Ok(())
    }
}
