//! Read-only Git/workspace capture. Repository code, filters, hooks and helpers are
//! never executed. Files are read through no-follow, directory-relative handles.
//!
//! Two equal observations detect ordinary concurrent edits, not an adversarial ABA
//! writer. The report retains that limitation; a live capture alone is never a
//! proof of atomic filesystem consistency. Captured bytes are immutable thereafter.

use celetixo_engine::ids::{Digest, Revision, Scope};
use celetixo_engine::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotError, SnapshotManifest,
    State, WorkspaceInput,
};
use sha1::Digest as _;
use sha2::Digest as _;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialPolicy {
    /// Version of the fixed admission rules. Unsupported versions reject.
    pub version: u32,
    /// Reviewed public content, keyed by exact logical path and content digest.
    /// This policy must be supplied by the trusted capture/governance caller;
    /// untrusted workers cannot authorize their own exceptions. A path-only or
    /// wildcard exception is deliberately unavailable.
    pub reviewed_public_inputs: BTreeMap<String, BTreeSet<Digest>>,
}

impl Default for CredentialPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            reviewed_public_inputs: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePolicy {
    /// Exact paths, or directory roots whose descendants are also excluded.
    /// Excluded trees are not traversed. Every actual exclusion has a rationale.
    pub excluded_paths: BTreeMap<String, String>,
    pub required_paths: BTreeSet<String>,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_depth: usize,
    pub git_timeout: Duration,
    pub credentials: CredentialPolicy,
}

impl Default for CapturePolicy {
    fn default() -> Self {
        Self {
            excluded_paths: BTreeMap::new(),
            required_paths: BTreeSet::new(),
            max_files: 100_000,
            max_file_bytes: 16 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_depth: 64,
            git_timeout: Duration::from_secs(30),
            credentials: CredentialPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub version: u64,
    /// None deletes the effective document; an empty vector is an empty file.
    pub bytes: Option<Vec<u8>>,
    pub mode: FileMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalInput {
    /// Canonical logical workspace path, independent of the absolute source path.
    pub path: String,
    pub source: PathBuf,
    /// Only Generated and Mounted are accepted.
    pub layer: InputLayer,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub root: PathBuf,
    pub scope: Scope,
    pub revision: Revision,
    pub policy: CapturePolicy,
    pub overlays: BTreeMap<String, Overlay>,
    pub external_inputs: Vec<ExternalInput>,
    /// Already immutable generated/mounted bytes; no filesystem reads are needed.
    pub immutable_inputs: Vec<WorkspaceInput>,
    /// Explicit non-secret analysis/execution inputs. Ambient process environment
    /// is not inherited by Git and is not claimed to have been captured.
    pub environment: BTreeMap<String, String>,
}

impl CaptureRequest {
    pub fn new(root: impl Into<PathBuf>, scope: Scope, revision: Revision) -> Self {
        Self {
            root: root.into(),
            scope,
            revision,
            policy: CapturePolicy::default(),
            overlays: BTreeMap::new(),
            external_inputs: Vec::new(),
            immutable_inputs: Vec::new(),
            environment: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CaptureIssue {
    /// Read twice, but not acquired through a provider with atomic snapshot/lease
    /// semantics. Cannot prove freedom from adversarial change-and-restore races.
    LiveSourceNotAtomic,
    MissingRequiredInput(String),
    MissingOptionalInput(String),
    UninitializedSubmodule(String),
    UnmergedIndex(String),
    ExcludedInput(String),
    NestedRepository(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureConsistency {
    ObservedTwoPass,
    GitTree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureReport {
    /// Interpretable policy retained alongside its manifest-bound digest.
    pub policy: CapturePolicy,
    pub consistency: CaptureConsistency,
    pub issues: BTreeSet<CaptureIssue>,
    pub content_digests: BTreeMap<(InputLayer, String), Digest>,
    pub excluded_paths: BTreeMap<String, String>,
    pub input_count: usize,
    pub content_bytes: u64,
    pub policy_digest: Digest,
}

impl CaptureReport {
    pub fn complete(&self) -> bool {
        self.issues.is_empty()
    }

    /// Census completeness is separate from atomic filesystem consistency.
    pub fn census_complete(&self) -> bool {
        self.issues
            .iter()
            .all(|issue| matches!(issue, CaptureIssue::LiveSourceNotAtomic))
    }

    pub fn uncertainty_digest(&self) -> Digest {
        uncertainty_digest(&self.issues)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedFile {
    pub path: String,
    pub kind: InputKind,
    pub mode: FileMode,
    pub bytes: Vec<u8>,
    pub digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureResult {
    pub snapshot: Snapshot,
    pub report: CaptureReport,
}

impl CaptureResult {
    /// Validate the content-addressed report binding before another authoritative
    /// subsystem consumes it. Public report fields are inspectable, not trusted.
    pub fn validate(&self) -> Result<(), CaptureError> {
        let manifest = self.snapshot.manifest();
        if self.report.policy_digest != policy_digest(&self.report.policy) {
            return Err(CaptureError::InvalidReport("policy contents"));
        }
        let mut policy_request = CaptureRequest::new(
            "<immutable-report-validation>",
            self.snapshot.scope().clone(),
            self.snapshot.revision(),
        );
        policy_request.policy = self.report.policy.clone();
        policy_request.environment = manifest
            .environment
            .iter()
            .filter(|(key, _)| !key.starts_with("celetixo."))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        validate_request(&policy_request)?;
        let consistency = consistency_name(self.report.consistency);
        let provenance = format!(
            "celetixo/git-capture/v1;policy={};consistency={consistency}",
            self.report.policy_digest
        );
        if manifest.provenance != provenance {
            return Err(CaptureError::InvalidReport("producer provenance binding"));
        }
        if manifest
            .environment
            .get("celetixo.capture.consistency")
            .map(String::as_str)
            != Some(consistency)
        {
            return Err(CaptureError::InvalidReport("consistency binding"));
        }
        if self.report.consistency == CaptureConsistency::ObservedTwoPass
            && !self
                .report
                .issues
                .contains(&CaptureIssue::LiveSourceNotAtomic)
        {
            return Err(CaptureError::InvalidReport(
                "live source atomicity uncertainty removed",
            ));
        }
        if self.report.consistency == CaptureConsistency::GitTree
            && self
                .report
                .issues
                .contains(&CaptureIssue::LiveSourceNotAtomic)
        {
            return Err(CaptureError::InvalidReport("tree source marked as live"));
        }
        if manifest.environment.get("celetixo.capture.uncertainty")
            != Some(&self.report.uncertainty_digest().to_string())
        {
            return Err(CaptureError::InvalidReport("uncertainty binding"));
        }
        if manifest.environment.get("celetixo.capture.policy")
            != Some(&self.report.policy_digest.to_string())
        {
            return Err(CaptureError::InvalidReport("policy binding"));
        }
        if manifest.inclusion_policy.excluded_paths != self.report.excluded_paths {
            return Err(CaptureError::InvalidReport("exclusion binding"));
        }
        if manifest.inputs.len() != self.report.input_count {
            return Err(CaptureError::InvalidReport("input count"));
        }
        if manifest.inputs.len() > self.report.policy.max_files {
            return Err(CaptureError::InvalidReport("policy entry budget"));
        }
        let mut bytes = 0_u64;
        let mut digests = BTreeMap::new();
        for input in &manifest.inputs {
            if let Some(content) = &input.bytes {
                if content.len() as u64 > self.report.policy.max_file_bytes {
                    return Err(CaptureError::InvalidReport("policy file budget"));
                }
                admit_input(&input.path, content, &self.report.policy.credentials)?;
                bytes = bytes
                    .checked_add(content.len() as u64)
                    .ok_or(CaptureError::InvalidReport("content byte overflow"))?;
                digests.insert(
                    (input.layer, input.path.clone()),
                    Digest::of("celetixo/source-content/v1", content),
                );
            }
        }
        if bytes != self.report.content_bytes || digests != self.report.content_digests {
            return Err(CaptureError::InvalidReport("captured content binding"));
        }
        if bytes > self.report.policy.max_total_bytes {
            return Err(CaptureError::InvalidReport("policy aggregate budget"));
        }
        effective_workspace(&self.snapshot)?;
        Ok(())
    }

    /// Effective workspace view, with absent tracked files and overlay tombstones
    /// applied. Symlinks remain symlinks; analyzers must not resolve them.
    pub fn effective_files(&self) -> Result<BTreeMap<String, CapturedFile>, CaptureError> {
        effective_files(&self.snapshot)
    }
}

/// Single effective-view contract shared with the source index. All original
/// layers remain in snapshot identity. Generated/mounted aliases require exact
/// agreement; current submodule identities supersede expected Gitlinks. Explicit
/// document overlays always apply last, including tombstones.
pub fn effective_workspace(
    snapshot: &Snapshot,
) -> Result<BTreeMap<String, WorkspaceInput>, CaptureError> {
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
                            return Err(CaptureError::ExternalCollision(input.path.clone()));
                        }
                        if layer == InputLayer::Submodule && existing.kind != InputKind::Submodule {
                            return Err(CaptureError::ExternalCollision(input.path.clone()));
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
                return Err(CaptureError::InvalidPolicy(
                    "effective inputs cross a file or symlink ancestor",
                ));
            }
        }
    }
    Ok(files)
}

pub fn effective_files(
    snapshot: &Snapshot,
) -> Result<BTreeMap<String, CapturedFile>, CaptureError> {
    Ok(effective_workspace(snapshot)?
        .into_iter()
        .filter_map(|(path, input)| {
            input.bytes.map(|bytes| {
                (
                    path,
                    CapturedFile {
                        path: input.path,
                        kind: input.kind,
                        mode: input.mode,
                        digest: Digest::of("celetixo/source-content/v1", &bytes),
                        bytes,
                    },
                )
            })
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEvent {
    BetweenPasses,
}

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("capture I/O failed at logical path {path:?}: {source}")]
    Io { path: String, source: io::Error },
    #[error("Git capture command failed ({operation}); repository output is withheld")]
    Git { operation: String },
    #[error("capture changed between or during observations; retry from a stable source")]
    ChangedDuringCapture,
    #[error("capture limit exceeded: {0}")]
    Limit(&'static str),
    #[error("unsupported or noncanonical workspace path (raw path withheld)")]
    InvalidPath,
    #[error("invalid capture policy: {0}")]
    InvalidPolicy(&'static str),
    #[error("capture report is inconsistent with its immutable snapshot: {0}")]
    InvalidReport(&'static str),
    #[error("unsupported filesystem object at {0:?}")]
    UnsupportedObject(String),
    #[error("external input collides with captured workspace path: {0:?}")]
    ExternalCollision(String),
    #[error(
        "input admission rejected potential credential content at logical path {0:?}; content withheld"
    )]
    SensitiveInput(String),
    #[error("this platform has no implemented safe no-follow capture backend (Linux required)")]
    UnsupportedPlatform,
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Observation {
    inputs: BTreeMap<(InputLayer, String), WorkspaceInput>,
    fingerprints: BTreeMap<String, Digest>,
    exclusions: BTreeMap<String, String>,
    issues: BTreeSet<CaptureIssue>,
    overlay_versions: BTreeMap<String, u64>,
    base: String,
    bytes: u64,
    /// Compared between passes but never included in snapshot identity: touching
    /// an otherwise equal file does not permanently change its content identity.
    physical_observations: BTreeMap<String, String>,
    physical_modes: BTreeMap<String, u32>,
    directory_modes: BTreeMap<String, u32>,
}

struct ExternalObservation {
    input: WorkspaceInput,
    fingerprint: String,
    mode: u32,
}

impl Observation {
    fn add(&mut self, input: WorkspaceInput, policy: &CapturePolicy) -> Result<(), CaptureError> {
        if let Some(bytes) = &input.bytes {
            if bytes.len() as u64 > policy.max_file_bytes {
                return Err(CaptureError::Limit("individual file bytes"));
            }
            self.bytes = self
                .bytes
                .checked_add(bytes.len() as u64)
                .ok_or(CaptureError::Limit("aggregate bytes"))?;
            if self.bytes > policy.max_total_bytes {
                return Err(CaptureError::Limit("aggregate bytes"));
            }
            admit_input(&input.path, bytes, &policy.credentials)?;
        }
        if self.inputs.len() >= policy.max_files {
            return Err(CaptureError::Limit("manifest entries"));
        }
        let key = (input.layer, input.path.clone());
        if self.inputs.insert(key, input).is_some() {
            return Err(CaptureError::InvalidPolicy("duplicate input layer/path"));
        }
        Ok(())
    }
}

pub fn capture(request: &CaptureRequest) -> Result<CaptureResult, CaptureError> {
    capture_with_hook(request, |_| {})
}

/// Deterministic fault injection is a caller-side observation hook, never a
/// subprocess or callback selected by untrusted repository content.
pub fn capture_with_hook(
    request: &CaptureRequest,
    mut hook: impl FnMut(CaptureEvent),
) -> Result<CaptureResult, CaptureError> {
    validate_request(request)?;
    let root = request
        .root
        .canonicalize()
        .map_err(|source| CaptureError::Io {
            path: "<workspace-root>".into(),
            source,
        })?;
    let first = observe(&root, request)?;
    hook(CaptureEvent::BetweenPasses);
    let second = observe(&root, request)?;
    if first != second {
        return Err(CaptureError::ChangedDuringCapture);
    }
    finish_observation(request, first, CaptureConsistency::ObservedTwoPass)
}

fn finish_observation(
    request: &CaptureRequest,
    first: Observation,
    consistency: CaptureConsistency,
) -> Result<CaptureResult, CaptureError> {
    let mut environment = request.environment.clone();
    for (key, digest) in &first.fingerprints {
        environment.insert(format!("celetixo.capture.{key}"), digest.to_string());
    }
    for (path, mode) in &first.physical_modes {
        let key = Digest::of("celetixo/captured-mode-path/v1", path.as_bytes());
        environment.insert(
            format!("celetixo.capture.unix-mode.{key}"),
            format!("{mode:o}"),
        );
    }
    // Empty directories and directory permissions can affect execution too.
    // Goal01's input kinds have no directory variant, so retain their exact
    // logical census in the manifest's versioned environment-input namespace.
    for (path, mode) in &first.directory_modes {
        let key = Digest::of("celetixo/captured-directory-path/v1", path.as_bytes());
        environment.insert(
            format!("celetixo.capture.directory.{key}"),
            format!("{mode:o}:{path}"),
        );
    }
    for (path, version) in &first.overlay_versions {
        let key = Digest::of("celetixo/overlay-path/v1", path.as_bytes());
        environment.insert(format!("celetixo.overlay.{key}"), version.to_string());
    }
    let mut issues = first.issues;
    if consistency == CaptureConsistency::ObservedTwoPass {
        issues.insert(CaptureIssue::LiveSourceNotAtomic);
    }
    environment.insert(
        "celetixo.capture.uncertainty".into(),
        uncertainty_digest(&issues).to_string(),
    );
    environment.insert(
        "celetixo.capture.consistency".into(),
        consistency_name(consistency).into(),
    );
    let policy_digest = policy_digest(&request.policy);
    environment.insert("celetixo.capture.policy".into(), policy_digest.to_string());
    let inputs: Vec<_> = first.inputs.into_values().collect();
    let content_digests = inputs
        .iter()
        .filter_map(|input| {
            input.bytes.as_ref().map(|bytes| {
                (
                    (input.layer, input.path.clone()),
                    Digest::of("celetixo/source-content/v1", bytes),
                )
            })
        })
        .collect();
    let input_count = inputs.len();
    let report = CaptureReport {
        policy: request.policy.clone(),
        consistency,
        issues,
        content_digests,
        excluded_paths: first.exclusions.clone(),
        input_count,
        content_bytes: first.bytes,
        policy_digest,
    };
    let snapshot = Snapshot::new(
        request.scope.clone(),
        request.revision,
        SnapshotManifest {
            base_commit: first.base,
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy {
                excluded_paths: first.exclusions,
                ..InclusionPolicy::complete()
            },
            provenance: format!(
                "celetixo/git-capture/v1;policy={policy_digest};consistency={}",
                consistency_name(consistency)
            ),
            inputs,
            environment,
        },
        State::new(),
    )?;
    let result = CaptureResult { snapshot, report };
    result.validate()?;
    Ok(result)
}

fn consistency_name(consistency: CaptureConsistency) -> &'static str {
    match consistency {
        CaptureConsistency::ObservedTwoPass => "observed-two-pass/v1",
        CaptureConsistency::GitTree => "git-tree/v1",
    }
}

#[derive(Default)]
struct CaptureCanonical(Vec<u8>);
impl CaptureCanonical {
    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }
    fn string(&mut self, value: &str) {
        self.u64(value.len() as u64);
        self.0.extend_from_slice(value.as_bytes());
    }
    fn finish(self, domain: &str) -> Digest {
        Digest::of(domain, &self.0)
    }
}

pub fn policy_digest(policy: &CapturePolicy) -> Digest {
    let mut bytes = CaptureCanonical::default();
    bytes.u64(1);
    bytes.u64(policy.excluded_paths.len() as u64);
    for (path, reason) in &policy.excluded_paths {
        bytes.string(path);
        bytes.string(reason);
    }
    bytes.u64(policy.required_paths.len() as u64);
    for path in &policy.required_paths {
        bytes.string(path);
    }
    for value in [
        policy.max_files as u64,
        policy.max_file_bytes,
        policy.max_total_bytes,
        policy.max_depth as u64,
        policy.git_timeout.as_secs(),
        u64::from(policy.git_timeout.subsec_nanos()),
    ] {
        bytes.u64(value);
    }
    bytes.u64(u64::from(policy.credentials.version));
    bytes.u64(policy.credentials.reviewed_public_inputs.len() as u64);
    for (path, digests) in &policy.credentials.reviewed_public_inputs {
        bytes.string(path);
        bytes.u64(digests.len() as u64);
        for digest in digests {
            bytes.string(&digest.to_string());
        }
    }
    bytes.finish("celetixo/capture-policy/v1")
}

fn uncertainty_digest(issues: &BTreeSet<CaptureIssue>) -> Digest {
    let mut bytes = CaptureCanonical::default();
    bytes.u64(1);
    bytes.u64(issues.len() as u64);
    for issue in issues {
        let (tag, path) = match issue {
            CaptureIssue::LiveSourceNotAtomic => (0, None),
            CaptureIssue::MissingRequiredInput(path) => (1, Some(path)),
            CaptureIssue::MissingOptionalInput(path) => (2, Some(path)),
            CaptureIssue::UninitializedSubmodule(path) => (3, Some(path)),
            CaptureIssue::UnmergedIndex(path) => (4, Some(path)),
            CaptureIssue::ExcludedInput(path) => (5, Some(path)),
            CaptureIssue::NestedRepository(path) => (6, Some(path)),
        };
        bytes.u64(tag);
        if let Some(path) = path {
            bytes.string(path);
        }
    }
    bytes.finish("celetixo/capture-issues/v1")
}

/// Conservative admission, not a proof that arbitrary bytes contain no secret.
/// Governed callers still classify inputs before capture. Rules and reviewed
/// exceptions are content-addressed; rejection happens before any Snapshot is
/// constructed or published, and diagnostics never contain the rejected bytes.
fn admit_input(path: &str, bytes: &[u8], policy: &CredentialPolicy) -> Result<(), CaptureError> {
    if !credential_path(path) && !credential_material(bytes) {
        return Ok(());
    }
    let digest = Digest::of("celetixo/source-content/v1", bytes);
    if policy
        .reviewed_public_inputs
        .get(path)
        .is_some_and(|digests| digests.contains(&digest))
    {
        return Ok(());
    }
    Err(CaptureError::SensitiveInput(path.into()))
}

fn credential_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let public_template = [".example", ".sample", ".template"]
        .iter()
        .any(|suffix| name.ends_with(suffix));
    if (name == ".env" || name.starts_with(".env.")) && !public_template {
        return true;
    }
    if matches!(
        name,
        ".git-credentials"
            | ".netrc"
            | "_netrc"
            | ".pypirc"
            | "id_rsa"
            | "id_dsa"
            | "id_ecdsa"
            | "id_ed25519"
            | "credentials"
            | "access_token"
            | "refresh_token"
            | "token"
    ) {
        return true;
    }
    if [".key", ".p12", ".pfx", ".ppk", ".p8", ".jks", ".keystore"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return true;
    }
    if ["credentials.", "secrets.", "tokens."]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        && !public_template
    {
        return true;
    }
    [
        ".aws/credentials",
        ".azure/accesstokens.json",
        ".docker/config.json",
    ]
    .iter()
    .any(|suffix| lower == *suffix || lower.ends_with(&format!("/{suffix}")))
}

fn credential_material(bytes: &[u8]) -> bool {
    let token = |prefix: &[u8], minimum: usize, valid: fn(u8) -> bool| {
        bytes
            .windows(prefix.len())
            .enumerate()
            .any(|(offset, window)| {
                window == prefix
                    && bytes
                        .get(offset + prefix.len()..offset + prefix.len() + minimum)
                        .is_some_and(|suffix| suffix.iter().all(|byte| valid(*byte)))
            })
    };
    let identifier = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-');
    for (prefix, minimum) in [
        (b"ghp_".as_slice(), 36),
        (b"gho_", 36),
        (b"ghu_", 36),
        (b"ghs_", 36),
        (b"ghr_", 36),
        (b"github_pat_", 30),
        (b"xoxb-", 20),
        (b"xoxp-", 20),
        (b"xoxa-", 20),
        (b"sk-proj-", 20),
        (b"sk-", 30),
        (b"AIza", 35),
    ] {
        if token(prefix, minimum, identifier) {
            return true;
        }
    }
    for prefix in [b"AKIA", b"ASIA"] {
        if token(prefix, 16, |byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit()
        }) {
            return true;
        }
    }
    if bytes
        .windows(b"openssh-key-v1\0".len())
        .any(|window| window == b"openssh-key-v1\0")
    {
        return true;
    }
    for header in [
        "-----BEGIN PRIVATE KEY-----",
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----BEGIN EC PRIVATE KEY-----",
        "-----BEGIN DSA PRIVATE KEY-----",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
        "-----BEGIN ENCRYPTED PRIVATE KEY-----",
    ] {
        for (offset, _) in bytes
            .windows(header.len())
            .enumerate()
            .filter(|(_, window)| *window == header.as_bytes())
        {
            let following = &bytes[offset + header.len()..];
            let following = following
                .strip_prefix(b"\r\n")
                .or_else(|| following.strip_prefix(b"\n"))
                .or_else(|| following.strip_prefix(b"\\n"));
            if following.is_some_and(|payload| {
                payload.get(..32).is_some_and(|payload| {
                    payload.iter().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
                    })
                })
            }) {
                return true;
            }
        }
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    for (offset, _) in text.match_indices("://") {
        let authority = text[offset + 3..]
            .split(['/', ' ', '\t', '\r', '\n', '"', '\''])
            .next()
            .unwrap_or("");
        if let Some((user_info, _)) = authority.rsplit_once('@')
            && let Some((_, password)) = user_info.split_once(':')
            && !password.is_empty()
            && !public_placeholder(password)
        {
            return true;
        }
    }
    for line in text.lines() {
        let Some(delimiter) = line.find(['=', ':']) else {
            continue;
        };
        let (key, value) = (&line[..delimiter], &line[delimiter + 1..]);
        let declared_variable = key.split_whitespace().count() > 1;
        let quoted_value = value.trim().starts_with(['\'', '"']);
        if declared_variable && !quoted_value {
            continue;
        }
        let key = key.trim().trim_matches(['\'', '"']).to_ascii_lowercase();
        let key = key.split_whitespace().last().unwrap_or("");
        if !credential_key(key) {
            continue;
        }
        let value = value.trim().trim_end_matches(',').trim_matches(['\'', '"']);
        if !public_placeholder(value) && value.len() >= 8 {
            return true;
        }
    }
    false
}

fn credential_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let parts = lower.split(['_', '-', '.']).collect::<Vec<_>>();
    parts.iter().any(|part| {
        matches!(
            *part,
            "password" | "passwd" | "token" | "secret" | "apikey" | "authtoken"
        )
    }) || parts.windows(2).any(|parts| {
        matches!(
            parts,
            ["api", "key"] | ["private", "key"] | ["access", "key"]
        )
    })
}

fn public_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.is_empty()
        || value.starts_with(['<', '$'])
        || [
            "example",
            "placeholder",
            "dummy",
            "fake",
            "not-a-secret",
            "redacted",
            "changeme",
            "change-me",
            "your_",
            "your-",
            "process.env",
            "os.environ",
            "std::env",
            "env::",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || matches!(
            lower.as_str(),
            "null" | "none" | "undefined" | "true" | "false" | "..."
        )
}

fn scoped_credential_reference(value: &str, scope: &Scope) -> bool {
    let Some(value) = value.strip_prefix("credential-ref://") else {
        return false;
    };
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0] == scope.organization.as_str()
        && parts[1] == scope.workspace.as_str()
        && parts[2] == scope.repository.as_str()
        && !parts[3].is_empty()
        && !matches!(parts[3], "." | "..")
        && parts[3]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Capture an explicitly selected immutable commit tree. This API intentionally
/// does not represent the current workspace: staged/unstaged/untracked files are
/// outside its declared source, and the caller must choose it explicitly.
/// Supplied overlays and immutable inputs participate; live external paths reject.
pub fn capture_git_tree(
    request: &CaptureRequest,
    commit: &str,
) -> Result<CaptureResult, CaptureError> {
    validate_request(request)?;
    if !request.external_inputs.is_empty() {
        return Err(CaptureError::InvalidPolicy(
            "GitTree capture requires immutable external bytes",
        ));
    }
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CaptureError::InvalidPolicy(
            "GitTree capture requires a full commit object ID",
        ));
    }
    let root = request
        .root
        .canonicalize()
        .map_err(|source| CaptureError::Io {
            path: "<workspace-root>".into(),
            source,
        })?;
    validate_repository_root(&root, &request.policy)?;
    let resolved = required_git(
        &root,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
        &request.policy,
    )?;
    if String::from_utf8_lossy(&resolved).trim() != commit {
        return Err(CaptureError::InvalidPolicy(
            "object is not the selected commit",
        ));
    }
    let entries = verified_tree_entries(&root, commit, &request.policy)?;
    let mut observation = Observation {
        base: commit.to_string(),
        ..Observation::default()
    };
    observation.exclusions.insert(".git".into(), "GitTree source consists of the selected immutable commit, not Git administration or current workspace".into());
    for entry in entries {
        if let Some(reason) = excluded(&entry.path, &request.policy) {
            observation.exclusions.insert(entry.path.clone(), reason);
            observation
                .issues
                .insert(CaptureIssue::ExcludedInput(entry.path));
            continue;
        }
        let bytes = if entry.mode == FileMode::Gitlink {
            observation
                .issues
                .insert(CaptureIssue::UninitializedSubmodule(entry.path.clone()));
            entry.oid.as_bytes().to_vec()
        } else {
            git_object(&root, "blob", &entry.oid, &request.policy)?
        };
        for layer in [InputLayer::Base, InputLayer::Index, InputLayer::WorkingTree] {
            observation.add(
                WorkspaceInput {
                    path: entry.path.clone(),
                    layer,
                    kind: kind(entry.mode),
                    mode: entry.mode,
                    bytes: Some(bytes.clone()),
                },
                &request.policy,
            )?;
        }
    }
    add_immutable_inputs(request, &mut observation)?;
    check_required(request, &mut observation)?;
    finish_observation(request, observation, CaptureConsistency::GitTree)
}

fn validate_request(request: &CaptureRequest) -> Result<(), CaptureError> {
    if !cfg!(target_os = "linux") {
        return Err(CaptureError::UnsupportedPlatform);
    }
    let policy = &request.policy;
    if policy.credentials.version != 1 {
        return Err(CaptureError::InvalidPolicy(
            "unsupported credential admission policy",
        ));
    }
    for (path, digests) in &policy.credentials.reviewed_public_inputs {
        canonical_path(path)?;
        if digests.is_empty() {
            return Err(CaptureError::InvalidPolicy(
                "public input review needs an exact content digest",
            ));
        }
    }
    let mut environment_bytes = 0_usize;
    if request.environment.len() > policy.max_files {
        return Err(CaptureError::Limit("environment inputs"));
    }
    for (key, value) in &request.environment {
        if key.is_empty()
            || key.contains(['=', '\0'])
            || key.chars().any(char::is_control)
            || value.contains('\0')
        {
            return Err(CaptureError::InvalidPolicy("invalid environment input"));
        }
        if value.len() as u64 > policy.max_file_bytes {
            return Err(CaptureError::Limit("environment value bytes"));
        }
        environment_bytes = environment_bytes
            .checked_add(key.len())
            .and_then(|size| size.checked_add(value.len()))
            .ok_or(CaptureError::Limit("environment bytes"))?;
        if environment_bytes as u64 > policy.max_total_bytes {
            return Err(CaptureError::Limit("environment bytes"));
        }
        if value.starts_with("credential-ref://") {
            if scoped_credential_reference(value, &request.scope) {
                continue;
            }
            return Err(CaptureError::SensitiveInput(format!("<environment:{key}>")));
        }
        if ((credential_key(key) || key.eq_ignore_ascii_case("key")) && !value.is_empty())
            || credential_material(value.as_bytes())
        {
            return Err(CaptureError::SensitiveInput(format!("<environment:{key}>")));
        }
    }
    if policy.max_files == 0
        || policy.max_file_bytes == 0
        || policy.max_total_bytes == 0
        || policy.max_depth == 0
        || policy.git_timeout.is_zero()
    {
        return Err(CaptureError::InvalidPolicy("limits must be nonzero"));
    }
    for (path, reason) in &policy.excluded_paths {
        canonical_path(path)?;
        if reason.trim().is_empty() || reason.chars().any(char::is_control) {
            return Err(CaptureError::InvalidPolicy("exclusion needs a rationale"));
        }
    }
    for path in &policy.required_paths {
        canonical_path(path)?;
    }
    for (path, overlay) in &request.overlays {
        canonical_path(path)?;
        if !matches!(overlay.mode, FileMode::Regular | FileMode::Executable) {
            return Err(CaptureError::InvalidPolicy(
                "overlays must be regular files",
            ));
        }
    }
    for external in &request.external_inputs {
        canonical_path(&external.path)?;
        if !matches!(external.layer, InputLayer::Generated | InputLayer::Mounted) {
            return Err(CaptureError::InvalidPolicy(
                "external layer must be Generated or Mounted",
            ));
        }
    }
    for input in &request.immutable_inputs {
        canonical_path(&input.path)?;
        if !matches!(input.layer, InputLayer::Generated | InputLayer::Mounted) {
            return Err(CaptureError::InvalidPolicy(
                "immutable input layer must be Generated or Mounted",
            ));
        }
    }
    if request
        .environment
        .keys()
        .any(|key| key.starts_with("celetixo."))
    {
        return Err(CaptureError::InvalidPolicy(
            "reserved environment namespace",
        ));
    }
    Ok(())
}

fn canonical_path(path: &str) -> Result<(), CaptureError> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(CaptureError::InvalidPath);
    }
    Ok(())
}

fn excluded(path: &str, policy: &CapturePolicy) -> Option<String> {
    policy.excluded_paths.iter().find_map(|(root, reason)| {
        (path == root || path.starts_with(&format!("{root}/"))).then(|| reason.clone())
    })
}

fn prefixed(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.into()
    } else {
        format!("{prefix}/{path}")
    }
}

fn observe(root: &Path, request: &CaptureRequest) -> Result<Observation, CaptureError> {
    let mut observation = Observation::default();
    capture_repository(root, "", request, &mut observation, 0)?;
    add_immutable_inputs(request, &mut observation)?;
    for external in &request.external_inputs {
        if let Some(reason) = excluded(&external.path, &request.policy) {
            observation.exclusions.insert(external.path.clone(), reason);
            observation
                .issues
                .insert(CaptureIssue::ExcludedInput(external.path.clone()));
            continue;
        }
        if observation
            .inputs
            .keys()
            .any(|(layer, path)| *layer != InputLayer::Overlay && path == &external.path)
        {
            return Err(CaptureError::ExternalCollision(external.path.clone()));
        }
        match read_external(external, &request.policy)? {
            Some(captured) => {
                observation
                    .physical_observations
                    .insert(format!("external:{}", external.path), captured.fingerprint);
                observation
                    .physical_modes
                    .insert(external.path.clone(), captured.mode);
                observation.add(captured.input, &request.policy)?;
            }
            None => {
                observation.issues.insert(if external.required {
                    CaptureIssue::MissingRequiredInput(external.path.clone())
                } else {
                    CaptureIssue::MissingOptionalInput(external.path.clone())
                });
            }
        }
    }
    check_required(request, &mut observation)?;
    Ok(observation)
}

fn add_immutable_inputs(
    request: &CaptureRequest,
    observation: &mut Observation,
) -> Result<(), CaptureError> {
    for (path, overlay) in &request.overlays {
        if let Some(reason) = excluded(path, &request.policy) {
            observation.exclusions.insert(path.clone(), reason);
            observation
                .issues
                .insert(CaptureIssue::ExcludedInput(path.clone()));
            continue;
        }
        observation
            .overlay_versions
            .insert(path.clone(), overlay.version);
        observation.add(
            WorkspaceInput {
                path: path.clone(),
                layer: InputLayer::Overlay,
                kind: InputKind::File,
                mode: overlay.mode,
                bytes: overlay.bytes.clone(),
            },
            &request.policy,
        )?;
    }
    for input in &request.immutable_inputs {
        if let Some(reason) = excluded(&input.path, &request.policy) {
            observation.exclusions.insert(input.path.clone(), reason);
            observation
                .issues
                .insert(CaptureIssue::ExcludedInput(input.path.clone()));
            continue;
        }
        if observation
            .inputs
            .keys()
            .any(|(layer, path)| *layer != InputLayer::Overlay && path == &input.path)
        {
            return Err(CaptureError::ExternalCollision(input.path.clone()));
        }
        observation.add(input.clone(), &request.policy)?;
    }
    Ok(())
}

fn check_required(
    request: &CaptureRequest,
    observation: &mut Observation,
) -> Result<(), CaptureError> {
    let provisional = Snapshot::new(
        request.scope.clone(),
        request.revision,
        SnapshotManifest {
            base_commit: observation.base.clone(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy {
                excluded_paths: observation.exclusions.clone(),
                ..InclusionPolicy::complete()
            },
            provenance: "celetixo/capture-census/v1".into(),
            inputs: observation.inputs.values().cloned().collect(),
            environment: BTreeMap::new(),
        },
        State::new(),
    )?;
    let effective = effective_files(&provisional)?;
    for path in effective.keys() {
        for (position, _) in path.match_indices('/') {
            if effective
                .get(&path[..position])
                .is_some_and(|file| file.kind != InputKind::Submodule)
            {
                return Err(CaptureError::InvalidPolicy(
                    "effective inputs cross a file or symlink ancestor",
                ));
            }
        }
    }
    for required in &request.policy.required_paths {
        if !effective.contains_key(required) {
            observation
                .issues
                .insert(CaptureIssue::MissingRequiredInput(required.clone()));
        }
    }
    Ok(())
}

fn git(
    root: &Path,
    args: &[&str],
    policy: &CapturePolicy,
    allow_failure: bool,
) -> Result<Option<Vec<u8>>, CaptureError> {
    let operation = args.first().copied().unwrap_or("unknown").to_string();
    // Resolve the subprocess working directory through an already opened handle.
    // CLOEXEC closes the inherited descriptor at exec, after the child's chdir.
    // A repository ancestor swapped for a symlink cannot redirect a Git read.
    #[cfg(target_os = "linux")]
    let directory_handle = physical::open_root(root)?;
    #[cfg(target_os = "linux")]
    let process_directory = {
        use std::os::fd::AsRawFd;
        PathBuf::from(format!("/proc/self/fd/{}", directory_handle.as_raw_fd()))
    };
    #[cfg(not(target_os = "linux"))]
    let process_directory = root.to_path_buf();
    let mut child = Command::new("git")
        .current_dir(&process_directory)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| CaptureError::Git {
            operation: operation.clone(),
        })?;
    let stdout = child.stdout.take().ok_or_else(|| CaptureError::Git {
        operation: operation.clone(),
    })?;
    let limit = policy.max_total_bytes.saturating_add(1);
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout.take(limit).read_to_end(&mut output).map(|_| output)
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < policy.git_timeout => {
                std::thread::sleep(Duration::from_millis(2))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(CaptureError::Git { operation });
            }
        }
    };
    let output = reader
        .join()
        .map_err(|_| CaptureError::Git {
            operation: operation.clone(),
        })?
        .map_err(|_| CaptureError::Git {
            operation: operation.clone(),
        })?;
    if output.len() as u64 > policy.max_total_bytes {
        return Err(CaptureError::Limit("Git output bytes"));
    }
    if !status.success() {
        return if allow_failure && status.code() == Some(1) {
            Ok(None)
        } else {
            Err(CaptureError::Git { operation })
        };
    }
    Ok(Some(output))
}

fn required_git(
    root: &Path,
    args: &[&str],
    policy: &CapturePolicy,
) -> Result<Vec<u8>, CaptureError> {
    git(root, args, policy, false)?.ok_or_else(|| CaptureError::Git {
        operation: args[0].into(),
    })
}

fn validate_repository_root(root: &Path, policy: &CapturePolicy) -> Result<(), CaptureError> {
    let top = required_git(root, &["rev-parse", "--show-toplevel"], policy)?;
    let top = std::str::from_utf8(&top)
        .map_err(|_| CaptureError::InvalidPath)?
        .trim_end_matches('\n');
    if Path::new(top) != root {
        return Err(CaptureError::InvalidPolicy(
            "capture root must be the repository workspace root",
        ));
    }
    Ok(())
}

/// Validate the returned object's bytes, rather than trust its database pathname.
fn git_object(
    root: &Path,
    object_kind: &str,
    oid: &str,
    policy: &CapturePolicy,
) -> Result<Vec<u8>, CaptureError> {
    let bytes = required_git(root, &["cat-file", object_kind, oid], policy)?;
    if bytes.len() as u64 > policy.max_file_bytes {
        return Err(CaptureError::Limit("Git object bytes"));
    }
    let prefix = format!("{object_kind} {}\0", bytes.len());
    let actual = if oid.len() == 40 {
        let mut hash = sha1::Sha1::new();
        hash.update(prefix.as_bytes());
        hash.update(&bytes);
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    } else {
        let mut hash = sha2::Sha256::new();
        hash.update(prefix.as_bytes());
        hash.update(&bytes);
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    if actual != oid.to_ascii_lowercase() {
        return Err(CaptureError::Git {
            operation: "object digest validation".into(),
        });
    }
    Ok(bytes)
}

fn verified_tree_entries(
    root: &Path,
    commit: &str,
    policy: &CapturePolicy,
) -> Result<Vec<GitEntry>, CaptureError> {
    let bytes = git_object(root, "commit", commit, policy)?;
    let line = bytes
        .split(|b| *b == b'\n')
        .next()
        .ok_or(CaptureError::InvalidPath)?;
    let tree = line
        .strip_prefix(b"tree ")
        .ok_or(CaptureError::InvalidPath)?;
    let tree = std::str::from_utf8(tree).map_err(|_| CaptureError::InvalidPath)?;
    let mut entries = Vec::new();
    walk_git_tree(root, tree, "", policy, &mut entries, 0)?;
    Ok(entries)
}

fn walk_git_tree(
    root: &Path,
    oid: &str,
    prefix: &str,
    policy: &CapturePolicy,
    entries: &mut Vec<GitEntry>,
    depth: usize,
) -> Result<(), CaptureError> {
    if depth > policy.max_depth {
        return Err(CaptureError::Limit("Git tree depth"));
    }
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CaptureError::InvalidPath);
    }
    let bytes = git_object(root, "tree", oid, policy)?;
    let hash_bytes = oid.len() / 2;
    let mut remaining = bytes.as_slice();
    while !remaining.is_empty() {
        let space = remaining
            .iter()
            .position(|b| *b == b' ')
            .ok_or(CaptureError::InvalidPath)?;
        let zero = remaining
            .iter()
            .position(|b| *b == 0)
            .ok_or(CaptureError::InvalidPath)?;
        if space >= zero || remaining.len() < zero + 1 + hash_bytes {
            return Err(CaptureError::InvalidPath);
        }
        let mode =
            std::str::from_utf8(&remaining[..space]).map_err(|_| CaptureError::InvalidPath)?;
        let name = std::str::from_utf8(&remaining[space + 1..zero])
            .map_err(|_| CaptureError::InvalidPath)?;
        canonical_path(name)?;
        if name.contains('/') {
            return Err(CaptureError::InvalidPath);
        }
        let path = prefixed(prefix, name);
        let child_oid: String = remaining[zero + 1..zero + 1 + hash_bytes]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if mode == "40000" {
            walk_git_tree(root, &child_oid, &path, policy, entries, depth + 1)?;
        } else {
            let mode = match mode {
                "100644" => FileMode::Regular,
                "100755" => FileMode::Executable,
                "120000" => FileMode::Symlink,
                "160000" => FileMode::Gitlink,
                _ => return Err(CaptureError::UnsupportedObject(path)),
            };
            entries.push(GitEntry {
                path,
                oid: child_oid,
                mode,
                stage: 0,
            });
            if entries.len() > policy.max_files {
                return Err(CaptureError::Limit("Git tree entries"));
            }
        }
        remaining = &remaining[zero + 1 + hash_bytes..];
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct GitEntry {
    path: String,
    oid: String,
    mode: FileMode,
    stage: u8,
}

fn parse_git_entries(bytes: &[u8], index: bool) -> Result<Vec<GitEntry>, CaptureError> {
    let mut entries = Vec::new();
    for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or(CaptureError::InvalidPath)?;
        let header = std::str::from_utf8(&record[..tab]).map_err(|_| CaptureError::InvalidPath)?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| CaptureError::InvalidPath)?
            .to_string();
        canonical_path(&path)?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len() != 3 {
            return Err(CaptureError::InvalidPath);
        }
        let mode = match fields[0] {
            "100644" => FileMode::Regular,
            "100755" => FileMode::Executable,
            "120000" => FileMode::Symlink,
            "160000" => FileMode::Gitlink,
            _ => return Err(CaptureError::UnsupportedObject(path)),
        };
        let oid = fields[if index { 1 } else { 2 }].to_string();
        if !oid.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(oid.len(), 40 | 64) {
            return Err(CaptureError::InvalidPath);
        }
        let stage = if index {
            fields[2].parse().map_err(|_| CaptureError::InvalidPath)?
        } else {
            0
        };
        entries.push(GitEntry {
            path,
            oid,
            mode,
            stage,
        });
    }
    Ok(entries)
}

fn kind(mode: FileMode) -> InputKind {
    match mode {
        FileMode::Symlink => InputKind::Symlink,
        FileMode::Gitlink => InputKind::Submodule,
        _ => InputKind::File,
    }
}

fn index_extended_flags(root: &Path, policy: &CapturePolicy) -> Result<Vec<u8>, CaptureError> {
    let bytes = required_git(root, &["ls-files", "--debug", "-z"], policy)?;
    let mut remaining = bytes.as_slice();
    let mut canonical = Vec::new();
    while !remaining.is_empty() {
        let end = remaining
            .iter()
            .position(|b| *b == 0)
            .ok_or(CaptureError::InvalidPath)?;
        let path = &remaining[..end];
        canonical_path(std::str::from_utf8(path).map_err(|_| CaptureError::InvalidPath)?)?;
        remaining = &remaining[end + 1..];
        let mut lines = remaining.splitn(6, |b| *b == b'\n');
        let mut flags = None;
        for _ in 0..5 {
            let line = lines.next().ok_or(CaptureError::InvalidPath)?;
            if let Some(position) = line.windows(7).position(|window| window == b"flags: ") {
                let value = &line[position + 7..];
                if value.is_empty() || !value.iter().all(u8::is_ascii_hexdigit) {
                    return Err(CaptureError::InvalidPath);
                }
                flags = Some(value);
            }
        }
        let flags = flags.ok_or(CaptureError::InvalidPath)?;
        canonical.extend_from_slice(&(path.len() as u64).to_be_bytes());
        canonical.extend_from_slice(path);
        canonical.extend_from_slice(&(flags.len() as u64).to_be_bytes());
        canonical.extend_from_slice(flags);
        remaining = lines.next().unwrap_or_default();
    }
    Ok(canonical)
}

fn capture_git_configuration_inputs(
    root: &Path,
    prefix: &str,
    policy: &CapturePolicy,
    output: &mut Observation,
) -> Result<(), CaptureError> {
    let mut inputs = Vec::new();
    for name in ["info/attributes", "info/exclude"] {
        let path = required_git(
            root,
            &["rev-parse", "--path-format=absolute", "--git-path", name],
            policy,
        )?;
        let path = std::str::from_utf8(&path)
            .map_err(|_| CaptureError::InvalidPath)?
            .trim_end_matches('\n');
        if path.chars().any(char::is_control) {
            return Err(CaptureError::InvalidPath);
        }
        inputs.push((format!(".git/{name}"), PathBuf::from(path), false));
    }
    if let Some(path) = git(
        root,
        &["config", "--path", "--get", "core.attributesFile"],
        policy,
        true,
    )? {
        let path = std::str::from_utf8(&path)
            .map_err(|_| CaptureError::InvalidPath)?
            .trim_end_matches('\n');
        if path.is_empty() || path.chars().any(char::is_control) {
            return Err(CaptureError::InvalidPath);
        }
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        inputs.push((".git/configured-attributes".into(), path, true));
    }
    for (logical, source, required) in inputs {
        let logical = prefixed(prefix, &logical);
        if let Some(reason) = excluded(&logical, policy) {
            output.exclusions.insert(logical.clone(), reason);
            output.issues.insert(CaptureIssue::ExcludedInput(logical));
            continue;
        }
        let external = ExternalInput {
            path: logical.clone(),
            source,
            layer: InputLayer::Mounted,
            required,
        };
        match read_external(&external, policy)? {
            Some(captured) => {
                if captured.input.kind != InputKind::File {
                    return Err(CaptureError::InvalidPolicy(
                        "Git configuration input must be a regular file",
                    ));
                }
                output
                    .physical_observations
                    .insert(format!("git-input:{logical}"), captured.fingerprint);
                output.physical_modes.insert(logical, captured.mode);
                output.add(captured.input, policy)?;
            }
            None if required => {
                output
                    .issues
                    .insert(CaptureIssue::MissingRequiredInput(logical));
            }
            None => {}
        }
    }
    Ok(())
}

fn capture_repository(
    root: &Path,
    prefix: &str,
    request: &CaptureRequest,
    output: &mut Observation,
    depth: usize,
) -> Result<(), CaptureError> {
    if depth > request.policy.max_depth {
        return Err(CaptureError::Limit("submodule depth"));
    }
    let policy = &request.policy;
    validate_repository_root(root, policy)?;
    let head_bytes = git(root, &["rev-parse", "--verify", "-q", "HEAD"], policy, true)?;
    let head = head_bytes
        .as_ref()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_string())
        .unwrap_or_else(|| "unborn".into());
    if prefix.is_empty() {
        output.base = head.clone();
    }
    let config = required_git(
        root,
        &["config", "--null", "--list", "--local", "--includes"],
        policy,
    )?;
    // Per-worktree overrides are distinct from common repository config when
    // extensions.worktreeConfig is enabled. In ordinary repositories Git's
    // --worktree view falls back to --local; retain both versioned views.
    let worktree_config = required_git(
        root,
        &["config", "--null", "--list", "--worktree", "--includes"],
        policy,
    )?;
    let index_bytes = required_git(root, &["ls-files", "--stage", "-z"], policy)?;
    let flags = required_git(root, &["ls-files", "-v", "-z"], policy)?;
    let extended_flags = index_extended_flags(root, policy)?;
    let base_entries = if head == "unborn" {
        Vec::new()
    } else {
        verified_tree_entries(root, &head, policy)?
    };
    let index_entries = parse_git_entries(&index_bytes, true)?;
    let fingerprint_name = if prefix.is_empty() {
        "root".into()
    } else {
        Digest::of("celetixo/submodule-path/v1", prefix.as_bytes()).to_string()
    };
    for (label, bytes) in [
        ("head", head.as_bytes()),
        ("index", index_bytes.as_slice()),
        ("index-flags", flags.as_slice()),
        ("index-extended-flags", extended_flags.as_slice()),
        ("git-config", config.as_slice()),
        ("git-worktree-config", worktree_config.as_slice()),
    ] {
        output.fingerprints.insert(
            format!("{fingerprint_name}.{label}"),
            Digest::of("celetixo/git-capture-input/v1", bytes),
        );
    }
    let git_metadata_path = prefixed(prefix, ".git");
    output.exclusions.insert(git_metadata_path, "Git administration; HEAD, index, object contents, effective configuration, attributes and exclude inputs captured separately".into());
    capture_git_configuration_inputs(root, prefix, policy, output)?;
    let mut tracked = BTreeMap::new();
    let mut indexed = BTreeMap::new();
    let mut submodules = BTreeSet::new();
    for (layer, entries) in [
        (InputLayer::Base, &base_entries),
        (InputLayer::Index, &index_entries),
    ] {
        for entry in entries {
            let path = prefixed(prefix, &entry.path);
            if let Some(reason) = excluded(&path, policy) {
                output.exclusions.insert(path.clone(), reason);
                output.issues.insert(CaptureIssue::ExcludedInput(path));
                continue;
            }
            if layer == InputLayer::Index && entry.stage != 0 {
                output.issues.insert(CaptureIssue::UnmergedIndex(path));
                continue;
            }
            if layer == InputLayer::Index && entry.mode == FileMode::Gitlink {
                submodules.insert(entry.path.clone());
            }
            let bytes = if entry.mode == FileMode::Gitlink {
                entry.oid.as_bytes().to_vec()
            } else if entry.oid.bytes().all(|b| b == b'0') {
                Vec::new()
            } else {
                git_object(root, "blob", &entry.oid, policy)?
            };
            output.add(
                WorkspaceInput {
                    path: path.clone(),
                    layer,
                    kind: kind(entry.mode),
                    mode: entry.mode,
                    bytes: Some(bytes),
                },
                policy,
            )?;
            tracked.insert(entry.path.clone(), entry.mode);
            if layer == InputLayer::Index {
                indexed.insert(entry.path.clone(), entry.mode);
            }
        }
    }
    // Absence from stage zero is meaningful too. Without an explicit index
    // tombstone, a staged deletion could resurrect the base Gitlink/file in
    // the effective view. A recreated path is an untracked input until staged.
    for entry in &base_entries {
        let path = prefixed(prefix, &entry.path);
        if !indexed.contains_key(&entry.path) && excluded(&path, policy).is_none() {
            output.add(
                WorkspaceInput {
                    path,
                    layer: InputLayer::Index,
                    kind: kind(entry.mode),
                    mode: entry.mode,
                    bytes: None,
                },
                policy,
            )?;
        }
    }
    let present = walk_workspace(root, prefix, &indexed, &submodules, policy, output)?;
    for (path, mode) in &tracked {
        if !present.contains(path) && *mode != FileMode::Gitlink {
            output.add(
                WorkspaceInput {
                    path: prefixed(prefix, path),
                    layer: InputLayer::WorkingTree,
                    kind: kind(*mode),
                    mode: *mode,
                    bytes: None,
                },
                policy,
            )?;
        }
    }
    for submodule in submodules {
        let path = prefixed(prefix, &submodule);
        let sub_root = root.join(&submodule);
        // An initialized checkout must have its own .git marker. Otherwise Git
        // would discover the parent repository and falsely report it as present.
        if !safe_is_directory(root, &submodule)? || !sub_root.join(".git").exists() {
            output
                .issues
                .insert(CaptureIssue::UninitializedSubmodule(path));
            continue;
        }
        let sub_head = required_git(&sub_root, &["rev-parse", "--verify", "HEAD"], policy)?;
        output.add(
            WorkspaceInput {
                path: path.clone(),
                layer: InputLayer::Submodule,
                kind: InputKind::Submodule,
                mode: FileMode::Gitlink,
                bytes: Some(sub_head.iter().copied().filter(|b| *b != b'\n').collect()),
            },
            policy,
        )?;
        capture_repository(&sub_root, &path, request, output, depth + 1)?;
    }
    if head_bytes != git(root, &["rev-parse", "--verify", "-q", "HEAD"], policy, true)?
        || index_bytes != required_git(root, &["ls-files", "--stage", "-z"], policy)?
        || flags != required_git(root, &["ls-files", "-v", "-z"], policy)?
        || extended_flags != index_extended_flags(root, policy)?
        || config
            != required_git(
                root,
                &["config", "--null", "--list", "--local", "--includes"],
                policy,
            )?
        || worktree_config
            != required_git(
                root,
                &["config", "--null", "--list", "--worktree", "--includes"],
                policy,
            )?
    {
        return Err(CaptureError::ChangedDuringCapture);
    }
    Ok(())
}

#[cfg(unix)]
mod physical {
    use super::*;
    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
    use std::os::unix::fs::MetadataExt;

    fn io_at(path: &str, error: impl Into<io::Error>) -> CaptureError {
        CaptureError::Io {
            path: path.into(),
            source: error.into(),
        }
    }
    pub(super) fn open_root(root: &Path) -> Result<File, CaptureError> {
        // Walk absolute ancestors too: external input roots must not smuggle a
        // symlink ancestor into an otherwise no-follow final-component open.
        let absolute = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| io_at("<capture-root>", e))?
                .join(root)
        };
        let mut fd = File::from(
            rustix::fs::open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| io_at("<capture-root>", e))?,
        );
        for component in absolute.components() {
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
                        .map_err(|e| io_at("<capture-root>", e))?,
                    );
                }
                _ => return Err(CaptureError::InvalidPath),
            }
        }
        Ok(fd)
    }
    fn fingerprint(metadata: &std::fs::Metadata) -> (u64, u64, u64, u64, i64, i64, i64, i64) {
        (
            metadata.dev(),
            metadata.ino(),
            metadata.mode() as u64,
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    }
    fn read_entry(
        parent: &File,
        name: &std::ffi::OsStr,
        path: &str,
        layer: InputLayer,
        policy: &CapturePolicy,
    ) -> Result<Option<WorkspaceInput>, CaptureError> {
        let before = match rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(io_at(path, error)),
        };
        let file_type = FileType::from_raw_mode(before.st_mode);
        let (mode, bytes) = if file_type == FileType::Symlink {
            let target =
                rustix::fs::readlinkat(parent, name, Vec::new()).map_err(|e| io_at(path, e))?;
            (FileMode::Symlink, target.into_bytes())
        } else if file_type == FileType::RegularFile {
            if before.st_size < 0 || before.st_size as u64 > policy.max_file_bytes {
                return Err(CaptureError::Limit("individual file bytes"));
            }
            let file = File::from(
                rustix::fs::openat(
                    parent,
                    name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|e| io_at(path, e))?,
            );
            let metadata = file.metadata().map_err(|e| io_at(path, e))?;
            if !metadata.is_file()
                || metadata.ino() != before.st_ino
                || metadata.dev() != before.st_dev
            {
                return Err(CaptureError::ChangedDuringCapture);
            }
            let initial = fingerprint(&metadata);
            let mut bytes = Vec::new();
            (&file)
                .take(policy.max_file_bytes.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| io_at(path, e))?;
            if bytes.len() as u64 > policy.max_file_bytes {
                return Err(CaptureError::Limit("individual file bytes"));
            }
            if initial != fingerprint(&file.metadata().map_err(|e| io_at(path, e))?) {
                return Err(CaptureError::ChangedDuringCapture);
            }
            let mode = if metadata.mode() & 0o111 != 0 {
                FileMode::Executable
            } else {
                FileMode::Regular
            };
            (mode, bytes)
        } else {
            return Err(CaptureError::UnsupportedObject(path.into()));
        };
        let after = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| CaptureError::ChangedDuringCapture)?;
        if before.st_dev != after.st_dev
            || before.st_ino != after.st_ino
            || before.st_mode != after.st_mode
            || before.st_size != after.st_size
            || before.st_mtime != after.st_mtime
            || before.st_mtime_nsec != after.st_mtime_nsec
            || before.st_ctime != after.st_ctime
            || before.st_ctime_nsec != after.st_ctime_nsec
        {
            return Err(CaptureError::ChangedDuringCapture);
        }
        Ok(Some(WorkspaceInput {
            path: path.into(),
            layer,
            kind: kind(mode),
            mode,
            bytes: Some(bytes),
        }))
    }
    pub(super) fn walk_workspace(
        root: &Path,
        prefix: &str,
        tracked: &BTreeMap<String, FileMode>,
        submodules: &BTreeSet<String>,
        policy: &CapturePolicy,
        output: &mut Observation,
    ) -> Result<BTreeSet<String>, CaptureError> {
        let root_fd = open_root(root)?;
        let mut present = BTreeSet::new();
        let mut walker = Walker {
            prefix,
            tracked,
            submodules,
            policy,
            output,
            present: &mut present,
        };
        walker.walk(&root_fd, "", 0)?;
        Ok(present)
    }
    struct Walker<'a> {
        prefix: &'a str,
        tracked: &'a BTreeMap<String, FileMode>,
        submodules: &'a BTreeSet<String>,
        policy: &'a CapturePolicy,
        output: &'a mut Observation,
        present: &'a mut BTreeSet<String>,
    }
    impl Walker<'_> {
        fn walk(&mut self, fd: &File, relative: &str, depth: usize) -> Result<(), CaptureError> {
            if depth > self.policy.max_depth {
                return Err(CaptureError::Limit("directory depth"));
            }
            if self.output.directory_modes.len() >= self.policy.max_files {
                return Err(CaptureError::Limit("directory census entries"));
            }
            let before = fingerprint(&fd.metadata().map_err(|e| io_at(relative, e))?);
            self.output.physical_observations.insert(
                format!("directory:{}", prefixed(self.prefix, relative)),
                format!("{before:?}"),
            );
            self.output
                .directory_modes
                .insert(prefixed(self.prefix, relative), before.2 as u32);
            let dir = Dir::read_from(fd).map_err(|e| io_at(relative, e))?;
            let mut names = Vec::new();
            for entry in dir {
                let entry = entry.map_err(|e| io_at(relative, e))?;
                let name = entry.file_name().to_bytes();
                if name == b"." || name == b".." {
                    continue;
                }
                let name = std::str::from_utf8(name)
                    .map_err(|_| CaptureError::InvalidPath)?
                    .to_string();
                canonical_path(&name)?;
                names.push(name);
                if names.len() > self.policy.max_files {
                    return Err(CaptureError::Limit("directory entries"));
                }
            }
            names.sort();
            for name in names {
                let local = prefixed(relative, &name);
                let logical = prefixed(self.prefix, &local);
                if name == ".git" {
                    if !relative.is_empty() {
                        self.output
                            .issues
                            .insert(CaptureIssue::NestedRepository(prefixed(
                                self.prefix,
                                relative,
                            )));
                    }
                    self.output.exclusions.insert(logical, "Git administration; HEAD, index, object contents, effective configuration, attributes and exclude inputs captured separately".into());
                    continue;
                }
                if let Some(reason) = excluded(&logical, self.policy) {
                    self.output.exclusions.insert(logical.clone(), reason);
                    self.output
                        .issues
                        .insert(CaptureIssue::ExcludedInput(logical));
                    continue;
                }
                let stat = rustix::fs::statat(fd, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|_| CaptureError::ChangedDuringCapture)?;
                if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
                    if self.submodules.contains(&local) {
                        self.present.insert(local);
                        continue;
                    }
                    let child = File::from(
                        rustix::fs::openat(
                            fd,
                            name.as_str(),
                            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                            Mode::empty(),
                        )
                        .map_err(|_| CaptureError::ChangedDuringCapture)?,
                    );
                    self.walk(&child, &local, depth + 1)?;
                    let after = rustix::fs::statat(fd, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|_| CaptureError::ChangedDuringCapture)?;
                    if stat.st_dev != after.st_dev || stat.st_ino != after.st_ino {
                        return Err(CaptureError::ChangedDuringCapture);
                    }
                } else {
                    let layer = if self.tracked.contains_key(&local) {
                        InputLayer::WorkingTree
                    } else {
                        InputLayer::Untracked
                    };
                    let input = read_entry(fd, name.as_ref(), &logical, layer, self.policy)?
                        .ok_or(CaptureError::ChangedDuringCapture)?;
                    let after = rustix::fs::statat(fd, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|_| CaptureError::ChangedDuringCapture)?;
                    self.output.physical_observations.insert(
                        format!("file:{logical}"),
                        format!(
                            "{}:{}:{}:{}:{}:{}:{}:{}",
                            after.st_dev,
                            after.st_ino,
                            after.st_mode,
                            after.st_size,
                            after.st_mtime,
                            after.st_mtime_nsec,
                            after.st_ctime,
                            after.st_ctime_nsec
                        ),
                    );
                    self.output.physical_modes.insert(logical, after.st_mode);
                    self.output.add(input, self.policy)?;
                    self.present.insert(local);
                }
            }
            if before != fingerprint(&fd.metadata().map_err(|e| io_at(relative, e))?) {
                return Err(CaptureError::ChangedDuringCapture);
            }
            Ok(())
        }
    }
    pub(super) fn read_external(
        external: &ExternalInput,
        policy: &CapturePolicy,
    ) -> Result<Option<ExternalObservation>, CaptureError> {
        let parent = external.source.parent().ok_or(CaptureError::InvalidPath)?;
        let name = external
            .source
            .file_name()
            .ok_or(CaptureError::InvalidPath)?;
        let fd = match open_root(parent) {
            Ok(fd) => fd,
            Err(CaptureError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let before = match rustix::fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(io_at(&external.path, error)),
        };
        let input = read_entry(&fd, name, &external.path, external.layer, policy)?
            .ok_or(CaptureError::ChangedDuringCapture)?;
        let after = rustix::fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| CaptureError::ChangedDuringCapture)?;
        let observation = |stat: &rustix::fs::Stat| {
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{}",
                stat.st_dev,
                stat.st_ino,
                stat.st_mode,
                stat.st_size,
                stat.st_mtime,
                stat.st_mtime_nsec,
                stat.st_ctime,
                stat.st_ctime_nsec
            )
        };
        let fingerprint = observation(&after);
        if observation(&before) != fingerprint {
            return Err(CaptureError::ChangedDuringCapture);
        }
        Ok(Some(ExternalObservation {
            input,
            fingerprint,
            mode: after.st_mode,
        }))
    }
    pub(super) fn safe_is_directory(root: &Path, path: &str) -> Result<bool, CaptureError> {
        match open_root(&root.join(path)) {
            Ok(_) => Ok(true),
            Err(CaptureError::Io { source, .. })
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(unix)]
use physical::{read_external, safe_is_directory, walk_workspace};

#[cfg(not(unix))]
fn walk_workspace(
    _: &Path,
    _: &str,
    _: &BTreeMap<String, FileMode>,
    _: &BTreeSet<String>,
    _: &CapturePolicy,
    _: &mut Observation,
) -> Result<BTreeSet<String>, CaptureError> {
    Err(CaptureError::UnsupportedPlatform)
}
#[cfg(not(unix))]
fn read_external(
    _: &ExternalInput,
    _: &CapturePolicy,
) -> Result<Option<ExternalObservation>, CaptureError> {
    Err(CaptureError::UnsupportedPlatform)
}
#[cfg(not(unix))]
fn safe_is_directory(_: &Path, _: &str) -> Result<bool, CaptureError> {
    Err(CaptureError::UnsupportedPlatform)
}
