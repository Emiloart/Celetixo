//! Deterministic graph queries over exact, validated source-index generations.
//! Predicted impact and verification suggestions never grant commit authority.
use crate::capture::{CaptureError, CaptureResult};
use crate::index::{EdgeKind, IndexEdge, IndexError, NodeKind, SourceIndex};
use celetixo_engine::access::{AccessSet, Condition, Invariant, Predicate};
use celetixo_engine::ids::{CheckId, Digest, ElementId, SnapshotId};
use celetixo_engine::repository::{
    AnalysisGap, RepositoryAnalysis, RepositoryAnalysisError, RepositoryAnalysisInput,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImpactError {
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Contract(#[from] RepositoryAnalysisError),
    #[error("impact request identifies a stale snapshot or analyzer generation")]
    StaleRequest,
    #[error("impact request contains an unknown semantic element: {0}")]
    UnknownElement(ElementId),
    #[error("index contains an edge without a current, known endpoint")]
    InvalidEdge,
}

#[derive(Debug, Clone)]
pub struct ImpactRequest {
    pub snapshot: SnapshotId,
    pub generation: Digest,
    pub changed: BTreeSet<ElementId>,
    pub transaction_accesses: AccessSet,
    pub invariants: Vec<Invariant>,
}

impl ImpactRequest {
    /// Binds the entire query, including predicates and invariant conditions even
    /// when two queries happen to produce the same graph traversal.
    pub fn digest(&self) -> Digest {
        let mut c = Encoding::default();
        c.digest(self.snapshot.0);
        c.digest(self.generation);
        c.elements(&self.changed);
        c.digest(self.transaction_accesses.digest());
        c.count(self.invariants.len());
        for invariant in &self.invariants {
            c.text(invariant.id.as_str());
            match &invariant.condition {
                Condition::Present(element) => {
                    c.text("present");
                    c.text(element.as_str());
                }
                Condition::Equals { element, value } => {
                    c.text("equals");
                    c.text(element.as_str());
                    c.bytes(&value.to_be_bytes());
                }
                Condition::SumAtMost { elements, maximum } => {
                    c.text("sum-at-most");
                    c.elements(elements);
                    c.bytes(&maximum.to_be_bytes());
                }
            }
        }
        Digest::of("celetixo/impact-request/v1", &c.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VerificationKind {
    Structural,
    Typecheck,
    TargetedTest,
    PackageBuild,
    GeneratedInput,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VerificationSuggestion {
    pub check: CheckId,
    pub kind: VerificationKind,
    pub path: String,
    pub affected: BTreeSet<ElementId>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactResult {
    pub id: Digest,
    pub snapshot: SnapshotId,
    pub index_generation: Digest,
    pub request: Digest,
    pub direct_changes: BTreeSet<ElementId>,
    pub direct_consumers: BTreeSet<ElementId>,
    pub downstream: BTreeSet<ElementId>,
    pub dependencies: BTreeSet<ElementId>,
    pub affected_files: BTreeSet<String>,
    pub reconsidered_reads: BTreeSet<ElementId>,
    pub reconsidered_writes: BTreeSet<ElementId>,
    pub reconsidered_predicates: BTreeSet<Predicate>,
    pub reconsidered_invariants: BTreeSet<celetixo_engine::ids::InvariantId>,
    pub verification: BTreeSet<VerificationSuggestion>,
    pub gaps: BTreeSet<AnalysisGap>,
    pub edges: BTreeSet<IndexEdge>,
}

impl ImpactResult {
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
    pub fn validate(&self, index: &SourceIndex) -> Result<(), ImpactError> {
        index.validate_generation()?;
        if self.snapshot != index.snapshot || self.index_generation != index.generation {
            return Err(ImpactError::StaleRequest);
        }
        if self.id != self.digest() {
            return Err(ImpactError::StaleRequest);
        }
        Ok(())
    }
    pub fn validate_request(
        &self,
        index: &SourceIndex,
        request: &ImpactRequest,
    ) -> Result<(), ImpactError> {
        self.validate(index)?;
        if self.request != request.digest() {
            return Err(ImpactError::StaleRequest);
        }
        Ok(())
    }
    fn digest(&self) -> Digest {
        let mut c = Encoding::default();
        c.digest(self.snapshot.0);
        c.digest(self.index_generation);
        c.digest(self.request);
        for elements in [
            &self.direct_changes,
            &self.direct_consumers,
            &self.downstream,
            &self.dependencies,
            &self.reconsidered_reads,
            &self.reconsidered_writes,
        ] {
            c.elements(elements);
        }
        c.count(self.affected_files.len());
        for path in &self.affected_files {
            c.text(path);
        }
        c.count(self.reconsidered_predicates.len());
        for predicate in &self.reconsidered_predicates {
            match predicate {
                Predicate::Prefix(prefix) => {
                    c.text("prefix/v1");
                    c.text(prefix);
                }
            }
        }
        c.count(self.reconsidered_invariants.len());
        for invariant in &self.reconsidered_invariants {
            c.text(invariant.as_str());
        }
        c.count(self.verification.len());
        for check in &self.verification {
            c.text(check.check.as_str());
            c.text(verification_tag(check.kind));
            c.text(&check.path);
            c.elements(&check.affected);
            c.text(&check.reason);
        }
        c.count(self.gaps.len());
        for gap in &self.gaps {
            c.text(&gap.stage);
            match &gap.path {
                Some(path) => {
                    c.count(1);
                    c.text(path);
                }
                None => c.count(0),
            }
            c.text(&gap.detail);
        }
        c.count(self.edges.len());
        for edge in &self.edges {
            c.text(edge.from.as_str());
            c.text(edge.to.as_str());
            c.text(edge_tag(edge.kind));
            c.digest(edge.provenance.snapshot.0);
            c.text(&edge.provenance.extractor);
            c.text(&edge.provenance.path);
            for number in [
                edge.provenance.span.start_byte,
                edge.provenance.span.end_byte,
                edge.provenance.span.start_row,
                edge.provenance.span.start_column,
                edge.provenance.span.end_row,
                edge.provenance.span.end_column,
            ] {
                c.count(number);
            }
            c.text(match edge.provenance.resolution {
                crate::index::Resolution::StructuralLexical => "lexical",
                crate::index::Resolution::StructuralModule => "module",
                crate::index::Resolution::DeclaredConfiguration => "configuration",
                crate::index::Resolution::Analyzer => "analyzer",
            });
        }
        Digest::of("celetixo/impact-result/v1", &c.0)
    }
}

/// Frozen validated generation with cached forward and reverse adjacency. Multiple
/// queries reuse the same immutable graph; incremental publication is owned by Indexer.
pub struct ImpactAnalyzer {
    index: SourceIndex,
    dependencies: BTreeMap<ElementId, BTreeSet<ElementId>>,
    consumers: BTreeMap<ElementId, BTreeSet<ElementId>>,
    gaps: BTreeSet<AnalysisGap>,
}

impl ImpactAnalyzer {
    pub fn new(capture: &CaptureResult, index: &SourceIndex) -> Result<Self, ImpactError> {
        capture.validate()?;
        index.validate(&capture.snapshot)?;
        let mut dependencies: BTreeMap<ElementId, BTreeSet<ElementId>> = BTreeMap::new();
        let mut consumers: BTreeMap<ElementId, BTreeSet<ElementId>> = BTreeMap::new();
        for edge in &index.edges {
            if edge.provenance.snapshot != index.snapshot
                || !index.nodes.contains_key(&edge.from)
                || !index.nodes.contains_key(&edge.to)
            {
                return Err(ImpactError::InvalidEdge);
            }
            dependencies
                .entry(edge.from.clone())
                .or_default()
                .insert(edge.to.clone());
            consumers
                .entry(edge.to.clone())
                .or_default()
                .insert(edge.from.clone());
        }
        let mut gaps: BTreeSet<_> = index
            .issues
            .iter()
            .map(|issue| AnalysisGap {
                stage: "source-index".into(),
                path: Some(issue.path.clone()),
                detail: format!("{:?}: {}", issue.kind, issue.detail),
            })
            .collect();
        for issue in &capture.report.issues {
            gaps.insert(AnalysisGap {
                stage: "workspace-capture".into(),
                path: None,
                detail: format!("{issue:?}"),
            });
        }
        Ok(Self {
            index: index.clone(),
            dependencies,
            consumers,
            gaps,
        })
    }
    pub fn index(&self) -> &SourceIndex {
        &self.index
    }
    pub fn analyze(&self, request: &ImpactRequest) -> Result<ImpactResult, ImpactError> {
        if request.snapshot != self.index.snapshot || request.generation != self.index.generation {
            return Err(ImpactError::StaleRequest);
        }
        for id in request
            .changed
            .iter()
            .chain(&request.transaction_accesses.reads)
            .chain(&request.transaction_accesses.writes)
            .chain(&request.transaction_accesses.invariant_scopes)
        {
            if !self.index.nodes.contains_key(id) {
                return Err(ImpactError::UnknownElement(id.clone()));
            }
        }
        for invariant in &request.invariants {
            for id in invariant.scope() {
                if !self.index.nodes.contains_key(&id) {
                    return Err(ImpactError::UnknownElement(id));
                }
            }
        }
        let direct_consumers = request
            .changed
            .iter()
            .filter_map(|id| self.consumers.get(id))
            .flatten()
            .filter(|id| !request.changed.contains(*id))
            .cloned()
            .collect();
        let downstream = closure(&self.consumers, &request.changed)
            .difference(&request.changed)
            .cloned()
            .collect::<BTreeSet<_>>();
        let impacted = request
            .changed
            .union(&downstream)
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut roots = impacted.clone();
        roots.extend(request.transaction_accesses.reads.iter().cloned());
        roots.extend(request.transaction_accesses.writes.iter().cloned());
        roots.extend(
            request
                .transaction_accesses
                .invariant_scopes
                .iter()
                .cloned(),
        );
        for predicate in &request.transaction_accesses.predicates {
            roots.extend(
                self.index
                    .nodes
                    .keys()
                    .filter(|id| predicate.matches(id))
                    .cloned(),
            );
        }
        for invariant in &request.invariants {
            roots.extend(invariant.scope());
        }
        // Query roots already have explicit roles (changed, predicted consumer,
        // or transaction assumption). Do not relabel those seeds as discovered
        // prerequisite dependencies merely because closure includes its roots.
        let dependencies = closure(&self.dependencies, &roots)
            .difference(&roots)
            .cloned()
            .collect::<BTreeSet<_>>();
        let affected_files = impacted
            .iter()
            .filter_map(|id| self.index.nodes.get(id))
            .map(|node| node.path.clone())
            .collect();
        let reconsidered_reads = request
            .transaction_accesses
            .reads
            .intersection(&impacted)
            .cloned()
            .collect();
        let reconsidered_writes = request
            .transaction_accesses
            .writes
            .intersection(&impacted)
            .cloned()
            .collect();
        let reconsidered_predicates = request
            .transaction_accesses
            .predicates
            .iter()
            .filter(|predicate| impacted.iter().any(|element| predicate.matches(element)))
            .cloned()
            .collect();
        let reconsidered_invariants = request
            .invariants
            .iter()
            .filter(|invariant| !invariant.scope().is_disjoint(&impacted))
            .map(|invariant| invariant.id.clone())
            .collect();
        let verification = self.verification(&impacted);
        let edges = self
            .index
            .edges
            .iter()
            .filter(|edge| {
                impacted.contains(&edge.from)
                    || impacted.contains(&edge.to)
                    || roots.contains(&edge.from)
                    || dependencies.contains(&edge.from)
            })
            .cloned()
            .collect();
        let mut result = ImpactResult {
            id: Digest::of("pending", b""),
            snapshot: self.index.snapshot,
            index_generation: self.index.generation,
            request: request.digest(),
            direct_changes: request.changed.clone(),
            direct_consumers,
            downstream,
            dependencies,
            affected_files,
            reconsidered_reads,
            reconsidered_writes,
            reconsidered_predicates,
            reconsidered_invariants,
            verification,
            gaps: self.gaps.clone(),
            edges,
        };
        result.id = result.digest();
        Ok(result)
    }
    fn verification(&self, affected: &BTreeSet<ElementId>) -> BTreeSet<VerificationSuggestion> {
        let mut paths: BTreeMap<String, BTreeSet<ElementId>> = BTreeMap::new();
        for id in affected {
            if let Some(node) = self.index.nodes.get(id) {
                paths
                    .entry(node.path.clone())
                    .or_default()
                    .insert(id.clone());
            }
        }
        let mut checks = BTreeSet::new();
        for (path, elements) in paths {
            let Some(file) = self.index.files.get(&path) else {
                continue;
            };
            let node = &self.index.nodes[&file.file];
            let mut kinds = vec![];
            if file.language.is_some() {
                kinds.extend([VerificationKind::Structural, VerificationKind::Typecheck]);
            }
            if is_test_path(&path)
                || elements
                    .iter()
                    .filter_map(|id| self.index.nodes.get(id))
                    .any(|node| node.name.starts_with("test_") || node.name.starts_with("Test"))
            {
                kinds.push(VerificationKind::TargetedTest);
            }
            if matches!(
                node.kind,
                NodeKind::Package | NodeKind::Configuration | NodeKind::Lockfile
            ) {
                kinds.push(VerificationKind::PackageBuild);
            }
            if self.index.edges.iter().any(|edge| {
                edge.kind == EdgeKind::Generated
                    && (elements.contains(&edge.from) || elements.contains(&edge.to))
            }) {
                kinds.push(VerificationKind::GeneratedInput);
            }
            for kind in kinds {
                let mut encoding = Encoding::default();
                encoding.text(&path);
                encoding.text(verification_tag(kind));
                let check = CheckId::new(format!(
                    "verify:{}",
                    Digest::of("celetixo/verification-target/v1", &encoding.0)
                ))
                .expect("bounded digest identifier");
                checks.insert(VerificationSuggestion {check,kind,path:path.clone(),affected:elements.clone(),reason:match kind {
                    VerificationKind::Structural=>"affected source requires syntax/structural verification",
                    VerificationKind::Typecheck=>"affected source may change type or interface assumptions; use the repository-configured semantic check",
                    VerificationKind::TargetedTest=>"the declared graph reaches a test source or test declaration",
                    VerificationKind::PackageBuild=>"manifest, dependency lock or build configuration participates in impact",
                    VerificationKind::GeneratedInput=>"declared generator input/output relationship participates in impact",
                }.into()});
            }
        }
        checks
    }
    pub fn engine_contract(
        &self,
        capture: &CaptureResult,
    ) -> Result<RepositoryAnalysis, ImpactError> {
        capture.validate()?;
        self.index.validate(&capture.snapshot)?;
        let all = self.index.nodes.keys().cloned().collect();
        let mut checks: BTreeMap<ElementId, BTreeSet<CheckId>> = BTreeMap::new();
        for suggestion in self.verification(&all) {
            for id in suggestion.affected {
                checks
                    .entry(id)
                    .or_default()
                    .insert(suggestion.check.clone());
            }
        }
        Ok(RepositoryAnalysis::new(
            &capture.snapshot,
            RepositoryAnalysisInput {
                index_generation: self.index.generation,
                analyzer_version: "celetixo-repository/v1".into(),
                configuration: self.index.config_digest,
                elements: self.index.nodes.keys().cloned().collect(),
                dependencies: self.dependencies.clone(),
                checks,
                gaps: self.gaps.clone(),
            },
        )?)
    }
}

fn closure(
    graph: &BTreeMap<ElementId, BTreeSet<ElementId>>,
    roots: &BTreeSet<ElementId>,
) -> BTreeSet<ElementId> {
    let mut seen = BTreeSet::new();
    let mut pending: Vec<_> = roots.iter().cloned().collect();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(next) = graph.get(&id) {
            pending.extend(next.iter().cloned());
        }
    }
    seen
}
fn is_test_path(path: &str) -> bool {
    path.starts_with("tests/")
        || path.contains("/tests/")
        || path.ends_with("_test.go")
        || path.contains(".test.")
        || path.contains(".spec.")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("test_"))
}
fn verification_tag(kind: VerificationKind) -> &'static str {
    match kind {
        VerificationKind::Structural => "structural",
        VerificationKind::Typecheck => "typecheck",
        VerificationKind::TargetedTest => "test",
        VerificationKind::PackageBuild => "build",
        VerificationKind::GeneratedInput => "generated",
    }
}
fn edge_tag(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Contains => "contains",
        EdgeKind::Reference => "reference",
        EdgeKind::Call => "call",
        EdgeKind::Import => "import",
        EdgeKind::Type => "type",
        EdgeKind::Implements => "implements",
        EdgeKind::Package => "package",
        EdgeKind::Build => "build",
        EdgeKind::Generated => "generated",
    }
}
#[derive(Default)]
struct Encoding(Vec<u8>);
impl Encoding {
    fn count(&mut self, value: usize) {
        self.0.extend_from_slice(&(value as u64).to_be_bytes());
    }
    fn bytes(&mut self, value: &[u8]) {
        self.count(value.len());
        self.0.extend_from_slice(value);
    }
    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }
    fn digest(&mut self, value: Digest) {
        self.bytes(value.as_bytes());
    }
    fn elements(&mut self, value: &BTreeSet<ElementId>) {
        self.count(value.len());
        for id in value {
            self.text(id.as_str());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpactQuality {
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
}
/// Reference labels are supplied independently by fixture authors, never inferred from
/// the computed graph. Undefined empty denominators remain undefined, rather than 100%.
pub fn measure_quality(actual: &BTreeSet<String>, reference: &BTreeSet<String>) -> ImpactQuality {
    let true_positives = actual.intersection(reference).count();
    ImpactQuality {
        true_positives,
        false_positives: actual.difference(reference).count(),
        false_negatives: reference.difference(actual).count(),
        precision: (!actual.is_empty()).then(|| true_positives as f64 / actual.len() as f64),
        recall: (!reference.is_empty()).then(|| true_positives as f64 / reference.len() as f64),
    }
}
