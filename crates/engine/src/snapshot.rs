//! Immutable, content-addressed inputs for the deterministic reference state.
//!
//! This module does not scan a filesystem or establish that a producer's census is truthful.
//! Producers must enumerate every participating input in every declared layer. Construction
//! validates the supported capture contract and freezes its bytes. Canonicalization v1 uses
//! exact, case-sensitive UTF-8 paths, `/` separators, raw file bytes, explicit modes, and
//! symlink target bytes without resolving or following them. It performs no Unicode, case,
//! newline, encoding, or filesystem normalization.

use crate::ids::{Canonical, Digest, ElementId, Revision, Scope, SnapshotId};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Numeric engineering elements for the reference environment, not a source-code index.
pub type State = BTreeMap<ElementId, i64>;

/// Independent captured views; the same path in several layers is intentionally distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputLayer {
    Base,
    Index,
    WorkingTree,
    Untracked,
    Overlay,
    Generated,
    Mounted,
    Submodule,
}

impl InputLayer {
    pub const ALL: [Self; 8] = [
        Self::Base,
        Self::Index,
        Self::WorkingTree,
        Self::Untracked,
        Self::Overlay,
        Self::Generated,
        Self::Mounted,
        Self::Submodule,
    ];

    fn tag(self) -> u64 {
        match self {
            Self::Base => 0,
            Self::Index => 1,
            Self::WorkingTree => 2,
            Self::Untracked => 3,
            Self::Overlay => 4,
            Self::Generated => 5,
            Self::Mounted => 6,
            Self::Submodule => 7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    File,
    Symlink,
    /// Opaque, nonempty submodule reference bytes; dirty contents are separate inputs.
    Submodule,
}

impl InputKind {
    fn tag(self) -> u64 {
        match self {
            Self::File => 0,
            Self::Symlink => 1,
            Self::Submodule => 2,
        }
    }
}

/// Supported Git-compatible mode distinctions. Unsupported filesystem objects reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Regular,
    Executable,
    Symlink,
    Gitlink,
}

impl FileMode {
    fn tag(self) -> u64 {
        match self {
            Self::Regular => 0o100644,
            Self::Executable => 0o100755,
            Self::Symlink => 0o120000,
            Self::Gitlink => 0o160000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceInput {
    pub path: String,
    pub layer: InputLayer,
    pub kind: InputKind,
    pub mode: FileMode,
    /// `None` is an explicit deletion/absence; `Some(vec![])` is an empty file.
    pub bytes: Option<Vec<u8>>,
}

/// Versioned capture contract. Every layer must be covered, even when it has no inputs.
/// Exclusions are exact paths with a recorded rationale; they cannot hide included inputs.
/// The caller remains responsible for establishing that excluded paths do not participate
/// in analysis/execution. No automatic glob, ignore-file, or default exclusion is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InclusionPolicy {
    pub version: u32,
    pub captured_layers: BTreeSet<InputLayer>,
    pub excluded_paths: BTreeMap<String, String>,
}

impl InclusionPolicy {
    pub fn complete() -> Self {
        Self {
            version: 1,
            captured_layers: InputLayer::ALL.into_iter().collect(),
            excluded_paths: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub base_commit: String,
    pub canonicalization_version: u32,
    pub inclusion_policy: InclusionPolicy,
    pub provenance: String,
    pub inputs: Vec<WorkspaceInput>,
    pub environment: BTreeMap<String, String>,
}

impl SnapshotManifest {
    /// Address the complete input manifest independently of the reference state.
    pub fn digest(&self) -> Digest {
        Digest::of("celetixo/source-manifest/v1", &self.canonical_bytes())
    }

    /// Complete input bytes and provenance for immutable artifact retention.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(&self.base_commit);
        c.u64(u64::from(self.canonicalization_version));
        c.str(&self.provenance);
        c.u64(u64::from(self.inclusion_policy.version));
        c.u64(self.inclusion_policy.captured_layers.len() as u64);
        for layer in &self.inclusion_policy.captured_layers {
            c.u64(layer.tag());
        }
        c.u64(self.inclusion_policy.excluded_paths.len() as u64);
        for (path, reason) in &self.inclusion_policy.excluded_paths {
            c.str(path);
            c.str(reason);
        }
        let mut inputs = self.inputs.iter().collect::<Vec<_>>();
        inputs.sort_by_key(|input| (input.layer, &input.path));
        c.u64(inputs.len() as u64);
        for input in inputs {
            c.str(&input.path);
            c.u64(input.layer.tag());
            c.u64(input.kind.tag());
            c.u64(input.mode.tag());
            match &input.bytes {
                Some(bytes) => {
                    c.u64(1);
                    c.bytes(bytes);
                }
                None => c.u64(0),
            }
        }
        c.u64(self.environment.len() as u64);
        for (key, value) in &self.environment {
            c.str(key);
            c.str(value);
        }
        c.into_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EffectiveWorkspaceError {
    #[error("external input conflicts with an effective workspace entry at {0:?}")]
    ExternalCollision(String),
    #[error("effective input crosses a file or symlink ancestor at {0:?}")]
    NonDirectoryAncestor(String),
}

/// Merge captured views without consulting a filesystem. Every original layer
/// remains part of snapshot identity; generated/mounted aliases require exact
/// agreement and editor overlays, including tombstones, apply last.
pub fn effective_workspace(
    snapshot: &Snapshot,
) -> Result<BTreeMap<String, WorkspaceInput>, EffectiveWorkspaceError> {
    let mut files: BTreeMap<String, WorkspaceInput> = BTreeMap::new();
    for layer in [
        InputLayer::Base,
        InputLayer::Index,
        InputLayer::WorkingTree,
        InputLayer::Untracked,
        InputLayer::Generated,
        InputLayer::Mounted,
        InputLayer::Submodule,
        InputLayer::Overlay,
    ] {
        for input in snapshot
            .manifest()
            .inputs
            .iter()
            .filter(|i| i.layer == layer)
        {
            match &input.bytes {
                Some(_) => {
                    if let Some(existing) = files.get(&input.path) {
                        if matches!(layer, InputLayer::Generated | InputLayer::Mounted)
                            && (existing.kind != input.kind
                                || existing.mode != input.mode
                                || existing.bytes != input.bytes)
                        {
                            return Err(EffectiveWorkspaceError::ExternalCollision(
                                input.path.clone(),
                            ));
                        }
                        if layer == InputLayer::Submodule && existing.kind != InputKind::Submodule {
                            return Err(EffectiveWorkspaceError::ExternalCollision(
                                input.path.clone(),
                            ));
                        }
                    }
                    files.insert(input.path.clone(), input.clone());
                }
                None => {
                    files.remove(&input.path);
                }
            }
        }
    }
    for path in files.keys() {
        for (position, _) in path.match_indices('/') {
            if files
                .get(&path[..position])
                .is_some_and(|file| file.kind != InputKind::Submodule)
            {
                return Err(EffectiveWorkspaceError::NonDirectoryAncestor(path.clone()));
            }
        }
    }
    Ok(files)
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SnapshotError {
    #[error("unsupported canonicalization version {0}")]
    UnsupportedCanonicalization(u32),
    #[error("unsupported inclusion policy version {0}")]
    UnsupportedInclusionPolicy(u32),
    #[error("capture does not cover layer {0:?}")]
    IncompleteCapture(InputLayer),
    #[error("{0} must contain nonempty, non-control provenance")]
    MissingProvenance(&'static str),
    #[error("path is not canonical: {0:?}")]
    InvalidPath(String),
    #[error("duplicate input in {layer:?}: {path:?}")]
    DuplicateInput { layer: InputLayer, path: String },
    #[error("input conflicts with ancestor in {layer:?}: {path:?}")]
    AmbiguousHierarchy { layer: InputLayer, path: String },
    #[error("excluded path has no rationale: {0:?}")]
    MissingExclusionReason(String),
    #[error("excluded path is also included: {0:?}")]
    ExcludedInput(String),
    #[error("mode does not match kind: {0:?}")]
    InvalidMode(String),
    #[error("invalid symlink target or submodule reference: {0:?}")]
    InvalidReference(String),
    #[error("base layer cannot contain a deletion: {0:?}")]
    DeletedBaseInput(String),
    #[error("invalid environment key or value")]
    InvalidEnvironment,
    #[error("successor revision must advance exactly once")]
    InvalidSuccessorRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    id: SnapshotId,
    scope: Scope,
    revision: Revision,
    manifest: SnapshotManifest,
    state: State,
}

impl Snapshot {
    pub fn new(
        scope: Scope,
        revision: Revision,
        mut manifest: SnapshotManifest,
        state: State,
    ) -> Result<Self, SnapshotError> {
        validate_manifest(&manifest)?;
        manifest
            .inputs
            .sort_by(|a, b| (a.layer, &a.path).cmp(&(b.layer, &b.path)));
        let id = snapshot_id(&scope, revision, &manifest, &state);
        Ok(Self {
            id,
            scope,
            revision,
            manifest,
            state,
        })
    }

    pub fn id(&self) -> SnapshotId {
        self.id
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn manifest(&self) -> &SnapshotManifest {
        &self.manifest
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Complete identity bytes for immutable source/evidence retention.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        snapshot_bytes(&self.scope, self.revision, &self.manifest, &self.state)
    }

    /// Reference-state advancement preserves captured workspace inputs. A repository
    /// adapter that mutates source bytes must instead construct a fresh manifest.
    pub fn successor(&self, revision: Revision, state: State) -> Result<Self, SnapshotError> {
        if self.revision.0.checked_add(1) != Some(revision.0) {
            return Err(SnapshotError::InvalidSuccessorRevision);
        }
        Self::new(self.scope.clone(), revision, self.manifest.clone(), state)
    }
}

fn valid_description(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn validate_path(path: &str) -> Result<(), SnapshotError> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(SnapshotError::InvalidPath(path.into()));
    }
    Ok(())
}

fn validate_manifest(manifest: &SnapshotManifest) -> Result<(), SnapshotError> {
    if manifest.canonicalization_version != 1 {
        return Err(SnapshotError::UnsupportedCanonicalization(
            manifest.canonicalization_version,
        ));
    }
    let policy = &manifest.inclusion_policy;
    if policy.version != 1 {
        return Err(SnapshotError::UnsupportedInclusionPolicy(policy.version));
    }
    for layer in InputLayer::ALL {
        if !policy.captured_layers.contains(&layer) {
            return Err(SnapshotError::IncompleteCapture(layer));
        }
    }
    for (name, value) in [
        ("base_commit", manifest.base_commit.as_str()),
        ("provenance", manifest.provenance.as_str()),
    ] {
        if !valid_description(value) {
            return Err(SnapshotError::MissingProvenance(name));
        }
    }
    for (path, rationale) in &policy.excluded_paths {
        validate_path(path)?;
        if !valid_description(rationale) {
            return Err(SnapshotError::MissingExclusionReason(path.clone()));
        }
    }
    for (key, value) in &manifest.environment {
        if key.is_empty()
            || key.contains(['=', '\0'])
            || key.chars().any(char::is_control)
            || value.contains('\0')
        {
            return Err(SnapshotError::InvalidEnvironment);
        }
    }
    let mut paths = BTreeMap::new();
    for input in &manifest.inputs {
        validate_path(&input.path)?;
        if policy.excluded_paths.contains_key(&input.path) {
            return Err(SnapshotError::ExcludedInput(input.path.clone()));
        }
        if paths
            .insert(
                (input.layer, input.path.as_str()),
                (input.kind, input.bytes.is_some()),
            )
            .is_some()
        {
            return Err(SnapshotError::DuplicateInput {
                layer: input.layer,
                path: input.path.clone(),
            });
        }
        if !matches!(
            (input.kind, input.mode),
            (InputKind::File, FileMode::Regular | FileMode::Executable)
                | (InputKind::Symlink, FileMode::Symlink)
                | (InputKind::Submodule, FileMode::Gitlink)
        ) {
            return Err(SnapshotError::InvalidMode(input.path.clone()));
        }
        match &input.bytes {
            None if input.layer == InputLayer::Base => {
                return Err(SnapshotError::DeletedBaseInput(input.path.clone()));
            }
            Some(bytes)
                if input.kind != InputKind::File && (bytes.is_empty() || bytes.contains(&0)) =>
            {
                return Err(SnapshotError::InvalidReference(input.path.clone()));
            }
            _ => {}
        }
    }
    for ((layer, path), (_, present)) in &paths {
        if !present {
            continue;
        }
        for (end, _) in path.match_indices('/') {
            if let Some((kind, ancestor_present)) = paths.get(&(*layer, &path[..end]))
                && *ancestor_present
                && *kind != InputKind::Submodule
            {
                return Err(SnapshotError::AmbiguousHierarchy {
                    layer: *layer,
                    path: (*path).into(),
                });
            }
        }
    }
    Ok(())
}

fn snapshot_id(
    scope: &Scope,
    revision: Revision,
    manifest: &SnapshotManifest,
    state: &State,
) -> SnapshotId {
    SnapshotId(Digest::of(
        "celetixo/snapshot/v1",
        &snapshot_bytes(scope, revision, manifest, state),
    ))
}

fn snapshot_bytes(
    scope: &Scope,
    revision: Revision,
    manifest: &SnapshotManifest,
    state: &State,
) -> Vec<u8> {
    let mut c = Canonical::default();
    c.str(scope.organization.as_str());
    c.str(scope.workspace.as_str());
    c.str(scope.repository.as_str());
    c.u64(revision.0);
    c.u64(u64::from(manifest.canonicalization_version));
    c.str(&manifest.base_commit);
    c.str(&manifest.provenance);
    c.u64(u64::from(manifest.inclusion_policy.version));
    c.u64(manifest.inclusion_policy.captured_layers.len() as u64);
    for layer in &manifest.inclusion_policy.captured_layers {
        c.u64(layer.tag());
    }
    c.u64(manifest.inclusion_policy.excluded_paths.len() as u64);
    for (path, rationale) in &manifest.inclusion_policy.excluded_paths {
        c.str(path);
        c.str(rationale);
    }
    c.u64(manifest.inputs.len() as u64);
    for input in &manifest.inputs {
        c.str(&input.path);
        c.u64(input.layer.tag());
        c.u64(input.kind.tag());
        c.u64(input.mode.tag());
        match &input.bytes {
            Some(bytes) => {
                c.u64(1);
                c.bytes(bytes);
            }
            None => c.u64(0),
        }
    }
    c.u64(manifest.environment.len() as u64);
    for (key, value) in &manifest.environment {
        c.str(key);
        c.str(value);
    }
    c.u64(state.len() as u64);
    for (element, value) in state {
        c.str(element.as_str());
        c.i64(*value);
    }
    c.into_bytes()
}
