//! Snapshot-bound repository-analysis contracts consumed by the authoritative Engine.
//!
//! Structural/analyzer adapters produce facts, not transaction or commit authority.
//! Installing a contract requires a scoped administrator capability in `Engine`.
use crate::access::{AccessSet, Completeness, ImpactAnalysis};
use crate::ids::{Canonical, CheckId, Digest, ElementId, Scope, SnapshotId};
use crate::model::EngineeringModel;
use crate::snapshot::Snapshot;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AnalysisGap {
    pub stage: String,
    pub path: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepositoryAnalysisError {
    #[error("repository analysis does not identify the exact registered snapshot and scope")]
    SnapshotMismatch,
    #[error("repository index generation differs from the expected generation")]
    StaleGeneration,
    #[error("analyzer version and gap provenance must be nonempty")]
    InvalidProvenance,
    #[error("analysis dependency/check refers to an unknown semantic identity: {0}")]
    UnknownElement(ElementId),
}

/// Immutable, self-digesting adapter output. Facts retain their full index generation
/// in addition to the Engine's monotonic installed-model generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryAnalysis {
    scope: Scope,
    snapshot: SnapshotId,
    index_generation: Digest,
    analyzer_version: String,
    configuration: Digest,
    elements: BTreeSet<ElementId>,
    dependencies: BTreeMap<ElementId, BTreeSet<ElementId>>,
    checks: BTreeMap<ElementId, BTreeSet<CheckId>>,
    gaps: BTreeSet<AnalysisGap>,
    digest: Digest,
}

#[derive(Debug, Clone)]
pub struct RepositoryAnalysisInput {
    pub index_generation: Digest,
    pub analyzer_version: String,
    pub configuration: Digest,
    pub elements: BTreeSet<ElementId>,
    pub dependencies: BTreeMap<ElementId, BTreeSet<ElementId>>,
    /// Justified verification targets, not permission to run a command or mark it green.
    pub checks: BTreeMap<ElementId, BTreeSet<CheckId>>,
    pub gaps: BTreeSet<AnalysisGap>,
}

impl RepositoryAnalysis {
    pub fn new(
        snapshot: &Snapshot,
        input: RepositoryAnalysisInput,
    ) -> Result<Self, RepositoryAnalysisError> {
        Self::from_original_binding(snapshot.scope().clone(), snapshot.id(), input)
    }
    /// Checked structural value construction. This binds the value to its own
    /// immutable original selectors; installation still requires validate().
    pub(crate) fn from_original_binding(
        scope: Scope,
        snapshot: SnapshotId,
        input: RepositoryAnalysisInput,
    ) -> Result<Self, RepositoryAnalysisError> {
        if input.analyzer_version.trim().is_empty()
            || input
                .gaps
                .iter()
                .any(|gap| gap.stage.trim().is_empty() || gap.detail.trim().is_empty())
        {
            return Err(RepositoryAnalysisError::InvalidProvenance);
        }
        for element in input
            .dependencies
            .keys()
            .chain(input.dependencies.values().flatten())
            .chain(input.checks.keys())
        {
            if !input.elements.contains(element) {
                return Err(RepositoryAnalysisError::UnknownElement(element.clone()));
            }
        }
        let mut record = Self {
            scope,
            snapshot,
            index_generation: input.index_generation,
            analyzer_version: input.analyzer_version,
            configuration: input.configuration,
            elements: input.elements,
            dependencies: input.dependencies,
            checks: input.checks,
            gaps: input.gaps,
            digest: Digest::from_bytes([0; 32]),
        };
        record.digest = Digest::of("celetixo/repository-analysis/v1", &record.canonical_bytes());
        Ok(record)
    }

    /// The existing v1 content-address preimage, exposed for checked persistence.
    /// This does not install facts or confer complete-impact/transaction authority.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut encoding = Canonical::default();
        encoding.str(self.scope().organization.as_str());
        encoding.str(self.scope().workspace.as_str());
        encoding.str(self.scope().repository.as_str());
        encoding.digest(self.snapshot().0);
        encoding.digest(self.index_generation);
        encoding.str(&self.analyzer_version);
        encoding.digest(self.configuration);
        encoding.u64(self.elements.len() as u64);
        for element in &self.elements {
            encoding.str(element.as_str());
        }
        encoding.u64(self.dependencies.len() as u64);
        for (consumer, dependencies) in &self.dependencies {
            encoding.str(consumer.as_str());
            encoding.u64(dependencies.len() as u64);
            for dependency in dependencies {
                encoding.str(dependency.as_str());
            }
        }
        encoding.u64(self.checks.len() as u64);
        for (element, checks) in &self.checks {
            encoding.str(element.as_str());
            encoding.u64(checks.len() as u64);
            for check in checks {
                encoding.str(check.as_str());
            }
        }
        encoding.u64(self.gaps.len() as u64);
        for gap in &self.gaps {
            encoding.str(&gap.stage);
            match &gap.path {
                Some(path) => {
                    encoding.u64(1);
                    encoding.str(path);
                }
                None => encoding.u64(0),
            }
            encoding.str(&gap.detail);
        }
        encoding.into_bytes()
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn index_generation(&self) -> Digest {
        self.index_generation
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn gaps(&self) -> &BTreeSet<AnalysisGap> {
        &self.gaps
    }
    pub fn elements(&self) -> &BTreeSet<ElementId> {
        &self.elements
    }
    pub fn dependencies(&self) -> &BTreeMap<ElementId, BTreeSet<ElementId>> {
        &self.dependencies
    }
    pub fn analyzer_version(&self) -> &str {
        &self.analyzer_version
    }
    pub fn configuration(&self) -> Digest {
        self.configuration
    }
    pub fn checks(&self) -> &BTreeMap<ElementId, BTreeSet<CheckId>> {
        &self.checks
    }
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
    pub fn validate(
        &self,
        snapshot: &Snapshot,
        expected_index: Digest,
    ) -> Result<(), RepositoryAnalysisError> {
        if self.scope != *snapshot.scope() || self.snapshot != snapshot.id() {
            return Err(RepositoryAnalysisError::SnapshotMismatch);
        }
        if self.index_generation != expected_index {
            return Err(RepositoryAnalysisError::StaleGeneration);
        }
        Ok(())
    }
    pub(crate) fn model(&self, generation: u64) -> EngineeringModel {
        EngineeringModel {
            generation,
            analyzer: "celetixo-repository".into(),
            version: self.analyzer_version.clone(),
            configuration: self.digest,
            dependencies: self.dependencies.clone(),
            completeness: if self.gaps.is_empty() {
                Completeness::Complete
            } else {
                Completeness::Incomplete {
                    reasons: self
                        .gaps
                        .iter()
                        .map(|gap| {
                            format!(
                                "{}:{}:{}",
                                gap.stage,
                                gap.path.as_deref().unwrap_or("<workspace>"),
                                gap.detail
                            )
                        })
                        .collect(),
                }
            },
        }
    }
    pub(crate) fn inspect(
        &self,
        generation: u64,
        explicit: &AccessSet,
    ) -> Result<RepositoryImpact, RepositoryAnalysisError> {
        for element in explicit
            .reads
            .iter()
            .chain(&explicit.writes)
            .chain(&explicit.invariant_scopes)
        {
            if !self.elements.contains(element) {
                return Err(RepositoryAnalysisError::UnknownElement(element.clone()));
            }
        }
        let mut expanded = explicit.clone();
        // Predicate membership includes isolated indexed entities as well as
        // entities participating in dependency edges.
        let membership: BTreeSet<_> = self
            .elements
            .iter()
            .filter(|element| {
                explicit
                    .predicates
                    .iter()
                    .any(|predicate| predicate.matches(element))
            })
            .cloned()
            .collect();
        expanded.reads.extend(membership.iter().cloned());
        let initial = self.model(generation).analyze(self.snapshot, &expanded);
        let mut derived = initial.derived().clone();
        derived
            .reads
            .extend(membership.difference(&explicit.reads).cloned());
        let analysis = ImpactAnalysis::new(
            initial.stamp().clone(),
            derived,
            initial.completeness().clone(),
        );
        let effective = explicit.union(analysis.derived());
        let impacted: BTreeSet<_> = effective
            .reads
            .union(&effective.writes)
            .chain(&effective.invariant_scopes)
            .cloned()
            .collect();
        let checks = impacted
            .iter()
            .filter_map(|id| self.checks.get(id))
            .flatten()
            .cloned()
            .collect();
        Ok(RepositoryImpact {
            scope: self.scope.clone(),
            snapshot: self.snapshot,
            index_generation: self.index_generation,
            artifact: self.digest,
            analysis,
            justified_checks: checks,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryImpact {
    pub scope: Scope,
    pub snapshot: SnapshotId,
    pub index_generation: Digest,
    pub artifact: Digest,
    pub analysis: ImpactAnalysis,
    pub justified_checks: BTreeSet<CheckId>,
}
