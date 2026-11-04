//! A declared, finite engineering dependency model for the reference environment.
//! This is deliberately not a source-code dependency extractor.
use crate::access::{AccessSet, AnalysisStamp, Completeness, ImpactAnalysis};
use crate::ids::{Digest, ElementId, SnapshotId};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineeringModel {
    pub generation: u64,
    pub analyzer: String,
    pub version: String,
    pub configuration: Digest,
    /// A consumer reads its dependencies, including transitively.
    pub dependencies: BTreeMap<ElementId, BTreeSet<ElementId>>,
    pub completeness: Completeness,
}

impl EngineeringModel {
    pub fn analyze(&self, snapshot: SnapshotId, explicit: &AccessSet) -> ImpactAnalysis {
        let mut derived = AccessSet::default();
        let mut pending: Vec<_> = explicit.reads.union(&explicit.writes).cloned().collect();
        pending.extend(explicit.invariant_scopes.iter().cloned());
        for root in self
            .dependencies
            .keys()
            .chain(self.dependencies.values().flatten())
        {
            if explicit
                .predicates
                .iter()
                .any(|predicate| predicate.matches(root))
            {
                pending.push(root.clone());
                derived.reads.insert(root.clone());
            }
        }
        let mut impacted: Vec<_> = explicit.writes.iter().cloned().collect();
        let mut seen_impact = BTreeSet::new();
        while let Some(element) = impacted.pop() {
            if !seen_impact.insert(element.clone()) {
                continue;
            }
            for (consumer, dependencies) in &self.dependencies {
                if dependencies.contains(&element) {
                    derived.reads.insert(consumer.clone());
                    pending.push(consumer.clone());
                    impacted.push(consumer.clone());
                }
            }
        }
        let mut visited = BTreeSet::new();
        while let Some(element) = pending.pop() {
            if !visited.insert(element.clone()) {
                continue;
            }
            if let Some(dependencies) = self.dependencies.get(&element) {
                for dependency in dependencies {
                    derived.reads.insert(dependency.clone());
                    pending.push(dependency.clone());
                }
            }
        }
        ImpactAnalysis::new(
            AnalysisStamp {
                snapshot,
                index_generation: self.generation,
                analyzer: self.analyzer.clone(),
                version: self.version.clone(),
                configuration: self.configuration,
            },
            derived,
            self.completeness.clone(),
        )
    }
}
