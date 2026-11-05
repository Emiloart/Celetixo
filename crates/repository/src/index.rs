//! Snapshot-bound structural source indexing. Parsing never implies semantic completeness.
//!
//! Every edge points from a consumer to its dependency. Indexing reads only immutable
//! snapshot bytes, never a live worktree, and publishes a new generation only on success.
use celetixo_engine::ids::{Digest, ElementId, Scope, SnapshotId};
use celetixo_engine::snapshot::{InputKind, Snapshot};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use tree_sitter::{Node, Parser};

pub const STRUCTURAL_EXTRACTOR_VERSION: &str = "celetixo-structural/v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Language {
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Rust,
}
impl Language {
    pub fn for_path(path: &str) -> Option<Self> {
        match path.rsplit('.').next()? {
            "ts" | "mts" | "cts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            "rs" => Some(Self::Rust),
            _ => None,
        }
    }
    pub fn parser_version(self) -> &'static str {
        match self {
            Self::TypeScript | Self::Tsx => "tree-sitter-typescript/0.23.2",
            Self::JavaScript => "tree-sitter-javascript/0.25.0",
            Self::Python => "tree-sitter-python/0.25.0",
            Self::Go => "tree-sitter-go/0.25.0",
            Self::Rust => "tree-sitter-rust/0.24.0",
        }
    }
    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_row: usize,
    pub start_column: usize,
    pub end_row: usize,
    pub end_column: usize,
}
impl SourceSpan {
    fn of(node: Node<'_>) -> Self {
        Self {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_row: node.start_position().row,
            start_column: node.start_position().column,
            end_row: node.end_position().row,
            end_column: node.end_position().column,
        }
    }
    pub fn contains(&self, other: &Self) -> bool {
        self.start_byte <= other.start_byte && other.end_byte <= self.end_byte
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeKind {
    File,
    Module,
    Scope,
    Function,
    Method,
    Class,
    Interface,
    Type,
    Struct,
    Trait,
    Enum,
    Variable,
    Constant,
    Parameter,
    Package,
    Configuration,
    Lockfile,
    Generated,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexNode {
    pub id: ElementId,
    pub path: String,
    pub name: String,
    pub kind: NodeKind,
    pub language: Option<Language>,
    pub span: SourceSpan,
    /// Exact declaration identifier range. Synthetic file/module/package/scope
    /// nodes have no selection range; containment alone is not a definition.
    pub name_span: Option<SourceSpan>,
    pub parent: Option<ElementId>,
    pub signature: Digest,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeKind {
    Contains,
    Reference,
    Call,
    Import,
    Type,
    Implements,
    Package,
    Build,
    Generated,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeProvenance {
    pub snapshot: SnapshotId,
    pub extractor: String,
    pub path: String,
    pub span: SourceSpan,
    pub resolution: Resolution,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Resolution {
    StructuralLexical,
    StructuralModule,
    DeclaredConfiguration,
    Analyzer,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexEdge {
    pub from: ElementId,
    pub to: ElementId,
    pub kind: EdgeKind,
    pub provenance: EdgeProvenance,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OccurrenceKind {
    Reference,
    Call,
    Import,
    Type,
    Implements,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Occurrence {
    pub id: Digest,
    pub path: String,
    pub span: SourceSpan,
    pub container: ElementId,
    pub name: String,
    pub kind: OccurrenceKind,
    pub target: Option<ElementId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueKind {
    SyntaxError,
    MissingSyntax,
    UnsupportedLanguage,
    UnsupportedConstruct,
    InvalidEncoding,
    NonRegularInput,
    UnresolvedReference,
    AmbiguousReference,
    UnresolvedImport,
    SemanticAnalysisUnavailable,
    ConfigurationUnresolved,
    GeneratedInputMissing,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexIssue {
    pub path: String,
    pub span: SourceSpan,
    pub kind: IssueKind,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    pub path: String,
    pub content_digest: Digest,
    pub language: Option<Language>,
    pub parser_version: Option<String>,
    pub file: ElementId,
    pub module: ElementId,
    pub bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexConfig {
    /// Caller-selected analysis profile identity. The implementation/extractor
    /// identity remains STRUCTURAL_EXTRACTOR_VERSION and is separately recorded on
    /// structural edges; changing this label does not install another analyzer.
    pub analyzer_version: String,
    pub language_configuration: BTreeMap<String, String>,
    /// Explicit generated output -> source/configuration inputs. No guessed generator relation.
    pub generated_inputs: BTreeMap<String, BTreeSet<String>>,
    pub max_file_bytes: usize,
    pub max_syntax_nodes: usize,
}
impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            analyzer_version: STRUCTURAL_EXTRACTOR_VERSION.into(),
            language_configuration: BTreeMap::new(),
            generated_inputs: BTreeMap::new(),
            max_file_bytes: 16 * 1024 * 1024,
            max_syntax_nodes: 2_000_000,
        }
    }
}
impl IndexConfig {
    pub fn digest(&self) -> Digest {
        let mut c = Canonical::default();
        c.str("tree-sitter/0.26.13");
        c.str(STRUCTURAL_EXTRACTOR_VERSION);
        c.value(self);
        for l in [
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
            Language::Go,
            Language::Rust,
        ] {
            c.str(l.parser_version());
        }
        c.finish("celetixo/index-config/v2")
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIndex {
    pub snapshot: SnapshotId,
    pub scope: Scope,
    /// Versioned repository incarnation namespace: scope, authoritative revision
    /// and Git base. Shape equality never recycles an earlier revision's IDs.
    pub identity_namespace: Digest,
    pub generation: Digest,
    pub config_digest: Digest,
    pub configuration: IndexConfig,
    pub files: BTreeMap<String, IndexedFile>,
    pub nodes: BTreeMap<ElementId, IndexNode>,
    pub edges: BTreeSet<IndexEdge>,
    pub occurrences: Vec<Occurrence>,
    pub issues: BTreeSet<IndexIssue>,
}
impl SourceIndex {
    /// Structural output is always explicitly incomplete without semantic service coverage.
    pub fn is_complete(&self) -> bool {
        self.issues.is_empty()
    }
    pub fn refresh_generation(&mut self) {
        self.generation = self.digest();
    }
    pub fn validate_generation(&self) -> Result<(), IndexError> {
        if self.config_digest != self.configuration.digest() {
            return Err(IndexError::ConfigurationMismatch);
        }
        if self.generation != self.digest() {
            return Err(IndexError::CorruptGeneration);
        }
        self.validate_structure()
    }
    pub fn validate(&self, snapshot: &Snapshot) -> Result<(), IndexError> {
        if self.snapshot != snapshot.id()
            || self.scope != *snapshot.scope()
            || self.identity_namespace != identity_namespace(snapshot)
        {
            return Err(IndexError::SnapshotMismatch);
        }
        self.validate_generation()?;
        let inputs = effective_inputs(snapshot)?;
        if inputs.len() != self.files.len() {
            return Err(IndexError::InvalidStructure("snapshot file census"));
        }
        for (path, (_, bytes)) in &inputs {
            let Some(file) = self.files.get(path) else {
                return Err(IndexError::InvalidStructure("snapshot file census"));
            };
            if file.bytes != bytes.len()
                || file.content_digest != Digest::of("celetixo/source-bytes/v1", bytes)
            {
                return Err(IndexError::InvalidStructure("snapshot file content"));
            }
        }
        for node in self.nodes.values() {
            if let Some(name_span) = node.name_span
                && inputs[&node.path]
                    .1
                    .get(name_span.start_byte..name_span.end_byte)
                    != Some(node.name.as_bytes())
            {
                return Err(IndexError::InvalidStructure(
                    "declaration selection content",
                ));
            }
        }
        Ok(())
    }

    /// A matching digest authenticates no producer and does not make malformed
    /// facts safe to consume. Validate cross-references before graph consumers
    /// dereference them. Analyzer provenance still needs trusted admission.
    fn validate_structure(&self) -> Result<(), IndexError> {
        let valid_span = |path: &str, span: SourceSpan| {
            self.files.get(path).is_some_and(|file| {
                span.start_byte <= span.end_byte
                    && span.end_byte <= file.bytes
                    && (span.start_row, span.start_column) <= (span.end_row, span.end_column)
            })
        };
        for (path, file) in &self.files {
            let Some(file_node) = self.nodes.get(&file.file) else {
                return Err(IndexError::InvalidStructure("missing file node"));
            };
            let Some(module) = self.nodes.get(&file.module) else {
                return Err(IndexError::InvalidStructure("missing file module"));
            };
            if path != &file.path
                || file_node.path != *path
                || module.path != *path
                || file_node.parent.is_some()
                || module.parent.as_ref() != Some(&file.file)
                || module.kind != NodeKind::Module
                || !matches!(
                    file_node.kind,
                    NodeKind::File
                        | NodeKind::Configuration
                        | NodeKind::Package
                        | NodeKind::Lockfile
                        | NodeKind::Generated
                )
                || file_node.signature != file.content_digest
                || module.signature != file.content_digest
                || file_node.span.start_byte != 0
                || file_node.span.end_byte != file.bytes
                || module.span != file_node.span
                || file.language != Language::for_path(path)
                || file.parser_version.as_deref() != file.language.map(Language::parser_version)
                || file_node.language != file.language
                || module.language != file.language
            {
                return Err(IndexError::InvalidStructure("file/module binding"));
            }
        }
        for (id, node) in &self.nodes {
            if id != &node.id
                || !valid_span(&node.path, node.span)
                || node.name_span.is_some_and(|name_span| {
                    !valid_span(&node.path, name_span) || !node.span.contains(&name_span)
                })
            {
                return Err(IndexError::InvalidStructure("node identity/location"));
            }
            if let Some(parent) = &node.parent {
                let Some(parent) = self.nodes.get(parent) else {
                    return Err(IndexError::InvalidStructure("missing parent node"));
                };
                if parent.path != node.path || !parent.span.contains(&node.span) {
                    return Err(IndexError::InvalidStructure("parent location"));
                }
            }
        }
        // Iterative linear parent traversal also rejects zero-width cycles,
        // which location containment by itself cannot rule out.
        let mut visited = BTreeSet::new();
        for start in self.nodes.keys() {
            let mut current = Some(start);
            let mut ancestors = BTreeSet::new();
            while let Some(id) = current {
                if visited.contains(id) {
                    break;
                }
                if !ancestors.insert(id) {
                    return Err(IndexError::InvalidStructure("parent cycle"));
                }
                current = self.nodes[id].parent.as_ref();
            }
            visited.extend(ancestors);
        }
        for edge in &self.edges {
            if !self.nodes.contains_key(&edge.from)
                || !self.nodes.contains_key(&edge.to)
                || edge.provenance.snapshot != self.snapshot
                || edge.provenance.extractor.trim().is_empty()
                || !valid_span(&edge.provenance.path, edge.provenance.span)
            {
                return Err(IndexError::InvalidStructure(
                    "dependency endpoint/provenance",
                ));
            }
        }
        let mut occurrence_ids = BTreeSet::new();
        for occurrence in &self.occurrences {
            let Some(container) = self.nodes.get(&occurrence.container) else {
                return Err(IndexError::InvalidStructure("missing occurrence container"));
            };
            if !occurrence_ids.insert(occurrence.id)
                || occurrence.id != occurrence_id(self.snapshot, occurrence)
                || container.path != occurrence.path
                || !container.span.contains(&occurrence.span)
                || !valid_span(&occurrence.path, occurrence.span)
                || occurrence
                    .target
                    .as_ref()
                    .is_some_and(|target| !self.nodes.contains_key(target))
            {
                return Err(IndexError::InvalidStructure("occurrence binding"));
            }
        }
        Ok(())
    }
    pub fn validate_configuration(&self, config: &IndexConfig) -> Result<(), IndexError> {
        if self.config_digest != config.digest() {
            return Err(IndexError::ConfigurationMismatch);
        }
        self.validate_generation()
    }
    pub fn symbols(&self, name: &str) -> Vec<&IndexNode> {
        self.nodes.values().filter(|n| n.name == name).collect()
    }
    pub fn dependencies(&self, id: &ElementId) -> BTreeSet<ElementId> {
        self.edges
            .iter()
            .filter(|e| &e.from == id)
            .map(|e| e.to.clone())
            .collect()
    }
    pub fn dependents(&self, id: &ElementId) -> BTreeSet<ElementId> {
        self.edges
            .iter()
            .filter(|e| &e.to == id)
            .map(|e| e.from.clone())
            .collect()
    }
    fn digest(&self) -> Digest {
        let mut c = Canonical::default();
        c.str(&self.snapshot.to_string());
        c.value(&self.scope);
        c.value(&self.identity_namespace);
        c.str(&self.config_digest.to_string());
        c.value(&self.configuration);
        c.value(&self.files);
        c.value(&self.nodes);
        c.value(&self.edges);
        c.value(&self.occurrences);
        c.value(&self.issues);
        c.finish("celetixo/source-index/v2")
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexStats {
    pub parsed_files: usize,
    pub reused_files: usize,
    pub indexed_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IndexError {
    #[error("snapshot or repository scope does not match the index")]
    SnapshotMismatch,
    #[error("index generation does not bind its current contents")]
    CorruptGeneration,
    #[error("index contains inconsistent structural facts: {0}")]
    InvalidStructure(&'static str),
    #[error("invalid analyzer configuration")]
    InvalidConfiguration,
    #[error("index was produced with a different analyzer/configuration generation")]
    ConfigurationMismatch,
    #[error("source input exceeds configured resource budget: {0}")]
    ResourceLimit(String),
    #[error("could not initialize grammar: {0}")]
    Grammar(String),
    #[error("parser was cancelled or failed: {0}")]
    ParseFailed(String),
    #[error("effective workspace cannot be established: {0}")]
    EffectiveWorkspace(String),
}
#[derive(Clone)]
struct CachedFile {
    digest: Digest,
    language: Language,
    tree: tree_sitter::Tree,
}
/// Scope-local cache. Cache hits reuse syntax trees, not unversioned semantic decisions.
pub struct Indexer {
    config: IndexConfig,
    cache: BTreeMap<String, CachedFile>,
    cache_scope: Option<Scope>,
    stats: IndexStats,
    published: Option<SourceIndex>,
}
impl Indexer {
    pub fn new(config: IndexConfig) -> Result<Self, IndexError> {
        if config.analyzer_version.trim().is_empty()
            || config.max_file_bytes == 0
            || config.max_syntax_nodes == 0
        {
            return Err(IndexError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            cache: BTreeMap::new(),
            cache_scope: None,
            stats: IndexStats::default(),
            published: None,
        })
    }
    pub fn config(&self) -> &IndexConfig {
        &self.config
    }
    pub fn stats(&self) -> IndexStats {
        self.stats
    }
    pub fn published(&self) -> Option<&SourceIndex> {
        self.published.as_ref()
    }
    pub fn build(&mut self, snapshot: &Snapshot) -> Result<SourceIndex, IndexError> {
        self.update(snapshot)
    }
    pub fn update(&mut self, snapshot: &Snapshot) -> Result<SourceIndex, IndexError> {
        self.update_with_resolution(snapshot, resolve_references)
    }
    fn update_with_resolution(
        &mut self,
        snapshot: &Snapshot,
        resolve: fn(&mut SourceIndex, &[Pending]),
    ) -> Result<SourceIndex, IndexError> {
        let inputs = effective_inputs(snapshot)?;
        let mut index = SourceIndex {
            snapshot: snapshot.id(),
            scope: snapshot.scope().clone(),
            identity_namespace: identity_namespace(snapshot),
            generation: Digest::of("pending", b""),
            config_digest: self.config.digest(),
            configuration: self.config.clone(),
            files: BTreeMap::new(),
            nodes: BTreeMap::new(),
            edges: BTreeSet::new(),
            occurrences: Vec::new(),
            issues: BTreeSet::new(),
        };
        for (path, rationale) in &snapshot.manifest().inclusion_policy.excluded_paths {
            issue(
                &mut index,
                path,
                SourceSpan::default(),
                IssueKind::ConfigurationUnresolved,
                &format!("capture exclusion requires impact policy: {rationale}"),
            );
        }
        let mut next_cache = BTreeMap::new();
        let mut stats = IndexStats::default();
        let mut pending = Vec::new();
        for (path, (kind, bytes)) in &inputs {
            if bytes.len() > self.config.max_file_bytes {
                return Err(IndexError::ResourceLimit(path.clone()));
            }
            stats.indexed_bytes += bytes.len();
            let language = Language::for_path(path);
            let digest = Digest::of("celetixo/source-bytes/v1", bytes);
            let file_kind = classify_noncode(path);
            let span = SourceSpan {
                end_byte: bytes.len(),
                ..SourceSpan::default()
            };
            let file = make_id(snapshot, path, "file", &format!("{digest}"));
            let module = make_id(snapshot, path, "module", &format!("{digest}"));
            index.nodes.insert(
                file.clone(),
                IndexNode {
                    id: file.clone(),
                    path: path.clone(),
                    name: path.clone(),
                    kind: file_kind,
                    language,
                    span,
                    name_span: None,
                    parent: None,
                    signature: digest,
                },
            );
            index.nodes.insert(
                module.clone(),
                IndexNode {
                    id: module.clone(),
                    path: path.clone(),
                    name: module_name(path),
                    kind: NodeKind::Module,
                    language,
                    span,
                    name_span: None,
                    parent: Some(file.clone()),
                    signature: digest,
                },
            );
            add_edge(
                &mut index,
                module.clone(),
                file.clone(),
                EdgeKind::Contains,
                path,
                span,
                Resolution::StructuralLexical,
            );
            index.files.insert(
                path.clone(),
                IndexedFile {
                    path: path.clone(),
                    content_digest: digest,
                    language,
                    parser_version: language.map(|l| l.parser_version().into()),
                    file: file.clone(),
                    module: module.clone(),
                    bytes: bytes.len(),
                },
            );
            if *kind != InputKind::File {
                issue(
                    &mut index,
                    path,
                    span,
                    IssueKind::NonRegularInput,
                    "symlink/gitlink is captured but not followed as source",
                );
                continue;
            }
            let Some(language) = language else {
                if !matches!(
                    file_kind,
                    NodeKind::Configuration | NodeKind::Package | NodeKind::Lockfile
                ) && !is_documentation(path)
                {
                    issue(
                        &mut index,
                        path,
                        span,
                        IssueKind::UnsupportedLanguage,
                        "no structural language adapter; input remains a dependency",
                    );
                }
                continue;
            };
            if std::str::from_utf8(bytes).is_err() {
                issue(
                    &mut index,
                    path,
                    span,
                    IssueKind::InvalidEncoding,
                    "source is not UTF-8",
                );
                continue;
            }
            let tree = if self.cache_scope.as_ref() == Some(snapshot.scope()) {
                self.cache
                    .get(path)
                    .filter(|v| v.digest == digest && v.language == language)
                    .map(|v| v.tree.clone())
            } else {
                None
            };
            let tree = match tree {
                Some(tree) => {
                    stats.reused_files += 1;
                    tree
                }
                None => {
                    stats.parsed_files += 1;
                    let mut parser = Parser::new();
                    parser
                        .set_language(&language.grammar())
                        .map_err(|e| IndexError::Grammar(e.to_string()))?;
                    parser
                        .parse(bytes, None)
                        .ok_or_else(|| IndexError::ParseFailed(path.clone()))?
                }
            };
            validate_tree_budget(tree.root_node(), path, self.config.max_syntax_nodes)?;
            if language == Language::Go {
                let mut cursor = tree.root_node().walk();
                if let Some(package) = tree
                    .root_node()
                    .named_children(&mut cursor)
                    .find(|n| n.kind() == "package_clause")
                    && let Some(name) = package.named_child(0)
                {
                    index
                        .nodes
                        .get_mut(&module)
                        .expect("file module exists")
                        .name = text(name, bytes).into();
                }
            }
            next_cache.insert(
                path.clone(),
                CachedFile {
                    digest,
                    language,
                    tree: tree.clone(),
                },
            );
            let mut count = 0;
            extract_declarations(
                tree.root_node(),
                bytes,
                path,
                language,
                &module,
                snapshot,
                &mut index,
                &mut count,
                self.config.max_syntax_nodes,
            )?;
            extract_occurrences(
                tree.root_node(),
                bytes,
                path,
                language,
                &module,
                &mut pending,
                &mut index,
            );
            issue(
                &mut index,
                path,
                span,
                IssueKind::SemanticAnalysisUnavailable,
                "structural lexical/module facts only; type checking, dispatch, external dependencies and macro/dynamic behavior require version-bound semantic analysis",
            );
        }
        if !self.config.language_configuration.is_empty() {
            issue(
                &mut index,
                "<configuration>",
                SourceSpan::default(),
                IssueKind::ConfigurationUnresolved,
                "language configuration is version-bound provenance; semantic compiler settings require a configured language-service adapter",
            );
        }
        resolve_imports(&mut index, &pending, &inputs);
        resolve(&mut index, &pending);
        configuration_edges(&mut index, &inputs, &self.config);
        go_package_edges(&mut index);
        index.occurrences.sort();
        index.generation = index.digest();
        index.validate(snapshot)?;
        self.cache = next_cache;
        self.cache_scope = Some(snapshot.scope().clone());
        self.stats = stats;
        self.published = Some(index.clone());
        Ok(index)
    }
}

/// Effective source view; all original layers remain part of snapshot identity.
/// Generated/mounted files may introduce paths, but aliases require explicit agreement.
pub fn effective_inputs(
    snapshot: &Snapshot,
) -> Result<BTreeMap<String, (InputKind, Vec<u8>)>, IndexError> {
    crate::capture::effective_workspace(snapshot)
        .map(|files| {
            files
                .into_iter()
                .map(|(path, input)| {
                    (
                        path,
                        (
                            input.kind,
                            input
                                .bytes
                                .expect("effective_workspace returns present inputs"),
                        ),
                    )
                })
                .collect()
        })
        .map_err(|error| IndexError::EffectiveWorkspace(error.to_string()))
}
#[derive(Default)]
struct Canonical(Vec<u8>);
impl Canonical {
    fn str(&mut self, value: &str) {
        self.0
            .extend_from_slice(&(value.len() as u64).to_be_bytes());
        self.0.extend_from_slice(value.as_bytes());
    }
    fn value(&mut self, value: &impl Encode) {
        value.encode(self);
    }
    fn usize(&mut self, value: usize) {
        self.0.extend_from_slice(&(value as u64).to_be_bytes());
    }
    fn finish(self, domain: &str) -> Digest {
        Digest::of(domain, &self.0)
    }
}
fn identity_namespace(snapshot: &Snapshot) -> Digest {
    let mut c = Canonical::default();
    c.value(snapshot.scope());
    c.0.extend_from_slice(&snapshot.revision().0.to_be_bytes());
    c.str(&snapshot.manifest().base_commit);
    c.finish("celetixo/repository-identity-namespace/v2")
}

fn make_id(snapshot: &Snapshot, path: &str, kind: &str, identity: &str) -> ElementId {
    let mut c = Canonical::default();
    c.value(&identity_namespace(snapshot));
    c.str(path);
    c.str(kind);
    c.str(identity);
    ElementId::new(format!(
        "source:{}",
        c.finish("celetixo/semantic-element/v2")
    ))
    .expect("digest identifier is bounded ASCII")
}
fn package_id(namespace: Digest, signature: Digest) -> ElementId {
    let mut c = Canonical::default();
    c.value(&namespace);
    c.value(&signature);
    ElementId::new(format!(
        "package:{}",
        c.finish("celetixo/package-element/v2")
    ))
    .expect("bounded package digest ID")
}
fn issue(index: &mut SourceIndex, path: &str, span: SourceSpan, kind: IssueKind, detail: &str) {
    index.issues.insert(IndexIssue {
        path: path.into(),
        span,
        kind,
        detail: detail.into(),
    });
}
fn add_edge(
    index: &mut SourceIndex,
    from: ElementId,
    to: ElementId,
    kind: EdgeKind,
    path: &str,
    span: SourceSpan,
    resolution: Resolution,
) {
    index.edges.insert(IndexEdge {
        from,
        to,
        kind,
        provenance: EdgeProvenance {
            snapshot: index.snapshot,
            extractor: STRUCTURAL_EXTRACTOR_VERSION.into(),
            path: path.into(),
            span,
            resolution,
        },
    });
}
fn classify_noncode(path: &str) -> NodeKind {
    let name = path.rsplit('/').next().unwrap_or(path);
    if git_configuration_root(path).is_some() {
        NodeKind::Configuration
    } else if matches!(
        name,
        "package.json"
            | "Cargo.toml"
            | "go.mod"
            | "pyproject.toml"
            | "setup.py"
            | "requirements.txt"
            | "Pipfile"
    ) {
        NodeKind::Package
    } else if matches!(
        name,
        "Cargo.lock"
            | "go.sum"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "uv.lock"
            | "poetry.lock"
            | "Pipfile.lock"
    ) {
        NodeKind::Lockfile
    } else if [".json", ".toml", ".yaml", ".yml", ".ini", ".cfg", ".config"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        || matches!(
            name,
            "Makefile"
                | "Dockerfile"
                | "Justfile"
                | ".gitignore"
                | ".gitattributes"
                | ".gitmodules"
                | "build.rs"
        )
    {
        NodeKind::Configuration
    } else {
        NodeKind::File
    }
}
fn git_configuration_root(path: &str) -> Option<&str> {
    for name in [
        ".git/info/attributes",
        ".git/info/exclude",
        ".git/configured-attributes",
    ] {
        if path == name {
            return Some("");
        }
        if let Some(prefix) = path
            .strip_suffix(name)
            .and_then(|prefix| prefix.strip_suffix('/'))
        {
            return Some(prefix);
        }
    }
    None
}
fn is_documentation(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("md" | "rst" | "txt" | "adoc" | "license")
    ) || path
        .rsplit('/')
        .next()
        .is_some_and(|p| matches!(p, "LICENSE" | "NOTICE"))
}
fn module_name(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .rsplit_once('.')
        .map_or(path, |v| v.0)
        .into()
}
fn text<'a>(node: Node<'_>, bytes: &'a [u8]) -> &'a str {
    std::str::from_utf8(&bytes[node.byte_range()]).unwrap_or("")
}
fn declaration_kind(kind: &str, language: Language) -> Option<NodeKind> {
    Some(match kind {
        "function_declaration"
        | "function_definition"
        | "function_item"
        | "function_signature_item"
        | "generator_function_declaration" => NodeKind::Function,
        "method_declaration" | "method_definition" | "method_signature" => NodeKind::Method,
        "class_declaration" | "class_definition" => NodeKind::Class,
        "interface_declaration" | "interface_type" => NodeKind::Interface,
        "struct_item" | "struct_type" => NodeKind::Struct,
        "trait_item" => NodeKind::Trait,
        "enum_item" | "enum_declaration" => NodeKind::Enum,
        "type_alias_declaration" | "type_item" | "type_spec" => NodeKind::Type,
        "const_item" | "const_spec" | "const_declaration" => NodeKind::Constant,
        "static_item"
        | "variable_declarator"
        | "var_spec"
        | "let_declaration"
        | "short_var_declaration" => NodeKind::Variable,
        "assignment" if language == Language::Python => NodeKind::Variable,
        "mod_item" => NodeKind::Module,
        "required_parameter" | "optional_parameter" | "parameter" | "parameter_declaration" => {
            NodeKind::Parameter
        }
        _ => return None,
    })
}
fn declaration_name<'a>(node: Node<'a>, language: Language) -> Option<Node<'a>> {
    for field in ["name", "pattern", "left"] {
        if let Some(n) = node.child_by_field_name(field) {
            if matches!(
                n.kind(),
                "identifier"
                    | "type_identifier"
                    | "field_identifier"
                    | "shorthand_property_identifier_pattern"
            ) {
                return Some(n);
            }
            let mut cursor = n.walk();
            if let Some(n) = n
                .named_children(&mut cursor)
                .find(|n| matches!(n.kind(), "identifier" | "type_identifier"))
            {
                return Some(n);
            }
        }
    }
    if language == Language::Python && matches!(node.kind(), "parameter" | "identifier") {
        return Some(node);
    }
    None
}
fn semantic_tokens(node: Node<'_>, bytes: &[u8]) -> String {
    fn push(node: Node<'_>, bytes: &[u8], c: &mut Canonical) {
        if node.kind().contains("comment") {
            return;
        }
        if node.child_count() == 0 {
            c.str(node.kind());
            c.str(text(node, bytes));
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                push(child, bytes, c);
            }
        }
    }
    let mut c = Canonical::default();
    push(node, bytes, &mut c);
    c.finish("celetixo/declaration-tokens/v1").to_string()
}
fn parent_scope_key(index: &SourceIndex, parent: &ElementId) -> String {
    let mut ancestors = Vec::new();
    let mut current = Some(parent);
    while let Some(id) = current {
        let Some(node) = index.nodes.get(id) else {
            break;
        };
        if node.kind != NodeKind::File && node.kind != NodeKind::Module {
            ancestors.push((&node.kind, &node.name));
        }
        current = node.parent.as_ref();
    }
    let mut c = Canonical::default();
    c.usize(ancestors.len());
    for (kind, name) in ancestors.into_iter().rev() {
        c.value(kind);
        c.str(name);
    }
    c.finish("celetixo/semantic-scope/v1").to_string()
}
#[allow(clippy::too_many_arguments)]
fn extract_declarations(
    node: Node<'_>,
    bytes: &[u8],
    path: &str,
    language: Language,
    parent: &ElementId,
    snapshot: &Snapshot,
    index: &mut SourceIndex,
    count: &mut usize,
    budget: usize,
) -> Result<(), IndexError> {
    *count += 1;
    if *count > budget {
        return Err(IndexError::ResourceLimit(path.into()));
    }
    let span = SourceSpan::of(node);
    if node.is_error() {
        issue(
            index,
            path,
            span,
            IssueKind::SyntaxError,
            "Tree-sitter ERROR node",
        );
    }
    if node.is_missing() {
        issue(
            index,
            path,
            span,
            IssueKind::MissingSyntax,
            "Tree-sitter missing syntax node",
        );
    }
    if matches!(
        node.kind(),
        "macro_invocation" | "macro_definition" | "eval" | "decorator"
    ) {
        issue(
            index,
            path,
            span,
            IssueKind::UnsupportedConstruct,
            "macro/decorator expansion requires semantic analysis",
        );
    }
    let mut next_parent = parent.clone();
    let declaration = if language == Language::Python
        && node.kind() == "identifier"
        && node
            .parent()
            .is_some_and(|p| matches!(p.kind(), "parameters" | "lambda_parameters"))
    {
        Some(NodeKind::Parameter)
    } else {
        declaration_kind(node.kind(), language)
    };
    if node.kind() == "impl_item"
        && let Some(ty) = node.child_by_field_name("type")
    {
        let name = format!("impl {}", text(ty, bytes));
        let signature = Digest::of(
            "celetixo/impl-shape/v1",
            semantic_tokens(node, bytes).as_bytes(),
        );
        let id = make_id(
            snapshot,
            path,
            "implementation",
            &format!("{name}:{signature}"),
        );
        index.nodes.insert(
            id.clone(),
            IndexNode {
                id: id.clone(),
                path: path.into(),
                name,
                kind: NodeKind::Scope,
                language: Some(language),
                span,
                name_span: None,
                parent: Some(parent.clone()),
                signature,
            },
        );
        add_edge(
            index,
            id.clone(),
            parent.clone(),
            EdgeKind::Contains,
            path,
            span,
            Resolution::StructuralLexical,
        );
        next_parent = id;
    }
    if let Some(mut kind) = declaration {
        if node.kind() == "type_spec"
            && let Some(ty) = node.child_by_field_name("type")
        {
            kind = match ty.kind() {
                "struct_type" => NodeKind::Struct,
                "interface_type" => NodeKind::Interface,
                _ => kind,
            };
        }
        if let Some(name) = declaration_name(node, language) {
            let signature = Digest::of(
                "celetixo/signature/v1",
                semantic_tokens(node, bytes).as_bytes(),
            );
            let mut identity_encoding = Canonical::default();
            identity_encoding.str(&parent_scope_key(index, parent));
            identity_encoding.value(&kind);
            identity_encoding.str(text(name, bytes));
            identity_encoding.value(&signature);
            let identity = identity_encoding
                .finish("celetixo/declaration-identity/v1")
                .to_string();
            let mut id = make_id(snapshot, path, "symbol", &identity);
            if index.nodes.contains_key(&id) {
                issue(
                    index,
                    path,
                    span,
                    IssueKind::AmbiguousReference,
                    "identical declaration shape in the same scope; identity is disambiguated and cannot establish continuity",
                );
                let ordinal = index
                    .nodes
                    .values()
                    .filter(|n| {
                        n.path == path
                            && n.name == text(name, bytes)
                            && n.kind == kind
                            && n.signature == signature
                    })
                    .count();
                id = make_id(
                    snapshot,
                    path,
                    "symbol",
                    &format!("{identity}:duplicate:{ordinal}"),
                );
            }
            index.nodes.insert(
                id.clone(),
                IndexNode {
                    id: id.clone(),
                    path: path.into(),
                    name: text(name, bytes).into(),
                    kind,
                    language: Some(language),
                    span,
                    name_span: Some(SourceSpan::of(name)),
                    parent: Some(parent.clone()),
                    signature,
                },
            );
            add_edge(
                index,
                id.clone(),
                parent.clone(),
                EdgeKind::Contains,
                path,
                span,
                Resolution::StructuralLexical,
            );
            if matches!(
                kind,
                NodeKind::Function
                    | NodeKind::Method
                    | NodeKind::Class
                    | NodeKind::Interface
                    | NodeKind::Struct
                    | NodeKind::Trait
                    | NodeKind::Enum
                    | NodeKind::Module
            ) {
                next_parent = id;
            }
        }
    }
    // Anonymous lexical scopes prevent same-name bindings in sibling blocks being conflated.
    if matches!(node.kind(), "block" | "statement_block") {
        let identity = format!(
            "{}:{}",
            parent_scope_key(index, &next_parent),
            semantic_tokens(node, bytes)
        );
        let mut id = make_id(snapshot, path, "scope", &identity);
        let mut ordinal = 0;
        while index.nodes.contains_key(&id) {
            ordinal += 1;
            id = make_id(snapshot, path, "scope", &format!("{identity}:{ordinal}"));
        }
        index.nodes.insert(
            id.clone(),
            IndexNode {
                id: id.clone(),
                path: path.into(),
                name: format!("scope:{ordinal}"),
                kind: NodeKind::Scope,
                language: Some(language),
                span,
                name_span: None,
                parent: Some(next_parent.clone()),
                signature: Digest::of("celetixo/scope-shape/v1", identity.as_bytes()),
            },
        );
        add_edge(
            index,
            id.clone(),
            next_parent,
            EdgeKind::Contains,
            path,
            span,
            Resolution::StructuralLexical,
        );
        next_parent = id;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_declarations(
            child,
            bytes,
            path,
            language,
            &next_parent,
            snapshot,
            index,
            count,
            budget,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PendingImport {
    path: String,
    span: SourceSpan,
    source: String,
    bindings: Vec<(String, String)>,
    language: Language,
    container: ElementId,
}
#[derive(Debug, Clone)]
struct PendingReference {
    path: String,
    span: SourceSpan,
    name: String,
    kind: OccurrenceKind,
    container: ElementId,
}
#[derive(Debug, Clone)]
enum Pending {
    Import(PendingImport),
    Reference(PendingReference),
}
fn container_at(
    index: &SourceIndex,
    path: &str,
    span: SourceSpan,
    module: &ElementId,
) -> ElementId {
    index
        .nodes
        .values()
        .filter(|n| {
            n.path == path
                && n.span.contains(&span)
                && matches!(
                    n.kind,
                    NodeKind::Function
                        | NodeKind::Method
                        | NodeKind::Class
                        | NodeKind::Interface
                        | NodeKind::Struct
                        | NodeKind::Trait
                        | NodeKind::Enum
                )
        })
        .min_by_key(|n| n.span.end_byte - n.span.start_byte)
        .map(|n| n.id.clone())
        .unwrap_or_else(|| module.clone())
}
fn import_scope_at(
    index: &SourceIndex,
    path: &str,
    span: SourceSpan,
    language: Language,
    module: &ElementId,
) -> ElementId {
    index
        .nodes
        .values()
        .filter(|node| {
            node.path == path
                && node.span.contains(&span)
                && (matches!(
                    node.kind,
                    NodeKind::Function | NodeKind::Method | NodeKind::Class | NodeKind::Module
                ) || node.kind == NodeKind::Scope && language != Language::Python)
        })
        .min_by_key(|node| {
            (
                node.span.end_byte - node.span.start_byte,
                node.kind == NodeKind::Module,
            )
        })
        .map(|node| node.id.clone())
        .unwrap_or_else(|| module.clone())
}
fn unquote(value: &str) -> String {
    value.trim_matches(['\'', '"', '`']).into()
}
fn descendant_nodes<'a>(node: Node<'a>, kind: &str) -> Vec<Node<'a>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == kind {
            out.push(n);
        }
        let mut c = n.walk();
        stack.extend(n.named_children(&mut c));
    }
    out.sort_by_key(Node::start_byte);
    out
}
fn import_info(
    node: Node<'_>,
    bytes: &[u8],
    language: Language,
) -> Vec<(String, Vec<(String, String)>)> {
    match language {
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            let Some(source) = node.child_by_field_name("source") else {
                return Vec::new();
            };
            let mut bindings = Vec::new();
            for spec in descendant_nodes(node, "import_specifier") {
                if let Some(name) = spec.child_by_field_name("name") {
                    let alias = spec.child_by_field_name("alias").unwrap_or(name);
                    bindings.push((text(alias, bytes).into(), text(name, bytes).into()));
                }
            }
            for spec in descendant_nodes(node, "namespace_import") {
                if let Some(name) = descendant_nodes(spec, "identifier").first() {
                    bindings.push((text(*name, bytes).into(), "*".into()));
                }
            }
            for clause in descendant_nodes(node, "import_clause") {
                let mut c = clause.walk();
                for n in clause.named_children(&mut c) {
                    if n.kind() == "identifier" {
                        bindings.push((text(n, bytes).into(), "default".into()));
                    }
                }
            }
            vec![(unquote(text(source, bytes)), bindings)]
        }
        Language::Python => {
            if node.kind() == "import_from_statement" {
                let Some(module) = node.child_by_field_name("module_name") else {
                    return Vec::new();
                };
                let mut bindings = Vec::new();
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    if child.id() == module.id() {
                        continue;
                    }
                    match child.kind() {
                        "dotted_name" | "identifier" => {
                            let v = text(child, bytes);
                            bindings.push((v.into(), v.into()));
                        }
                        "aliased_import" => {
                            if let Some(name) = child.child_by_field_name("name") {
                                let alias = child.child_by_field_name("alias").unwrap_or(name);
                                bindings
                                    .push((text(alias, bytes).into(), text(name, bytes).into()));
                            }
                        }
                        "wildcard_import" => bindings.push(("*".into(), "*".into())),
                        _ => {}
                    }
                }
                vec![(text(module, bytes).into(), bindings)]
            } else {
                let mut imports = Vec::new();
                let mut c = node.walk();
                for child in node.named_children(&mut c) {
                    if matches!(child.kind(), "dotted_name" | "identifier") {
                        let value = text(child, bytes);
                        imports.push((
                            value.into(),
                            vec![(value.split('.').next().unwrap_or(value).into(), "*".into())],
                        ));
                    } else if child.kind() == "aliased_import"
                        && let Some(name) = child.child_by_field_name("name")
                    {
                        let alias = child.child_by_field_name("alias").unwrap_or(name);
                        imports.push((
                            text(name, bytes).into(),
                            vec![(text(alias, bytes).into(), "*".into())],
                        ));
                    }
                }
                imports
            }
        }
        Language::Go => {
            let mut out = Vec::new();
            for spec in descendant_nodes(node, "import_spec") {
                if let Some(path) = spec.child_by_field_name("path") {
                    let source = unquote(text(path, bytes));
                    let alias = spec
                        .child_by_field_name("name")
                        .map(|n| text(n, bytes).to_owned())
                        .unwrap_or_else(|| source.rsplit('/').next().unwrap_or(&source).into());
                    out.push((source, vec![(alias, "*".into())]));
                }
            }
            out
        }
        Language::Rust => {
            let Some(arg) = node.child_by_field_name("argument") else {
                return Vec::new();
            };
            let raw = text(arg, bytes);
            if raw.contains(['{', '*']) {
                return vec![(raw.into(), Vec::new())];
            }
            let (source, alias) = raw
                .split_once(" as ")
                .map_or((raw, None), |(s, a)| (s, Some(a)));
            let imported = source.rsplit("::").next().unwrap_or(source);
            vec![(
                source.into(),
                vec![(alias.unwrap_or(imported).into(), imported.into())],
            )]
        }
    }
}
fn is_import_node(kind: &str) -> bool {
    matches!(
        kind,
        "import_statement"
            | "import_from_statement"
            | "import_declaration"
            | "use_declaration"
            | "export_statement"
    )
}
fn call_target(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(node.kind(), "call_expression" | "call" | "new_expression") {
        node.child_by_field_name("function")
            .or_else(|| node.child_by_field_name("constructor"))
    } else {
        None
    }
}
fn qualified_receiver(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "member_expression" | "attribute" => node.child_by_field_name("object"),
        "selector_expression" => node.child_by_field_name("operand"),
        "field_expression" => node.child_by_field_name("value"),
        _ => None,
    }
}
fn is_declaration_identifier(node: Node<'_>, language: Language) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if declaration_kind(parent.kind(), language).is_some()
        && declaration_name(parent, language).is_some_and(|n| n.id() == node.id())
    {
        return true;
    }
    if matches!(parent.kind(), "parameters" | "lambda_parameters") && language == Language::Python {
        return true;
    }
    matches!(
        parent.kind(),
        "package_clause"
            | "label_name"
            | "break_statement"
            | "continue_statement"
            | "field_declaration"
            | "enum_variant"
    )
}
fn extract_occurrences(
    node: Node<'_>,
    bytes: &[u8],
    path: &str,
    language: Language,
    module: &ElementId,
    pending: &mut Vec<Pending>,
    index: &mut SourceIndex,
) {
    if node.kind() == "mod_item"
        && node.child_by_field_name("body").is_none()
        && let Some(name) = node.child_by_field_name("name")
    {
        pending.push(Pending::Import(PendingImport {
            path: path.into(),
            span: SourceSpan::of(node),
            source: format!("self::{}", text(name, bytes)),
            bindings: vec![(text(name, bytes).into(), "*".into())],
            language,
            container: module.clone(),
        }));
    }
    if node.kind() == "impl_item"
        && let Some(trait_node) = node.child_by_field_name("trait")
    {
        let span = SourceSpan::of(trait_node);
        let implementation = index
            .nodes
            .values()
            .find(|n| n.path == path && n.kind == NodeKind::Scope && n.span == SourceSpan::of(node))
            .map(|n| n.id.clone())
            .unwrap_or_else(|| module.clone());
        pending.push(Pending::Reference(PendingReference {
            path: path.into(),
            span,
            name: text(trait_node, bytes).into(),
            kind: OccurrenceKind::Implements,
            container: implementation.clone(),
        }));
        if let Some(ty) = node.child_by_field_name("type") {
            pending.push(Pending::Reference(PendingReference {
                path: path.into(),
                span: SourceSpan::of(ty),
                name: text(ty, bytes).into(),
                kind: OccurrenceKind::Type,
                container: implementation,
            }));
        }
    }
    if is_import_node(node.kind()) {
        let imports = import_info(node, bytes, language);
        if imports.is_empty()
            && (node.kind() == "use_declaration"
                || node.kind() == "export_statement"
                    && node.child_by_field_name("source").is_some())
        {
            issue(
                index,
                path,
                SourceSpan::of(node),
                IssueKind::UnsupportedConstruct,
                "re-export/glob/import form needs semantic resolver",
            );
        }
        for (source, bindings) in imports {
            pending.push(Pending::Import(PendingImport {
                path: path.into(),
                span: SourceSpan::of(node),
                source,
                bindings,
                language,
                container: import_scope_at(index, path, SourceSpan::of(node), language, module),
            }));
        }
        // Exported declarations still require indexing their expressions and references.
        if node.kind() != "export_statement" {
            return;
        }
    }
    if let Some(target) = call_target(node) {
        let span = SourceSpan::of(target);
        let name = text(target, bytes).to_owned();
        pending.push(Pending::Reference(PendingReference {
            path: path.into(),
            span,
            name,
            kind: OccurrenceKind::Call,
            container: container_at(index, path, span, module),
        }));
        // Dispatch may be unknown while evaluating its receiver still calls a
        // statically identifiable function: factory().method() depends on
        // factory even when the method's implementation is unavailable.
        if let Some(receiver) = qualified_receiver(target) {
            extract_occurrences(receiver, bytes, path, language, module, pending, index);
        } else if target.named_child_count() > 0 {
            extract_occurrences(target, bytes, path, language, module, pending, index);
        }
        let mut c = node.walk();
        for child in node.named_children(&mut c) {
            if child.id() != target.id() {
                extract_occurrences(child, bytes, path, language, module, pending, index);
            }
        }
        return;
    }
    let is_qualified = matches!(
        node.kind(),
        "member_expression"
            | "field_expression"
            | "selector_expression"
            | "scoped_identifier"
            | "scoped_type_identifier"
            | "attribute"
    );
    if is_qualified {
        let span = SourceSpan::of(node);
        pending.push(Pending::Reference(PendingReference {
            path: path.into(),
            span,
            name: text(node, bytes).into(),
            kind: OccurrenceKind::Reference,
            container: container_at(index, path, span, module),
        }));
        if let Some(receiver) = qualified_receiver(node) {
            extract_occurrences(receiver, bytes, path, language, module, pending, index);
        }
        return;
    }
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "shorthand_property_identifier"
    ) && !is_declaration_identifier(node, language)
    {
        let span = SourceSpan::of(node);
        let mut kind = if node.kind() == "type_identifier" {
            OccurrenceKind::Type
        } else {
            OccurrenceKind::Reference
        };
        if node.parent().is_some_and(|p| {
            matches!(
                p.kind(),
                "extends_type_clause" | "implements_clause" | "class_heritage" | "extends_clause"
            )
        }) {
            kind = OccurrenceKind::Implements;
        }
        if node.parent().is_some_and(|p| {
            p.parent().is_some_and(|grand| {
                grand.kind() == "class_definition"
                    && grand
                        .child_by_field_name("superclasses")
                        .is_some_and(|n| n.id() == p.id())
            })
        }) {
            kind = OccurrenceKind::Implements;
        }
        pending.push(Pending::Reference(PendingReference {
            path: path.into(),
            span,
            name: text(node, bytes).into(),
            kind,
            container: container_at(index, path, span, module),
        }));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_occurrences(child, bytes, path, language, module, pending, index);
    }
}
fn occurrence(
    index: &mut SourceIndex,
    p: &PendingReference,
    target: Option<ElementId>,
    resolution: Resolution,
) {
    let mut c = Canonical::default();
    c.str(&index.snapshot.to_string());
    c.str(&p.path);
    c.value(&p.span);
    c.value(&p.kind);
    c.str(&p.name);
    c.str(p.container.as_str());
    c.value(&target);
    index.occurrences.push(Occurrence {
        id: c.finish("celetixo/occurrence/v1"),
        path: p.path.clone(),
        span: p.span,
        container: p.container.clone(),
        name: p.name.clone(),
        kind: p.kind,
        target: target.clone(),
    });
    if let Some(target) = target {
        let kind = match p.kind {
            OccurrenceKind::Reference => EdgeKind::Reference,
            OccurrenceKind::Call => EdgeKind::Call,
            OccurrenceKind::Import => EdgeKind::Import,
            OccurrenceKind::Type => EdgeKind::Type,
            OccurrenceKind::Implements => EdgeKind::Implements,
        };
        add_edge(
            index,
            p.container.clone(),
            target,
            kind,
            &p.path,
            p.span,
            resolution,
        );
    }
}
fn normalize_path(path: &str) -> Option<String> {
    let mut out = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            _ => out.push(part),
        }
    }
    Some(out.join("/"))
}
fn source_candidates(
    import: &PendingImport,
    inputs: &BTreeMap<String, (InputKind, Vec<u8>)>,
) -> Vec<String> {
    let parent = import.path.rsplit_once('/').map_or("", |v| v.0);
    let mut bases = Vec::new();
    match import.language {
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            if import.source.starts_with('.')
                && let Some(path) = normalize_path(&format!("{parent}/{}", import.source))
            {
                bases.push(path);
            }
        }
        Language::Python => {
            let leading = import.source.bytes().take_while(|b| *b == b'.').count();
            let body = import.source[leading..].replace('.', "/");
            if leading > 0 {
                let mut base = parent.to_owned();
                for _ in 1..leading {
                    base = base.rsplit_once('/').map_or("", |v| v.0).into();
                }
                bases.push(format!("{base}/{body}").trim_start_matches('/').into());
            } else {
                bases.push(body.clone());
                bases.push(format!("src/{body}"));
            }
        }
        Language::Go => {
            for (path, (_, bytes)) in inputs {
                if path.rsplit('/').next() == Some("go.mod")
                    && let Ok(source) = std::str::from_utf8(bytes)
                    && let Some(module) =
                        source.lines().find_map(|line| line.strip_prefix("module "))
                {
                    let module = module.trim();
                    if import.source == module || import.source.starts_with(&format!("{module}/")) {
                        let root = path.strip_suffix("go.mod").unwrap_or("");
                        bases.push(format!(
                            "{root}{}",
                            import
                                .source
                                .strip_prefix(module)
                                .unwrap_or("")
                                .trim_start_matches('/')
                        ));
                    }
                }
            }
        }
        Language::Rust => {
            let mut parts = import.source.split("::").collect::<Vec<_>>();
            if parts.first() == Some(&"crate") {
                parts.remove(0);
                bases.push(format!("src/{}", parts.join("/")));
            } else if parts.first() == Some(&"self") {
                parts.remove(0);
                bases.push(format!("{parent}/{}", parts.join("/")));
            } else if parts.first() == Some(&"super") {
                parts.remove(0);
                let up = parent.rsplit_once('/').map_or("", |v| v.0);
                bases.push(format!("{up}/{}", parts.join("/")));
            } else {
                bases.push(format!("{parent}/{}", parts.join("/")));
            }
            let shortened = bases
                .iter()
                .filter_map(|v| v.rsplit_once('/').map(|v| v.0.to_owned()))
                .collect::<Vec<_>>();
            bases.extend(shortened);
        }
    }
    let mut candidates = BTreeSet::new();
    for base in bases {
        if import.language == Language::Go {
            for path in inputs.keys() {
                if path.rsplit_once('/').map_or("", |v| v.0) == base.trim_end_matches('/')
                    && path.ends_with(".go")
                {
                    candidates.insert(path.clone());
                }
            }
            continue;
        }
        for suffix in match import.language {
            Language::TypeScript | Language::Tsx | Language::JavaScript => vec![
                "",
                ".ts",
                ".tsx",
                ".js",
                ".jsx",
                ".mjs",
                ".cjs",
                "/index.ts",
                "/index.tsx",
                "/index.js",
            ],
            Language::Python => vec![".py", ".pyi", "/__init__.py"],
            Language::Rust => vec![".rs", "/mod.rs"],
            Language::Go => unreachable!(),
        } {
            let path = format!("{base}{suffix}");
            if inputs.contains_key(&path) {
                candidates.insert(path);
            }
        }
    }
    candidates.into_iter().collect()
}
#[cfg(test)]
fn top_level_symbol_by_scan(index: &SourceIndex, path: &str, name: &str) -> Vec<ElementId> {
    index
        .nodes
        .values()
        .filter(|n| {
            n.path == path
                && n.name == name
                && !matches!(
                    n.kind,
                    NodeKind::File | NodeKind::Module | NodeKind::Scope | NodeKind::Parameter
                )
                && n.parent
                    .as_ref()
                    .and_then(|p| index.nodes.get(p))
                    .is_some_and(|p| p.kind == NodeKind::Module)
        })
        .map(|n| n.id.clone())
        .collect()
}
fn resolve_imports(
    index: &mut SourceIndex,
    pending: &[Pending],
    inputs: &BTreeMap<String, (InputKind, Vec<u8>)>,
) {
    for import in pending.iter().filter_map(|p| {
        if let Pending::Import(i) = p {
            Some(i)
        } else {
            None
        }
    }) {
        let candidates = source_candidates(import, inputs);
        // Go packages contain several source files; other resolvers require a unique module.
        if candidates.is_empty() || (candidates.len() > 1 && import.language != Language::Go) {
            issue(
                index,
                &import.path,
                import.span,
                if candidates.is_empty() {
                    IssueKind::UnresolvedImport
                } else {
                    IssueKind::AmbiguousReference
                },
                &format!(
                    "import {:?}: {} candidate modules",
                    import.source,
                    candidates.len()
                ),
            );
            let p = PendingReference {
                path: import.path.clone(),
                span: import.span,
                name: import.source.clone(),
                kind: OccurrenceKind::Import,
                container: import.container.clone(),
            };
            occurrence(index, &p, None, Resolution::StructuralModule);
            continue;
        }
        for path in candidates {
            if let Some(file) = index.files.get(&path) {
                let target = file.module.clone();
                let p = PendingReference {
                    path: import.path.clone(),
                    span: import.span,
                    name: import.source.clone(),
                    kind: OccurrenceKind::Import,
                    container: import.container.clone(),
                };
                occurrence(index, &p, Some(target), Resolution::StructuralModule);
            }
        }
    }
}
#[cfg(test)]
fn lexical_candidates_by_scan(index: &SourceIndex, p: &PendingReference) -> Vec<ElementId> {
    let name = p.name.as_str();
    let mut scopes = index
        .nodes
        .values()
        .filter(|n| {
            n.path == p.path
                && n.span.contains(&p.span)
                && matches!(
                    n.kind,
                    NodeKind::Scope
                        | NodeKind::Module
                        | NodeKind::Function
                        | NodeKind::Method
                        | NodeKind::Class
                )
        })
        .collect::<Vec<_>>();
    scopes.sort_by_key(|n| n.span.end_byte - n.span.start_byte);
    for scope in scopes {
        let candidates = index
            .nodes
            .values()
            .filter(|n| {
                n.path == p.path
                    && n.name == name
                    && n.parent.as_ref() == Some(&scope.id)
                    && !matches!(n.kind, NodeKind::Module | NodeKind::File | NodeKind::Scope)
            })
            .map(|n| n.id.clone())
            .collect::<Vec<_>>();
        if !candidates.is_empty() {
            return candidates;
        }
    }
    if index
        .files
        .get(&p.path)
        .is_some_and(|file| file.language == Some(Language::Go))
    {
        let directory = p.path.rsplit_once('/').map_or("", |v| v.0);
        let package = &index.nodes[&index.files[&p.path].module].name;
        let mut candidates = Vec::new();
        for file in index.files.values().filter(|file| {
            file.path != p.path
                && file.language == Some(Language::Go)
                && file.path.rsplit_once('/').map_or("", |v| v.0) == directory
                && &index.nodes[&file.module].name == package
        }) {
            candidates.extend(top_level_symbol_by_scan(index, &file.path, &p.name));
        }
        return candidates;
    }
    Vec::new()
}
#[derive(Default)]
struct FileDeclarations {
    /// Sorted by width, preserving the original node-ID iteration order on
    /// ties. Equal-width scopes must retain the scan resolver's exact order.
    scopes: Vec<(SourceSpan, ElementId)>,
    by_scope: BTreeMap<ElementId, BTreeMap<String, Vec<ElementId>>>,
    top_level: BTreeMap<String, Vec<ElementId>>,
}

/// Ephemeral lookup for one immutable set of declarations. This is rebuilt for
/// every index generation: syntax-tree reuse never reuses resolution decisions.
struct LexicalLookup {
    files: BTreeMap<String, FileDeclarations>,
    go_membership: BTreeMap<String, (String, String)>,
    go_packages: BTreeMap<(String, String), Vec<String>>,
}

impl LexicalLookup {
    fn new(index: &SourceIndex) -> Self {
        let mut files: BTreeMap<String, FileDeclarations> = BTreeMap::new();
        for node in index.nodes.values() {
            let file = files.entry(node.path.clone()).or_default();
            if matches!(
                node.kind,
                NodeKind::Scope
                    | NodeKind::Module
                    | NodeKind::Function
                    | NodeKind::Method
                    | NodeKind::Class
            ) {
                file.scopes.push((node.span, node.id.clone()));
            }
            if !matches!(
                node.kind,
                NodeKind::Module | NodeKind::File | NodeKind::Scope
            ) && let Some(parent) = &node.parent
            {
                file.by_scope
                    .entry(parent.clone())
                    .or_default()
                    .entry(node.name.clone())
                    .or_default()
                    .push(node.id.clone());
                if node.kind != NodeKind::Parameter
                    && index
                        .nodes
                        .get(parent)
                        .is_some_and(|parent| parent.kind == NodeKind::Module)
                {
                    file.top_level
                        .entry(node.name.clone())
                        .or_default()
                        .push(node.id.clone());
                }
            }
        }
        for file in files.values_mut() {
            file.scopes
                .sort_by_key(|(span, _)| span.end_byte - span.start_byte);
        }
        let mut go_membership = BTreeMap::new();
        let mut go_packages: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for file in index
            .files
            .values()
            .filter(|file| file.language == Some(Language::Go))
        {
            let directory = file.path.rsplit_once('/').map_or("", |value| value.0);
            let package = &index.nodes[&file.module].name;
            let key = (directory.to_owned(), package.clone());
            go_packages
                .entry(key.clone())
                .or_default()
                .push(file.path.clone());
            go_membership.insert(file.path.clone(), key);
        }
        Self {
            files,
            go_membership,
            go_packages,
        }
    }

    fn top_level(&self, path: &str, name: &str) -> Vec<ElementId> {
        self.files
            .get(path)
            .and_then(|file| file.top_level.get(name))
            .cloned()
            .unwrap_or_default()
    }

    fn candidates(&self, reference: &PendingReference) -> Vec<ElementId> {
        if let Some(file) = self.files.get(&reference.path) {
            for (span, scope) in &file.scopes {
                if span.contains(&reference.span)
                    && let Some(candidates) = file
                        .by_scope
                        .get(scope)
                        .and_then(|names| names.get(&reference.name))
                {
                    return candidates.clone();
                }
            }
        }
        if let Some(package) = self.go_membership.get(&reference.path) {
            let mut candidates = Vec::new();
            for path in &self.go_packages[package] {
                if path != &reference.path {
                    candidates.extend(self.top_level(path, &reference.name));
                }
            }
            return candidates;
        }
        Vec::new()
    }
}

fn resolve_references(index: &mut SourceIndex, pending: &[Pending]) {
    let lookup = LexicalLookup::new(index);
    resolve_references_using(
        index,
        pending,
        |_, reference| lookup.candidates(reference),
        |_, path, name| lookup.top_level(path, name),
    );
}

#[cfg(test)]
fn resolve_references_by_scan(index: &mut SourceIndex, pending: &[Pending]) {
    resolve_references_using(
        index,
        pending,
        lexical_candidates_by_scan,
        top_level_symbol_by_scan,
    );
}

fn resolve_references_using(
    index: &mut SourceIndex,
    pending: &[Pending],
    lexical_candidates: impl Fn(&SourceIndex, &PendingReference) -> Vec<ElementId>,
    top_level_candidates: impl Fn(&SourceIndex, &str, &str) -> Vec<ElementId>,
) {
    // Import facts are intentionally recomputed from resolved module occurrences, so reuse
    // of a syntax tree never reuses a dependency decision from an earlier snapshot.
    for reference in pending.iter().filter_map(|p| {
        if let Pending::Reference(r) = p {
            Some(r)
        } else {
            None
        }
    }) {
        let mut candidates = lexical_candidates(index, reference);
        let name = reference.name.replace("::", ".");
        let (alias, member) = name
            .split_once('.')
            .map_or((name.as_str(), None), |(a, b)| (a, Some(b)));
        let alias_candidates = if alias == reference.name {
            candidates.clone()
        } else {
            let alias_reference = PendingReference {
                name: alias.into(),
                ..reference.clone()
            };
            lexical_candidates(index, &alias_reference)
        };
        let rank = |id: &ElementId| {
            let node = &index.nodes[id];
            (
                node.span.end_byte - node.span.start_byte,
                node.kind == NodeKind::Module,
            )
        };
        let lexical_rank = alias_candidates
            .iter()
            .filter_map(|id| index.nodes[id].parent.as_ref())
            .map(rank)
            .min();
        let visible_imports = pending
            .iter()
            .filter_map(|pending| {
                if let Pending::Import(import) = pending {
                    Some(import)
                } else {
                    None
                }
            })
            .filter(|import| {
                import.path == reference.path
                    && import.bindings.iter().any(|(local, _)| local == alias)
                    && index
                        .nodes
                        .get(&import.container)
                        .is_some_and(|scope| scope.span.contains(&reference.span))
            })
            .collect::<Vec<_>>();
        let import_rank = visible_imports
            .iter()
            .map(|import| rank(&import.container))
            .min();
        // A local binding can shadow a module alias, and an inner import can
        // shadow an outer declaration/import. Do not resolve through invisible
        // sibling imports or through a namespace hidden by a local parameter.
        if let Some(import_rank) = import_rank
            && lexical_rank.is_none_or(|lexical_rank| import_rank <= lexical_rank)
        {
            if lexical_rank.is_some_and(|lexical_rank| import_rank < lexical_rank) {
                candidates.clear();
            }
            for import in visible_imports
                .into_iter()
                .filter(|import| rank(&import.container) == import_rank)
            {
                for (local, remote) in &import.bindings {
                    if local != alias {
                        continue;
                    }
                    let modules = index
                        .occurrences
                        .iter()
                        .filter(|occurrence| {
                            occurrence.path == import.path
                                && occurrence.span == import.span
                                && occurrence.kind == OccurrenceKind::Import
                        })
                        .filter_map(|occurrence| occurrence.target.as_ref())
                        .filter_map(|id| index.nodes.get(id))
                        .map(|node| node.path.clone())
                        .collect::<Vec<_>>();
                    for path in modules {
                        let target_name = member.unwrap_or(remote);
                        if target_name == "*" {
                            if let Some(file) = index.files.get(&path) {
                                candidates.push(file.module.clone());
                            }
                        } else {
                            candidates.extend(top_level_candidates(index, &path, target_name));
                        }
                    }
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        let target = if candidates.len() == 1 {
            Some(candidates[0].clone())
        } else {
            None
        };
        if target.is_none() {
            issue(
                index,
                &reference.path,
                reference.span,
                if candidates.is_empty() {
                    IssueKind::UnresolvedReference
                } else {
                    IssueKind::AmbiguousReference
                },
                &format!(
                    "{:?}: {:?} ({} candidates)",
                    reference.kind,
                    reference.name,
                    candidates.len()
                ),
            );
        }
        occurrence(index, reference, target, Resolution::StructuralLexical);
    }
}

fn configuration_edges(
    index: &mut SourceIndex,
    inputs: &BTreeMap<String, (InputKind, Vec<u8>)>,
    config: &IndexConfig,
) {
    let files = index.files.values().cloned().collect::<Vec<_>>();
    for cfg in &files {
        let kind = index.nodes[&cfg.file].kind;
        if !matches!(
            kind,
            NodeKind::Configuration | NodeKind::Package | NodeKind::Lockfile
        ) {
            continue;
        }
        let directory = git_configuration_root(&cfg.path)
            .unwrap_or_else(|| cfg.path.rsplit_once('/').map_or("", |v| v.0));
        for consumer in &files {
            if cfg.path == consumer.path {
                continue;
            }
            if directory.is_empty()
                || directory == ".github"
                || directory.starts_with(".github/")
                || consumer.path.starts_with(&format!("{directory}/"))
            {
                add_edge(
                    index,
                    consumer.module.clone(),
                    cfg.file.clone(),
                    if kind == NodeKind::Package || kind == NodeKind::Lockfile {
                        EdgeKind::Package
                    } else {
                        EdgeKind::Build
                    },
                    &cfg.path,
                    SourceSpan::default(),
                    Resolution::DeclaredConfiguration,
                );
            }
        }
        if cfg.path.ends_with("package.json") {
            match serde_json::from_slice::<serde_json::Value>(&inputs[&cfg.path].1) {
                Ok(value) => {
                    for field in [
                        "dependencies",
                        "devDependencies",
                        "peerDependencies",
                        "optionalDependencies",
                    ] {
                        if let Some(deps) = value.get(field).and_then(|v| v.as_object()) {
                            if !deps.is_empty() {
                                issue(
                                    index,
                                    &cfg.path,
                                    SourceSpan::default(),
                                    IssueKind::ConfigurationUnresolved,
                                    "package version specifications are recorded; installed dependency content and external exports require semantic resolution",
                                );
                            }
                            for (name, version) in deps {
                                let mut canonical = Canonical::default();
                                canonical.str(index.scope.organization.as_str());
                                canonical.str(index.scope.workspace.as_str());
                                canonical.str(index.scope.repository.as_str());
                                canonical.str(&cfg.path);
                                canonical.str(field);
                                canonical.str(name);
                                canonical.str(&version.to_string());
                                let signature = canonical.finish("celetixo/package-dependency/v1");
                                let id = package_id(index.identity_namespace, signature);
                                index.nodes.insert(
                                    id.clone(),
                                    IndexNode {
                                        id: id.clone(),
                                        path: cfg.path.clone(),
                                        name: name.clone(),
                                        kind: NodeKind::Package,
                                        language: None,
                                        span: SourceSpan::default(),
                                        name_span: None,
                                        parent: Some(cfg.file.clone()),
                                        signature,
                                    },
                                );
                                add_edge(
                                    index,
                                    cfg.file.clone(),
                                    id,
                                    EdgeKind::Package,
                                    &cfg.path,
                                    SourceSpan::default(),
                                    Resolution::DeclaredConfiguration,
                                );
                            }
                        }
                    }
                    if value.get("scripts").is_some() {
                        issue(
                            index,
                            &cfg.path,
                            SourceSpan::default(),
                            IssueKind::ConfigurationUnresolved,
                            "package scripts are captured inputs; executable script dependencies need a configured build analyzer",
                        );
                    }
                }
                Err(_) => issue(
                    index,
                    &cfg.path,
                    SourceSpan::default(),
                    IssueKind::ConfigurationUnresolved,
                    "invalid JSON package manifest (source content withheld)",
                ),
            }
        } else {
            if cfg.path.ends_with("Cargo.toml")
                || cfg.path.ends_with("pyproject.toml")
                || cfg.path.ends_with("Pipfile")
            {
                toml_dependencies(index, cfg, &inputs[&cfg.path].1);
            }
            if cfg.path.ends_with("go.mod") {
                go_dependencies(index, cfg, &inputs[&cfg.path].1);
            }
        }
        issue(
            index,
            &cfg.path,
            SourceSpan::default(),
            IssueKind::ConfigurationUnresolved,
            "conservative configuration/package scope is recorded; full build/dependency evaluation requires a configured analyzer",
        );
    }
    for (output, sources) in &config.generated_inputs {
        let Some(generated) = index.files.get(output).cloned() else {
            issue(
                index,
                output,
                SourceSpan::default(),
                IssueKind::GeneratedInputMissing,
                "declared generated output is absent",
            );
            continue;
        };
        for source in sources {
            if let Some(input) = index.files.get(source) {
                add_edge(
                    index,
                    generated.file.clone(),
                    input.file.clone(),
                    EdgeKind::Generated,
                    output,
                    SourceSpan::default(),
                    Resolution::DeclaredConfiguration,
                );
            } else {
                issue(
                    index,
                    source,
                    SourceSpan::default(),
                    IssueKind::GeneratedInputMissing,
                    "declared generator input is absent",
                );
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContinuityKind {
    Unchanged,
    ExactContentMove,
    Copy,
    Ambiguous,
    Deleted,
    Created,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Continuity {
    pub old_paths: Vec<String>,
    pub new_paths: Vec<String>,
    pub kind: ContinuityKind,
    pub evidence: Digest,
    pub analyzer: String,
    /// Navigation evidence only; even unique content correspondence is not OCC authority.
    pub authorizes_identity: bool,
}
/// Compare immutable file content and namespace. Copies never collapse two entities.
pub fn continuity(old: &SourceIndex, new: &SourceIndex) -> Result<Vec<Continuity>, IndexError> {
    old.validate_generation()?;
    new.validate_generation()?;
    if old.scope != new.scope {
        return Err(IndexError::SnapshotMismatch);
    }
    let mut digests = BTreeSet::new();
    digests.extend(old.files.values().map(|f| f.content_digest));
    digests.extend(new.files.values().map(|f| f.content_digest));
    let mut output = Vec::new();
    for digest in digests {
        let old_paths = old
            .files
            .values()
            .filter(|f| f.content_digest == digest)
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        let new_paths = new
            .files
            .values()
            .filter(|f| f.content_digest == digest)
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        let kind = if old_paths == new_paths {
            ContinuityKind::Unchanged
        } else if old_paths.is_empty() {
            ContinuityKind::Created
        } else if new_paths.is_empty() {
            ContinuityKind::Deleted
        } else if old_paths.len() == 1 && new_paths.len() == 1 {
            ContinuityKind::ExactContentMove
        } else if old_paths.iter().all(|p| new_paths.contains(p)) {
            ContinuityKind::Copy
        } else {
            ContinuityKind::Ambiguous
        };
        let mut c = Canonical::default();
        c.str(&old.snapshot.to_string());
        c.str(&new.snapshot.to_string());
        c.str(&digest.to_string());
        for p in &old_paths {
            c.str(p);
        }
        c.str("new-paths");
        for p in &new_paths {
            c.str(p);
        }
        output.push(Continuity {
            old_paths,
            new_paths,
            kind,
            evidence: c.finish("celetixo/continuity/v1"),
            analyzer: "celetixo-exact-content-continuity/v1".into(),
            authorizes_identity: false,
        });
    }
    output.sort();
    Ok(output)
}

fn occurrence_id(snapshot: SnapshotId, occurrence: &Occurrence) -> Digest {
    let mut canonical = Canonical::default();
    canonical.str(&snapshot.to_string());
    canonical.str(&occurrence.path);
    canonical.value(&occurrence.span);
    canonical.value(&occurrence.kind);
    canonical.str(&occurrence.name);
    canonical.str(occurrence.container.as_str());
    canonical.value(&occurrence.target);
    canonical.finish("celetixo/occurrence/v1")
}

// Explicit canonical wire encoding, independent of Debug, machine word width and HashMap order.
trait Encode {
    fn encode(&self, c: &mut Canonical);
}
impl Encode for String {
    fn encode(&self, c: &mut Canonical) {
        c.str(self);
    }
}
impl Encode for Digest {
    fn encode(&self, c: &mut Canonical) {
        c.0.extend_from_slice(self.as_bytes());
    }
}
impl Encode for ElementId {
    fn encode(&self, c: &mut Canonical) {
        c.str(self.as_str());
    }
}
impl Encode for Scope {
    fn encode(&self, c: &mut Canonical) {
        c.str(self.organization.as_str());
        c.str(self.workspace.as_str());
        c.str(self.repository.as_str());
    }
}
impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, c: &mut Canonical) {
        c.usize(self.len());
        for value in self {
            value.encode(c);
        }
    }
}
impl<T: Encode + Ord> Encode for BTreeSet<T> {
    fn encode(&self, c: &mut Canonical) {
        c.usize(self.len());
        for value in self {
            value.encode(c);
        }
    }
}
impl<K: Encode + Ord, V: Encode> Encode for BTreeMap<K, V> {
    fn encode(&self, c: &mut Canonical) {
        c.usize(self.len());
        for (k, v) in self {
            k.encode(c);
            v.encode(c);
        }
    }
}
impl<T: Encode> Encode for Option<T> {
    fn encode(&self, c: &mut Canonical) {
        match self {
            Some(value) => {
                c.str("some");
                value.encode(c);
            }
            None => c.str("none"),
        }
    }
}
macro_rules! enum_encoding {($name:ident: $($variant:ident),+ $(,)?)=>{impl Encode for $name{fn encode(&self,c:&mut Canonical){c.str(match self{$(Self::$variant=>stringify!($variant)),+});}}};}
enum_encoding!(Language:TypeScript,Tsx,JavaScript,Python,Go,Rust);
enum_encoding!(NodeKind:File,Module,Scope,Function,Method,Class,Interface,Type,Struct,Trait,Enum,Variable,Constant,Parameter,Package,Configuration,Lockfile,Generated);
enum_encoding!(EdgeKind:Contains,Reference,Call,Import,Type,Implements,Package,Build,Generated);
enum_encoding!(Resolution:StructuralLexical,StructuralModule,DeclaredConfiguration,Analyzer);
enum_encoding!(OccurrenceKind:Reference,Call,Import,Type,Implements);
enum_encoding!(IssueKind:SyntaxError,MissingSyntax,UnsupportedLanguage,UnsupportedConstruct,InvalidEncoding,NonRegularInput,UnresolvedReference,AmbiguousReference,UnresolvedImport,SemanticAnalysisUnavailable,ConfigurationUnresolved,GeneratedInputMissing);
impl Encode for SourceSpan {
    fn encode(&self, c: &mut Canonical) {
        for n in [
            self.start_byte,
            self.end_byte,
            self.start_row,
            self.start_column,
            self.end_row,
            self.end_column,
        ] {
            c.usize(n);
        }
    }
}
impl Encode for IndexConfig {
    fn encode(&self, c: &mut Canonical) {
        c.str(&self.analyzer_version);
        c.value(&self.language_configuration);
        c.value(&self.generated_inputs);
        c.usize(self.max_file_bytes);
        c.usize(self.max_syntax_nodes);
    }
}
impl Encode for IndexNode {
    fn encode(&self, c: &mut Canonical) {
        c.value(&self.id);
        c.str(&self.path);
        c.str(&self.name);
        c.value(&self.kind);
        c.value(&self.language);
        c.value(&self.span);
        c.value(&self.name_span);
        c.value(&self.parent);
        c.value(&self.signature);
    }
}
impl Encode for IndexedFile {
    fn encode(&self, c: &mut Canonical) {
        c.str(&self.path);
        c.value(&self.content_digest);
        c.value(&self.language);
        c.value(&self.parser_version);
        c.value(&self.file);
        c.value(&self.module);
        c.usize(self.bytes);
    }
}
impl Encode for IndexEdge {
    fn encode(&self, c: &mut Canonical) {
        c.value(&self.from);
        c.value(&self.to);
        c.value(&self.kind);
        c.value(&self.provenance);
    }
}
impl Encode for EdgeProvenance {
    fn encode(&self, c: &mut Canonical) {
        c.value(&self.snapshot.0);
        c.str(&self.extractor);
        c.str(&self.path);
        c.value(&self.span);
        c.value(&self.resolution);
    }
}
impl Encode for Occurrence {
    fn encode(&self, c: &mut Canonical) {
        c.value(&self.id);
        c.str(&self.path);
        c.value(&self.span);
        c.value(&self.container);
        c.str(&self.name);
        c.value(&self.kind);
        c.value(&self.target);
    }
}
impl Encode for IndexIssue {
    fn encode(&self, c: &mut Canonical) {
        c.str(&self.path);
        c.value(&self.span);
        c.value(&self.kind);
        c.str(&self.detail);
    }
}

fn validate_tree_budget(root: Node<'_>, path: &str, max_nodes: usize) -> Result<(), IndexError> {
    let mut stack = vec![(root, 0usize)];
    let mut count = 0;
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if count > max_nodes || depth > 256 {
            return Err(IndexError::ResourceLimit(format!(
                "{path}: syntax node/depth budget"
            )));
        }
        let mut c = node.walk();
        for child in node.children(&mut c) {
            stack.push((child, depth + 1));
        }
    }
    Ok(())
}

fn package_dependency(
    index: &mut SourceIndex,
    file: &IndexedFile,
    section: &str,
    name: &str,
    specification: &str,
) {
    let mut c = Canonical::default();
    c.value(&index.scope);
    c.str(&file.path);
    c.str(section);
    c.str(name);
    c.str(specification);
    let signature = c.finish("celetixo/package-specification/v1");
    let id = package_id(index.identity_namespace, signature);
    index.nodes.insert(
        id.clone(),
        IndexNode {
            id: id.clone(),
            path: file.path.clone(),
            name: name.into(),
            kind: NodeKind::Package,
            language: None,
            span: SourceSpan::default(),
            name_span: None,
            parent: Some(file.file.clone()),
            signature,
        },
    );
    add_edge(
        index,
        file.file.clone(),
        id,
        EdgeKind::Package,
        &file.path,
        SourceSpan::default(),
        Resolution::DeclaredConfiguration,
    );
}
fn toml_dependencies(index: &mut SourceIndex, file: &IndexedFile, bytes: &[u8]) {
    let value = std::str::from_utf8(bytes)
        .map_err(|e| e.to_string())
        .and_then(|source| toml::from_str::<toml::Value>(source).map_err(|e| e.to_string()));
    let value = match value {
        Ok(value) => value,
        Err(_) => {
            issue(
                index,
                &file.path,
                SourceSpan::default(),
                IssueKind::ConfigurationUnresolved,
                "invalid TOML manifest (source content withheld)",
            );
            return;
        }
    };
    fn walk(value: &toml::Value, prefix: &str, dependencies: &mut Vec<(String, String, String)>) {
        if let Some(table) = value.as_table() {
            for (key, value) in table {
                let section = format!("{prefix}.{key}");
                if matches!(
                    key.as_str(),
                    "dependencies"
                        | "dev-dependencies"
                        | "build-dependencies"
                        | "packages"
                        | "dev-packages"
                ) {
                    if let Some(table) = value.as_table() {
                        for (name, specification) in table {
                            dependencies.push((
                                section.clone(),
                                name.clone(),
                                specification.to_string(),
                            ));
                        }
                    }
                    if let Some(array) = value.as_array() {
                        for specification in array {
                            let spec = specification.as_str().unwrap_or("");
                            if !spec.is_empty() {
                                dependencies.push((section.clone(), spec.into(), spec.into()));
                            }
                        }
                    }
                }
                if key == "optional-dependencies"
                    && let Some(groups) = value.as_table()
                {
                    for (group, values) in groups {
                        if let Some(values) = values.as_array() {
                            for specification in values {
                                if let Some(spec) = specification.as_str() {
                                    dependencies.push((
                                        format!("{section}.{group}"),
                                        spec.into(),
                                        spec.into(),
                                    ));
                                }
                            }
                        }
                    }
                }
                walk(value, &section, dependencies);
            }
        }
    }
    let mut dependencies = Vec::new();
    walk(&value, "", &mut dependencies);
    dependencies.sort();
    dependencies.dedup();
    for (section, name, specification) in dependencies {
        package_dependency(index, file, &section, &name, &specification);
    }
}
fn go_dependencies(index: &mut SourceIndex, file: &IndexedFile, bytes: &[u8]) {
    let Ok(source) = std::str::from_utf8(bytes) else {
        issue(
            index,
            &file.path,
            SourceSpan::default(),
            IssueKind::InvalidEncoding,
            "Go manifest is not UTF-8",
        );
        return;
    };
    let mut require_block = false;
    for raw in source.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line == ")" {
            require_block = false;
            continue;
        }
        if line == "require (" {
            require_block = true;
            continue;
        }
        let requirement = if require_block {
            Some(line)
        } else {
            line.strip_prefix("require ")
        };
        if let Some(requirement) = requirement {
            let words = requirement.split_whitespace().collect::<Vec<_>>();
            if words.len() == 2 && !words[0].contains(['"', '`']) {
                package_dependency(index, file, "require", words[0], words[1]);
            } else {
                issue(
                    index,
                    &file.path,
                    SourceSpan::default(),
                    IssueKind::ConfigurationUnresolved,
                    "unsupported or invalid Go require directive",
                );
            }
        } else if !["module ", "go ", "toolchain "]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            issue(
                index,
                &file.path,
                SourceSpan::default(),
                IssueKind::ConfigurationUnresolved,
                "Go replace/exclude/retract/tool or unsupported directive requires a module analyzer",
            );
        }
    }
    if require_block {
        issue(
            index,
            &file.path,
            SourceSpan::default(),
            IssueKind::ConfigurationUnresolved,
            "unterminated Go require block",
        );
    }
}

fn go_package_edges(index: &mut SourceIndex) {
    let mut packages: BTreeMap<(String, String), Vec<IndexedFile>> = BTreeMap::new();
    for file in index
        .files
        .values()
        .filter(|file| file.language == Some(Language::Go))
    {
        let directory = file.path.rsplit_once('/').map_or("", |v| v.0);
        let name = index.nodes[&file.module].name.clone();
        packages
            .entry((directory.into(), name))
            .or_default()
            .push(file.clone());
    }
    for ((directory, name), files) in packages {
        let mut c = Canonical::default();
        c.value(&index.scope);
        c.str("go");
        c.str(&directory);
        c.str(&name);
        c.str("declared-package-scope/v1");
        let signature = c.finish("celetixo/go-package/v1");
        let id = package_id(index.identity_namespace, signature);
        index.nodes.insert(
            id.clone(),
            IndexNode {
                id: id.clone(),
                path: files[0].path.clone(),
                name,
                kind: NodeKind::Package,
                language: Some(Language::Go),
                span: SourceSpan::default(),
                name_span: None,
                parent: None,
                signature,
            },
        );
        for file in files {
            add_edge(
                index,
                file.module.clone(),
                id.clone(),
                EdgeKind::Package,
                &file.path,
                SourceSpan::default(),
                Resolution::StructuralModule,
            );
            add_edge(
                index,
                id.clone(),
                file.module,
                EdgeKind::Package,
                &file.path,
                SourceSpan::default(),
                Resolution::StructuralModule,
            );
        }
    }
}

#[cfg(test)]
mod lexical_lookup_tests {
    use super::*;
    use celetixo_engine::ids::{OrganizationId, RepositoryId, Revision, WorkspaceId};
    use celetixo_engine::snapshot::{
        FileMode, InclusionPolicy, InputLayer, SnapshotManifest, State, WorkspaceInput,
    };

    fn snapshot(files: Vec<(String, Vec<u8>)>) -> Snapshot {
        Snapshot::new(
            Scope {
                organization: OrganizationId::new("lookup-oracle-org").unwrap(),
                workspace: WorkspaceId::new("lookup-oracle-workspace").unwrap(),
                repository: RepositoryId::new("lookup-oracle-repository").unwrap(),
            },
            Revision(0),
            SnapshotManifest {
                base_commit: "lookup-oracle-fixture".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "lookup-oracle/v1".into(),
                inputs: files
                    .into_iter()
                    .map(|(path, bytes)| WorkspaceInput {
                        path,
                        bytes: Some(bytes),
                        layer: InputLayer::WorkingTree,
                        kind: InputKind::File,
                        mode: FileMode::Regular,
                    })
                    .collect(),
                environment: BTreeMap::new(),
            },
            State::new(),
        )
        .unwrap()
    }

    fn fixture_files(language: &str) -> Vec<(String, Vec<u8>)> {
        fn read(
            root: &std::path::Path,
            directory: &std::path::Path,
            files: &mut Vec<(String, Vec<u8>)>,
        ) {
            let mut entries = std::fs::read_dir(directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    read(root, &path, files);
                } else {
                    files.push((
                        path.strip_prefix(root)
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .replace('\\', "/"),
                        std::fs::read(&path).unwrap(),
                    ));
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/index")
            .join(language);
        let mut files = Vec::new();
        read(&root, &root, &mut files);
        files
    }

    fn compare_candidates(index: &SourceIndex) {
        let lookup = LexicalLookup::new(index);
        let mut queries = BTreeSet::new();
        for occurrence in &index.occurrences {
            queries.insert((
                occurrence.path.clone(),
                occurrence.span,
                occurrence.name.clone(),
            ));
            let name = occurrence.name.replace("::", ".");
            queries.insert((
                occurrence.path.clone(),
                occurrence.span,
                name.split('.').next().unwrap().into(),
            ));
        }
        let mut names = BTreeSet::from(["not_declared".to_owned()]);
        for node in index.nodes.values() {
            names.insert(node.name.clone());
            if let Some(span) = node.name_span {
                queries.insert((node.path.clone(), span, node.name.clone()));
            }
        }
        for path in index.files.keys() {
            for name in &names {
                assert_eq!(
                    lookup.top_level(path, name),
                    top_level_symbol_by_scan(index, path, name),
                    "top-level {path}: {name}"
                );
                queries.insert((path.clone(), SourceSpan::default(), name.clone()));
            }
        }
        queries.insert(("missing.py".into(), SourceSpan::default(), "unknown".into()));
        for (path, span, name) in queries {
            let reference = PendingReference {
                path,
                span,
                name,
                kind: OccurrenceKind::Reference,
                container: ElementId::new("lookup-query-container").unwrap(),
            };
            assert_eq!(
                lookup.candidates(&reference),
                lexical_candidates_by_scan(index, &reference),
                "{} {:?}: {}",
                reference.path,
                reference.span,
                reference.name
            );
        }
    }

    fn compare_generations(states: Vec<Snapshot>, config: IndexConfig) {
        let mut indexed = Indexer::new(config.clone()).unwrap();
        let mut scanned = Indexer::new(config.clone()).unwrap();
        for state in states {
            let actual = indexed.update(&state).unwrap();
            let reference = scanned
                .update_with_resolution(&state, resolve_references_by_scan)
                .unwrap();
            assert_eq!(
                actual, reference,
                "entire artifact, including generation and provenance"
            );
            assert_eq!(indexed.stats(), scanned.stats());
            assert_eq!(
                actual,
                Indexer::new(config.clone()).unwrap().build(&state).unwrap(),
                "incremental versus clean rebuild"
            );
            compare_candidates(&actual);
        }
    }

    #[test]
    fn indexed_lookup_preserves_original_scan_artifacts_across_languages_and_transitions() {
        for language in ["typescript", "javascript", "python", "go", "rust"] {
            let original = fixture_files(language);
            let dependency = original
                .iter()
                .position(|(_, bytes)| std::str::from_utf8(bytes).unwrap().contains("+ 1"))
                .unwrap();
            let mut changed = original.clone();
            let callee = if language == "go" { "Base" } else { "base" };
            changed[dependency].1 = String::from_utf8(changed[dependency].1.clone())
                .unwrap()
                .replace(callee, "renamed_dependency")
                .into_bytes();
            let mut deleted = original.clone();
            deleted.remove(dependency);
            let mut moved = original.clone();
            moved[dependency].0 = format!("relocated/{}", moved[dependency].0);
            let mut copied = original.clone();
            copied.push((
                format!("copied/{}", original[dependency].0),
                original[dependency].1.clone(),
            ));
            compare_generations(
                vec![
                    snapshot(original.clone()),
                    snapshot(changed),
                    snapshot(deleted),
                    snapshot(moved),
                    snapshot(copied),
                    snapshot(vec![]),
                    snapshot(original),
                ],
                IndexConfig::default(),
            );
        }
    }

    #[test]
    fn lookup_oracle_covers_shadowing_ambiguity_packages_and_partial_analysis() {
        let files = vec![
            ("a.ts", "function same(){return 1;} function same(){return 1;} function run(){ {let value=1; same(value);} {let value=2; same(value);} }"),
            ("left.py", "def work(): return 1\n"),
            ("right.py", "def work(): return 2\n"),
            ("app.py", "def first():\n    from left import work\n    return work()\ndef second():\n    from right import work\n    return work()\ndef outside(): return work()\n"),
            ("a.go", "package shared\nfunc Run() int {return helper()}"),
            ("b.go", "package shared\nfunc helper() int {return 1}"),
            ("c.go", "package shared\nfunc helper() int {return 2}"),
            ("outside_test.go", "package shared_test\nfunc helper() int {return 3}"),
            ("nested/d.go", "package shared\nfunc helper() int {return 4}"),
            ("broken.ts", "export function broken( {"),
            ("unsupported.rb", "puts 'unknown'"),
            ("generated.py", "def generated(): return first()\n"),
            ("schema.json", "{}"),
        ].into_iter().map(|(path, bytes)| (path.into(), bytes.as_bytes().to_vec())).collect();
        compare_generations(
            vec![snapshot(files)],
            IndexConfig {
                generated_inputs: BTreeMap::from([(
                    "generated.py".into(),
                    BTreeSet::from(["schema.json".into()]),
                )]),
                ..IndexConfig::default()
            },
        );
    }
}
