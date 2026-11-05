//! Identity regressions distinguish an accepted repository revision from repeated
//! analysis of the same immutable state. Shape/content correspondence alone must
//! never recycle an earlier incarnation after an accepted deletion.
use celetixo_engine::ids::{Digest, OrganizationId, RepositoryId, Revision, Scope, WorkspaceId};
use celetixo_engine::snapshot::{
    FileMode, InclusionPolicy, InputKind, InputLayer, Snapshot, SnapshotManifest, State,
    WorkspaceInput,
};
use celetixo_repository::index::{
    IndexConfig, IndexNode, Indexer, Language, NodeKind, SourceIndex,
};
use std::collections::{BTreeMap, BTreeSet};

const BASE_A: &str = "1111111111111111111111111111111111111111";
const BASE_B: &str = "2222222222222222222222222222222222222222";

struct LanguageFixture {
    path: &'static str,
    target: &'static str,
    source: &'static str,
    without_target: &'static str,
    comment: &'static str,
}

const LANGUAGES: &[LanguageFixture] = &[
    LanguageFixture {
        path: "source.ts",
        target: "target",
        source: "export function target() { return 1; }\nexport function keeper() { return target(); }\n",
        without_target: "export function keeper() { return target(); }\n",
        comment: "// source location changed\n\n",
    },
    LanguageFixture {
        path: "source.js",
        target: "target",
        source: "export function target() { return 1; }\nexport function keeper() { return target(); }\n",
        without_target: "export function keeper() { return target(); }\n",
        comment: "// source location changed\n\n",
    },
    LanguageFixture {
        path: "source.py",
        target: "target",
        source: "def target():\n    return 1\n\ndef keeper():\n    return target()\n",
        without_target: "def keeper():\n    return target()\n",
        comment: "# source location changed\n\n",
    },
    LanguageFixture {
        path: "source.go",
        target: "Target",
        source: "package sample\n\nfunc Target() int { return 1 }\nfunc Keeper() int { return Target() }\n",
        without_target: "package sample\n\nfunc Keeper() int { return Target() }\n",
        comment: "// source location changed\n\n",
    },
    LanguageFixture {
        path: "source.rs",
        target: "target",
        source: "pub fn target() -> i32 { 1 }\npub fn keeper() -> i32 { target() }\n",
        without_target: "pub fn keeper() -> i32 { target() }\n",
        comment: "// source location changed\n\n",
    },
];

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("identity-organization").unwrap(),
        workspace: WorkspaceId::new("identity-workspace").unwrap(),
        repository: RepositoryId::new("identity-repository").unwrap(),
    }
}

fn snapshot(revision: u64, base_commit: &str, files: &[(&str, &str)]) -> Snapshot {
    Snapshot::new(
        scope(),
        Revision(revision),
        SnapshotManifest {
            base_commit: base_commit.into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "identity-regression-fixture/v1".into(),
            inputs: files
                .iter()
                .map(|(path, source)| WorkspaceInput {
                    path: (*path).into(),
                    layer: InputLayer::WorkingTree,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(source.as_bytes().to_vec()),
                })
                .collect(),
            environment: BTreeMap::new(),
        },
        State::new(),
    )
    .unwrap()
}

fn indexer() -> Indexer {
    Indexer::new(IndexConfig::default()).unwrap()
}

fn update_and_rebuild(builder: &mut Indexer, snapshot: &Snapshot) -> SourceIndex {
    let incremental = builder.update(snapshot).unwrap();
    let rebuilt = indexer().build(snapshot).unwrap();
    assert_eq!(
        incremental,
        rebuilt,
        "incremental output differs at revision {} with base {}",
        snapshot.revision().0,
        snapshot.manifest().base_commit
    );
    incremental.validate(snapshot).unwrap();
    incremental
}

fn target<'a>(index: &'a SourceIndex, fixture: &LanguageFixture) -> &'a IndexNode {
    let matches = index
        .symbols(fixture.target)
        .into_iter()
        .filter(|node| node.path == fixture.path && node.kind == NodeKind::Function)
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "{}: {matches:?}", fixture.path);
    matches[0]
}

fn assert_disjoint_identities(before: &SourceIndex, after: &SourceIndex, context: &str) {
    assert!(!before.nodes.is_empty(), "empty baseline: {context}");
    assert!(!after.nodes.is_empty(), "empty successor: {context}");
    let previous = before.nodes.keys().cloned().collect::<BTreeSet<_>>();
    let recycled = after
        .nodes
        .keys()
        .filter(|id| previous.contains(*id))
        .collect::<Vec<_>>();
    assert!(
        recycled.is_empty(),
        "recycled IDs in {context}: {recycled:?}"
    );
}

fn shapes(index: &SourceIndex) -> Vec<(String, String, NodeKind, Digest)> {
    let mut shapes = index
        .nodes
        .values()
        .map(|node| {
            (
                node.path.clone(),
                node.name.clone(),
                node.kind,
                node.signature,
            )
        })
        .collect::<Vec<_>>();
    shapes.sort();
    shapes
}

#[test]
fn exact_byte_file_recreation_at_a_later_revision_never_recycles_any_language_identity() {
    for fixture in LANGUAGES {
        let original = snapshot(10, BASE_A, &[(fixture.path, fixture.source)]);
        let deleted = snapshot(11, BASE_A, &[]);
        let restored = snapshot(12, BASE_A, &[(fixture.path, fixture.source)]);
        let mut builder = indexer();
        let before = update_and_rebuild(&mut builder, &original);
        assert_eq!(target(&before, fixture).name, fixture.target);
        let absent = update_and_rebuild(&mut builder, &deleted);
        assert!(absent.files.is_empty());
        assert!(absent.nodes.is_empty());
        let after = update_and_rebuild(&mut builder, &restored);

        assert_eq!(shapes(&before), shapes(&after), "{}", fixture.path);
        assert_ne!(before.identity_namespace, after.identity_namespace);
        assert_ne!(
            before.files[fixture.path].file,
            after.files[fixture.path].file
        );
        assert_ne!(
            before.files[fixture.path].module,
            after.files[fixture.path].module
        );
        assert_ne!(target(&before, fixture).id, target(&after, fixture).id);
        assert_eq!(
            target(&before, fixture).signature,
            target(&after, fixture).signature
        );
        assert_disjoint_identities(&before, &after, fixture.path);

        // Re-reading historical immutable state is not a new accepted revision.
        assert_eq!(update_and_rebuild(&mut builder, &original), before);
    }
}

#[test]
fn an_exact_symbol_restored_inside_a_surviving_file_gets_a_new_incarnation() {
    for fixture in LANGUAGES {
        let original = snapshot(20, BASE_A, &[(fixture.path, fixture.source)]);
        let removed = snapshot(21, BASE_A, &[(fixture.path, fixture.without_target)]);
        let restored = snapshot(22, BASE_A, &[(fixture.path, fixture.source)]);
        let mut builder = indexer();
        let before = update_and_rebuild(&mut builder, &original);
        let absent = update_and_rebuild(&mut builder, &removed);
        assert!(absent.files.contains_key(fixture.path));
        assert!(
            absent.symbols(fixture.target).is_empty(),
            "{}",
            fixture.path
        );
        let after = update_and_rebuild(&mut builder, &restored);

        assert_eq!(shapes(&before), shapes(&after), "{}", fixture.path);
        assert_eq!(
            target(&before, fixture).signature,
            target(&after, fixture).signature
        );
        assert_ne!(target(&before, fixture).id, target(&after, fixture).id);
        assert_disjoint_identities(&before, &after, fixture.path);
    }
}

#[test]
fn a_later_authoritative_revision_namespaces_identical_bytes_even_with_the_same_git_base() {
    for fixture in LANGUAGES {
        let original = snapshot(30, BASE_A, &[(fixture.path, fixture.source)]);
        let later = snapshot(31, BASE_A, &[(fixture.path, fixture.source)]);
        let mut builder = indexer();
        let before = update_and_rebuild(&mut builder, &original);
        let after = update_and_rebuild(&mut builder, &later);

        assert_eq!(shapes(&before), shapes(&after), "{}", fixture.path);
        assert_ne!(before.identity_namespace, after.identity_namespace);
        assert_disjoint_identities(&before, &after, fixture.path);
        assert_eq!(builder.stats().parsed_files, 0);
        assert_eq!(builder.stats().reused_files, 1);
    }
}

#[test]
fn a_new_git_base_namespaces_identical_bytes_at_the_same_revision_without_breaking_revisits() {
    for fixture in LANGUAGES {
        let original = snapshot(40, BASE_A, &[(fixture.path, fixture.source)]);
        let imported = snapshot(40, BASE_B, &[(fixture.path, fixture.source)]);
        let mut builder = indexer();
        let before = update_and_rebuild(&mut builder, &original);
        let after = update_and_rebuild(&mut builder, &imported);

        assert_eq!(shapes(&before), shapes(&after), "{}", fixture.path);
        assert_ne!(before.identity_namespace, after.identity_namespace);
        assert_disjoint_identities(&before, &after, fixture.path);
        assert_eq!(builder.stats().parsed_files, 0);
        assert_eq!(builder.stats().reused_files, 1);
        assert_eq!(update_and_rebuild(&mut builder, &original), before);
    }
}

#[test]
fn comment_and_line_changes_within_an_epoch_preserve_declaration_identity_and_refresh_occurrences()
{
    for fixture in LANGUAGES {
        let original = snapshot(50, BASE_A, &[(fixture.path, fixture.source)]);
        let shifted_source = format!("{}{}", fixture.comment, fixture.source);
        let shifted = snapshot(50, BASE_A, &[(fixture.path, &shifted_source)]);
        let mut builder = indexer();
        let before = update_and_rebuild(&mut builder, &original);
        let after = update_and_rebuild(&mut builder, &shifted);

        assert_ne!(original.id(), shifted.id());
        assert_eq!(before.identity_namespace, after.identity_namespace);
        assert_eq!(target(&before, fixture).id, target(&after, fixture).id);
        assert_eq!(
            target(&before, fixture).signature,
            target(&after, fixture).signature
        );
        assert_ne!(target(&before, fixture).span, target(&after, fixture).span);
        assert_ne!(
            target(&before, fixture).name_span,
            target(&after, fixture).name_span
        );
        assert!(!before.occurrences.is_empty(), "{}", fixture.path);
        let previous = before
            .occurrences
            .iter()
            .map(|occurrence| occurrence.id)
            .collect::<BTreeSet<_>>();
        assert!(
            after
                .occurrences
                .iter()
                .all(|occurrence| !previous.contains(&occurrence.id))
        );
    }
}

#[test]
fn package_configuration_and_synthetic_go_nodes_share_nonrecycling_revision_namespaces() {
    let files = [
        (
            "package.json",
            r#"{"dependencies":{"fixture-library":"^1.0.0"}}"#,
        ),
        ("tsconfig.json", r#"{"compilerOptions":{"strict":true}}"#),
        (
            "Cargo.toml",
            "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nserde='1'\n",
        ),
        ("Cargo.lock", "version = 4\n"),
        (
            "pyproject.toml",
            "[project]\nname='fixture'\ndependencies=['httpx>=0.28']\n",
        ),
        (
            "go.mod",
            "module example.test/fixture\ngo 1.24\nrequire example.test/dependency v1.2.3\n",
        ),
        ("src/lib.rs", "pub fn answer() -> i32 { 42 }\n"),
        ("src/app.ts", "export function answer() { return 42; }\n"),
        (
            "go/math.go",
            "package sample\nfunc Base() int { return 1 }\n",
        ),
        (
            "go/app.go",
            "package sample\nfunc App() int { return Base() }\n",
        ),
    ];
    let original = snapshot(60, BASE_A, &files);
    let deleted = snapshot(61, BASE_A, &[]);
    let restored = snapshot(62, BASE_A, &files);
    let mut builder = indexer();
    let before = update_and_rebuild(&mut builder, &original);
    for name in [
        "fixture-library",
        "serde",
        "httpx>=0.28",
        "example.test/dependency",
    ] {
        assert!(
            before
                .nodes
                .values()
                .any(|node| node.kind == NodeKind::Package && node.name == name),
            "missing package fixture: {name}"
        );
    }
    assert!(before.nodes.values().any(|node| {
        node.kind == NodeKind::Package
            && node.language == Some(Language::Go)
            && node.name == "sample"
            && node.parent.is_none()
    }));
    assert!(
        before
            .nodes
            .values()
            .any(|node| node.kind == NodeKind::Configuration)
    );
    assert!(
        before
            .nodes
            .values()
            .any(|node| node.kind == NodeKind::Lockfile)
    );
    assert!(update_and_rebuild(&mut builder, &deleted).nodes.is_empty());
    let after = update_and_rebuild(&mut builder, &restored);

    assert_eq!(shapes(&before), shapes(&after));
    assert_disjoint_identities(
        &before,
        &after,
        "configuration, manifest packages and synthetic Go package",
    );
    assert_eq!(update_and_rebuild(&mut builder, &original), before);
}

#[test]
fn namespace_is_generation_bound_and_revalidated_against_the_exact_snapshot_context() {
    let files = [("source.rs", "pub fn answer() -> i32 { 42 }\n")];
    let original = snapshot(70, BASE_A, &files);
    let later = snapshot(71, BASE_A, &files);
    let before = indexer().build(&original).unwrap();
    let after = indexer().build(&later).unwrap();
    assert_ne!(before.identity_namespace, after.identity_namespace);

    let mut modified = before.clone();
    modified.identity_namespace = after.identity_namespace;
    assert!(modified.validate_generation().is_err());
    modified.refresh_generation();
    assert!(modified.validate(&original).is_err());
    assert!(before.validate(&later).is_err());

    // Security scope is retained even when revision, base and source agree.
    let mut other_scope = scope();
    other_scope.workspace = WorkspaceId::new("other-identity-workspace").unwrap();
    let other_snapshot = Snapshot::new(
        other_scope,
        original.revision(),
        original.manifest().clone(),
        State::new(),
    )
    .unwrap();
    let other = indexer().build(&other_snapshot).unwrap();
    assert_ne!(before.identity_namespace, other.identity_namespace);
    assert_disjoint_identities(&before, &other, "separate workspace scope");
}
