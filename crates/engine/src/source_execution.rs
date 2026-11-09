//! Pure, versioned source-verification policy and planning contracts.
//!
//! Constructing a policy or plan conveys no authorization. The Engine installs
//! policy, rederives requirements, validates capture provenance and admits work.
//! This module never starts a process, accesses a cache or publishes source.

use crate::credentials::{CredentialError, CredentialPolicy, credential_key, credential_material};
use crate::evidence::CheckSpec;
use crate::ids::{
    Canonical, CheckId, Digest, ElementId, InvariantId, Scope, SnapshotId, SourcePatchId,
    SourcePlanId,
};
use crate::snapshot::{FileMode, InputKind, Snapshot, effective_workspace};
use crate::source_patch::{
    OperationKind, PatchOperation, PatchPolicy, Preimage, SourcePatch, validate_source_path,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourcePlanError {
    #[error("invalid source execution policy: {0}")]
    InvalidPolicy(&'static str),
    #[error("invalid check definition: {0}")]
    InvalidCheck(&'static str),
    #[error("invalid source-relative path: {0:?}")]
    InvalidPath(String),
    #[error("check identity is duplicated: {0}")]
    DuplicateCheck(CheckId),
    #[error("required check is missing: {0}")]
    MissingCheck(CheckId),
    #[error("declared check substitutes policy-owned configuration: {0}")]
    SubstitutedCheck(CheckId),
    #[error("required source invariant is missing: {0}")]
    MissingInvariant(InvariantId),
    #[error("source check dependency graph contains a cycle")]
    DependencyCycle,
    #[error("check exceeds a policy resource ceiling: {0}")]
    ResourceCeiling(CheckId),
    #[error("check requests an unapproved execution tier or backend: {0}")]
    UnapprovedBackend(CheckId),
    #[error("policy, source or analysis binding differs from the plan")]
    BindingMismatch,
    #[error("capture is incomplete or not an immutable source")]
    IncompleteCapture,
    #[error("patch policy differs from the installed policy")]
    PatchPolicyMismatch,
    #[error("patch path has no sufficient policy-owned write authorization: {0}")]
    UnauthorizedPath(String),
    #[error("path rule does not match the exact base preimage: {0}")]
    PathPreimage(String),
    #[error("a source check cannot have both a required and optional declaration: {0}")]
    ConflictingRequirement(CheckId),
    #[error("cache identity names a check outside this plan")]
    UnknownCacheCheck,
    #[error("required check input differs from the exact resulting source: {0:?}")]
    CheckInputMismatch(String),
    #[error("credential admission policy rejected: {0}")]
    Credential(#[from] CredentialError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExecutionTier {
    T0,
    T1,
    T2,
    T3,
}

impl ExecutionTier {
    fn tag(self) -> u64 {
        match self {
            Self::T0 => 0,
            Self::T1 => 1,
            Self::T2 => 2,
            Self::T3 => 3,
        }
    }
}

/// Every field is a maximum. A backend must enforce the complete contract;
/// this value is not evidence that a particular backend implements it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimits {
    pub wall_time_ms: u64,
    pub cpu_time_ms: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub process_count: u64,
    pub file_descriptors: u64,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub artifact_bytes: u64,
}

impl ResourceLimits {
    pub fn validate(&self) -> Result<(), SourcePlanError> {
        if self.values().contains(&0) {
            return Err(SourcePlanError::InvalidCheck("zero resource limit"));
        }
        Ok(())
    }

    pub fn is_within(&self, ceiling: &Self) -> bool {
        self.values()
            .into_iter()
            .zip(ceiling.values())
            .all(|(requested, maximum)| requested > 0 && requested <= maximum)
    }

    fn values(&self) -> [u64; 9] {
        [
            self.wall_time_ms,
            self.cpu_time_ms,
            self.memory_bytes,
            self.disk_bytes,
            self.process_count,
            self.file_descriptors,
            self.stdout_bytes,
            self.stderr_bytes,
            self.artifact_bytes,
        ]
    }

    fn encode(&self, c: &mut Canonical) {
        for value in self.values() {
            c.u64(value);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingDirectory {
    WorkspaceRoot,
    Relative(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendIdentity {
    pub name: String,
    pub version: String,
    pub image: Digest,
    pub profile: Digest,
}

impl BackendIdentity {
    pub fn digest(&self) -> Digest {
        let mut c = Canonical::default();
        self.encode(&mut c);
        c.finish("celetixo/source-backend/v1")
    }

    fn encode(&self, c: &mut Canonical) {
        c.str(&self.name);
        c.str(&self.version);
        c.digest(self.image);
        c.digest(self.profile);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainIdentity {
    /// Absolute path inside the isolated execution root, never a host lookup.
    pub executable: String,
    pub executable_digest: Digest,
    /// Includes libraries, interpreter/runtime files and trusted dependencies.
    pub runtime_closure: Digest,
    pub version: String,
    pub platform: String,
    pub architecture: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NetworkDestination {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkPolicy {
    DenyAll,
    /// A planner records this grant; only a backend validating and enforcing
    /// destination resolution, expiry and the capability may execute it.
    ScopedAllowlist {
        destinations: BTreeSet<NetworkDestination>,
        grant: Digest,
        expires_at_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentValue {
    Literal(String),
    /// `credential-ref://organization/workspace/repository/name` only.
    ProtectedReference(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRequirement {
    pub name: String,
    pub media_type: String,
    pub max_bytes: u64,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePolicy {
    FailAttempt,
    /// Only meaningful for an optional check. A mandatory failure always fails.
    RecordOptionalFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckInput {
    pub check: CheckSpec,
    pub working_directory: WorkingDirectory,
    /// Source paths and expected content identities, including scripts/config.
    pub inputs: BTreeMap<String, Digest>,
    /// Relative paths in the separate writable output area, not source paths.
    pub outputs: BTreeSet<String>,
    pub toolchain: ToolchainIdentity,
    pub dependency_inputs: BTreeMap<String, Digest>,
    pub environment: BTreeMap<String, EnvironmentValue>,
    pub tier: ExecutionTier,
    pub backend: BackendIdentity,
    pub resources: ResourceLimits,
    pub network: NetworkPolicy,
    pub credential_references: BTreeSet<String>,
    pub expected_artifacts: Vec<ArtifactRequirement>,
    pub depends_on: BTreeSet<CheckId>,
    pub failure_policy: FailurePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckDefinition {
    input: SourceCheckInput,
    digest: Digest,
}

impl SourceCheckDefinition {
    pub fn new(mut input: SourceCheckInput) -> Result<Self, SourcePlanError> {
        input.resources.validate()?;
        if input.check.command().len() > 4096
            || input
                .check
                .command()
                .iter()
                .any(|arg| arg.len() > 1024 * 1024)
            || input.inputs.len() > 65_536
            || input.outputs.len() > 65_536
            || input.dependency_inputs.len() > 65_536
            || input.environment.len() > 4096
            || input.expected_artifacts.len() > 4096
            || input.depends_on.len() > 4096
            || input.credential_references.len() > 4096
        {
            return Err(SourcePlanError::InvalidCheck("definition budget"));
        }
        if let WorkingDirectory::Relative(path) = &input.working_directory {
            validate_relative(path)?;
        }
        for path in input
            .inputs
            .keys()
            .chain(&input.outputs)
            .chain(input.dependency_inputs.keys())
        {
            validate_relative(path)?;
        }
        if !valid_description(&input.backend.name)
            || !valid_description(&input.backend.version)
            || !valid_description(&input.toolchain.version)
            || !valid_description(&input.toolchain.platform)
            || !valid_description(&input.toolchain.architecture)
            || !valid_guest_executable(&input.toolchain.executable)
            || input.check.command().first() != Some(&input.toolchain.executable)
            || input.check.toolchain() != input.toolchain.version
        {
            return Err(SourcePlanError::InvalidCheck("toolchain/backend identity"));
        }
        if input.tier == ExecutionTier::T0 {
            // T0 is reserved for typed deterministic operations, not argv-based
            // repository programs. A future operation enum can extend it.
            return Err(SourcePlanError::InvalidCheck("commands cannot run in T0"));
        }
        if let NetworkPolicy::ScopedAllowlist {
            destinations,
            expires_at_ms,
            ..
        } = &input.network
            && (destinations.is_empty()
                || *expires_at_ms == 0
                || destinations.iter().any(|d| {
                    d.port == 0
                        || d.host.is_empty()
                        || d.host.len() > 253
                        || !d
                            .host
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':'))
                }))
        {
            return Err(SourcePlanError::InvalidCheck("network grant"));
        }
        for (key, value) in &input.environment {
            if key.is_empty()
                || key.len() > 256
                || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || key.as_bytes()[0].is_ascii_digit()
            {
                return Err(SourcePlanError::InvalidCheck("environment key"));
            }
            match value {
                EnvironmentValue::Literal(value)
                    if value.len() > 16_384
                        || value.contains('\0')
                        || sensitive_environment_key(key)
                        || credential_material(value.as_bytes()) =>
                {
                    return Err(SourcePlanError::InvalidCheck("literal environment value"));
                }
                EnvironmentValue::ProtectedReference(value) if !valid_reference_shape(value) => {
                    return Err(SourcePlanError::InvalidCheck("credential reference"));
                }
                _ => {}
            }
        }
        if input
            .credential_references
            .iter()
            .any(|value| !valid_reference_shape(value))
        {
            return Err(SourcePlanError::InvalidCheck("credential reference"));
        }
        input.expected_artifacts.sort_by(|a, b| a.name.cmp(&b.name));
        let mut names = BTreeSet::new();
        if input.expected_artifacts.is_empty()
            || input.expected_artifacts.iter().any(|artifact| {
                !valid_description(&artifact.name)
                    || !valid_description(&artifact.media_type)
                    || artifact.max_bytes == 0
                    || artifact.max_bytes > input.resources.artifact_bytes
                    || !names.insert(artifact.name.clone())
            })
        {
            return Err(SourcePlanError::InvalidCheck("artifact contract"));
        }
        if ["stdout", "stderr", "diagnostics"].iter().any(|name| {
            !input
                .expected_artifacts
                .iter()
                .any(|artifact| artifact.name == *name && artifact.required)
        }) {
            return Err(SourcePlanError::InvalidCheck("required execution logs"));
        }
        if input.depends_on.contains(input.check.id()) {
            return Err(SourcePlanError::DependencyCycle);
        }
        let mut c = Canonical::default();
        encode_check(&mut c, &input);
        Ok(Self {
            input,
            digest: c.finish("celetixo/source-check/v1"),
        })
    }

    pub fn id(&self) -> &CheckId {
        self.input.check.id()
    }
    pub fn input(&self) -> &SourceCheckInput {
        &self.input
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        encode_check(&mut c, &self.input);
        c.into_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCaptureConsistency {
    ImmutableGitTree,
    /// Deliberately finite, trusted reference fixtures; never parser completeness.
    ReferenceFixture,
    LiveObserved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCaptureBinding {
    pub snapshot: SnapshotId,
    pub manifest: Digest,
    pub capture_policy: Digest,
    pub uncertainty: Digest,
    pub consistency: SourceCaptureConsistency,
    pub gaps: BTreeSet<String>,
}

impl SourceCaptureBinding {
    /// Goal 02 capture-issues/v1 encodes the schema and entry count as raw
    /// big-endian u64 values. This is deliberately distinct from Canonical.
    pub fn empty_uncertainty_digest() -> Digest {
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&1_u64.to_be_bytes());
        bytes.extend_from_slice(&0_u64.to_be_bytes());
        Digest::of("celetixo/capture-issues/v1", &bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePathRule {
    pub allowed_operations: BTreeSet<OperationKind>,
    pub required_writes: BTreeSet<ElementId>,
    pub expected_base: Option<Preimage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInvariantSpec {
    pub id: InvariantId,
    pub scope: BTreeSet<ElementId>,
    pub contract_digest: Digest,
    pub required_checks: BTreeSet<CheckId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePolicyInput {
    pub version: u32,
    pub generation: u64,
    pub scope: Scope,
    pub snapshot: SnapshotId,
    pub analysis_artifact: Digest,
    pub index_generation: Digest,
    pub capture: SourceCaptureBinding,
    /// Exact approved patch policy. Workers cannot widen even one budget.
    pub patch_policy: PatchPolicy,
    /// Trusted governance policy for every resulting input, including bytes
    /// introduced by a patch. Workers cannot supply their own exceptions.
    pub credentials: CredentialPolicy,
    pub paths: BTreeMap<String, SourcePathRule>,
    pub checks: Vec<SourceCheckDefinition>,
    pub invariants: Vec<SourceInvariantSpec>,
    pub always_required: BTreeSet<CheckId>,
    pub approved_tiers: BTreeSet<ExecutionTier>,
    pub approved_backends: BTreeSet<Digest>,
    pub resource_ceiling: ResourceLimits,
    pub network_allowed: bool,
    pub approved_credential_references: BTreeSet<String>,
    pub expires_at_ms: u64,
    pub retention_policy: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceExecutionPolicy {
    input: SourcePolicyInput,
    checks: BTreeMap<CheckId, SourceCheckDefinition>,
    invariants: BTreeMap<InvariantId, SourceInvariantSpec>,
    digest: Digest,
}

impl SourceExecutionPolicy {
    pub fn new(mut input: SourcePolicyInput) -> Result<Self, SourcePlanError> {
        if input.version != 1
            || input.generation == 0
            || input.expires_at_ms == 0
            || input.capture.snapshot != input.snapshot
            || input.paths.is_empty()
            || input.checks.is_empty()
            || input.approved_tiers.is_empty()
            || input.approved_backends.is_empty()
            || input.paths.len() > 65_536
            || input.checks.len() > 4096
            || input.invariants.len() > 4096
        {
            return Err(SourcePlanError::InvalidPolicy("identity or empty policy"));
        }
        input.resource_ceiling.validate()?;
        input.credentials.validate()?;
        validate_source_path("p", &input.patch_policy)
            .map_err(|_| SourcePlanError::InvalidPolicy("patch policy"))?;
        if !input.capture.gaps.is_empty()
            || input.capture.consistency == SourceCaptureConsistency::LiveObserved
        {
            return Err(SourcePlanError::IncompleteCapture);
        }
        for (path, rule) in &input.paths {
            validate_source_path(path, &input.patch_policy)
                .map_err(|_| SourcePlanError::InvalidPath(path.clone()))?;
            if rule.allowed_operations.is_empty() || rule.required_writes.is_empty() {
                return Err(SourcePlanError::InvalidPolicy("empty path authorization"));
            }
        }
        let mut checks = BTreeMap::new();
        for check in &input.checks {
            if checks.insert(check.id().clone(), check.clone()).is_some() {
                return Err(SourcePlanError::DuplicateCheck(check.id().clone()));
            }
            if !check.input.resources.is_within(&input.resource_ceiling) {
                return Err(SourcePlanError::ResourceCeiling(check.id().clone()));
            }
            if !input.approved_tiers.contains(&check.input.tier)
                || !input
                    .approved_backends
                    .contains(&check.input.backend.digest())
            {
                return Err(SourcePlanError::UnapprovedBackend(check.id().clone()));
            }
            if !input.network_allowed && check.input.network != NetworkPolicy::DenyAll {
                return Err(SourcePlanError::InvalidPolicy("network widening"));
            }
            for reference in check.input.credential_references.iter().chain(
                check
                    .input
                    .environment
                    .values()
                    .filter_map(|value| match value {
                        EnvironmentValue::ProtectedReference(reference) => Some(reference),
                        EnvironmentValue::Literal(_) => None,
                    }),
            ) {
                if !reference_in_scope(reference, &input.scope)
                    || !input.approved_credential_references.contains(reference)
                {
                    return Err(SourcePlanError::InvalidPolicy("credential scope or grant"));
                }
            }
        }
        if input
            .approved_credential_references
            .iter()
            .any(|reference| !reference_in_scope(reference, &input.scope))
        {
            return Err(SourcePlanError::InvalidPolicy("credential scope"));
        }
        let mut invariants = BTreeMap::new();
        for invariant in &input.invariants {
            if invariant.scope.is_empty()
                || invariant.required_checks.is_empty()
                || invariants
                    .insert(invariant.id.clone(), invariant.clone())
                    .is_some()
            {
                return Err(SourcePlanError::InvalidPolicy("source invariant"));
            }
        }
        // Resolve the entire registry, not just currently requested checks. A
        // malformed dormant check cannot become runnable via a later request.
        for check in checks.values() {
            for dependency in &check.input.depends_on {
                if !checks.contains_key(dependency) {
                    return Err(SourcePlanError::MissingCheck(dependency.clone()));
                }
            }
        }
        topological_order(&checks, &checks.keys().cloned().collect())?;
        for required in input
            .always_required
            .iter()
            .chain(invariants.values().flat_map(|i| &i.required_checks))
        {
            if !checks.contains_key(required) {
                return Err(SourcePlanError::MissingCheck(required.clone()));
            }
        }
        input.checks.sort_by(|a, b| a.id().cmp(b.id()));
        input.invariants.sort_by(|a, b| a.id.cmp(&b.id));
        let mut c = Canonical::default();
        encode_policy(&mut c, &input);
        Ok(Self {
            input,
            checks,
            invariants,
            digest: c.finish("celetixo/source-execution-policy/v1"),
        })
    }

    pub fn input(&self) -> &SourcePolicyInput {
        &self.input
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        encode_policy(&mut c, &self.input);
        c.into_bytes()
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn checks(&self) -> &BTreeMap<CheckId, SourceCheckDefinition> {
        &self.checks
    }
    pub fn invariants(&self) -> &BTreeMap<InvariantId, SourceInvariantSpec> {
        &self.invariants
    }

    /// Checks immutable manifest bindings only. The Engine must additionally
    /// validate the trusted capture producer/report and installed analysis.
    pub fn validate_snapshot(&self, snapshot: &Snapshot) -> Result<(), SourcePlanError> {
        if snapshot.scope() != &self.input.scope
            || snapshot.id() != self.input.snapshot
            || snapshot.manifest().digest() != self.input.capture.manifest
        {
            return Err(SourcePlanError::BindingMismatch);
        }
        let exclusions = &snapshot.manifest().inclusion_policy.excluded_paths;
        let environment = &snapshot.manifest().environment;
        match self.input.capture.consistency {
            SourceCaptureConsistency::ImmutableGitTree => {
                if snapshot.manifest().provenance
                    != format!(
                        "celetixo/git-capture/v1;policy={};consistency=git-tree/v1",
                        self.input.capture.capture_policy
                    )
                    || environment
                        .get("celetixo.capture.consistency")
                        .map(String::as_str)
                        != Some("git-tree/v1")
                    || environment.get("celetixo.capture.policy")
                        != Some(&self.input.capture.capture_policy.to_string())
                    || environment.get("celetixo.capture.uncertainty")
                        != Some(&self.input.capture.uncertainty.to_string())
                    || self.input.capture.uncertainty
                        != SourceCaptureBinding::empty_uncertainty_digest()
                    // Git administration is not part of a selected immutable
                    // tree. This exact capture-producer explanation is the
                    // sole recognized non-operational exclusion; arbitrary
                    // source exclusions still make capture incomplete.
                    || exclusions.iter().any(|(path, reason)| {
                        path != ".git" || reason != "GitTree source consists of the selected immutable commit, not Git administration or current workspace"
                    })
                {
                    return Err(SourcePlanError::IncompleteCapture);
                }
            }
            SourceCaptureConsistency::ReferenceFixture => {
                if !snapshot
                    .manifest()
                    .provenance
                    .starts_with("celetixo/reference-source/")
                    || !exclusions.is_empty()
                    || environment
                        .keys()
                        .any(|key| key.starts_with("celetixo.capture."))
                {
                    return Err(SourcePlanError::IncompleteCapture);
                }
            }
            SourceCaptureConsistency::LiveObserved => {
                return Err(SourcePlanError::IncompleteCapture);
            }
        }
        Ok(())
    }

    pub fn validate_patch_paths(
        &self,
        patch: &SourcePatch,
        base: &Snapshot,
        effective_writes: &BTreeSet<ElementId>,
    ) -> Result<(), SourcePlanError> {
        self.validate_snapshot(base)?;
        if patch.policy().digest() != self.input.patch_policy.digest() {
            return Err(SourcePlanError::PatchPolicyMismatch);
        }
        if patch.binding().scope != self.input.scope
            || patch.binding().base_snapshot != self.input.snapshot
            || patch.binding().analysis_artifact != self.input.analysis_artifact
            || patch.binding().index_generation != self.input.index_generation
        {
            return Err(SourcePlanError::BindingMismatch);
        }
        let files = effective_workspace(base).map_err(|_| SourcePlanError::BindingMismatch)?;
        for operation in patch.operations() {
            for path in [Some(operation.source_path()), operation.destination_path()]
                .into_iter()
                .flatten()
            {
                let rule = self
                    .input
                    .paths
                    .get(path)
                    .ok_or_else(|| SourcePlanError::UnauthorizedPath(path.into()))?;
                if !rule.allowed_operations.contains(&operation.kind())
                    || !rule.required_writes.is_subset(effective_writes)
                {
                    return Err(SourcePlanError::UnauthorizedPath(path.into()));
                }
                let actual = files
                    .get(path)
                    .map(Preimage::of)
                    .transpose()
                    .map_err(|_| SourcePlanError::PathPreimage(path.into()))?;
                if actual != rule.expected_base {
                    return Err(SourcePlanError::PathPreimage(path.into()));
                }
                if path == operation.source_path() {
                    let expected = match operation {
                        PatchOperation::Create { .. } => None,
                        PatchOperation::Replace { expected, .. }
                        | PatchOperation::Delete { expected, .. }
                        | PatchOperation::Rename { expected, .. }
                        | PatchOperation::SetMode { expected, .. } => Some(*expected),
                    };
                    if expected != actual {
                        return Err(SourcePlanError::PathPreimage(path.into()));
                    }
                }
                if let PatchOperation::Create { .. } = operation
                    && actual.is_some()
                {
                    return Err(SourcePlanError::PathPreimage(path.into()));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePlanBinding {
    pub scope: Scope,
    pub base_snapshot: SnapshotId,
    pub analysis_artifact: Digest,
    pub index_generation: Digest,
    pub impact_request: Digest,
    pub impact_result: Digest,
    pub patch: SourcePatchId,
    pub operation_digest: Digest,
    pub resulting_snapshot: SnapshotId,
    pub resulting_manifest: Digest,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanRequirements {
    pub declared_checks: Vec<CheckSpec>,
    pub impact_checks: BTreeSet<CheckId>,
    pub invariants: BTreeSet<InvariantId>,
    pub optional_checks: BTreeSet<CheckId>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckRationale {
    Declared,
    Impact,
    Policy,
    Invariant(InvariantId),
    DependencyOf(CheckId),
    OptionalRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCheck {
    definition: SourceCheckDefinition,
    required: bool,
    rationales: BTreeSet<CheckRationale>,
}

impl PlannedCheck {
    pub fn definition(&self) -> &SourceCheckDefinition {
        &self.definition
    }
    pub fn required(&self) -> bool {
        self.required
    }
    pub fn rationales(&self) -> &BTreeSet<CheckRationale> {
        &self.rationales
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationPlan {
    id: SourcePlanId,
    policy: Digest,
    binding: SourcePlanBinding,
    checks: Vec<PlannedCheck>,
    invariant_contracts: BTreeMap<InvariantId, Digest>,
}

impl VerificationPlan {
    /// Requirements must be derived and checked by Engine admission. Calling
    /// this pure resolver cannot authenticate or authorize any execution.
    pub fn resolve(
        policy: &SourceExecutionPolicy,
        binding: SourcePlanBinding,
        requirements: PlanRequirements,
    ) -> Result<Self, SourcePlanError> {
        if binding.scope != policy.input.scope
            || binding.base_snapshot != policy.input.snapshot
            || binding.analysis_artifact != policy.input.analysis_artifact
            || binding.index_generation != policy.input.index_generation
        {
            return Err(SourcePlanError::BindingMismatch);
        }
        let mut selected: BTreeMap<CheckId, (bool, BTreeSet<CheckRationale>)> = BTreeMap::new();
        let mut declared = BTreeSet::new();
        for check in &requirements.declared_checks {
            if !declared.insert(check.id().clone()) {
                return Err(SourcePlanError::DuplicateCheck(check.id().clone()));
            }
            let configured = policy
                .checks
                .get(check.id())
                .ok_or_else(|| SourcePlanError::MissingCheck(check.id().clone()))?;
            if configured.input.check != *check {
                return Err(SourcePlanError::SubstitutedCheck(check.id().clone()));
            }
            select(
                &mut selected,
                check.id().clone(),
                true,
                CheckRationale::Declared,
            );
        }
        for id in &requirements.impact_checks {
            select(&mut selected, id.clone(), true, CheckRationale::Impact);
        }
        for id in &policy.input.always_required {
            select(&mut selected, id.clone(), true, CheckRationale::Policy);
        }
        let mut invariant_contracts = BTreeMap::new();
        for id in &requirements.invariants {
            let invariant = policy
                .invariants
                .get(id)
                .ok_or_else(|| SourcePlanError::MissingInvariant(id.clone()))?;
            invariant_contracts.insert(id.clone(), invariant.contract_digest);
            for check in &invariant.required_checks {
                select(
                    &mut selected,
                    check.clone(),
                    true,
                    CheckRationale::Invariant(id.clone()),
                );
            }
        }
        for id in &requirements.optional_checks {
            if selected.get(id).is_some_and(|(required, _)| *required) {
                return Err(SourcePlanError::ConflictingRequirement(id.clone()));
            }
            select(
                &mut selected,
                id.clone(),
                false,
                CheckRationale::OptionalRequest,
            );
        }
        // Monotone closure propagates mandatory status through dependencies,
        // including a node first discovered through an optional branch.
        loop {
            let before = selected.clone();
            for (id, (required, _)) in &before {
                let check = policy
                    .checks
                    .get(id)
                    .ok_or_else(|| SourcePlanError::MissingCheck(id.clone()))?;
                for dependency in &check.input.depends_on {
                    select(
                        &mut selected,
                        dependency.clone(),
                        *required,
                        CheckRationale::DependencyOf(id.clone()),
                    );
                }
            }
            if selected == before {
                break;
            }
        }
        if selected.is_empty() {
            return Err(SourcePlanError::InvalidPolicy("empty verification plan"));
        }
        let ids = selected.keys().cloned().collect();
        let order = topological_order(&policy.checks, &ids)?;
        let checks = order
            .into_iter()
            .map(|id| {
                let (required, rationales) = selected.remove(&id).expect("selected check exists");
                PlannedCheck {
                    definition: policy.checks[&id].clone(),
                    required,
                    rationales,
                }
            })
            .collect();
        let mut plan = Self {
            id: SourcePlanId(Digest::of("celetixo/source-plan/uninitialized", b"")),
            policy: policy.digest,
            binding,
            checks,
            invariant_contracts,
        };
        plan.id = SourcePlanId(Digest::of(
            "celetixo/source-verification-plan/v1",
            &plan.canonical_bytes(),
        ));
        Ok(plan)
    }

    pub fn id(&self) -> SourcePlanId {
        self.id
    }
    pub fn policy(&self) -> Digest {
        self.policy
    }
    pub fn binding(&self) -> &SourcePlanBinding {
        &self.binding
    }
    pub fn checks(&self) -> &[PlannedCheck] {
        &self.checks
    }
    pub fn invariant_contracts(&self) -> &BTreeMap<InvariantId, Digest> {
        &self.invariant_contracts
    }

    /// Admission must call this before issuing runnable authority. Input and
    /// dependency digests use Goal 02's `celetixo/source-content/v1` domain and
    /// bind candidate bytes, never the original pre-patch file contents.
    pub fn validate_resulting_snapshot(&self, snapshot: &Snapshot) -> Result<(), SourcePlanError> {
        if snapshot.scope() != &self.binding.scope
            || snapshot.id() != self.binding.resulting_snapshot
            || snapshot.manifest().digest() != self.binding.resulting_manifest
        {
            return Err(SourcePlanError::BindingMismatch);
        }
        let files = effective_workspace(snapshot).map_err(|_| SourcePlanError::BindingMismatch)?;
        for check in &self.checks {
            let input = &check.definition.input;
            for (path, expected) in input.inputs.iter().chain(&input.dependency_inputs) {
                let actual = files
                    .get(path)
                    .filter(|entry| entry.kind == InputKind::File)
                    .and_then(|entry| entry.bytes.as_deref())
                    .map(|bytes| Digest::of("celetixo/source-content/v1", bytes));
                if actual != Some(*expected) {
                    return Err(SourcePlanError::CheckInputMismatch(path.clone()));
                }
            }
            if let WorkingDirectory::Relative(path) = &input.working_directory
                && (files.contains_key(path)
                    || !files.keys().any(|file| {
                        file.strip_prefix(path)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                    }))
            {
                return Err(SourcePlanError::CheckInputMismatch(path.clone()));
            }
        }
        Ok(())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.u64(1);
        c.digest(self.policy);
        encode_plan_binding(&mut c, &self.binding);
        c.u64(self.checks.len() as u64);
        for check in &self.checks {
            c.digest(check.definition.digest);
            c.u64(u64::from(check.required));
            c.u64(check.rationales.len() as u64);
            for rationale in &check.rationales {
                match rationale {
                    CheckRationale::Declared => c.u64(0),
                    CheckRationale::Impact => c.u64(1),
                    CheckRationale::Policy => c.u64(2),
                    CheckRationale::Invariant(id) => {
                        c.u64(3);
                        c.str(id.as_str());
                    }
                    CheckRationale::DependencyOf(id) => {
                        c.u64(4);
                        c.str(id.as_str());
                    }
                    CheckRationale::OptionalRequest => c.u64(5),
                }
            }
        }
        c.u64(self.invariant_contracts.len() as u64);
        for (id, digest) in &self.invariant_contracts {
            c.str(id.as_str());
            c.digest(*digest);
        }
        c.into_bytes()
    }
}

/// A key definition, not a cache adapter or permission to reuse evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCacheContext {
    pub platform: String,
    pub architecture: String,
    pub external_inputs: BTreeMap<String, Digest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceCacheKey(Digest);

impl SourceCacheKey {
    pub fn for_check(
        plan: &VerificationPlan,
        check: &CheckId,
        context: &SourceCacheContext,
    ) -> Result<Self, SourcePlanError> {
        if !valid_description(&context.platform) || !valid_description(&context.architecture) {
            return Err(SourcePlanError::InvalidCheck("cache platform"));
        }
        let selected = plan
            .checks
            .iter()
            .find(|item| item.definition.id() == check)
            .ok_or(SourcePlanError::UnknownCacheCheck)?;
        let mut c = Canonical::default();
        c.digest(plan.id.0);
        c.digest(selected.definition.digest);
        c.str(&context.platform);
        c.str(&context.architecture);
        encode_digest_map(&mut c, &context.external_inputs);
        Ok(Self(c.finish("celetixo/source-check-cache-key/v1")))
    }
    pub fn digest(self) -> Digest {
        self.0
    }
    pub fn reuse_enabled(self) -> bool {
        false
    }
}

fn select(
    selected: &mut BTreeMap<CheckId, (bool, BTreeSet<CheckRationale>)>,
    id: CheckId,
    required: bool,
    rationale: CheckRationale,
) {
    let entry = selected.entry(id).or_default();
    entry.0 |= required;
    entry.1.insert(rationale);
}

fn topological_order(
    checks: &BTreeMap<CheckId, SourceCheckDefinition>,
    selected: &BTreeSet<CheckId>,
) -> Result<Vec<CheckId>, SourcePlanError> {
    let mut pending = selected.clone();
    let mut emitted = BTreeSet::new();
    let mut order = Vec::new();
    while !pending.is_empty() {
        let next = pending.iter().find(|id| {
            checks.get(*id).is_some_and(|check| {
                check.input.depends_on.iter().all(|dependency| {
                    !selected.contains(dependency) || emitted.contains(dependency)
                })
            })
        });
        let id = next.cloned().ok_or(SourcePlanError::DependencyCycle)?;
        pending.remove(&id);
        emitted.insert(id.clone());
        order.push(id);
    }
    Ok(order)
}

fn valid_description(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}

fn validate_relative(path: &str) -> Result<(), SourcePlanError> {
    validate_source_path(path, &PatchPolicy::default())
        .map_err(|_| SourcePlanError::InvalidPath(path.into()))
}

fn valid_guest_executable(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && validate_relative(path.trim_start_matches('/')).is_ok()
}

fn sensitive_environment_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    credential_key(&key)
        || key == "KEY"
        || key.contains("TOKEN")
        || key.contains("SECRET")
        || key.contains("PASSWORD")
        || key.ends_with("_KEY")
        || key.contains("CREDENTIAL")
}

fn valid_reference_shape(value: &str) -> bool {
    value.strip_prefix("credential-ref://").is_some_and(|rest| {
        let parts: Vec<_> = rest.split('/').collect();
        parts.len() == 4
            && parts.iter().all(|part| {
                !part.is_empty()
                    && *part != "."
                    && *part != ".."
                    && part.len() <= 256
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            })
    })
}

fn reference_in_scope(value: &str, scope: &Scope) -> bool {
    valid_reference_shape(value)
        && value.starts_with(&format!(
            "credential-ref://{}/{}/{}/",
            scope.organization, scope.workspace, scope.repository
        ))
}

fn encode_scope(c: &mut Canonical, scope: &Scope) {
    c.str(scope.organization.as_str());
    c.str(scope.workspace.as_str());
    c.str(scope.repository.as_str());
}

fn encode_digest_map(c: &mut Canonical, map: &BTreeMap<String, Digest>) {
    c.u64(map.len() as u64);
    for (name, digest) in map {
        c.str(name);
        c.digest(*digest);
    }
}

fn encode_check(c: &mut Canonical, check: &SourceCheckInput) {
    c.digest(check.check.digest());
    match &check.working_directory {
        WorkingDirectory::WorkspaceRoot => c.u64(0),
        WorkingDirectory::Relative(path) => {
            c.u64(1);
            c.str(path);
        }
    }
    encode_digest_map(c, &check.inputs);
    c.u64(check.outputs.len() as u64);
    for path in &check.outputs {
        c.str(path);
    }
    c.str(&check.toolchain.executable);
    c.digest(check.toolchain.executable_digest);
    c.digest(check.toolchain.runtime_closure);
    c.str(&check.toolchain.version);
    c.str(&check.toolchain.platform);
    c.str(&check.toolchain.architecture);
    encode_digest_map(c, &check.dependency_inputs);
    c.u64(check.environment.len() as u64);
    for (name, value) in &check.environment {
        c.str(name);
        match value {
            EnvironmentValue::Literal(value) => {
                c.u64(0);
                c.str(value);
            }
            EnvironmentValue::ProtectedReference(reference) => {
                c.u64(1);
                c.str(reference);
            }
        }
    }
    c.u64(check.tier.tag());
    check.backend.encode(c);
    check.resources.encode(c);
    match &check.network {
        NetworkPolicy::DenyAll => c.u64(0),
        NetworkPolicy::ScopedAllowlist {
            destinations,
            grant,
            expires_at_ms,
        } => {
            c.u64(1);
            c.u64(destinations.len() as u64);
            for destination in destinations {
                c.str(&destination.host);
                c.u64(u64::from(destination.port));
            }
            c.digest(*grant);
            c.u64(*expires_at_ms);
        }
    }
    c.u64(check.credential_references.len() as u64);
    for reference in &check.credential_references {
        c.str(reference);
    }
    c.u64(check.expected_artifacts.len() as u64);
    for artifact in &check.expected_artifacts {
        c.str(&artifact.name);
        c.str(&artifact.media_type);
        c.u64(artifact.max_bytes);
        c.u64(u64::from(artifact.required));
    }
    c.u64(check.depends_on.len() as u64);
    for dependency in &check.depends_on {
        c.str(dependency.as_str());
    }
    c.u64(match check.failure_policy {
        FailurePolicy::FailAttempt => 0,
        FailurePolicy::RecordOptionalFailure => 1,
    });
}

fn encode_policy(c: &mut Canonical, p: &SourcePolicyInput) {
    c.u64(u64::from(p.version));
    c.u64(p.generation);
    encode_scope(c, &p.scope);
    c.digest(p.snapshot.0);
    c.digest(p.analysis_artifact);
    c.digest(p.index_generation);
    c.digest(p.capture.snapshot.0);
    c.digest(p.capture.manifest);
    c.digest(p.capture.capture_policy);
    c.digest(p.capture.uncertainty);
    c.u64(match p.capture.consistency {
        SourceCaptureConsistency::ImmutableGitTree => 0,
        SourceCaptureConsistency::ReferenceFixture => 1,
        SourceCaptureConsistency::LiveObserved => 2,
    });
    c.u64(p.capture.gaps.len() as u64);
    for gap in &p.capture.gaps {
        c.str(gap);
    }
    c.digest(p.patch_policy.digest());
    c.bytes(&p.credentials.canonical_bytes());
    c.u64(p.paths.len() as u64);
    for (path, rule) in &p.paths {
        c.str(path);
        c.u64(rule.allowed_operations.len() as u64);
        for operation in &rule.allowed_operations {
            c.u64(match operation {
                OperationKind::Create => 0,
                OperationKind::Replace => 1,
                OperationKind::Delete => 2,
                OperationKind::Rename => 3,
                OperationKind::SetMode => 4,
            });
        }
        c.u64(rule.required_writes.len() as u64);
        for element in &rule.required_writes {
            c.str(element.as_str());
        }
        match rule.expected_base {
            None => c.u64(0),
            Some(expected) => {
                c.u64(1);
                c.u64(match expected.kind {
                    InputKind::File => 0,
                    InputKind::Symlink => 1,
                    InputKind::Submodule => 2,
                });
                c.u64(match expected.mode {
                    FileMode::Regular => 0o100644,
                    FileMode::Executable => 0o100755,
                    FileMode::Symlink => 0o120000,
                    FileMode::Gitlink => 0o160000,
                });
                c.digest(expected.content);
            }
        }
    }
    c.u64(p.checks.len() as u64);
    for check in &p.checks {
        c.digest(check.digest);
    }
    c.u64(p.invariants.len() as u64);
    for invariant in &p.invariants {
        c.str(invariant.id.as_str());
        c.u64(invariant.scope.len() as u64);
        for element in &invariant.scope {
            c.str(element.as_str());
        }
        c.digest(invariant.contract_digest);
        c.u64(invariant.required_checks.len() as u64);
        for check in &invariant.required_checks {
            c.str(check.as_str());
        }
    }
    c.u64(p.always_required.len() as u64);
    for check in &p.always_required {
        c.str(check.as_str());
    }
    c.u64(p.approved_tiers.len() as u64);
    for tier in &p.approved_tiers {
        c.u64(tier.tag());
    }
    c.u64(p.approved_backends.len() as u64);
    for backend in &p.approved_backends {
        c.digest(*backend);
    }
    p.resource_ceiling.encode(c);
    c.u64(u64::from(p.network_allowed));
    c.u64(p.approved_credential_references.len() as u64);
    for reference in &p.approved_credential_references {
        c.str(reference);
    }
    c.u64(p.expires_at_ms);
    c.digest(p.retention_policy);
}

fn encode_plan_binding(c: &mut Canonical, binding: &SourcePlanBinding) {
    encode_scope(c, &binding.scope);
    c.digest(binding.base_snapshot.0);
    c.digest(binding.analysis_artifact);
    c.digest(binding.index_generation);
    c.digest(binding.impact_request);
    c.digest(binding.impact_result);
    c.digest(binding.patch.0);
    c.digest(binding.operation_digest);
    c.digest(binding.resulting_snapshot.0);
    c.digest(binding.resulting_manifest);
}
