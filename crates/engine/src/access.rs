//! Declared/derived dependency model for the bounded integer-state reference harness.
//!
//! Element identities can describe symbols, relationships, configuration or artifacts.
//! Prefix predicates model membership reads; this is not a language dependency extractor.
use crate::ids::{Canonical, Digest, ElementId, InvariantId, SnapshotId};
use crate::snapshot::State;
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Predicate {
    /// Reads all existing and future identities beginning with the given prefix.
    Prefix(String),
}
impl Predicate {
    pub fn matches(&self, element: &ElementId) -> bool {
        match self {
            Self::Prefix(prefix) => element.as_str().starts_with(prefix),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessSet {
    pub reads: BTreeSet<ElementId>,
    pub writes: BTreeSet<ElementId>,
    pub predicates: BTreeSet<Predicate>,
    pub invariant_scopes: BTreeSet<ElementId>,
}

impl AccessSet {
    pub fn union(&self, other: &Self) -> Self {
        Self {
            reads: self.reads.union(&other.reads).cloned().collect(),
            writes: self.writes.union(&other.writes).cloned().collect(),
            predicates: self.predicates.union(&other.predicates).cloned().collect(),
            invariant_scopes: self
                .invariant_scopes
                .union(&other.invariant_scopes)
                .cloned()
                .collect(),
        }
    }

    /// Deterministic, directional explanations. Read/read overlap is compatible.
    pub fn conflicts(&self, other: &Self) -> Vec<Conflict> {
        let mut conflicts = BTreeSet::new();
        for (kind, left, right) in [
            (ConflictKind::WriteWrite, &self.writes, &other.writes),
            (ConflictKind::ReadWrite, &self.reads, &other.writes),
            (ConflictKind::WriteRead, &self.writes, &other.reads),
            (
                ConflictKind::InvariantOverlap,
                &self.invariant_scopes,
                &other.invariant_scopes,
            ),
            (
                ConflictKind::InvariantWrite,
                &self.invariant_scopes,
                &other.writes,
            ),
            (
                ConflictKind::WriteInvariant,
                &self.writes,
                &other.invariant_scopes,
            ),
        ] {
            for element in left.intersection(right) {
                conflicts.insert(Conflict {
                    kind,
                    element: element.clone(),
                    predicate: None,
                });
            }
        }
        for (kind, predicates, writes) in [
            (
                ConflictKind::PredicateWrite,
                &self.predicates,
                &other.writes,
            ),
            (
                ConflictKind::WritePredicate,
                &other.predicates,
                &self.writes,
            ),
        ] {
            for predicate in predicates {
                for element in writes.iter().filter(|e| predicate.matches(e)) {
                    conflicts.insert(Conflict {
                        kind,
                        element: element.clone(),
                        predicate: Some(predicate.clone()),
                    });
                }
            }
        }
        conflicts.into_iter().collect()
    }

    pub(crate) fn encode(&self, out: &mut Canonical) {
        for set in [&self.reads, &self.writes, &self.invariant_scopes] {
            out.u64(set.len() as u64);
            for element in set {
                out.str(element.as_str());
            }
        }
        out.u64(self.predicates.len() as u64);
        for predicate in &self.predicates {
            match predicate {
                Predicate::Prefix(prefix) => {
                    out.str("prefix/v1");
                    out.str(prefix);
                }
            }
        }
    }

    pub fn digest(&self) -> Digest {
        let mut out = Canonical::default();
        self.encode(&mut out);
        out.finish("celetixo/access-set/v1")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConflictKind {
    WriteWrite,
    ReadWrite,
    WriteRead,
    PredicateWrite,
    WritePredicate,
    InvariantOverlap,
    InvariantWrite,
    WriteInvariant,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Conflict {
    pub kind: ConflictKind,
    pub element: ElementId,
    pub predicate: Option<Predicate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisStamp {
    pub snapshot: SnapshotId,
    pub index_generation: u64,
    pub analyzer: String,
    pub version: String,
    pub configuration: Digest,
}
impl AnalysisStamp {
    pub fn digest(&self) -> Digest {
        let mut out = Canonical::default();
        out.digest(self.snapshot.0);
        out.u64(self.index_generation);
        out.str(&self.analyzer);
        out.str(&self.version);
        out.digest(self.configuration);
        out.finish("celetixo/analysis-stamp/v1")
    }
    pub fn is_valid(&self) -> bool {
        !self.analyzer.trim().is_empty() && !self.version.trim().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Incomplete { reasons: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactAnalysis {
    stamp: AnalysisStamp,
    derived: AccessSet,
    completeness: Completeness,
}
impl ImpactAnalysis {
    pub fn new(stamp: AnalysisStamp, derived: AccessSet, completeness: Completeness) -> Self {
        Self {
            stamp,
            derived,
            completeness,
        }
    }
    pub fn stamp(&self) -> &AnalysisStamp {
        &self.stamp
    }
    pub fn derived(&self) -> &AccessSet {
        &self.derived
    }
    pub fn completeness(&self) -> &Completeness {
        &self.completeness
    }
    pub fn is_complete(&self) -> bool {
        self.stamp.is_valid() && self.completeness == Completeness::Complete
    }
    pub fn digest(&self) -> Digest {
        let mut out = Canonical::default();
        out.digest(self.stamp.digest());
        self.derived.encode(&mut out);
        match &self.completeness {
            Completeness::Complete => out.str("complete"),
            Completeness::Incomplete { reasons } => {
                out.str("incomplete");
                out.u64(reasons.len() as u64);
                for reason in reasons {
                    out.str(reason);
                }
            }
        }
        out.finish("celetixo/impact-analysis/v1")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invariant {
    pub id: InvariantId,
    pub condition: Condition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    Equals {
        element: ElementId,
        value: i64,
    },
    Present(ElementId),
    SumAtMost {
        elements: BTreeSet<ElementId>,
        maximum: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InvariantViolation {
    #[error("invariant {invariant} requires missing element {element}")]
    Missing {
        invariant: InvariantId,
        element: ElementId,
    },
    #[error("invariant {invariant} failed")]
    Unsatisfied { invariant: InvariantId },
    #[error("invariant {invariant} sum overflowed")]
    Overflow { invariant: InvariantId },
}

impl Invariant {
    pub fn scope(&self) -> BTreeSet<ElementId> {
        match &self.condition {
            Condition::Equals { element, .. } | Condition::Present(element) => {
                [element.clone()].into()
            }
            Condition::SumAtMost { elements, .. } => elements.clone(),
        }
    }
    pub fn evaluate(&self, state: &State) -> Result<(), InvariantViolation> {
        let missing = |element: &ElementId| InvariantViolation::Missing {
            invariant: self.id.clone(),
            element: element.clone(),
        };
        let valid = match &self.condition {
            Condition::Present(element) => {
                state.get(element).ok_or_else(|| missing(element))?;
                true
            }
            Condition::Equals { element, value } => {
                state.get(element).ok_or_else(|| missing(element))? == value
            }
            Condition::SumAtMost { elements, maximum } => {
                let mut sum = 0_i64;
                for element in elements {
                    sum = sum
                        .checked_add(*state.get(element).ok_or_else(|| missing(element))?)
                        .ok_or_else(|| InvariantViolation::Overflow {
                            invariant: self.id.clone(),
                        })?;
                }
                sum <= *maximum
            }
        };
        if valid {
            Ok(())
        } else {
            Err(InvariantViolation::Unsatisfied {
                invariant: self.id.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn e(s: &str) -> ElementId {
        ElementId::new(s).unwrap()
    }
    #[test]
    fn compatibility_matrix_is_directional_and_deterministic() {
        let read = AccessSet {
            reads: [e("symbol:x")].into(),
            ..Default::default()
        };
        let write = AccessSet {
            writes: [e("symbol:x")].into(),
            ..Default::default()
        };
        assert!(read.conflicts(&read).is_empty());
        assert_eq!(read.conflicts(&write)[0].kind, ConflictKind::ReadWrite);
        assert_eq!(write.conflicts(&read)[0].kind, ConflictKind::WriteRead);
        assert_eq!(write.conflicts(&write)[0].kind, ConflictKind::WriteWrite);
        let separate = AccessSet {
            writes: [e("symbol:y")].into(),
            ..Default::default()
        };
        assert!(write.conflicts(&separate).is_empty());
    }
    #[test]
    fn predicates_detect_phantom_members_and_invariants_detect_scoped_writes() {
        let query = AccessSet {
            predicates: [Predicate::Prefix("package:".into())].into(),
            invariant_scopes: [e("config:limit")].into(),
            ..Default::default()
        };
        let mutation = AccessSet {
            writes: [e("package:new"), e("config:limit")].into(),
            ..Default::default()
        };
        let kinds: BTreeSet<_> = query
            .conflicts(&mutation)
            .into_iter()
            .map(|c| c.kind)
            .collect();
        assert_eq!(
            kinds,
            [ConflictKind::PredicateWrite, ConflictKind::InvariantWrite].into()
        );
        assert_eq!(query.conflicts(&mutation), query.conflicts(&mutation));
    }
    #[test]
    fn union_retains_explicit_and_derived_dependencies() {
        let explicit = AccessSet {
            reads: [e("source")].into(),
            ..Default::default()
        };
        let derived = AccessSet {
            reads: [e("dependency")].into(),
            writes: [e("generated")].into(),
            ..Default::default()
        };
        let combined = explicit.union(&derived);
        assert_eq!(combined.reads.len(), 2);
        assert_eq!(combined.writes, derived.writes);
        assert_eq!(explicit.reads.len(), 1);
    }
    #[test]
    fn invariant_checks_values_missing_inputs_and_overflow() {
        let elements = [e("a"), e("b")].into();
        let invariant = Invariant {
            id: InvariantId::new("budget").unwrap(),
            condition: Condition::SumAtMost {
                elements,
                maximum: 10,
            },
        };
        assert!(
            invariant
                .evaluate(&[(e("a"), 4), (e("b"), 6)].into())
                .is_ok()
        );
        assert!(matches!(
            invariant.evaluate(&[(e("a"), 5), (e("b"), 6)].into()),
            Err(InvariantViolation::Unsatisfied { .. })
        ));
        assert!(matches!(
            invariant.evaluate(&[(e("a"), 5)].into()),
            Err(InvariantViolation::Missing { .. })
        ));
        assert!(matches!(
            invariant.evaluate(&[(e("a"), i64::MAX), (e("b"), 1)].into()),
            Err(InvariantViolation::Overflow { .. })
        ));
    }
    #[test]
    fn analysis_binding_includes_version_configuration_completeness_and_dependencies() {
        let stamp = AnalysisStamp {
            snapshot: SnapshotId(Digest::of("snapshot", b"base")),
            index_generation: 1,
            analyzer: "reference".into(),
            version: "1".into(),
            configuration: Digest::of("config", b"1"),
        };
        let initial =
            ImpactAnalysis::new(stamp.clone(), AccessSet::default(), Completeness::Complete);
        let mut changed = stamp.clone();
        changed.version = "2".into();
        assert_ne!(
            initial.digest(),
            ImpactAnalysis::new(changed, AccessSet::default(), Completeness::Complete).digest()
        );
        let partial = ImpactAnalysis::new(
            stamp,
            AccessSet::default(),
            Completeness::Incomplete {
                reasons: vec!["unsupported region".into()],
            },
        );
        assert!(!partial.is_complete());
        assert_ne!(initial.digest(), partial.digest());
    }
}
