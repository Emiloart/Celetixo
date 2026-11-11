//! Checked reconstruction of immutable domain values from durable storage.
//!
//! These codecs restore data through existing validated constructors. They never
//! deserialize sessions, capabilities, reader objects, or transaction authority.
use crate::ids::{
    Digest, ElementId, OrganizationId, RepositoryId, Revision, Scope, SnapshotId, WorkspaceId,
};
use crate::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotError, SnapshotManifest,
    State, WorkspaceInput,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct SnapshotDecodeLimits {
    pub encoded_bytes: usize,
    pub collection_entries: usize,
    pub field_bytes: usize,
}

impl Default for SnapshotDecodeLimits {
    fn default() -> Self {
        Self {
            encoded_bytes: 64 * 1024 * 1024,
            collection_entries: 100_000,
            field_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RestoreError {
    #[error("durable snapshot exceeds its explicit decoding budget")]
    Limit,
    #[error("durable snapshot content digest differs from its reference")]
    DigestMismatch,
    #[error("durable snapshot encoding is malformed or unsupported")]
    Encoding,
    #[error("durable snapshot is not canonically encoded")]
    NonCanonical,
    #[error("durable snapshot contains an invalid scoped identity")]
    Identity,
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
}

/// Restore the existing Goal 01 canonical snapshot format without changing its
/// identity. Digest validity alone is insufficient: all constructor invariants
/// and exact canonical ordering are checked before returning a Snapshot.
pub fn restore_snapshot(
    expected: SnapshotId,
    bytes: &[u8],
    limits: SnapshotDecodeLimits,
) -> Result<Snapshot, RestoreError> {
    if bytes.len() > limits.encoded_bytes
        || limits.collection_entries == 0
        || limits.field_bytes == 0
    {
        return Err(RestoreError::Limit);
    }
    if SnapshotId(Digest::of("celetixo/snapshot/v1", bytes)) != expected {
        return Err(RestoreError::DigestMismatch);
    }
    let mut r = Reader {
        bytes,
        offset: 0,
        limits,
        entries: 0,
    };
    let scope = Scope {
        organization: OrganizationId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        workspace: WorkspaceId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        repository: RepositoryId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
    };
    let revision = Revision(r.number()?);
    let canonicalization_version = r.version()?;
    let base_commit = r.text()?;
    let provenance = r.text()?;
    let version = r.version()?;
    let mut captured_layers = BTreeSet::new();
    for _ in 0..r.count()? {
        let layer = r.layer()?;
        if !captured_layers.insert(layer) {
            return Err(RestoreError::NonCanonical);
        }
    }
    let mut excluded_paths = BTreeMap::new();
    for _ in 0..r.count()? {
        let path = r.text()?;
        if excluded_paths.insert(path, r.text()?).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    let mut inputs = Vec::new();
    for _ in 0..r.count()? {
        let path = r.text()?;
        let layer = r.layer()?;
        let kind = match r.number()? {
            0 => InputKind::File,
            1 => InputKind::Symlink,
            2 => InputKind::Submodule,
            _ => return Err(RestoreError::Encoding),
        };
        let mode = match r.number()? {
            0o100644 => FileMode::Regular,
            0o100755 => FileMode::Executable,
            0o120000 => FileMode::Symlink,
            0o160000 => FileMode::Gitlink,
            _ => return Err(RestoreError::Encoding),
        };
        let bytes = match r.number()? {
            0 => None,
            1 => Some(r.field()?.to_vec()),
            _ => return Err(RestoreError::Encoding),
        };
        inputs.push(WorkspaceInput {
            path,
            layer,
            kind,
            mode,
            bytes,
        });
    }
    let mut environment = BTreeMap::new();
    for _ in 0..r.count()? {
        let key = r.text()?;
        if environment.insert(key, r.text()?).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    let mut state = State::new();
    for _ in 0..r.count()? {
        let key = ElementId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
        let value = i64::from_be_bytes(r.field()?.try_into().map_err(|_| RestoreError::Encoding)?);
        if state.insert(key, value).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    if r.offset != bytes.len() {
        return Err(RestoreError::NonCanonical);
    }
    let snapshot = Snapshot::new(
        scope,
        revision,
        SnapshotManifest {
            base_commit,
            canonicalization_version,
            inclusion_policy: InclusionPolicy {
                version,
                captured_layers,
                excluded_paths,
            },
            provenance,
            inputs,
            environment,
        },
        state,
    )?;
    if snapshot.id() != expected || snapshot.canonical_bytes() != bytes {
        return Err(RestoreError::NonCanonical);
    }
    Ok(snapshot)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
    limits: SnapshotDecodeLimits,
    entries: usize,
}

impl<'a> Reader<'a> {
    fn field(&mut self) -> Result<&'a [u8], RestoreError> {
        let prefix_end = self.offset.checked_add(8).ok_or(RestoreError::Encoding)?;
        let prefix = self
            .bytes
            .get(self.offset..prefix_end)
            .ok_or(RestoreError::Encoding)?;
        let length = usize::try_from(u64::from_be_bytes(
            prefix.try_into().map_err(|_| RestoreError::Encoding)?,
        ))
        .map_err(|_| RestoreError::Limit)?;
        if length > self.limits.field_bytes {
            return Err(RestoreError::Limit);
        }
        let end = prefix_end.checked_add(length).ok_or(RestoreError::Limit)?;
        let value = self
            .bytes
            .get(prefix_end..end)
            .ok_or(RestoreError::Encoding)?;
        self.offset = end;
        Ok(value)
    }
    fn text(&mut self) -> Result<String, RestoreError> {
        String::from_utf8(self.field()?.to_vec()).map_err(|_| RestoreError::Encoding)
    }
    fn number(&mut self) -> Result<u64, RestoreError> {
        Ok(u64::from_be_bytes(
            self.field()?
                .try_into()
                .map_err(|_| RestoreError::Encoding)?,
        ))
    }
    fn version(&mut self) -> Result<u32, RestoreError> {
        u32::try_from(self.number()?).map_err(|_| RestoreError::Encoding)
    }
    fn count(&mut self) -> Result<usize, RestoreError> {
        let count = usize::try_from(self.number()?).map_err(|_| RestoreError::Limit)?;
        self.entries = self.entries.checked_add(count).ok_or(RestoreError::Limit)?;
        if self.entries > self.limits.collection_entries
            || count > (self.bytes.len() - self.offset) / 8
        {
            return Err(RestoreError::Limit);
        }
        Ok(count)
    }
    fn layer(&mut self) -> Result<InputLayer, RestoreError> {
        match self.number()? {
            0 => Ok(InputLayer::Base),
            1 => Ok(InputLayer::Index),
            2 => Ok(InputLayer::WorkingTree),
            3 => Ok(InputLayer::Untracked),
            4 => Ok(InputLayer::Overlay),
            5 => Ok(InputLayer::Generated),
            6 => Ok(InputLayer::Mounted),
            7 => Ok(InputLayer::Submodule),
            _ => Err(RestoreError::Encoding),
        }
    }
}
