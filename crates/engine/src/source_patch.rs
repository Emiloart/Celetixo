//! Canonical, bounded source patches and deterministic T0 application.
//!
//! This is an in-process domain format, not a competing RPC protocol. Application
//! neither grants authorization nor executes programs or advances a repository.
//! All preimages refer to the same immutable base. Renames are simultaneous.

use crate::ids::{
    Canonical, Digest, PrincipalId, Revision, Scope, SnapshotId, SourceAttemptId, SourcePatchId,
    TransactionId,
};
use crate::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotManifest, WorkspaceInput,
    effective_workspace,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

const SCHEMA: u64 = 1;
const FORMAT: &str = "celetixo/source-patch/v1";
const PAYLOAD_DOMAIN: &str = "celetixo/source-content/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProfile {
    /// Exact ASCII bytes, portable component rules and case-fold collision checks.
    /// Non-ASCII paths fail explicitly; no Unicode normalization is performed.
    PortableAsciiV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymlinkPolicy {
    /// No resulting symlinks, including inherited ones, are permitted.
    Deny,
    /// UTF-8 relative links must resolve inside the final immutable source graph.
    InternalRelativeOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchPolicy {
    pub version: u32,
    pub path_profile: PathProfile,
    pub symlinks: SymlinkPolicy,
    pub max_operations: usize,
    pub max_payload_bytes: u64,
    pub max_total_payload_bytes: u64,
    pub max_result_files: usize,
    pub max_result_bytes: u64,
    pub max_path_bytes: usize,
    pub max_depth: usize,
}

impl Default for PatchPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            path_profile: PathProfile::PortableAsciiV1,
            symlinks: SymlinkPolicy::Deny,
            max_operations: 4096,
            max_payload_bytes: 16 * 1024 * 1024,
            max_total_payload_bytes: 32 * 1024 * 1024,
            max_result_files: 100_000,
            max_result_bytes: 256 * 1024 * 1024,
            max_path_bytes: 4096,
            max_depth: 64,
        }
    }
}

impl PatchPolicy {
    pub fn digest(&self) -> Digest {
        let mut c = Canonical::default();
        encode_policy(&mut c, self);
        c.finish("celetixo/source-patch-policy/v1")
    }

    fn validate(&self) -> Result<(), SourcePatchError> {
        if self.version != 1 {
            return Err(SourcePatchError::UnsupportedVersion);
        }
        if self.max_operations == 0
            || self.max_operations > 65_536
            || self.max_payload_bytes == 0
            || self.max_payload_bytes > 256 * 1024 * 1024
            || self.max_total_payload_bytes < self.max_payload_bytes
            || self.max_total_payload_bytes > 512 * 1024 * 1024
            || self.max_result_files == 0
            || self.max_result_files > 1_000_000
            || self.max_result_bytes == 0
            || self.max_result_bytes > 1024 * 1024 * 1024
            || self.max_path_bytes == 0
            || self.max_path_bytes > 4096
            || self.max_depth == 0
            || self.max_depth > 256
        {
            return Err(SourcePatchError::InvalidPolicy);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchBinding {
    pub scope: Scope,
    pub principal: PrincipalId,
    pub transaction: TransactionId,
    pub transaction_attempt: u64,
    pub source_attempt: SourceAttemptId,
    pub base_snapshot: SnapshotId,
    pub base_revision: Revision,
    pub analysis_artifact: Digest,
    pub index_generation: Digest,
    pub impact_request: Digest,
    pub impact_result: Digest,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    /// Attribution only. This string conveys no authority or authenticated identity.
    pub producer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    bytes: Vec<u8>,
    digest: Digest,
}

impl Payload {
    pub fn new(bytes: Vec<u8>) -> Self {
        let digest = Digest::of(PAYLOAD_DOMAIN, &bytes);
        Self { bytes, digest }
    }

    pub fn with_digest(bytes: Vec<u8>, expected: Digest) -> Result<Self, SourcePatchError> {
        let result = Self::new(bytes);
        if result.digest != expected {
            return Err(SourcePatchError::PayloadDigest);
        }
        Ok(result)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn digest(&self) -> Digest {
        self.digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchEntry {
    kind: InputKind,
    mode: FileMode,
    payload: Payload,
}

impl PatchEntry {
    pub fn file(bytes: Vec<u8>, mode: FileMode) -> Result<Self, SourcePatchError> {
        Self::new(InputKind::File, mode, Payload::new(bytes))
    }

    pub fn symlink(target: Vec<u8>) -> Result<Self, SourcePatchError> {
        Self::new(InputKind::Symlink, FileMode::Symlink, Payload::new(target))
    }

    pub fn new(
        kind: InputKind,
        mode: FileMode,
        payload: Payload,
    ) -> Result<Self, SourcePatchError> {
        if !matches!(
            (kind, mode),
            (InputKind::File, FileMode::Regular | FileMode::Executable)
                | (InputKind::Symlink, FileMode::Symlink)
        ) {
            return Err(SourcePatchError::InvalidEntry);
        }
        if kind == InputKind::Symlink && (payload.bytes.is_empty() || payload.bytes.contains(&0)) {
            return Err(SourcePatchError::InvalidEntry);
        }
        Ok(Self {
            kind,
            mode,
            payload,
        })
    }

    pub fn kind(&self) -> InputKind {
        self.kind
    }
    pub fn mode(&self) -> FileMode {
        self.mode
    }
    pub fn payload(&self) -> &Payload {
        &self.payload
    }

    fn at(&self, path: String) -> WorkspaceInput {
        WorkspaceInput {
            path,
            layer: InputLayer::Base,
            kind: self.kind,
            mode: self.mode,
            bytes: Some(self.payload.bytes.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preimage {
    pub kind: InputKind,
    pub mode: FileMode,
    pub content: Digest,
}

impl Preimage {
    pub fn of(input: &WorkspaceInput) -> Result<Self, SourcePatchError> {
        let bytes = input.bytes.as_ref().ok_or(SourcePatchError::InvalidEntry)?;
        Ok(Self {
            kind: input.kind,
            mode: input.mode,
            content: Digest::of(PAYLOAD_DOMAIN, bytes),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperationKind {
    Create,
    Replace,
    Delete,
    Rename,
    SetMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchOperation {
    Create {
        path: String,
        entry: PatchEntry,
    },
    Replace {
        path: String,
        expected: Preimage,
        entry: PatchEntry,
    },
    Delete {
        path: String,
        expected: Preimage,
    },
    Rename {
        from: String,
        to: String,
        expected: Preimage,
    },
    SetMode {
        path: String,
        expected: Preimage,
        mode: FileMode,
    },
}

impl PatchOperation {
    pub fn kind(&self) -> OperationKind {
        match self {
            Self::Create { .. } => OperationKind::Create,
            Self::Replace { .. } => OperationKind::Replace,
            Self::Delete { .. } => OperationKind::Delete,
            Self::Rename { .. } => OperationKind::Rename,
            Self::SetMode { .. } => OperationKind::SetMode,
        }
    }

    pub fn source_path(&self) -> &str {
        match self {
            Self::Create { path, .. }
            | Self::Replace { path, .. }
            | Self::Delete { path, .. }
            | Self::SetMode { path, .. } => path,
            Self::Rename { from, .. } => from,
        }
    }

    pub fn destination_path(&self) -> Option<&str> {
        match self {
            Self::Rename { to, .. } => Some(to),
            _ => None,
        }
    }

    fn key(&self) -> (OperationKind, &str, Option<&str>) {
        (self.kind(), self.source_path(), self.destination_path())
    }

    fn expected(&self) -> Option<Preimage> {
        match self {
            Self::Create { .. } => None,
            Self::Replace { expected, .. }
            | Self::Delete { expected, .. }
            | Self::Rename { expected, .. }
            | Self::SetMode { expected, .. } => Some(*expected),
        }
    }

    fn entry(&self) -> Option<&PatchEntry> {
        match self {
            Self::Create { entry, .. } | Self::Replace { entry, .. } => Some(entry),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    pub max_envelope_bytes: usize,
    pub max_operations: usize,
    pub max_payload_bytes: usize,
    pub max_total_payload_bytes: usize,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_envelope_bytes: 40 * 1024 * 1024,
            max_operations: 4096,
            max_payload_bytes: 16 * 1024 * 1024,
            max_total_payload_bytes: 32 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourcePatchError {
    #[error("unsupported source-patch schema, format, or policy version")]
    UnsupportedVersion,
    #[error("invalid source-patch policy")]
    InvalidPolicy,
    #[error("patch binding has invalid expiry or producer provenance")]
    InvalidBinding,
    #[error("source patch must contain at least one effective operation")]
    Empty,
    #[error("source-patch limit exceeded: {0}")]
    Limit(&'static str),
    #[error("noncanonical or malformed source-patch encoding")]
    InvalidEncoding,
    #[error("binary payload digest mismatch")]
    PayloadDigest,
    #[error("invalid source entry kind, mode, or reference")]
    InvalidEntry,
    #[error("unsupported or ambiguous source path: {0:?}")]
    InvalidPath(String),
    #[error("reserved operational control path: {0:?}")]
    ReservedPath(String),
    #[error("unknown reserved source-application environment input: {0:?}")]
    ReservedEnvironment(String),
    #[error("duplicate or conflicting operations at {0:?}")]
    ConflictingOperations(String),
    #[error("patch scope, snapshot or revision differs from its exact base")]
    WrongBase,
    #[error("base effective inputs are inconsistent")]
    InvalidBase,
    #[error("path is absent or its kind, mode or digest differs from the preimage: {0:?}")]
    PreimageMismatch(String),
    #[error("create or rename destination is occupied: {0:?}")]
    OccupiedDestination(String),
    #[error("operation has no effect at {0:?}")]
    NoOp(String),
    #[error("operation intersects a source exclusion: {0:?}")]
    ExcludedPath(String),
    #[error("case-fold or file/directory collision: {0:?}")]
    PathCollision(String),
    #[error("symlinks are forbidden by the selected policy: {0:?}")]
    SymlinkDenied(String),
    #[error("symlink target is escaping, absent, cyclic or unsupported: {0:?}")]
    UnsafeSymlink(String),
    #[error("source application report or resulting snapshot does not match independent replay")]
    InvalidReport,
    #[error("resulting immutable snapshot is invalid")]
    InvalidResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePatch {
    binding: PatchBinding,
    policy: PatchPolicy,
    operations: Vec<PatchOperation>,
    affected_paths: BTreeSet<String>,
    operation_digest: Digest,
    id: SourcePatchId,
}

impl SourcePatch {
    pub fn new(
        binding: PatchBinding,
        policy: PatchPolicy,
        mut operations: Vec<PatchOperation>,
    ) -> Result<Self, SourcePatchError> {
        policy.validate()?;
        if binding.expires_at_ms <= binding.created_at_ms
            || binding.producer.trim().is_empty()
            || binding.producer.len() > 4096
            || binding.producer.chars().any(char::is_control)
        {
            return Err(SourcePatchError::InvalidBinding);
        }
        if operations.is_empty() {
            return Err(SourcePatchError::Empty);
        }
        if operations.len() > policy.max_operations {
            return Err(SourcePatchError::Limit("operation count"));
        }
        operations.sort_by(|a, b| a.key().cmp(&b.key()));
        let mut ordinary = BTreeSet::new();
        let mut rename_sources = BTreeSet::new();
        let mut rename_destinations = BTreeSet::new();
        let mut affected_paths = BTreeSet::new();
        let mut payload_bytes = 0_u64;
        for operation in &operations {
            let path = operation.source_path();
            validate_path(path, &policy)?;
            affected_paths.insert(path.to_owned());
            if let Some(to) = operation.destination_path() {
                validate_path(to, &policy)?;
                affected_paths.insert(to.to_owned());
                if path == to {
                    return Err(SourcePatchError::NoOp(path.into()));
                }
                if !rename_sources.insert(path) || !rename_destinations.insert(to) {
                    return Err(SourcePatchError::ConflictingOperations(path.into()));
                }
            } else if !ordinary.insert(path) {
                return Err(SourcePatchError::ConflictingOperations(path.into()));
            }
            if let Some(entry) = operation.entry() {
                let length = entry.payload.bytes.len() as u64;
                if length > policy.max_payload_bytes {
                    return Err(SourcePatchError::Limit("individual payload bytes"));
                }
                payload_bytes = payload_bytes
                    .checked_add(length)
                    .ok_or(SourcePatchError::Limit("payload byte overflow"))?;
                if payload_bytes > policy.max_total_payload_bytes {
                    return Err(SourcePatchError::Limit("aggregate payload bytes"));
                }
                if entry.kind == InputKind::Symlink && policy.symlinks == SymlinkPolicy::Deny {
                    return Err(SourcePatchError::SymlinkDenied(path.into()));
                }
            }
            if let Some(expected) = operation.expected()
                && !matches!(
                    (expected.kind, expected.mode),
                    (InputKind::File, FileMode::Regular | FileMode::Executable)
                        | (InputKind::Symlink, FileMode::Symlink)
                        | (InputKind::Submodule, FileMode::Gitlink)
                )
            {
                return Err(SourcePatchError::InvalidEntry);
            }
            if let PatchOperation::SetMode { mode, expected, .. } = operation
                && (expected.kind != InputKind::File
                    || !matches!(mode, FileMode::Regular | FileMode::Executable))
            {
                return Err(SourcePatchError::InvalidEntry);
            }
        }
        if let Some(path) = ordinary
            .iter()
            .find(|p| rename_sources.contains(*p) || rename_destinations.contains(*p))
        {
            return Err(SourcePatchError::ConflictingOperations((*path).into()));
        }
        let mut c = Canonical::default();
        encode_operations(&mut c, &operations);
        let operation_digest = c.finish("celetixo/source-patch-operations/v1");
        let mut result = Self {
            binding,
            policy,
            operations,
            affected_paths,
            operation_digest,
            id: SourcePatchId(Digest::of(FORMAT, b"uninitialized")),
        };
        result.id = SourcePatchId(Digest::of(FORMAT, &result.canonical_bytes()));
        Ok(result)
    }

    pub fn id(&self) -> SourcePatchId {
        self.id
    }
    pub fn binding(&self) -> &PatchBinding {
        &self.binding
    }
    pub fn policy(&self) -> &PatchPolicy {
        &self.policy
    }
    pub fn operations(&self) -> &[PatchOperation] {
        &self.operations
    }
    pub fn affected_paths(&self) -> &BTreeSet<String> {
        &self.affected_paths
    }
    pub fn operation_digest(&self) -> Digest {
        self.operation_digest
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(FORMAT);
        c.u64(SCHEMA);
        encode_binding(&mut c, &self.binding);
        encode_policy(&mut c, &self.policy);
        encode_operations(&mut c, &self.operations);
        c.digest(self.operation_digest);
        c.into_bytes()
    }

    pub fn from_canonical_bytes(
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<Self, SourcePatchError> {
        if bytes.len() > limits.max_envelope_bytes {
            return Err(SourcePatchError::Limit("envelope bytes"));
        }
        let mut d = Decoder {
            bytes,
            offset: 0,
            limits,
            payload_bytes: 0,
        };
        if d.string(64)? != FORMAT || d.number()? != SCHEMA {
            return Err(SourcePatchError::UnsupportedVersion);
        }
        let binding = d.binding()?;
        let policy = d.policy()?;
        policy.validate()?;
        let count = d.count(limits.max_operations.min(policy.max_operations))?;
        let mut operations = Vec::with_capacity(count);
        for _ in 0..count {
            operations.push(d.operation(&policy)?);
        }
        let supplied_operations_digest = d.digest()?;
        if d.offset != bytes.len() {
            return Err(SourcePatchError::InvalidEncoding);
        }
        let result = Self::new(binding, policy, operations)?;
        if result.operation_digest != supplied_operations_digest
            || result.canonical_bytes() != bytes
        {
            return Err(SourcePatchError::InvalidEncoding);
        }
        Ok(result)
    }

    /// Pure T0 transformation. No side effects, authorization decision or revision publication.
    pub fn apply(&self, base: &Snapshot) -> Result<AppliedSourcePatch, SourcePatchError> {
        if base.id() != self.binding.base_snapshot
            || base.scope() != &self.binding.scope
            || base.revision() != self.binding.base_revision
        {
            return Err(SourcePatchError::WrongBase);
        }
        let effective = effective_workspace(base).map_err(|_| SourcePatchError::InvalidBase)?;
        let mut files = BTreeMap::new();
        let mut non_operational_inputs = Vec::new();
        for (path, input) in effective {
            if is_known_git_metadata(&path)
                && input.layer == InputLayer::Mounted
                && input.kind == InputKind::File
            {
                non_operational_inputs.push(NonOperationalInput {
                    input,
                    reason:
                        "Captured Git administration is provenance, not an execution Git directory"
                            .into(),
                });
            } else {
                validate_path(&path, &self.policy)?;
                files.insert(path, input);
            }
        }
        let original_files = files.clone();
        let rename_sources: BTreeSet<_> = self
            .operations
            .iter()
            .filter_map(|op| match op {
                PatchOperation::Rename { from, .. } => Some(from.as_str()),
                _ => None,
            })
            .collect();
        for operation in &self.operations {
            for path in [Some(operation.source_path()), operation.destination_path()]
                .into_iter()
                .flatten()
            {
                if base
                    .manifest()
                    .inclusion_policy
                    .excluded_paths
                    .keys()
                    .any(|excluded| {
                        path == excluded
                            || path
                                .strip_prefix(excluded)
                                .is_some_and(|rest| rest.starts_with('/'))
                    })
                {
                    return Err(SourcePatchError::ExcludedPath(path.into()));
                }
            }
            let path = operation.source_path();
            match operation.expected() {
                Some(expected)
                    if files.get(path).map(Preimage::of).transpose()? != Some(expected) =>
                {
                    return Err(SourcePatchError::PreimageMismatch(path.into()));
                }
                None if files.contains_key(path) => {
                    return Err(SourcePatchError::OccupiedDestination(path.into()));
                }
                _ => {}
            }
            if let PatchOperation::Rename { to, .. } = operation
                && files.contains_key(to)
                && !rename_sources.contains(to.as_str())
            {
                return Err(SourcePatchError::OccupiedDestination(to.clone()));
            }
            if operation
                .expected()
                .is_some_and(|p| p.kind == InputKind::Submodule)
            {
                return Err(SourcePatchError::InvalidEntry);
            }
            if let Some(expected) = operation.expected()
                && expected.kind == InputKind::Symlink
                && self.policy.symlinks == SymlinkPolicy::Deny
            {
                return Err(SourcePatchError::SymlinkDenied(path.into()));
            }
            if let Some(entry) = operation.entry()
                && files.get(path).map(Preimage::of).transpose()?
                    == Some(Preimage {
                        kind: entry.kind,
                        mode: entry.mode,
                        content: entry.payload.digest,
                    })
            {
                return Err(SourcePatchError::NoOp(path.into()));
            }
            if let PatchOperation::SetMode { expected, mode, .. } = operation
                && expected.mode == *mode
            {
                return Err(SourcePatchError::NoOp(path.into()));
            }
        }
        // Removal and insertion are separate phases: every rename reads the base,
        // including swaps and cycles, and never observes another operation's write.
        for operation in &self.operations {
            if matches!(
                operation,
                PatchOperation::Delete { .. } | PatchOperation::Rename { .. }
            ) {
                files.remove(operation.source_path());
            }
        }
        for operation in &self.operations {
            match operation {
                PatchOperation::Create { path, entry }
                | PatchOperation::Replace { path, entry, .. } => {
                    files.insert(path.clone(), entry.at(path.clone()));
                }
                PatchOperation::Rename { from, to, .. } => {
                    let mut input = original_files
                        .get(from)
                        .ok_or(SourcePatchError::InvalidBase)?
                        .clone();
                    input.path = to.clone();
                    files.insert(to.clone(), input);
                }
                PatchOperation::SetMode { path, mode, .. } => {
                    files
                        .get_mut(path)
                        .ok_or(SourcePatchError::InvalidBase)?
                        .mode = *mode;
                }
                PatchOperation::Delete { .. } => {}
            }
        }
        validate_final_graph(&files, &self.policy)?;
        if logical_entries_equal(&files, &original_files) {
            return Err(SourcePatchError::Empty);
        }
        let base_manifest_digest = manifest_digest(base.manifest());
        let provenance_digest = application_provenance_digest(base, self, &non_operational_inputs);
        let mut environment = BTreeMap::new();
        for (key, value) in &base.manifest().environment {
            if is_capture_metadata_key(key) {
                environment.insert(
                    format!("celetixo.source_application.origin.{key}"),
                    value.clone(),
                );
            } else if matches!(
                key.as_str(),
                "celetixo.source_application.base_snapshot"
                    | "celetixo.source_application.base_manifest"
                    | "celetixo.source_application.patch"
                    | "celetixo.source_application.provenance"
            ) {
                // Previous application headers remain in the full base manifest
                // retained by this report and bound by its manifest digest.
            } else if let Some(origin) = key.strip_prefix("celetixo.source_application.origin.") {
                if !is_capture_metadata_key(origin) {
                    return Err(SourcePatchError::ReservedEnvironment(key.clone()));
                }
                environment.insert(key.clone(), value.clone());
            } else if key.starts_with("celetixo.source_application.") {
                return Err(SourcePatchError::ReservedEnvironment(key.clone()));
            } else {
                // Unknown/future application inputs are preserved. The namespace
                // alone does not make an input irrelevant or authoritative.
                environment.insert(key.clone(), value.clone());
            }
        }
        environment.extend([
            (
                "celetixo.source_application.base_snapshot".into(),
                base.id().to_string(),
            ),
            (
                "celetixo.source_application.base_manifest".into(),
                base_manifest_digest.to_string(),
            ),
            (
                "celetixo.source_application.patch".into(),
                self.id.to_string(),
            ),
            (
                "celetixo.source_application.provenance".into(),
                provenance_digest.to_string(),
            ),
        ]);
        let inputs = files
            .into_values()
            .map(|mut input| {
                input.layer = InputLayer::Base;
                input
            })
            .collect();
        let manifest = SnapshotManifest {
            base_commit: base.manifest().base_commit.clone(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy {
                excluded_paths: base.manifest().inclusion_policy.excluded_paths.clone(),
                ..InclusionPolicy::complete()
            },
            provenance: format!(
                "celetixo/source-application/v1;base={};patch={};provenance={provenance_digest}",
                base.id(),
                self.id
            ),
            inputs,
            environment,
        };
        let snapshot = Snapshot::new(
            base.scope().clone(),
            base.revision(),
            manifest,
            base.state().clone(),
        )
        .map_err(|_| SourcePatchError::InvalidResult)?;
        let mut outcomes = Vec::new();
        for operation in &self.operations {
            let destination = operation
                .destination_path()
                .unwrap_or(operation.source_path());
            outcomes.push(OperationOutcome {
                kind: operation.kind(),
                source: operation.source_path().into(),
                destination: operation.destination_path().map(str::to_owned),
                before: original_files
                    .get(operation.source_path())
                    .map(Preimage::of)
                    .transpose()?,
                after: snapshot
                    .manifest()
                    .inputs
                    .iter()
                    .find(|i| i.path == destination)
                    .map(Preimage::of)
                    .transpose()?,
            });
        }
        let report = ApplicationReport {
            base_snapshot: base.id(),
            base_manifest: base.manifest().clone(),
            base_manifest_digest,
            resulting_snapshot: snapshot.id(),
            resulting_manifest_digest: manifest_digest(snapshot.manifest()),
            patch: self.id,
            operation_digest: self.operation_digest,
            policy_digest: self.policy.digest(),
            provenance_digest,
            non_operational_inputs,
            outcomes,
        };
        Ok(AppliedSourcePatch { snapshot, report })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonOperationalInput {
    pub input: WorkspaceInput,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationOutcome {
    pub kind: OperationKind,
    pub source: String,
    pub destination: Option<String>,
    pub before: Option<Preimage>,
    pub after: Option<Preimage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationReport {
    base_snapshot: SnapshotId,
    base_manifest: SnapshotManifest,
    base_manifest_digest: Digest,
    resulting_snapshot: SnapshotId,
    resulting_manifest_digest: Digest,
    patch: SourcePatchId,
    operation_digest: Digest,
    policy_digest: Digest,
    provenance_digest: Digest,
    non_operational_inputs: Vec<NonOperationalInput>,
    outcomes: Vec<OperationOutcome>,
}

impl ApplicationReport {
    pub fn base_snapshot(&self) -> SnapshotId {
        self.base_snapshot
    }
    pub fn base_manifest(&self) -> &SnapshotManifest {
        &self.base_manifest
    }
    pub fn base_manifest_digest(&self) -> Digest {
        self.base_manifest_digest
    }
    pub fn resulting_snapshot(&self) -> SnapshotId {
        self.resulting_snapshot
    }
    pub fn resulting_manifest_digest(&self) -> Digest {
        self.resulting_manifest_digest
    }
    pub fn patch(&self) -> SourcePatchId {
        self.patch
    }
    pub fn operation_digest(&self) -> Digest {
        self.operation_digest
    }
    pub fn policy_digest(&self) -> Digest {
        self.policy_digest
    }
    pub fn provenance_digest(&self) -> Digest {
        self.provenance_digest
    }
    pub fn non_operational_inputs(&self) -> &[NonOperationalInput] {
        &self.non_operational_inputs
    }
    pub fn outcomes(&self) -> &[OperationOutcome] {
        &self.outcomes
    }

    pub fn digest(&self) -> Digest {
        let mut c = Canonical::default();
        c.digest(self.base_snapshot.0);
        c.digest(self.base_manifest_digest);
        c.digest(self.base_manifest.digest());
        c.digest(self.resulting_snapshot.0);
        c.digest(self.resulting_manifest_digest);
        c.digest(self.patch.0);
        c.digest(self.operation_digest);
        c.digest(self.policy_digest);
        c.digest(self.provenance_digest);
        c.u64(self.non_operational_inputs.len() as u64);
        for input in &self.non_operational_inputs {
            encode_input(&mut c, &input.input);
            c.str(&input.reason);
        }
        c.u64(self.outcomes.len() as u64);
        for outcome in &self.outcomes {
            c.u64(operation_tag(outcome.kind));
            c.str(&outcome.source);
            encode_optional_string(&mut c, outcome.destination.as_deref());
            encode_optional_preimage(&mut c, outcome.before);
            encode_optional_preimage(&mut c, outcome.after);
        }
        c.finish("celetixo/source-application-report/v1")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedSourcePatch {
    snapshot: Snapshot,
    report: ApplicationReport,
}

impl AppliedSourcePatch {
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub fn report(&self) -> &ApplicationReport {
        &self.report
    }

    /// Replay from the trusted immutable base; an executor-supplied success flag
    /// or a self-consistent digest is not used as proof of correct application.
    pub fn validate(&self, base: &Snapshot, patch: &SourcePatch) -> Result<(), SourcePatchError> {
        if &patch.apply(base)? != self {
            return Err(SourcePatchError::InvalidReport);
        }
        Ok(())
    }
}

fn logical_entries_equal(
    a: &BTreeMap<String, WorkspaceInput>,
    b: &BTreeMap<String, WorkspaceInput>,
) -> bool {
    a.len() == b.len()
        && a.iter().all(|(path, left)| {
            b.get(path).is_some_and(|right| {
                left.kind == right.kind && left.mode == right.mode && left.bytes == right.bytes
            })
        })
}

fn is_known_git_metadata(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    parts
        .iter()
        .position(|part| *part == ".git")
        .is_some_and(|index| {
            matches!(
                parts[index + 1..].join("/").as_str(),
                "config"
                    | "config.worktree"
                    | "info/attributes"
                    | "info/exclude"
                    | "configured-attributes"
            )
        })
}

fn is_capture_metadata_key(key: &str) -> bool {
    if matches!(
        key,
        "celetixo.capture.policy" | "celetixo.capture.uncertainty" | "celetixo.capture.consistency"
    ) {
        return true;
    }
    fn digest_suffix(key: &str, prefix: &str) -> bool {
        key.strip_prefix(prefix).is_some_and(|suffix| {
            suffix.len() == 64 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        })
    }
    if digest_suffix(key, "celetixo.overlay.")
        || digest_suffix(key, "celetixo.capture.directory.")
        || digest_suffix(key, "celetixo.capture.unix-mode.")
    {
        return true;
    }
    let Some(suffix) = key.strip_prefix("celetixo.capture.") else {
        return false;
    };
    let Some((repository, field)) = suffix.split_once('.') else {
        return false;
    };
    (repository == "root"
        || (repository.len() == 64 && repository.bytes().all(|b| b.is_ascii_hexdigit())))
        && matches!(
            field,
            "head"
                | "index"
                | "index-flags"
                | "index-extended-flags"
                | "git-config"
                | "git-worktree-config"
        )
}

/// Validate the exact source path policy shared by patch and verification-plan
/// contracts. This grants neither filesystem access nor execution authority.
pub fn validate_source_path(path: &str, policy: &PatchPolicy) -> Result<(), SourcePatchError> {
    policy.validate()?;
    validate_path(path, policy)
}

fn validate_path(path: &str, policy: &PatchPolicy) -> Result<(), SourcePatchError> {
    if path.is_empty()
        || path.len() > policy.max_path_bytes
        || !path.is_ascii()
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path.split('/').count() > policy.max_depth
    {
        return Err(SourcePatchError::InvalidPath(path.into()));
    }
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part.contains(['<', '>', '"', '|', '?', '*'])
        {
            return Err(SourcePatchError::InvalidPath(path.into()));
        }
        if part.eq_ignore_ascii_case(".git") || part.eq_ignore_ascii_case(".celetixo") {
            return Err(SourcePatchError::ReservedPath(path.into()));
        }
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
        ) || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'))
        {
            return Err(SourcePatchError::InvalidPath(path.into()));
        }
    }
    Ok(())
}

fn validate_final_graph(
    files: &BTreeMap<String, WorkspaceInput>,
    policy: &PatchPolicy,
) -> Result<(), SourcePatchError> {
    if files.len() > policy.max_result_files {
        return Err(SourcePatchError::Limit("result file count"));
    }
    let mut bytes = 0_u64;
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for (path, input) in files {
        validate_path(path, policy)?;
        let content = input.bytes.as_ref().ok_or(SourcePatchError::InvalidEntry)?;
        bytes = bytes
            .checked_add(content.len() as u64)
            .ok_or(SourcePatchError::Limit("result byte overflow"))?;
        if bytes > policy.max_result_bytes {
            return Err(SourcePatchError::Limit("result bytes"));
        }
        // Include inferred directories: `A/x` and `a/y` alias even when neither
        // directory is represented by a file entry.
        for end in path
            .match_indices('/')
            .map(|(i, _)| i)
            .chain(std::iter::once(path.len()))
        {
            let prefix = &path[..end];
            if let Some(existing) = aliases.insert(prefix.to_ascii_lowercase(), prefix.into())
                && existing != prefix
            {
                return Err(SourcePatchError::PathCollision(path.clone()));
            }
            if end != path.len()
                && files
                    .get(prefix)
                    .is_some_and(|ancestor| ancestor.kind != InputKind::Submodule)
            {
                return Err(SourcePatchError::PathCollision(path.clone()));
            }
        }
        if input.kind == InputKind::Symlink {
            if policy.symlinks == SymlinkPolicy::Deny {
                return Err(SourcePatchError::SymlinkDenied(path.clone()));
            }
            resolve_link(path, files, policy)?;
        }
    }
    Ok(())
}

fn resolve_link(
    path: &str,
    files: &BTreeMap<String, WorkspaceInput>,
    policy: &PatchPolicy,
) -> Result<(), SourcePatchError> {
    let mut current = path.to_owned();
    let mut seen = BTreeSet::new();
    for _ in 0..=policy.max_depth {
        let Some(input) = files.get(&current) else {
            // An inferred directory is a valid target, but an absent path is not.
            return if files
                .keys()
                .any(|p| p.strip_prefix(&current).is_some_and(|s| s.starts_with('/')))
            {
                Ok(())
            } else {
                Err(SourcePatchError::UnsafeSymlink(path.into()))
            };
        };
        if input.kind != InputKind::Symlink {
            return Ok(());
        }
        if !seen.insert(current.clone()) {
            return Err(SourcePatchError::UnsafeSymlink(path.into()));
        }
        let target = std::str::from_utf8(
            input
                .bytes
                .as_deref()
                .ok_or(SourcePatchError::InvalidEntry)?,
        )
        .map_err(|_| SourcePatchError::UnsafeSymlink(path.into()))?;
        if target.is_empty()
            || target.len() > policy.max_path_bytes
            || target.starts_with('/')
            || target.contains(['\\', ':'])
            || !target.is_ascii()
            || target.chars().any(char::is_control)
        {
            return Err(SourcePatchError::UnsafeSymlink(path.into()));
        }
        let mut parts: Vec<_> = current.split('/').collect();
        parts.pop();
        let mut components = target.split('/').peekable();
        while let Some(component) = components.next() {
            match component {
                "" => return Err(SourcePatchError::UnsafeSymlink(path.into())),
                "." => {
                    if !is_source_directory(&parts.join("/"), files) {
                        return Err(SourcePatchError::UnsafeSymlink(path.into()));
                    }
                }
                ".." => {
                    if !is_source_directory(&parts.join("/"), files) || parts.pop().is_none() {
                        return Err(SourcePatchError::UnsafeSymlink(path.into()));
                    }
                }
                value => parts.push(value),
            }
            let prefix = parts.join("/");
            if !prefix.is_empty() {
                validate_path(&prefix, policy)
                    .map_err(|_| SourcePatchError::UnsafeSymlink(path.into()))?;
            }
            // Inspect every component before consuming `..`. Lexically reducing
            // `symlink/../file`, `regular-file/../file` or `missing/../file`
            // would otherwise accept a target with different filesystem meaning.
            if components.peek().is_some() && !is_source_directory(&prefix, files) {
                return Err(SourcePatchError::UnsafeSymlink(path.into()));
            }
        }
        current = parts.join("/");
        if current.is_empty() {
            return Ok(()); // The implicit workspace root is an internal directory.
        }
        validate_path(&current, policy)
            .map_err(|_| SourcePatchError::UnsafeSymlink(path.into()))?;
        // A target containing a symlink ancestor requires complete component
        // resolution; fail closed instead of checking only its lexical endpoint.
        for (end, _) in current.match_indices('/') {
            if files
                .get(&current[..end])
                .is_some_and(|i| i.kind == InputKind::Symlink)
            {
                return Err(SourcePatchError::UnsafeSymlink(path.into()));
            }
        }
    }
    Err(SourcePatchError::UnsafeSymlink(path.into()))
}

fn is_source_directory(path: &str, files: &BTreeMap<String, WorkspaceInput>) -> bool {
    if path.is_empty() {
        return true;
    }
    if let Some(input) = files.get(path) {
        return input.kind == InputKind::Submodule;
    }
    files.keys().any(|candidate| {
        candidate
            .strip_prefix(path)
            .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

/// A content-addressed manifest identity independent of scope/revision/state.
pub fn manifest_digest(manifest: &SnapshotManifest) -> Digest {
    manifest.digest()
}

fn application_provenance_digest(
    base: &Snapshot,
    patch: &SourcePatch,
    metadata: &[NonOperationalInput],
) -> Digest {
    let mut c = Canonical::default();
    c.digest(base.id().0);
    c.digest(manifest_digest(base.manifest()));
    c.digest(patch.id.0);
    c.digest(patch.policy.digest());
    c.u64(metadata.len() as u64);
    for record in metadata {
        encode_input(&mut c, &record.input);
        c.str(&record.reason);
    }
    c.finish("celetixo/source-application-provenance/v1")
}

fn encode_input(c: &mut Canonical, input: &WorkspaceInput) {
    c.str(&input.path);
    c.u64(layer_tag(input.layer));
    c.u64(kind_tag(input.kind));
    c.u64(mode_tag(input.mode));
    match &input.bytes {
        Some(bytes) => {
            c.u64(1);
            c.bytes(bytes);
        }
        None => c.u64(0),
    }
}

fn encode_binding(c: &mut Canonical, b: &PatchBinding) {
    c.str(b.scope.organization.as_str());
    c.str(b.scope.workspace.as_str());
    c.str(b.scope.repository.as_str());
    c.str(b.principal.as_str());
    c.str(b.transaction.as_str());
    c.u64(b.transaction_attempt);
    c.str(b.source_attempt.as_str());
    c.digest(b.base_snapshot.0);
    c.u64(b.base_revision.0);
    c.digest(b.analysis_artifact);
    c.digest(b.index_generation);
    c.digest(b.impact_request);
    c.digest(b.impact_result);
    c.u64(b.created_at_ms);
    c.u64(b.expires_at_ms);
    c.str(&b.producer);
}

fn encode_policy(c: &mut Canonical, p: &PatchPolicy) {
    c.u64(u64::from(p.version));
    c.u64(match p.path_profile {
        PathProfile::PortableAsciiV1 => 0,
    });
    c.u64(match p.symlinks {
        SymlinkPolicy::Deny => 0,
        SymlinkPolicy::InternalRelativeOnly => 1,
    });
    c.u64(p.max_operations as u64);
    c.u64(p.max_payload_bytes);
    c.u64(p.max_total_payload_bytes);
    c.u64(p.max_result_files as u64);
    c.u64(p.max_result_bytes);
    c.u64(p.max_path_bytes as u64);
    c.u64(p.max_depth as u64);
}

fn encode_operations(c: &mut Canonical, operations: &[PatchOperation]) {
    c.u64(operations.len() as u64);
    for operation in operations {
        c.u64(operation_tag(operation.kind()));
        c.str(operation.source_path());
        match operation {
            PatchOperation::Create { entry, .. } => encode_entry(c, entry),
            PatchOperation::Replace {
                expected, entry, ..
            } => {
                encode_preimage(c, *expected);
                encode_entry(c, entry);
            }
            PatchOperation::Delete { expected, .. } => encode_preimage(c, *expected),
            PatchOperation::Rename { to, expected, .. } => {
                c.str(to);
                encode_preimage(c, *expected);
            }
            PatchOperation::SetMode { expected, mode, .. } => {
                encode_preimage(c, *expected);
                c.u64(mode_tag(*mode));
            }
        }
    }
}

fn encode_entry(c: &mut Canonical, entry: &PatchEntry) {
    c.u64(kind_tag(entry.kind));
    c.u64(mode_tag(entry.mode));
    c.digest(entry.payload.digest);
    c.bytes(&entry.payload.bytes);
}
fn encode_preimage(c: &mut Canonical, preimage: Preimage) {
    c.u64(kind_tag(preimage.kind));
    c.u64(mode_tag(preimage.mode));
    c.digest(preimage.content);
}
fn encode_optional_preimage(c: &mut Canonical, value: Option<Preimage>) {
    match value {
        Some(p) => {
            c.u64(1);
            encode_preimage(c, p);
        }
        None => c.u64(0),
    }
}
fn encode_optional_string(c: &mut Canonical, value: Option<&str>) {
    match value {
        Some(v) => {
            c.u64(1);
            c.str(v);
        }
        None => c.u64(0),
    }
}
fn kind_tag(k: InputKind) -> u64 {
    match k {
        InputKind::File => 0,
        InputKind::Symlink => 1,
        InputKind::Submodule => 2,
    }
}
fn mode_tag(m: FileMode) -> u64 {
    match m {
        FileMode::Regular => 0o100644,
        FileMode::Executable => 0o100755,
        FileMode::Symlink => 0o120000,
        FileMode::Gitlink => 0o160000,
    }
}
fn operation_tag(k: OperationKind) -> u64 {
    match k {
        OperationKind::Create => 0,
        OperationKind::Replace => 1,
        OperationKind::Delete => 2,
        OperationKind::Rename => 3,
        OperationKind::SetMode => 4,
    }
}
fn layer_tag(l: InputLayer) -> u64 {
    match l {
        InputLayer::Base => 0,
        InputLayer::Index => 1,
        InputLayer::WorkingTree => 2,
        InputLayer::Untracked => 3,
        InputLayer::Overlay => 4,
        InputLayer::Generated => 5,
        InputLayer::Mounted => 6,
        InputLayer::Submodule => 7,
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
    limits: DecodeLimits,
    payload_bytes: usize,
}
impl Decoder<'_> {
    fn field(&mut self, max: usize) -> Result<&[u8], SourcePatchError> {
        let length_end = self
            .offset
            .checked_add(8)
            .ok_or(SourcePatchError::InvalidEncoding)?;
        let length_bytes: [u8; 8] = self
            .bytes
            .get(self.offset..length_end)
            .ok_or(SourcePatchError::InvalidEncoding)?
            .try_into()
            .map_err(|_| SourcePatchError::InvalidEncoding)?;
        let length = usize::try_from(u64::from_be_bytes(length_bytes))
            .map_err(|_| SourcePatchError::InvalidEncoding)?;
        if length > max {
            return Err(SourcePatchError::Limit("encoded field bytes"));
        }
        let end = length_end
            .checked_add(length)
            .ok_or(SourcePatchError::InvalidEncoding)?;
        let result = self
            .bytes
            .get(length_end..end)
            .ok_or(SourcePatchError::InvalidEncoding)?;
        self.offset = end;
        Ok(result)
    }
    fn number(&mut self) -> Result<u64, SourcePatchError> {
        Ok(u64::from_be_bytes(
            self.field(8)?
                .try_into()
                .map_err(|_| SourcePatchError::InvalidEncoding)?,
        ))
    }
    fn count(&mut self, max: usize) -> Result<usize, SourcePatchError> {
        let count =
            usize::try_from(self.number()?).map_err(|_| SourcePatchError::InvalidEncoding)?;
        if count > max {
            return Err(SourcePatchError::Limit("encoded count"));
        }
        Ok(count)
    }
    fn string(&mut self, max: usize) -> Result<String, SourcePatchError> {
        String::from_utf8(self.field(max)?.to_vec()).map_err(|_| SourcePatchError::InvalidEncoding)
    }
    fn digest(&mut self) -> Result<Digest, SourcePatchError> {
        Ok(Digest::from_bytes(
            self.field(32)?
                .try_into()
                .map_err(|_| SourcePatchError::InvalidEncoding)?,
        ))
    }
    fn binding(&mut self) -> Result<PatchBinding, SourcePatchError> {
        use crate::ids::{OrganizationId, RepositoryId, WorkspaceId};
        let invalid = |_| SourcePatchError::InvalidEncoding;
        Ok(PatchBinding {
            scope: Scope {
                organization: OrganizationId::new(self.string(256)?).map_err(invalid)?,
                workspace: WorkspaceId::new(self.string(256)?).map_err(invalid)?,
                repository: RepositoryId::new(self.string(256)?).map_err(invalid)?,
            },
            principal: PrincipalId::new(self.string(256)?).map_err(invalid)?,
            transaction: TransactionId::new(self.string(256)?).map_err(invalid)?,
            transaction_attempt: self.number()?,
            source_attempt: SourceAttemptId::new(self.string(256)?).map_err(invalid)?,
            base_snapshot: SnapshotId(self.digest()?),
            base_revision: Revision(self.number()?),
            analysis_artifact: self.digest()?,
            index_generation: self.digest()?,
            impact_request: self.digest()?,
            impact_result: self.digest()?,
            created_at_ms: self.number()?,
            expires_at_ms: self.number()?,
            producer: self.string(4096)?,
        })
    }
    fn policy(&mut self) -> Result<PatchPolicy, SourcePatchError> {
        let version =
            u32::try_from(self.number()?).map_err(|_| SourcePatchError::UnsupportedVersion)?;
        let path_profile = match self.number()? {
            0 => PathProfile::PortableAsciiV1,
            _ => return Err(SourcePatchError::UnsupportedVersion),
        };
        let symlinks = match self.number()? {
            0 => SymlinkPolicy::Deny,
            1 => SymlinkPolicy::InternalRelativeOnly,
            _ => return Err(SourcePatchError::UnsupportedVersion),
        };
        Ok(PatchPolicy {
            version,
            path_profile,
            symlinks,
            max_operations: self.count(65_536)?,
            max_payload_bytes: self.number()?,
            max_total_payload_bytes: self.number()?,
            max_result_files: self.count(1_000_000)?,
            max_result_bytes: self.number()?,
            max_path_bytes: self.count(4096)?,
            max_depth: self.count(256)?,
        })
    }
    fn kind(&mut self) -> Result<InputKind, SourcePatchError> {
        match self.number()? {
            0 => Ok(InputKind::File),
            1 => Ok(InputKind::Symlink),
            2 => Ok(InputKind::Submodule),
            _ => Err(SourcePatchError::InvalidEncoding),
        }
    }
    fn mode(&mut self) -> Result<FileMode, SourcePatchError> {
        match self.number()? {
            0o100644 => Ok(FileMode::Regular),
            0o100755 => Ok(FileMode::Executable),
            0o120000 => Ok(FileMode::Symlink),
            0o160000 => Ok(FileMode::Gitlink),
            _ => Err(SourcePatchError::InvalidEncoding),
        }
    }
    fn preimage(&mut self) -> Result<Preimage, SourcePatchError> {
        Ok(Preimage {
            kind: self.kind()?,
            mode: self.mode()?,
            content: self.digest()?,
        })
    }
    fn entry(&mut self, policy: &PatchPolicy) -> Result<PatchEntry, SourcePatchError> {
        let kind = self.kind()?;
        let mode = self.mode()?;
        let digest = self.digest()?;
        let maximum = self.limits.max_payload_bytes.min(
            usize::try_from(policy.max_payload_bytes)
                .map_err(|_| SourcePatchError::InvalidPolicy)?,
        );
        let remaining = self
            .limits
            .max_total_payload_bytes
            .checked_sub(self.payload_bytes)
            .ok_or(SourcePatchError::Limit("decoded payload bytes"))?;
        let bytes = self.field(maximum.min(remaining))?.to_vec();
        self.payload_bytes = self
            .payload_bytes
            .checked_add(bytes.len())
            .ok_or(SourcePatchError::Limit("decoded payload byte overflow"))?;
        if self.payload_bytes as u64 > policy.max_total_payload_bytes {
            return Err(SourcePatchError::Limit("decoded aggregate payload bytes"));
        }
        PatchEntry::new(kind, mode, Payload::with_digest(bytes, digest)?)
    }
    fn operation(&mut self, policy: &PatchPolicy) -> Result<PatchOperation, SourcePatchError> {
        let tag = self.number()?;
        let path = self.string(policy.max_path_bytes)?;
        Ok(match tag {
            0 => PatchOperation::Create {
                path,
                entry: self.entry(policy)?,
            },
            1 => PatchOperation::Replace {
                path,
                expected: self.preimage()?,
                entry: self.entry(policy)?,
            },
            2 => PatchOperation::Delete {
                path,
                expected: self.preimage()?,
            },
            3 => PatchOperation::Rename {
                from: path,
                to: self.string(policy.max_path_bytes)?,
                expected: self.preimage()?,
            },
            4 => PatchOperation::SetMode {
                path,
                expected: self.preimage()?,
                mode: self.mode()?,
            },
            _ => return Err(SourcePatchError::InvalidEncoding),
        })
    }
}

#[cfg(test)]
mod report_integrity_tests {
    use super::*;
    use crate::ids::{OrganizationId, RepositoryId, WorkspaceId};
    use crate::snapshot::State;

    fn fixture() -> (Snapshot, SourcePatch) {
        let scope = Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("ws").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        };
        let mut manifest = SnapshotManifest {
            base_commit: "immutable fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "original complete record with explicit live uncertainty".into(),
            inputs: vec![
                PatchEntry::file(b"old".to_vec(), FileMode::Regular)
                    .unwrap()
                    .at("a".into()),
                WorkspaceInput {
                    path: ".git/config".into(),
                    layer: InputLayer::Mounted,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(b"metadata".to_vec()),
                },
            ],
            environment: BTreeMap::new(),
        };
        manifest.environment.insert(
            "celetixo.capture.uncertainty".into(),
            "retained-uncertainty".into(),
        );
        let base = Snapshot::new(scope.clone(), Revision(0), manifest, State::new()).unwrap();
        let digest = Digest::of("test/identity", b"fixture");
        let binding = PatchBinding {
            scope,
            principal: PrincipalId::new("human").unwrap(),
            transaction: TransactionId::new("tx").unwrap(),
            transaction_attempt: 1,
            source_attempt: SourceAttemptId::new("attempt").unwrap(),
            base_snapshot: base.id(),
            base_revision: base.revision(),
            analysis_artifact: digest,
            index_generation: digest,
            impact_request: digest,
            impact_result: digest,
            created_at_ms: 1,
            expires_at_ms: 2,
            producer: "fixture".into(),
        };
        let patch = SourcePatch::new(
            binding,
            PatchPolicy::default(),
            vec![PatchOperation::Replace {
                path: "a".into(),
                expected: Preimage::of(&effective_workspace(&base).unwrap()["a"]).unwrap(),
                entry: PatchEntry::file(b"new".to_vec(), FileMode::Regular).unwrap(),
            }],
        )
        .unwrap();
        (base, patch)
    }

    #[test]
    fn replay_rejects_tampered_report_fields_and_digest_binds_actual_retained_payloads() {
        let (base, patch) = fixture();
        let applied = patch.apply(&base).unwrap();
        for field in 0..8 {
            let mut tampered = applied.clone();
            match field {
                0 => {
                    tampered
                        .report
                        .base_manifest
                        .environment
                        .remove("celetixo.capture.uncertainty");
                }
                1 => tampered.report.base_manifest.inputs[0].bytes = Some(b"forged base".to_vec()),
                2 => tampered.report.non_operational_inputs[0].reason = "forged exclusion".into(),
                3 => {
                    tampered.report.non_operational_inputs[0].input.bytes =
                        Some(b"forged metadata".to_vec())
                }
                4 => tampered.report.outcomes.clear(),
                5 => tampered.report.resulting_snapshot = base.id(),
                6 => tampered.report.base_manifest_digest = Digest::of("wrong", b"manifest"),
                _ => tampered.report.provenance_digest = Digest::of("wrong", b"provenance"),
            }
            assert_ne!(applied.report.digest(), tampered.report.digest());
            assert_eq!(
                tampered.validate(&base, &patch),
                Err(SourcePatchError::InvalidReport)
            );
        }
    }

    #[test]
    fn replay_rejects_a_self_consistent_but_wrong_resulting_snapshot() {
        let (base, patch) = fixture();
        let mut applied = patch.apply(&base).unwrap();
        let mut manifest = applied.snapshot.manifest().clone();
        manifest.inputs[0].bytes = Some(b"executor forged result".to_vec());
        applied.snapshot = Snapshot::new(
            base.scope().clone(),
            base.revision(),
            manifest,
            State::new(),
        )
        .unwrap();
        applied.report.resulting_snapshot = applied.snapshot.id();
        applied.report.resulting_manifest_digest = applied.snapshot.manifest().digest();
        assert_eq!(
            applied.validate(&base, &patch),
            Err(SourcePatchError::InvalidReport)
        );
    }
}
