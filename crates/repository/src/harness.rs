//! Immutable repository selection of embedding-approved verification checks.
//!
//! Version 1 JSON selects exact check-definition addresses. It cannot supply a
//! command, flags, a backend, resources or credentials. This adapter performs no
//! filesystem access, process execution, tool installation or Engine policy
//! installation. Parsing configuration does not establish semantic coverage.

use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::{CheckId, Digest, Scope, SnapshotId};
use celetixo_engine::snapshot::{FileMode, InputKind, Snapshot, effective_workspace};
use celetixo_engine::source_execution::{
    SourceCheckDefinition, SourceExecutionPolicy, SourcePlanError, SourcePolicyInput,
};
use celetixo_engine::source_patch::{PatchPolicy, validate_source_path};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessLimits {
    pub max_config_bytes: usize,
    pub max_selected_checks: usize,
    pub max_registry_checks: usize,
    pub max_registry_bytes: usize,
    pub max_dependency_edges: usize,
}

impl Default for HarnessLimits {
    fn default() -> Self {
        Self {
            max_config_bytes: 1024 * 1024,
            max_selected_checks: 4096,
            max_registry_checks: 4096,
            max_registry_bytes: 32 * 1024 * 1024,
            max_dependency_edges: 16_384,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HarnessError {
    #[error("invalid harness limit or exceeded {0} budget")]
    Limit(&'static str),
    #[error("harness configuration needs a canonical source path")]
    InvalidPath,
    #[error("snapshot has an invalid effective workspace")]
    InvalidWorkspace,
    #[error("harness configuration is missing or is not an ordinary source file")]
    MissingConfiguration,
    #[error("harness configuration is not strict JSON at line {line}, column {column}")]
    InvalidJson { line: usize, column: usize },
    #[error("unsupported harness configuration version")]
    UnsupportedVersion,
    #[error("harness configuration and approved registry must be nonempty")]
    EmptySelection,
    #[error("invalid harness check identity or definition digest")]
    InvalidSelection,
    #[error("harness check identity is duplicated: {0}")]
    DuplicateCheck(CheckId),
    #[error("approved check is missing: {0}")]
    MissingCheck(CheckId),
    #[error("selected check differs from its embedding-approved definition: {0}")]
    DefinitionMismatch(CheckId),
    #[error("harness prerequisite graph contains a cycle")]
    DependencyCycle,
    #[error("approved check declares a different input at the harness path: {0}")]
    ConflictingConfigurationInput(CheckId),
    #[error("harness configuration omits a trusted mandatory check: {0}")]
    MissingMandatory(CheckId),
    #[error("policy substitutes the embedding-approved check definition: {0}")]
    PolicyDefinitionMismatch(CheckId),
    #[error("harness, snapshot and policy bindings differ")]
    BindingMismatch,
    #[error("configured verification policy is invalid: {0}")]
    Policy(#[from] SourcePlanError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    version: u32,
    checks: Vec<Selection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    id: String,
    definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredHarness {
    scope: Scope,
    snapshot: SnapshotId,
    manifest: Digest,
    path: String,
    content: Digest,
    roots: BTreeSet<CheckId>,
    derived: BTreeSet<CheckId>,
    approved: BTreeMap<CheckId, Digest>,
    definitions: Vec<SourceCheckDefinition>,
    digest: Digest,
}

impl ConfiguredHarness {
    /// The registry is supplied by the trusted embedding. Repository JSON only
    /// selects exact addresses; it never expands the registry's authority.
    pub fn from_snapshot(
        snapshot: &Snapshot,
        path: &str,
        approved_registry: impl IntoIterator<Item = SourceCheckDefinition>,
        limits: HarnessLimits,
    ) -> Result<Self, HarnessError> {
        if [
            limits.max_config_bytes,
            limits.max_selected_checks,
            limits.max_registry_checks,
            limits.max_registry_bytes,
            limits.max_dependency_edges,
        ]
        .contains(&0)
        {
            return Err(HarnessError::Limit("nonzero limits"));
        }
        validate_source_path(path, &PatchPolicy::default())
            .map_err(|_| HarnessError::InvalidPath)?;
        let files = effective_workspace(snapshot).map_err(|_| HarnessError::InvalidWorkspace)?;
        let bytes = files
            .get(path)
            .filter(|input| {
                input.kind == InputKind::File
                    && matches!(input.mode, FileMode::Regular | FileMode::Executable)
            })
            .and_then(|input| input.bytes.as_deref())
            .ok_or(HarnessError::MissingConfiguration)?;
        if bytes.len() > limits.max_config_bytes {
            return Err(HarnessError::Limit("configuration bytes"));
        }
        // Derived Deserialize rejects duplicate struct fields. Parsing directly
        // into these structs avoids Value/map parsing's last-key-wins behavior.
        let config: Configuration =
            serde_json::from_slice(bytes).map_err(|error| HarnessError::InvalidJson {
                line: error.line(),
                column: error.column(),
            })?;
        if config.version != 1 {
            return Err(HarnessError::UnsupportedVersion);
        }
        if config.checks.len() > limits.max_selected_checks {
            return Err(HarnessError::Limit("selected checks"));
        }
        let mut registry = BTreeMap::new();
        let mut registry_bytes = 0_usize;
        let mut dependency_edges = 0_usize;
        for check in approved_registry {
            registry_bytes = registry_bytes
                .checked_add(check.canonical_bytes().len())
                .ok_or(HarnessError::Limit("registry bytes"))?;
            dependency_edges = dependency_edges
                .checked_add(check.input().depends_on.len())
                .ok_or(HarnessError::Limit("dependency edges"))?;
            if registry_bytes > limits.max_registry_bytes {
                return Err(HarnessError::Limit("registry bytes"));
            }
            if dependency_edges > limits.max_dependency_edges {
                return Err(HarnessError::Limit("dependency edges"));
            }
            let id = check.id().clone();
            if registry.insert(id.clone(), check).is_some() {
                return Err(HarnessError::DuplicateCheck(id));
            }
            if registry.len() > limits.max_registry_checks {
                return Err(HarnessError::Limit("registry checks"));
            }
        }
        if config.checks.is_empty() || registry.is_empty() {
            return Err(HarnessError::EmptySelection);
        }
        let mut roots = BTreeSet::new();
        for selection in config.checks {
            let id = CheckId::new(selection.id).map_err(|_| HarnessError::InvalidSelection)?;
            let expected = decode_digest(&selection.definition)?;
            if !roots.insert(id.clone()) {
                return Err(HarnessError::DuplicateCheck(id));
            }
            let check = registry
                .get(&id)
                .ok_or_else(|| HarnessError::MissingCheck(id.clone()))?;
            if check.digest() != expected {
                return Err(HarnessError::DefinitionMismatch(id));
            }
        }
        let mut selected = roots.clone();
        let mut pending = roots.clone();
        while let Some(id) = pending.pop_first() {
            let check = registry
                .get(&id)
                .ok_or_else(|| HarnessError::MissingCheck(id.clone()))?;
            for dependency in &check.input().depends_on {
                if !registry.contains_key(dependency) {
                    return Err(HarnessError::MissingCheck(dependency.clone()));
                }
                if selected.insert(dependency.clone()) {
                    pending.insert(dependency.clone());
                }
            }
        }
        let order = topological_order(&registry, &selected)?;
        let content = Digest::of("celetixo/source-content/v1", bytes);
        let mut definitions = Vec::with_capacity(order.len());
        let mut approved = BTreeMap::new();
        for id in order {
            let original = &registry[&id];
            let mut input = original.input().clone();
            if input
                .inputs
                .get(path)
                .is_some_and(|digest| *digest != content)
                || input
                    .dependency_inputs
                    .get(path)
                    .is_some_and(|digest| *digest != content)
            {
                return Err(HarnessError::ConflictingConfigurationInput(id));
            }
            input.inputs.insert(path.into(), content);
            let mut binding = Vec::with_capacity(96);
            binding.extend_from_slice(original.input().check.configuration().as_bytes());
            binding.extend_from_slice(content.as_bytes());
            binding.extend_from_slice(original.digest().as_bytes());
            input.check = CheckSpec::new(
                id.clone(),
                input.check.command().to_vec(),
                input.check.toolchain().into(),
                input.check.environment(),
                Digest::of(
                    "celetixo/repository-harness-check-configuration/v1",
                    &binding,
                ),
            )
            .map_err(|_| HarnessError::InvalidSelection)?;
            definitions.push(SourceCheckDefinition::new(input)?);
            approved.insert(id, original.digest());
        }
        let derived = selected.difference(&roots).cloned().collect();
        let mut harness = Self {
            scope: snapshot.scope().clone(),
            snapshot: snapshot.id(),
            manifest: snapshot.manifest().digest(),
            path: path.into(),
            content,
            roots,
            derived,
            approved,
            definitions,
            digest: Digest::of("celetixo/repository-harness/uninitialized", b""),
        };
        harness.digest = Digest::of("celetixo/repository-harness/v1", &harness.canonical_bytes());
        Ok(harness)
    }

    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn configuration_path(&self) -> &str {
        &self.path
    }
    pub fn configuration_content(&self) -> Digest {
        self.content
    }
    pub fn selected_roots(&self) -> &BTreeSet<CheckId> {
        &self.roots
    }
    pub fn derived_prerequisites(&self) -> &BTreeSet<CheckId> {
        &self.derived
    }
    /// Dependency-first order; independent ready checks are ordered by CheckId.
    pub fn definitions(&self) -> &[SourceCheckDefinition] {
        &self.definitions
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [
            self.scope.organization.as_str(),
            self.scope.workspace.as_str(),
            self.scope.repository.as_str(),
            &self.path,
        ] {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        bytes.extend_from_slice(self.snapshot.0.as_bytes());
        bytes.extend_from_slice(self.manifest.as_bytes());
        bytes.extend_from_slice(self.content.as_bytes());
        bytes.extend_from_slice(&(self.definitions.len() as u64).to_be_bytes());
        for definition in &self.definitions {
            let id = definition.id();
            bytes.extend_from_slice(&(id.as_str().len() as u64).to_be_bytes());
            bytes.extend_from_slice(id.as_str().as_bytes());
            bytes.push(u8::from(self.roots.contains(id)));
            bytes.extend_from_slice(self.approved[id].as_bytes());
            bytes.extend_from_slice(definition.digest().as_bytes());
        }
        bytes
    }

    /// Return a validated input for the trusted caller to submit to the Engine.
    /// This method neither installs policy nor grants execution authority.
    pub fn apply_to_policy(
        &self,
        snapshot: &Snapshot,
        mut policy: SourcePolicyInput,
    ) -> Result<SourcePolicyInput, HarnessError> {
        if snapshot.scope() != &self.scope
            || snapshot.id() != self.snapshot
            || snapshot.manifest().digest() != self.manifest
            || policy.scope != self.scope
            || policy.snapshot != self.snapshot
            || policy.capture.snapshot != self.snapshot
            || policy.capture.manifest != self.manifest
        {
            return Err(HarnessError::BindingMismatch);
        }
        for id in policy.always_required.iter().chain(
            policy
                .invariants
                .iter()
                .flat_map(|item| &item.required_checks),
        ) {
            if !self.approved.contains_key(id) {
                return Err(HarnessError::MissingMandatory(id.clone()));
            }
        }
        let mut checks = BTreeMap::new();
        for check in policy.checks {
            let id = check.id().clone();
            if checks.insert(id.clone(), check).is_some() {
                return Err(HarnessError::DuplicateCheck(id));
            }
        }
        for configured in &self.definitions {
            let id = configured.id();
            if checks.get(id).is_some_and(|existing| {
                existing.digest() != self.approved[id] && existing != configured
            }) {
                return Err(HarnessError::PolicyDefinitionMismatch(id.clone()));
            }
            checks.insert(id.clone(), configured.clone());
        }
        policy.checks = checks.into_values().collect();
        policy.always_required.extend(self.roots.iter().cloned());
        SourceExecutionPolicy::new(policy.clone())?.validate_snapshot(snapshot)?;
        Ok(policy)
    }
}

fn decode_digest(value: &str) -> Result<Digest, HarnessError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(HarnessError::InvalidSelection);
    }
    let mut bytes = [0; 32];
    for (destination, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => unreachable!("digest alphabet was checked"),
        };
        *destination = (digit(pair[0]) << 4) | digit(pair[1]);
    }
    Ok(Digest::from_bytes(bytes))
}

fn topological_order(
    registry: &BTreeMap<CheckId, SourceCheckDefinition>,
    selected: &BTreeSet<CheckId>,
) -> Result<Vec<CheckId>, HarnessError> {
    let mut degree = BTreeMap::new();
    let mut consumers: BTreeMap<CheckId, BTreeSet<CheckId>> = BTreeMap::new();
    for id in selected {
        let dependencies = &registry[id].input().depends_on;
        degree.insert(id.clone(), dependencies.len());
        for dependency in dependencies {
            consumers
                .entry(dependency.clone())
                .or_default()
                .insert(id.clone());
        }
    }
    let mut ready: BTreeSet<_> = degree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut order = Vec::with_capacity(selected.len());
    while let Some(id) = ready.pop_first() {
        order.push(id.clone());
        if let Some(next) = consumers.get(&id) {
            for consumer in next {
                let remaining = degree.get_mut(consumer).expect("consumer is selected");
                *remaining -= 1;
                if *remaining == 0 {
                    ready.insert(consumer.clone());
                }
            }
        }
    }
    if order.len() != selected.len() {
        return Err(HarnessError::DependencyCycle);
    }
    Ok(order)
}
