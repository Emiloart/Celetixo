//! Checked recovery of the existing source check, policy and plan formats.
//!
//! Their canonical formats refer to immutable dependencies by digest. Callers
//! supply those checked dependencies and the expected snapshot/analysis/request
//! context; payloads cannot select their own freshness or grant authority.
use super::*;
use crate::credentials::CredentialPolicy;
use crate::evidence::CheckSpec;
use crate::ids::{Canonical, CheckId, InvariantId, SourcePatchId, SourcePlanId};
use crate::repository::RepositoryAnalysis;
use crate::source_execution::{
    ArtifactRequirement, BackendIdentity, CheckRationale, EnvironmentValue, ExecutionTier,
    FailurePolicy, NetworkDestination, NetworkPolicy, PlanRequirements, ResourceLimits,
    SourceCaptureBinding, SourceCaptureConsistency, SourceCheckDefinition, SourceCheckInput,
    SourceExecutionPolicy, SourceInvariantSpec, SourcePathRule, SourcePlanBinding,
    SourcePolicyInput, ToolchainIdentity, VerificationPlan, WorkingDirectory,
};
use crate::source_patch::{OperationKind, PatchPolicy, PathProfile, Preimage, SymlinkPolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckSpecRecord {
    check: CheckSpec,
}
impl CheckSpecRecord {
    pub const FORMAT: &str = "celetixo/check-spec/v1";
    pub fn new(check: CheckSpec) -> Self {
        Self { check }
    }
    pub fn check(&self) -> &CheckSpec {
        &self.check
    }
    pub fn digest(&self) -> Digest {
        self.check.digest()
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(self.check.id().as_str());
        c.u64(self.check.command().len() as u64);
        for arg in self.check.command() {
            c.str(arg);
        }
        c.str(self.check.toolchain());
        c.digest(self.check.environment());
        c.digest(self.check.configuration());
        c.into_bytes()
    }
}

pub fn restore_check_spec(
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<CheckSpecRecord, RestoreError> {
    let mut r = Reader::new(expected, CheckSpecRecord::FORMAT, bytes, limits)?;
    let id = CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let mut command = Vec::new();
    for _ in 0..r.count()? {
        command.push(r.text()?);
    }
    let check = CheckSpec::new(id, command, r.text()?, r.digest()?, r.digest()?)?;
    r.end()?;
    let record = CheckSpecRecord::new(check);
    exact(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

/// The source patch envelope also carries this preimage, but a policy may be
/// installed before its first patch. Keep standalone immutable recovery possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchPolicyRecord {
    policy: PatchPolicy,
}
impl PatchPolicyRecord {
    pub const FORMAT: &str = "celetixo/source-patch-policy/v1";
    pub fn new(policy: PatchPolicy) -> Result<Self, RestoreError> {
        crate::source_patch::validate_source_path("p", &policy)?;
        Ok(Self { policy })
    }
    pub fn policy(&self) -> &PatchPolicy {
        &self.policy
    }
    pub fn digest(&self) -> Digest {
        self.policy.digest()
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let p = &self.policy;
        let mut c = Canonical::default();
        for value in [
            u64::from(p.version),
            match p.path_profile {
                PathProfile::PortableAsciiV1 => 0,
            },
            match p.symlinks {
                SymlinkPolicy::Deny => 0,
                SymlinkPolicy::InternalRelativeOnly => 1,
            },
            p.max_operations as u64,
            p.max_payload_bytes,
            p.max_total_payload_bytes,
            p.max_result_files as u64,
            p.max_result_bytes,
            p.max_path_bytes as u64,
            p.max_depth as u64,
        ] {
            c.u64(value);
        }
        c.into_bytes()
    }
}
pub fn restore_patch_policy(
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<PatchPolicyRecord, RestoreError> {
    let mut r = Reader::new(expected, PatchPolicyRecord::FORMAT, bytes, limits)?;
    let policy = PatchPolicy {
        version: r.version()?,
        path_profile: match r.number()? {
            0 => PathProfile::PortableAsciiV1,
            _ => return Err(RestoreError::Encoding),
        },
        symlinks: match r.number()? {
            0 => SymlinkPolicy::Deny,
            1 => SymlinkPolicy::InternalRelativeOnly,
            _ => return Err(RestoreError::Encoding),
        },
        max_operations: usize_number(&mut r)?,
        max_payload_bytes: r.number()?,
        max_total_payload_bytes: r.number()?,
        max_result_files: usize_number(&mut r)?,
        max_result_bytes: r.number()?,
        max_path_bytes: usize_number(&mut r)?,
        max_depth: usize_number(&mut r)?,
    };
    r.end()?;
    let record = PatchPolicyRecord::new(policy)?;
    exact(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckDefinitionRecord {
    definition: SourceCheckDefinition,
}
impl SourceCheckDefinitionRecord {
    pub const FORMAT: &str = "celetixo/source-check/v1";
    pub fn new(definition: SourceCheckDefinition) -> Self {
        Self { definition }
    }
    pub fn definition(&self) -> &SourceCheckDefinition {
        &self.definition
    }
    pub fn digest(&self) -> Digest {
        self.definition.digest()
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.definition.canonical_bytes()
    }
    pub fn recovery_size(definition: &SourceCheckDefinition) -> Result<RecoverySize, RestoreError> {
        let cost = definition_cost(definition)?;
        Ok(RecoverySize {
            entries: cost.entries,
            bytes: cost.bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceExecutionPolicyRecord {
    policy: SourceExecutionPolicy,
}
impl SourceExecutionPolicyRecord {
    pub const FORMAT: &str = "celetixo/source-execution-policy/v1";
    pub fn new(
        snapshot: &Snapshot,
        analysis: &RepositoryAnalysis,
        policy: SourceExecutionPolicy,
    ) -> Result<Self, RestoreError> {
        analysis.validate(snapshot, analysis.index_generation())?;
        if policy.input().analysis_artifact != analysis.digest()
            || policy.input().index_generation != analysis.index_generation()
        {
            return Err(RestoreError::BindingMismatch);
        }
        policy.validate_snapshot(snapshot)?;
        Ok(Self { policy })
    }
    pub fn policy(&self) -> &SourceExecutionPolicy {
        &self.policy
    }
    pub fn scope(&self) -> &Scope {
        &self.policy.input().scope
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.policy.input().snapshot
    }
    pub fn digest(&self) -> Digest {
        self.policy.digest()
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.policy.canonical_bytes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationPlanRecord {
    plan: VerificationPlan,
}
impl VerificationPlanRecord {
    pub const FORMAT: &str = "celetixo/source-verification-plan/v1";
    /// Requirements must come from the Engine's admission analysis. Resolving
    /// these pure values does not authenticate the request or admit execution.
    pub fn new(
        policy: &SourceExecutionPolicy,
        binding: SourcePlanBinding,
        requirements: PlanRequirements,
    ) -> Result<Self, RestoreError> {
        let plan = VerificationPlan::resolve(policy, binding, requirements)?;
        Ok(Self { plan })
    }
    pub fn plan(&self) -> &VerificationPlan {
        &self.plan
    }
    pub fn digest(&self) -> Digest {
        self.plan.id().0
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.plan.canonical_bytes()
    }
}

/// CheckSpec is a separately retained immutable value: the published source
/// check format contains its digest, not its command/configuration preimage.
pub fn restore_source_check_definition(
    check: &CheckSpec,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceCheckDefinitionRecord, RestoreError> {
    let mut r = Reader::new(expected, SourceCheckDefinitionRecord::FORMAT, bytes, limits)?;
    let mut cost = Cost::default();
    cost.spec(check)?;
    charge(&mut r, bytes.len(), cost)?;
    if r.digest()? != check.digest() {
        return Err(RestoreError::BindingMismatch);
    }
    let working_directory = match r.number()? {
        0 => WorkingDirectory::WorkspaceRoot,
        1 => WorkingDirectory::Relative(r.text()?),
        _ => return Err(RestoreError::Encoding),
    };
    let inputs = digest_map(&mut r)?;
    let outputs = strings(&mut r)?;
    let toolchain = ToolchainIdentity {
        executable: r.text()?,
        executable_digest: r.digest()?,
        runtime_closure: r.digest()?,
        version: r.text()?,
        platform: r.text()?,
        architecture: r.text()?,
    };
    let dependency_inputs = digest_map(&mut r)?;
    let mut environment = BTreeMap::new();
    for _ in 0..r.count()? {
        let name = r.text()?;
        let value = match r.number()? {
            0 => EnvironmentValue::Literal(r.text()?),
            1 => EnvironmentValue::ProtectedReference(r.text()?),
            _ => return Err(RestoreError::Encoding),
        };
        unique(environment.insert(name, value).is_none())?;
    }
    let tier = tier(&mut r)?;
    let backend = BackendIdentity {
        name: r.text()?,
        version: r.text()?,
        image: r.digest()?,
        profile: r.digest()?,
    };
    let resources = resources(&mut r)?;
    let network = match r.number()? {
        0 => NetworkPolicy::DenyAll,
        1 => {
            let mut destinations = BTreeSet::new();
            for _ in 0..r.count()? {
                let host = r.text()?;
                let port = u16::try_from(r.number()?).map_err(|_| RestoreError::Encoding)?;
                unique(destinations.insert(NetworkDestination { host, port }))?;
            }
            NetworkPolicy::ScopedAllowlist {
                destinations,
                grant: r.digest()?,
                expires_at_ms: r.number()?,
            }
        }
        _ => return Err(RestoreError::Encoding),
    };
    let credential_references = strings(&mut r)?;
    let mut expected_artifacts = Vec::new();
    for _ in 0..r.count()? {
        expected_artifacts.push(ArtifactRequirement {
            name: r.text()?,
            media_type: r.text()?,
            max_bytes: r.number()?,
            required: boolean(&mut r)?,
        });
    }
    let depends_on = checks(&mut r)?;
    let failure_policy = match r.number()? {
        0 => FailurePolicy::FailAttempt,
        1 => FailurePolicy::RecordOptionalFailure,
        _ => return Err(RestoreError::Encoding),
    };
    r.end()?;
    let definition = SourceCheckDefinition::new(SourceCheckInput {
        check: check.clone(),
        working_directory,
        inputs,
        outputs,
        toolchain,
        dependency_inputs,
        environment,
        tier,
        backend,
        resources,
        network,
        credential_references,
        expected_artifacts,
        depends_on,
        failure_policy,
    })?;
    exact(
        definition.digest(),
        expected,
        &definition.canonical_bytes(),
        bytes,
    )?;
    Ok(SourceCheckDefinitionRecord::new(definition))
}

/// Restore references through the supplied checked catalog. A policy payload
/// cannot substitute a patch policy, check definition, snapshot or analysis.
#[allow(clippy::too_many_arguments)]
pub fn restore_source_execution_policy(
    snapshot: &Snapshot,
    analysis: &RepositoryAnalysis,
    expected_generation: u64,
    patch_policy: &PatchPolicy,
    definitions: &BTreeMap<Digest, SourceCheckDefinition>,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceExecutionPolicyRecord, RestoreError> {
    let mut r = Reader::new(expected, SourceExecutionPolicyRecord::FORMAT, bytes, limits)?;
    let version = r.version()?;
    let generation = r.number()?;
    if generation != expected_generation {
        return Err(RestoreError::BindingMismatch);
    }
    let scope = r.scope()?;
    let snapshot_id = SnapshotId(r.digest()?);
    let analysis_artifact = r.digest()?;
    let index_generation = r.digest()?;
    if &scope != snapshot.scope()
        || snapshot_id != snapshot.id()
        || analysis_artifact != analysis.digest()
        || index_generation != analysis.index_generation()
    {
        return Err(RestoreError::BindingMismatch);
    }
    let capture = SourceCaptureBinding {
        snapshot: SnapshotId(r.digest()?),
        manifest: r.digest()?,
        capture_policy: r.digest()?,
        uncertainty: r.digest()?,
        consistency: match r.number()? {
            0 => SourceCaptureConsistency::ImmutableGitTree,
            1 => SourceCaptureConsistency::ReferenceFixture,
            2 => SourceCaptureConsistency::LiveObserved,
            _ => return Err(RestoreError::Encoding),
        },
        gaps: strings(&mut r)?,
    };
    if r.digest()? != patch_policy.digest() {
        return Err(RestoreError::BindingMismatch);
    }
    // The nested credential format shares the parent's entry budget, including
    // every digest in a reviewed path's set. It is already part of parent bytes.
    let credentials = credentials(&mut r)?;
    let mut paths = BTreeMap::new();
    for _ in 0..r.count()? {
        let path = r.text()?;
        let mut allowed_operations = BTreeSet::new();
        for _ in 0..r.count()? {
            let operation = match r.number()? {
                0 => OperationKind::Create,
                1 => OperationKind::Replace,
                2 => OperationKind::Delete,
                3 => OperationKind::Rename,
                4 => OperationKind::SetMode,
                _ => return Err(RestoreError::Encoding),
            };
            unique(allowed_operations.insert(operation))?;
        }
        let required_writes = elements(&mut r)?;
        let expected_base = match r.number()? {
            0 => None,
            1 => Some(Preimage {
                kind: match r.number()? {
                    0 => InputKind::File,
                    1 => InputKind::Symlink,
                    2 => InputKind::Submodule,
                    _ => return Err(RestoreError::Encoding),
                },
                mode: match r.number()? {
                    0o100644 => FileMode::Regular,
                    0o100755 => FileMode::Executable,
                    0o120000 => FileMode::Symlink,
                    0o160000 => FileMode::Gitlink,
                    _ => return Err(RestoreError::Encoding),
                },
                content: r.digest()?,
            }),
            _ => return Err(RestoreError::Encoding),
        };
        unique(
            paths
                .insert(
                    path,
                    SourcePathRule {
                        allowed_operations,
                        required_writes,
                        expected_base,
                    },
                )
                .is_none(),
        )?;
    }
    let mut configured_checks = Vec::new();
    let mut total_bytes = bytes.len();
    for _ in 0..r.count()? {
        let id = r.digest()?;
        let definition = definitions.get(&id).ok_or(RestoreError::BindingMismatch)?;
        if definition.digest() != id {
            return Err(RestoreError::BindingMismatch);
        }
        let cost = definition_cost(definition)?;
        total_bytes = charge(&mut r, total_bytes, cost)?;
        configured_checks.push(definition.clone());
    }
    let mut invariants = Vec::new();
    for _ in 0..r.count()? {
        invariants.push(SourceInvariantSpec {
            id: InvariantId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
            scope: elements(&mut r)?,
            contract_digest: r.digest()?,
            required_checks: checks(&mut r)?,
        });
    }
    let always_required = checks(&mut r)?;
    let mut approved_tiers = BTreeSet::new();
    for _ in 0..r.count()? {
        unique(approved_tiers.insert(tier(&mut r)?))?;
    }
    let mut approved_backends = BTreeSet::new();
    for _ in 0..r.count()? {
        unique(approved_backends.insert(r.digest()?))?;
    }
    let resource_ceiling = resources(&mut r)?;
    let network_allowed = boolean(&mut r)?;
    let approved_credential_references = strings(&mut r)?;
    let expires_at_ms = r.number()?;
    let retention_policy = r.digest()?;
    r.end()?;
    let policy = SourceExecutionPolicy::new(SourcePolicyInput {
        version,
        generation,
        scope,
        snapshot: snapshot_id,
        analysis_artifact,
        index_generation,
        capture,
        patch_policy: patch_policy.clone(),
        credentials,
        paths,
        checks: configured_checks,
        invariants,
        always_required,
        approved_tiers,
        approved_backends,
        resource_ceiling,
        network_allowed,
        approved_credential_references,
        expires_at_ms,
        retention_policy,
    })?;
    exact(policy.digest(), expected, &policy.canonical_bytes(), bytes)?;
    SourceExecutionPolicyRecord::new(snapshot, analysis, policy)
}

/// Re-resolve against caller-supplied Engine-derived requirements. Decoding a
/// self-consistent list of required flags/rationales cannot choose a weaker plan.
pub fn restore_verification_plan(
    policy: &SourceExecutionPolicy,
    expected_binding: &SourcePlanBinding,
    requirements: PlanRequirements,
    expected: SourcePlanId,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<VerificationPlanRecord, RestoreError> {
    let mut r = Reader::new(expected.0, VerificationPlanRecord::FORMAT, bytes, limits)?;
    if r.number()? != 1 {
        return Err(RestoreError::Encoding);
    }
    if r.digest()? != policy.digest() {
        return Err(RestoreError::BindingMismatch);
    }
    let binding = SourcePlanBinding {
        scope: r.scope()?,
        base_snapshot: SnapshotId(r.digest()?),
        analysis_artifact: r.digest()?,
        index_generation: r.digest()?,
        impact_request: r.digest()?,
        impact_result: r.digest()?,
        patch: SourcePatchId(r.digest()?),
        operation_digest: r.digest()?,
        resulting_snapshot: SnapshotId(r.digest()?),
        resulting_manifest: r.digest()?,
    };
    if &binding != expected_binding {
        return Err(RestoreError::BindingMismatch);
    }
    // Conservatively account the full checked registry before re-resolution.
    // Untrusted bytes may omit a mandatory/dependency branch which the resolver
    // must nevertheless select. Charging only named definitions is unsafe.
    charge(
        &mut r,
        bytes.len(),
        plan_context_cost(policy, &requirements)?,
    )?;
    let known_definitions: BTreeSet<_> = policy
        .checks()
        .values()
        .map(SourceCheckDefinition::digest)
        .collect();
    let mut selected_definitions = BTreeSet::new();
    for _ in 0..r.count()? {
        let id = r.digest()?;
        if !known_definitions.contains(&id) {
            return Err(RestoreError::BindingMismatch);
        }
        unique(selected_definitions.insert(id))?;
        boolean(&mut r)?;
        let mut rationales = BTreeSet::new();
        for _ in 0..r.count()? {
            let rationale = match r.number()? {
                0 => CheckRationale::Declared,
                1 => CheckRationale::Impact,
                2 => CheckRationale::Policy,
                3 => CheckRationale::Invariant(
                    InvariantId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
                ),
                4 => CheckRationale::DependencyOf(
                    CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
                ),
                5 => CheckRationale::OptionalRequest,
                _ => return Err(RestoreError::Encoding),
            };
            unique(rationales.insert(rationale))?;
        }
    }
    let mut contracts = BTreeSet::new();
    for _ in 0..r.count()? {
        let id = InvariantId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
        r.digest()?;
        unique(contracts.insert(id))?;
    }
    r.end()?;
    let record = VerificationPlanRecord::new(policy, binding, requirements)?;
    exact(
        record.digest(),
        expected.0,
        &record.canonical_bytes(),
        bytes,
    )?;
    Ok(record)
}

fn boolean(r: &mut Reader<'_>) -> Result<bool, RestoreError> {
    match r.number()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(RestoreError::Encoding),
    }
}
fn usize_number(r: &mut Reader<'_>) -> Result<usize, RestoreError> {
    usize::try_from(r.number()?).map_err(|_| RestoreError::Limit)
}
fn unique(value: bool) -> Result<(), RestoreError> {
    if value {
        Ok(())
    } else {
        Err(RestoreError::NonCanonical)
    }
}
fn exact(
    actual: Digest,
    expected: Digest,
    canonical: &[u8],
    bytes: &[u8],
) -> Result<(), RestoreError> {
    if actual != expected || canonical != bytes {
        Err(RestoreError::NonCanonical)
    } else {
        Ok(())
    }
}
fn strings(r: &mut Reader<'_>) -> Result<BTreeSet<String>, RestoreError> {
    let mut values = BTreeSet::new();
    for _ in 0..r.count()? {
        unique(values.insert(r.text()?))?;
    }
    Ok(values)
}
fn checks(r: &mut Reader<'_>) -> Result<BTreeSet<CheckId>, RestoreError> {
    let mut values = BTreeSet::new();
    for _ in 0..r.count()? {
        unique(values.insert(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?))?;
    }
    Ok(values)
}
fn elements(r: &mut Reader<'_>) -> Result<BTreeSet<ElementId>, RestoreError> {
    let mut values = BTreeSet::new();
    for _ in 0..r.count()? {
        unique(values.insert(ElementId::new(r.text()?).map_err(|_| RestoreError::Identity)?))?;
    }
    Ok(values)
}
fn digest_map(r: &mut Reader<'_>) -> Result<BTreeMap<String, Digest>, RestoreError> {
    let mut values = BTreeMap::new();
    for _ in 0..r.count()? {
        unique(values.insert(r.text()?, r.digest()?).is_none())?;
    }
    Ok(values)
}
fn tier(r: &mut Reader<'_>) -> Result<ExecutionTier, RestoreError> {
    match r.number()? {
        0 => Ok(ExecutionTier::T0),
        1 => Ok(ExecutionTier::T1),
        2 => Ok(ExecutionTier::T2),
        3 => Ok(ExecutionTier::T3),
        _ => Err(RestoreError::Encoding),
    }
}
fn resources(r: &mut Reader<'_>) -> Result<ResourceLimits, RestoreError> {
    Ok(ResourceLimits {
        wall_time_ms: r.number()?,
        cpu_time_ms: r.number()?,
        memory_bytes: r.number()?,
        disk_bytes: r.number()?,
        process_count: r.number()?,
        file_descriptors: r.number()?,
        stdout_bytes: r.number()?,
        stderr_bytes: r.number()?,
        artifact_bytes: r.number()?,
    })
}
fn credentials(r: &mut Reader<'_>) -> Result<CredentialPolicy, RestoreError> {
    let bytes = r.field()?;
    let domain = "celetixo/credential-admission-policy/v1";
    let mut nested = Reader::new(Digest::of(domain, bytes), domain, bytes, r.limits)?;
    nested.entries = r.entries;
    let version = nested.version()?;
    let mut reviewed_public_inputs = BTreeMap::new();
    for _ in 0..nested.count()? {
        let path = nested.text()?;
        let mut addresses = BTreeSet::new();
        for _ in 0..nested.count()? {
            unique(addresses.insert(nested.digest()?))?;
        }
        unique(reviewed_public_inputs.insert(path, addresses).is_none())?;
    }
    nested.end()?;
    r.entries = nested.entries;
    let policy = CredentialPolicy {
        version,
        reviewed_public_inputs,
    };
    policy
        .validate()
        .map_err(|_| RestoreError::InvalidDomainValue)?;
    if policy.canonical_bytes() != bytes {
        return Err(RestoreError::NonCanonical);
    }
    Ok(policy)
}

/// Account referenced immutable objects before cloning. Canonical byte and
/// collection budgets apply to the complete recovered value, not each digest
/// reference independently. Measurement allocates no canonical payload buffers.
#[derive(Default)]
pub(crate) struct Cost {
    pub(crate) bytes: usize,
    pub(crate) entries: usize,
    pub(crate) max_field: usize,
}
impl Cost {
    pub(crate) fn merge(&mut self, other: Self) -> Result<(), RestoreError> {
        self.bytes(other.bytes)?;
        self.entries = self
            .entries
            .checked_add(other.entries)
            .ok_or(RestoreError::Limit)?;
        self.max_field = self.max_field.max(other.max_field);
        Ok(())
    }
    pub(crate) fn bytes(&mut self, n: usize) -> Result<(), RestoreError> {
        self.bytes = self.bytes.checked_add(n).ok_or(RestoreError::Limit)?;
        Ok(())
    }
    pub(crate) fn number(&mut self) -> Result<(), RestoreError> {
        self.max_field = self.max_field.max(8);
        self.bytes(16)
    }
    pub(crate) fn digest(&mut self) -> Result<(), RestoreError> {
        self.max_field = self.max_field.max(32);
        self.bytes(40)
    }
    pub(crate) fn text(&mut self, s: &str) -> Result<(), RestoreError> {
        self.max_field = self.max_field.max(s.len());
        self.bytes(8)?;
        self.bytes(s.len())
    }
    pub(crate) fn count(&mut self, n: usize) -> Result<(), RestoreError> {
        self.entries = self.entries.checked_add(n).ok_or(RestoreError::Limit)?;
        self.number()
    }
    pub(crate) fn spec(&mut self, spec: &CheckSpec) -> Result<(), RestoreError> {
        self.text(spec.id().as_str())?;
        self.count(spec.command().len())?;
        for arg in spec.command() {
            self.text(arg)?;
        }
        self.text(spec.toolchain())?;
        self.digest()?;
        self.digest()
    }
    pub(crate) fn digest_map(
        &mut self,
        map: &BTreeMap<String, Digest>,
    ) -> Result<(), RestoreError> {
        self.count(map.len())?;
        for name in map.keys() {
            self.text(name)?;
            self.digest()?;
        }
        Ok(())
    }
}
pub(crate) fn charge(
    r: &mut Reader<'_>,
    already: usize,
    cost: Cost,
) -> Result<usize, RestoreError> {
    let total = already.checked_add(cost.bytes).ok_or(RestoreError::Limit)?;
    r.entries = r
        .entries
        .checked_add(cost.entries)
        .ok_or(RestoreError::Limit)?;
    if total > r.limits.encoded_bytes
        || r.entries > r.limits.collection_entries
        || cost.max_field > r.limits.field_bytes
    {
        return Err(RestoreError::Limit);
    }
    Ok(total)
}
pub(crate) fn definition_cost(definition: &SourceCheckDefinition) -> Result<Cost, RestoreError> {
    let i = definition.input();
    let mut c = Cost::default();
    c.spec(&i.check)?;
    c.digest()?;
    c.number()?;
    if let WorkingDirectory::Relative(path) = &i.working_directory {
        c.text(path)?;
    }
    c.digest_map(&i.inputs)?;
    c.count(i.outputs.len())?;
    for path in &i.outputs {
        c.text(path)?;
    }
    c.text(&i.toolchain.executable)?;
    c.digest()?;
    c.digest()?;
    for value in [
        &i.toolchain.version,
        &i.toolchain.platform,
        &i.toolchain.architecture,
    ] {
        c.text(value)?;
    }
    c.digest_map(&i.dependency_inputs)?;
    c.count(i.environment.len())?;
    for (key, value) in &i.environment {
        c.text(key)?;
        c.number()?;
        match value {
            EnvironmentValue::Literal(s) | EnvironmentValue::ProtectedReference(s) => c.text(s)?,
        }
    }
    c.number()?;
    c.text(&i.backend.name)?;
    c.text(&i.backend.version)?;
    c.digest()?;
    c.digest()?;
    for _ in 0..9 {
        c.number()?;
    }
    c.number()?;
    if let NetworkPolicy::ScopedAllowlist { destinations, .. } = &i.network {
        c.count(destinations.len())?;
        for d in destinations {
            c.text(&d.host)?;
            c.number()?;
        }
        c.digest()?;
        c.number()?;
    }
    c.count(i.credential_references.len())?;
    for value in &i.credential_references {
        c.text(value)?;
    }
    c.count(i.expected_artifacts.len())?;
    for artifact in &i.expected_artifacts {
        c.text(&artifact.name)?;
        c.text(&artifact.media_type)?;
        c.number()?;
        c.number()?;
    }
    c.count(i.depends_on.len())?;
    for id in &i.depends_on {
        c.text(id.as_str())?;
    }
    c.number()?;
    Ok(c)
}

/// Full-registry preflight also bounds resolver-generated rationales. Invariant
/// fan-out and dependency edges may be absent from an untrusted plan payload.
pub(crate) fn plan_context_cost(
    policy: &SourceExecutionPolicy,
    requirements: &PlanRequirements,
) -> Result<Cost, RestoreError> {
    let mut c = Cost::default();
    c.count(policy.checks().len())?;
    for definition in policy.checks().values() {
        c.merge(definition_cost(definition)?)?;
        c.count(definition.input().depends_on.len())?;
        for _ in &definition.input().depends_on {
            c.text(definition.id().as_str())?;
        }
    }
    c.count(policy.input().always_required.len())?;
    for id in &policy.input().always_required {
        c.text(id.as_str())?;
    }
    c.count(policy.invariants().len())?;
    for invariant in policy.invariants().values() {
        c.text(invariant.id.as_str())?;
        c.digest()?;
        c.count(invariant.scope.len())?;
        for element in &invariant.scope {
            c.text(element.as_str())?;
        }
        c.count(invariant.required_checks.len())?;
        for id in &invariant.required_checks {
            c.text(id.as_str())?;
            c.text(invariant.id.as_str())?;
        }
    }
    c.count(requirements.declared_checks.len())?;
    for check in &requirements.declared_checks {
        c.spec(check)?;
    }
    c.count(requirements.impact_checks.len())?;
    for id in &requirements.impact_checks {
        c.text(id.as_str())?;
    }
    c.count(requirements.invariants.len())?;
    for id in &requirements.invariants {
        c.text(id.as_str())?;
    }
    c.count(requirements.optional_checks.len())?;
    for id in &requirements.optional_checks {
        c.text(id.as_str())?;
    }
    Ok(c)
}
