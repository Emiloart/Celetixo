use celetixo_engine::ids::{OrganizationId, RepositoryId, Revision, Scope, WorkspaceId};
use celetixo_engine::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotManifest, State,
    WorkspaceInput,
};
use celetixo_repository::index::*;
use std::collections::{BTreeMap, BTreeSet};

fn snapshot(files: &[(&str, &str)]) -> Snapshot {
    snapshot_inputs(
        files
            .iter()
            .map(|(path, bytes)| WorkspaceInput {
                path: (*path).into(),
                bytes: Some(bytes.as_bytes().to_vec()),
                kind: InputKind::File,
                mode: FileMode::Regular,
                layer: InputLayer::WorkingTree,
            })
            .collect(),
    )
}
fn snapshot_inputs(inputs: Vec<WorkspaceInput>) -> Snapshot {
    Snapshot::new(
        Scope {
            organization: OrganizationId::new("test-org").unwrap(),
            workspace: WorkspaceId::new("test-ws").unwrap(),
            repository: RepositoryId::new("test-repo").unwrap(),
        },
        Revision(0),
        SnapshotManifest {
            base_commit: "fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "deterministic-index-fixture/v1".into(),
            inputs,
            environment: BTreeMap::new(),
        },
        State::new(),
    )
    .unwrap()
}
fn index(snapshot: &Snapshot) -> SourceIndex {
    Indexer::new(IndexConfig::default())
        .unwrap()
        .build(snapshot)
        .unwrap()
}
fn fixture(language: &str) -> Snapshot {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/index")
        .join(language);
    fn read(root: &std::path::Path, path: &std::path::Path, files: &mut Vec<(String, String)>) {
        let mut entries = std::fs::read_dir(path)
            .unwrap()
            .map(|v| v.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                read(root, &path, files)
            } else {
                files.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }
    let mut files = Vec::new();
    read(&root, &root, &mut files);
    snapshot(
        &files
            .iter()
            .map(|(p, b)| (p.as_str(), b.as_str()))
            .collect::<Vec<_>>(),
    )
}
fn named<'a>(index: &'a SourceIndex, name: &str) -> &'a IndexNode {
    let matches = index
        .symbols(name)
        .into_iter()
        .filter(|n| matches!(n.kind, NodeKind::Function | NodeKind::Method))
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "{name}: {matches:?}");
    matches[0]
}
fn has_call(index: &SourceIndex, from: &str, to: &str) -> bool {
    let from = &named(index, from).id;
    let to = &named(index, to).id;
    index
        .edges
        .iter()
        .any(|e| e.kind == EdgeKind::Call && &e.from == from && &e.to == to)
}

#[test]
fn all_language_adapters_parse_real_multifile_calls_and_imports() {
    for (language, caller, callee, test) in [
        ("typescript", "app", "base", "test_app"),
        ("javascript", "app", "base", "View"),
        ("python", "app", "base", "test_app"),
        ("go", "App", "Base", "TestApp"),
        ("rust", "app", "base", "test_app"),
    ] {
        let snap = fixture(language);
        let idx = index(&snap);
        idx.validate(&snap).unwrap();
        assert!(
            !idx.issues
                .iter()
                .any(|i| matches!(i.kind, IssueKind::SyntaxError | IssueKind::MissingSyntax)),
            "{language}: {:?}",
            idx.issues
        );
        assert!(
            has_call(&idx, caller, callee),
            "{language}: {:#?}",
            idx.occurrences
        );
        // Rust macro invocations intentionally remain unresolved until expansion analysis.
        if language != "rust" {
            assert!(
                has_call(&idx, test, caller),
                "{language}: {:#?}",
                idx.occurrences
            );
        }
        assert!(
            idx.edges.iter().any(|e| e.kind == EdgeKind::Import),
            "{language}"
        );
        assert!(!idx.is_complete());
        assert!(
            idx.issues
                .iter()
                .any(|i| i.kind == IssueKind::SemanticAnalysisUnavailable)
        );
    }
}
#[test]
fn cold_rebuild_and_incremental_update_are_identical() {
    let first = snapshot(&[
        ("a.ts", "export function a() { return 1; }"),
        (
            "b.ts",
            "import {a} from './a'; export function b(){return a();}",
        ),
    ]);
    let second = snapshot(&[
        ("a.ts", "export function a() { return 2; }"),
        (
            "b.ts",
            "import {a} from './a'; export function b(){return a();}",
        ),
    ]);
    let mut builder = Indexer::new(IndexConfig::default()).unwrap();
    builder.build(&first).unwrap();
    let changed = builder.update(&second).unwrap();
    assert_eq!(builder.stats().parsed_files, 1);
    assert_eq!(builder.stats().reused_files, 1);
    assert_eq!(changed, index(&second));
    assert_eq!(builder.update(&second).unwrap(), changed);
    assert_eq!(builder.stats().parsed_files, 0);
}
#[test]
fn configuration_and_analyzer_versions_invalidate_generation_and_cache() {
    let snap = fixture("python");
    let original = index(&snap);
    let config = IndexConfig {
        analyzer_version: "custom-analysis-profile/v1".into(),
        ..IndexConfig::default()
    };
    let changed = Indexer::new(config).unwrap().build(&snap).unwrap();
    assert_ne!(original.generation, changed.generation);
    assert_ne!(original.config_digest, changed.config_digest);
    let config = IndexConfig {
        language_configuration: BTreeMap::from([("python.import-path".into(), "src".into())]),
        ..IndexConfig::default()
    };
    assert_ne!(
        original.generation,
        Indexer::new(config)
            .unwrap()
            .build(&snap)
            .unwrap()
            .generation
    );
}
#[test]
fn malformed_dynamic_and_unsupported_inputs_are_explicit() {
    let snap = snapshot(&[
        ("broken.ts", "export function broken( {"),
        ("unknown.rb", "puts 'hello'"),
        ("dynamic.py", "def run(x):\n    return x.unknown()\n"),
    ]);
    let idx = index(&snap);
    for kind in [
        IssueKind::SyntaxError,
        IssueKind::UnsupportedLanguage,
        IssueKind::UnresolvedReference,
        IssueKind::SemanticAnalysisUnavailable,
    ] {
        assert!(
            idx.issues.iter().any(|i| i.kind == kind),
            "missing {kind:?}: {:?}",
            idx.issues
        );
    }
    assert!(idx.occurrences.iter().any(|o| o.target.is_none()));
}
#[test]
fn stale_snapshot_and_tampered_generation_rejected() {
    let first = snapshot(&[("a.rs", "fn a() {}")]);
    let second = snapshot(&[("a.rs", "fn a() { b(); }")]);
    let mut idx = index(&first);
    assert_eq!(idx.validate(&second), Err(IndexError::SnapshotMismatch));
    idx.issues.clear();
    assert_eq!(
        idx.validate_generation(),
        Err(IndexError::CorruptGeneration)
    );
}
#[test]
fn layer_precedence_deletions_and_overlay_are_respected() {
    let mut inputs = Vec::new();
    for (layer, path, content) in [
        (InputLayer::Base, "a.py", Some("def base(): pass")),
        (InputLayer::Index, "a.py", Some("def staged(): pass")),
        (InputLayer::WorkingTree, "a.py", Some("def dirty(): pass")),
        (InputLayer::Overlay, "a.py", Some("def overlay(): pass")),
        (InputLayer::Base, "gone.py", Some("def gone(): pass")),
        (InputLayer::WorkingTree, "gone.py", None),
        (InputLayer::Untracked, "new.py", Some("def fresh(): pass")),
    ] {
        inputs.push(WorkspaceInput {
            path: path.into(),
            layer,
            kind: InputKind::File,
            mode: FileMode::Regular,
            bytes: content.map(|s| s.as_bytes().to_vec()),
        });
    }
    let idx = index(&snapshot_inputs(inputs));
    assert!(idx.symbols("overlay").len() == 1);
    assert!(idx.symbols("fresh").len() == 1);
    assert!(idx.symbols("base").is_empty());
    assert!(!idx.files.contains_key("gone.py"));
}
#[test]
fn generated_aliases_and_missing_inputs_fail_explicitly() {
    let snap = snapshot_inputs(vec![
        WorkspaceInput {
            path: "a.py".into(),
            layer: InputLayer::WorkingTree,
            kind: InputKind::File,
            mode: FileMode::Regular,
            bytes: Some(b"def a():pass".to_vec()),
        },
        WorkspaceInput {
            path: "a.py".into(),
            layer: InputLayer::Generated,
            kind: InputKind::File,
            mode: FileMode::Regular,
            bytes: Some(b"def other():pass".to_vec()),
        },
    ]);
    assert!(matches!(
        Indexer::new(IndexConfig::default()).unwrap().build(&snap),
        Err(IndexError::EffectiveWorkspace(_))
    ));
    let snap = snapshot(&[("generated.py", "def generated(): pass")]);
    let config = IndexConfig {
        generated_inputs: BTreeMap::from([(
            "generated.py".into(),
            BTreeSet::from(["missing.schema".into()]),
        )]),
        ..IndexConfig::default()
    };
    let idx = Indexer::new(config).unwrap().build(&snap).unwrap();
    assert!(
        idx.issues
            .iter()
            .any(|i| i.kind == IssueKind::GeneratedInputMissing)
    );
}
#[test]
fn failure_does_not_publish_a_partial_generation_or_poison_cache() {
    let first = snapshot(&[("a.py", "def a():pass")]);
    let second = snapshot(&[("a.py", "def a():pass"), ("large.py", &"x".repeat(1000))]);
    let config = IndexConfig {
        max_file_bytes: 100,
        ..IndexConfig::default()
    };
    let mut builder = Indexer::new(config).unwrap();
    let before = builder.build(&first).unwrap();
    assert!(matches!(
        builder.update(&second),
        Err(IndexError::ResourceLimit(_))
    ));
    assert_eq!(builder.published(), Some(&before));
    assert_eq!(builder.update(&first).unwrap(), before);
    assert_eq!(builder.stats().parsed_files, 0);
}
#[test]
fn symbol_identity_survives_line_moves_but_not_reused_declarations() {
    let a = index(&snapshot(&[("a.py", "def f():\n    return 'a b'\n")]));
    let b = index(&snapshot(&[(
        "a.py",
        "\n\n# comment\ndef f():\n    return 'a b'\n",
    )]));
    let c = index(&snapshot(&[("a.py", "def f():\n    return 'ab'\n")]));
    assert_eq!(named(&a, "f").id, named(&b, "f").id);
    assert_ne!(named(&a, "f").span, named(&b, "f").span);
    assert_ne!(named(&a, "f").id, named(&c, "f").id);
    assert!(
        a.occurrences
            .iter()
            .all(|o| !b.occurrences.iter().any(|p| o.id == p.id))
    );
}
#[test]
fn exact_renames_copies_and_ambiguous_moves_keep_separate_identities() {
    let a = index(&snapshot(&[("old.py", "def f(): pass")]));
    let b = index(&snapshot(&[("new.py", "def f(): pass")]));
    let c = index(&snapshot(&[
        ("old.py", "def f(): pass"),
        ("copy.py", "def f(): pass"),
    ]));
    let d = index(&snapshot(&[
        ("new1.py", "def f(): pass"),
        ("new2.py", "def f(): pass"),
    ]));
    assert_eq!(
        continuity(&a, &b).unwrap()[0].kind,
        ContinuityKind::ExactContentMove
    );
    assert_ne!(named(&a, "f").id, named(&b, "f").id);
    assert_eq!(continuity(&a, &c).unwrap()[0].kind, ContinuityKind::Copy);
    assert_eq!(
        continuity(&a, &d).unwrap()[0].kind,
        ContinuityKind::Ambiguous
    );
    assert!(!continuity(&a, &b).unwrap()[0].authorizes_identity);
}
#[test]
fn lexical_scopes_prevent_same_name_cross_scope_resolution() {
    let snap = snapshot(&[(
        "a.py",
        "def outer():\n    def f(): return 1\n    return f()\n\ndef other():\n    def f(): return 2\n    return f()\n",
    )]);
    let idx = index(&snap);
    let outer = named(&idx, "outer");
    let other = named(&idx, "other");
    let calls = idx
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Call)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    let outer_call = calls.iter().find(|e| e.from == outer.id).unwrap();
    let other_call = calls.iter().find(|e| e.from == other.id).unwrap();
    assert_ne!(outer_call.to, other_call.to);
    assert_eq!(
        idx.nodes[&outer_call.to].parent.as_ref(),
        Some(
            &idx.nodes
                .values()
                .find(|n| n.kind == NodeKind::Scope && n.parent.as_ref() == Some(&outer.id))
                .unwrap()
                .id
        )
    );
}
#[test]
fn configuration_package_and_generated_relationships_are_preserved() {
    let snap = snapshot(&[
        ("package.json", r#"{"dependencies":{"library":"^1.0.0"}}"#),
        ("schema.json", "{}"),
        ("src/generated.ts", "export const x = 1;"),
        ("src/app.ts", "export function app(){return 1;}"),
    ]);
    let config = IndexConfig {
        generated_inputs: BTreeMap::from([(
            "src/generated.ts".into(),
            BTreeSet::from(["schema.json".into()]),
        )]),
        ..IndexConfig::default()
    };
    let idx = Indexer::new(config).unwrap().build(&snap).unwrap();
    assert!(
        idx.nodes
            .values()
            .any(|n| n.kind == NodeKind::Package && n.name == "library")
    );
    assert!(idx.edges.iter().any(|e| e.kind == EdgeKind::Generated));
    assert!(idx.edges.iter().any(|e| e.kind == EdgeKind::Build));
    assert!(idx.edges.iter().any(|e| e.kind == EdgeKind::Package));
}

#[test]
fn typed_manifests_supply_language_package_dependencies() {
    let snap = snapshot(&[
        (
            "Cargo.toml",
            "[package]\nname='sample'\nversion='0.1.0'\n[dependencies]\nserde='1'\n[target.'cfg(unix)'.dependencies]\nrustix={version='1',features=['fs']}\n",
        ),
        (
            "pyproject.toml",
            "[project]\nname='sample'\ndependencies=['httpx>=0.28']\n[project.optional-dependencies]\ntest=['pytest>=8']\n",
        ),
        (
            "go.mod",
            "module example.test/m\ngo 1.24\nrequire (\n example.test/dependency v1.2.3\n)\n",
        ),
    ]);
    let idx = index(&snap);
    for name in [
        "serde",
        "rustix",
        "httpx>=0.28",
        "pytest>=8",
        "example.test/dependency",
    ] {
        assert!(
            idx.nodes
                .values()
                .any(|n| n.kind == NodeKind::Package && n.name == name),
            "{name}: {:?} {:?}",
            idx.nodes,
            idx.issues
        );
    }
}
#[test]
fn interfaces_inheritance_and_cycles_are_explicit_structural_relations() {
    let snap = snapshot(&[
        (
            "a.ts",
            "interface Base {} interface Child extends Base {} class Impl implements Base {} function a(){b()} function b(){a()}",
        ),
        ("a.py", "class Base: pass\nclass Child(Base): pass\n"),
        (
            "a.rs",
            "trait Read { fn read(&self); } struct Value; impl Read for Value { fn read(&self) {} }",
        ),
    ]);
    let idx = index(&snap);
    assert!(has_call(&idx, "a", "b"));
    assert!(has_call(&idx, "b", "a"));
    for path in ["a.ts", "a.py", "a.rs"] {
        assert!(
            idx.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Implements && e.provenance.path == path),
            "{path}: {:?}",
            idx.occurrences
        );
    }
}
#[test]
fn deep_syntax_is_resource_bounded_before_recursive_extraction() {
    let source = format!("x = {}0{}", "(".repeat(400), ")".repeat(400));
    let snap = snapshot(&[("deep.py", &source)]);
    assert!(matches!(
        Indexer::new(IndexConfig::default()).unwrap().build(&snap),
        Err(IndexError::ResourceLimit(_))
    ));
}

#[test]
fn configuration_validation_rejects_analyzer_generation_mismatch() {
    let snap = fixture("python");
    let idx = index(&snap);
    idx.validate_configuration(&IndexConfig::default()).unwrap();
    let config = IndexConfig {
        analyzer_version: "another-analyzer/v1".into(),
        ..IndexConfig::default()
    };
    assert_eq!(
        idx.validate_configuration(&config),
        Err(IndexError::ConfigurationMismatch)
    );
    let mut corrupted = idx;
    corrupted.configuration.max_file_bytes += 1;
    corrupted.refresh_generation();
    assert_eq!(
        corrupted.validate_generation(),
        Err(IndexError::ConfigurationMismatch)
    );
}

#[test]
fn rename_delete_and_recreation_incremental_results_match_full_rebuild() {
    for (extension, body) in [
        ("ts", "export function f(){return 1;}"),
        ("js", "export function f(){return 1;}"),
        ("py", "def f(): return 1\n"),
        ("go", "package sample\nfunc f() int {return 1}"),
        ("rs", "fn f()->i32 {1}"),
    ] {
        let old_path = format!("old.{extension}");
        let new_path = format!("new.{extension}");
        let old = snapshot(&[(&old_path, body)]);
        let renamed = snapshot(&[(&new_path, body)]);
        let deleted = snapshot(&[]);
        let recreated = snapshot(&[(&old_path, &body.replace('1', "2"))]);
        let mut builder = Indexer::new(IndexConfig::default()).unwrap();
        let before = builder.build(&old).unwrap();
        let after = builder.update(&renamed).unwrap();
        assert_eq!(after, index(&renamed));
        assert_eq!(
            continuity(&before, &after).unwrap()[0].kind,
            ContinuityKind::ExactContentMove
        );
        assert_ne!(named(&before, "f").id, named(&after, "f").id);
        assert_eq!(builder.update(&deleted).unwrap(), index(&deleted));
        let rebuilt = builder.update(&recreated).unwrap();
        assert_eq!(rebuilt, index(&recreated));
        assert_ne!(named(&before, "f").id, named(&rebuilt, "f").id);
    }
}
#[test]
fn indexing_and_capture_share_exact_effective_layer_semantics() {
    let snap = snapshot_inputs(vec![
        WorkspaceInput {
            path: "generated.py".into(),
            layer: InputLayer::Generated,
            kind: InputKind::File,
            mode: FileMode::Regular,
            bytes: Some(b"def generated(): pass".to_vec()),
        },
        WorkspaceInput {
            path: "generated.py".into(),
            layer: InputLayer::Overlay,
            kind: InputKind::File,
            mode: FileMode::Regular,
            bytes: Some(b"def buffer(): pass".to_vec()),
        },
    ]);
    let captured = celetixo_repository::capture::effective_workspace(&snap).unwrap();
    let effective = effective_inputs(&snap).unwrap();
    assert_eq!(
        effective["generated.py"].1,
        captured["generated.py"].bytes.clone().unwrap()
    );
    assert_eq!(index(&snap).symbols("buffer").len(), 1);
    assert!(
        !index(&snap)
            .symbols("generated")
            .iter()
            .any(|node| node.kind == NodeKind::Function)
    );
}

#[test]
fn go_package_scope_resolves_sibling_files_without_crossing_package_boundaries() {
    let snap = snapshot(&[
        ("a.go", "package shared\nfunc App() int {return helper()}"),
        ("b.go", "package shared\nfunc helper() int {return 1}"),
        (
            "external_test.go",
            "package shared_test\nfunc helper() int {return 2}",
        ),
        (
            "subdir/c.go",
            "package shared\nfunc helper() int {return 3}",
        ),
    ]);
    let idx = index(&snap);
    let app = named(&idx, "App");
    let target = idx
        .edges
        .iter()
        .find(|edge| edge.from == app.id && edge.kind == EdgeKind::Call)
        .unwrap();
    assert_eq!(idx.nodes[&target.to].path, "b.go");
    assert!(idx.edges.iter().any(|edge| edge.kind == EdgeKind::Package));
}

#[test]
fn digest_valid_structural_corruption_is_rejected_without_panics() {
    let snap = snapshot(&[("a.py", "def f(): return f()\n")]);
    let original = index(&snap);
    let mut mutations = Vec::new();
    let mut bad = original.clone();
    bad.nodes.remove(&bad.files["a.py"].file);
    mutations.push(bad);
    let mut bad = original.clone();
    bad.nodes.get_mut(&bad.files["a.py"].module).unwrap().parent = None;
    mutations.push(bad);
    let mut bad = original.clone();
    bad.nodes.get_mut(&named(&original, "f").id).unwrap().id =
        celetixo_engine::ids::ElementId::new("wrong-node-key").unwrap();
    mutations.push(bad);
    let mut bad = original.clone();
    bad.occurrences[0].span.end_byte = usize::MAX;
    mutations.push(bad);
    let mut bad = original.clone();
    bad.occurrences[0].target =
        Some(celetixo_engine::ids::ElementId::new("missing-target").unwrap());
    mutations.push(bad);
    let mut bad = original.clone();
    let mut edge = bad.edges.pop_first().unwrap();
    edge.provenance.snapshot = snapshot(&[]).id();
    bad.edges.insert(edge);
    mutations.push(bad);
    let mut bad = original.clone();
    let mut edge = bad.edges.pop_first().unwrap();
    edge.from = celetixo_engine::ids::ElementId::new("missing-edge-node").unwrap();
    bad.edges.insert(edge);
    mutations.push(bad);
    let mut bad = original.clone();
    bad.files.get_mut("a.py").unwrap().path = "wrong.py".into();
    mutations.push(bad);
    let mut bad = original.clone();
    bad.occurrences[0].id =
        celetixo_engine::ids::Digest::of("stale-occurrence", b"previous snapshot");
    mutations.push(bad);
    let mut bad = original.clone();
    let scope = bad
        .nodes
        .values()
        .find(|node| node.kind == NodeKind::Scope)
        .unwrap()
        .id
        .clone();
    let node = bad.nodes.get_mut(&scope).unwrap();
    node.parent = Some(scope);
    mutations.push(bad);
    for mut bad in mutations {
        bad.refresh_generation();
        assert!(matches!(
            bad.validate_generation(),
            Err(IndexError::InvalidStructure(_))
        ));
        assert!(matches!(
            bad.validate(&snap),
            Err(IndexError::InvalidStructure(_))
        ));
    }
    // A structurally empty rehashed index still cannot stand in for a nonempty
    // snapshot. The entire effective file census/content remains bound.
    let mut missing = index(&snapshot(&[]));
    missing.snapshot = snap.id();
    missing.refresh_generation();
    assert!(matches!(
        missing.validate(&snap),
        Err(IndexError::InvalidStructure(_))
    ));
}

#[test]
fn nested_receiver_calls_remain_dependencies_when_dispatch_is_unresolved() {
    for (path, source, caller, callee) in [
        (
            "a.ts",
            "function factory(){return null;} function run(){return factory().unknown();}",
            "run",
            "factory",
        ),
        (
            "a.js",
            "function factory(){return null;} function run(){return factory().unknown();}",
            "run",
            "factory",
        ),
        (
            "a.py",
            "def factory(): return None\ndef run(): return factory().unknown()\n",
            "run",
            "factory",
        ),
        (
            "a.go",
            "package example\nfunc Factory() int {return 1}\nfunc Run(){Factory().Unknown()}",
            "Run",
            "Factory",
        ),
        (
            "a.rs",
            "fn factory()->i32 {1} fn run(){factory().unknown();}",
            "run",
            "factory",
        ),
    ] {
        let idx = index(&snapshot(&[(path, source)]));
        assert!(
            has_call(&idx, caller, callee),
            "{path}: {:?}",
            idx.occurrences
        );
        assert!(idx.occurrences.iter().any(
            |occurrence| occurrence.kind == OccurrenceKind::Call && occurrence.target.is_none()
        ));
    }
}

#[test]
fn local_python_imports_do_not_leak_into_sibling_functions() {
    let idx = index(&snapshot(&[
        ("left.py", "def work(): return 1\n"),
        ("right.py", "def work(): return 2\n"),
        (
            "app.py",
            "def first():\n    from left import work\n    return work()\ndef second():\n    from right import work\n    return work()\ndef unrelated(): return work()\n",
        ),
    ]));
    for (caller, target_path) in [("first", "left.py"), ("second", "right.py")] {
        let caller = named(&idx, caller);
        let call = idx
            .edges
            .iter()
            .find(|edge| edge.from == caller.id && edge.kind == EdgeKind::Call)
            .unwrap();
        assert_eq!(idx.nodes[&call.to].path, target_path);
    }
    let unrelated = named(&idx, "unrelated");
    assert!(
        !idx.edges
            .iter()
            .any(|edge| edge.from == unrelated.id && edge.kind == EdgeKind::Call)
    );
    assert!(
        idx.occurrences
            .iter()
            .any(|occurrence| occurrence.container == unrelated.id
                && occurrence.kind == OccurrenceKind::Call
                && occurrence.target.is_none())
    );
}

#[test]
fn git_metadata_configuration_reaches_the_corresponding_workspace_scope() {
    let idx = index(&snapshot(&[
        (".git/info/attributes", "*.py text\n"),
        ("a.py", "def f(): return 1\n"),
        ("vendor/.git/info/attributes", "*.rs text\n"),
        ("vendor/lib.rs", "fn f(){}"),
    ]));
    let attrs = &idx.files[".git/info/attributes"].file;
    let sub_attrs = &idx.files["vendor/.git/info/attributes"].file;
    assert!(idx.edges.iter().any(|edge| edge.kind == EdgeKind::Build
        && &edge.to == attrs
        && edge.from == idx.files["a.py"].module));
    assert!(idx.edges.iter().any(|edge| edge.kind == EdgeKind::Build
        && &edge.to == sub_attrs
        && edge.from == idx.files["vendor/lib.rs"].module));
    assert!(!idx.edges.iter().any(|edge| edge.kind == EdgeKind::Build
        && &edge.to == sub_attrs
        && edge.from == idx.files["a.py"].module));
}

#[test]
fn unimplemented_package_semantics_cannot_be_complete_without_source_files() {
    let idx = index(&snapshot(&[(
        "package.json",
        r#"{"name":"fixture","exports":"./generated.js","workspaces":["packages/*"]}"#,
    )]));
    assert!(!idx.is_complete());
    assert!(
        idx.issues.iter().any(|issue| issue.path == "package.json"
            && issue.kind == IssueKind::ConfigurationUnresolved)
    );
}

#[test]
fn every_language_recomputes_cached_consumers_when_a_dependency_changes_or_disappears() {
    for (language, caller, callee) in [
        ("typescript", "app", "base"),
        ("javascript", "app", "base"),
        ("python", "app", "base"),
        ("go", "App", "Base"),
        ("rust", "app", "base"),
    ] {
        let first = fixture(language);
        let mut builder = Indexer::new(IndexConfig::default()).unwrap();
        let before = builder.build(&first).unwrap();
        let mut inputs = first.manifest().inputs.clone();
        let dependency = inputs
            .iter_mut()
            .find(|input| {
                std::str::from_utf8(input.bytes.as_ref().unwrap())
                    .unwrap()
                    .contains("+ 1")
            })
            .unwrap();
        let source = String::from_utf8(dependency.bytes.take().unwrap()).unwrap();
        dependency.bytes = Some(source.replace("+ 1", "+ 2").into_bytes());
        let changed = snapshot_inputs(inputs.clone());
        let after = builder.update(&changed).unwrap();
        assert_eq!(after, index(&changed), "{language}");
        assert_eq!(builder.stats().parsed_files, 1, "{language}");
        assert_ne!(
            named(&before, callee).id,
            named(&after, callee).id,
            "{language}"
        );
        assert!(has_call(&after, caller, callee), "{language}");
        let dependency = inputs
            .iter_mut()
            .find(|input| {
                std::str::from_utf8(input.bytes.as_ref().unwrap())
                    .unwrap()
                    .contains("+ 2")
            })
            .unwrap();
        let source = String::from_utf8(dependency.bytes.take().unwrap()).unwrap();
        dependency.bytes = Some(source.replace(callee, "replacement").into_bytes());
        let removed = snapshot_inputs(inputs);
        let after = builder.update(&removed).unwrap();
        assert_eq!(after, index(&removed), "{language}");
        assert!(after.symbols(callee).is_empty(), "{language}");
        assert!(
            after.occurrences.iter().any(|occurrence| {
                occurrence.container == named(&after, caller).id
                    && occurrence.kind == OccurrenceKind::Call
                    && occurrence.target.is_none()
            }),
            "{language}: {:?}",
            after.occurrences
        );
        assert_eq!(builder.update(&first).unwrap(), before, "{language}");
    }
}

#[test]
fn exact_declaration_selections_are_bound_to_current_snapshot_bytes() {
    for language in ["typescript", "javascript", "python", "go", "rust"] {
        let snap = fixture(language);
        let idx = index(&snap);
        let files = effective_inputs(&snap).unwrap();
        let mut selected = 0;
        for node in idx.nodes.values() {
            if let Some(span) = node.name_span {
                selected += 1;
                assert!(node.span.contains(&span));
                assert_eq!(
                    &files[&node.path].1[span.start_byte..span.end_byte],
                    node.name.as_bytes()
                );
            }
        }
        assert!(selected > 0, "{language}");
        let mut malformed = idx.clone();
        let declaration = malformed
            .nodes
            .values_mut()
            .find(|node| node.name_span.is_some())
            .unwrap();
        declaration.name = "forged-selection".into();
        malformed.refresh_generation();
        assert!(matches!(
            malformed.validate(&snap),
            Err(IndexError::InvalidStructure(
                "declaration selection content"
            ))
        ));
    }
}
