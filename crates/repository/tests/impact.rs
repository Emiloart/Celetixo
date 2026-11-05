#![cfg(unix)]

use celetixo_engine::access::{AccessSet, Condition, Invariant, Predicate};
use celetixo_engine::ids::{
    Digest, ElementId, InvariantId, OrganizationId, RepositoryId, Revision, Scope, WorkspaceId,
};
use celetixo_repository::capture::{CaptureRequest, CaptureResult, capture, capture_git_tree};
use celetixo_repository::impact::*;
use celetixo_repository::index::{IndexConfig, Indexer, NodeKind, SourceIndex};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("impact-org").unwrap(),
        workspace: WorkspaceId::new("impact-workspace").unwrap(),
        repository: RepositoryId::new("impact-repository").unwrap(),
    }
}
fn fixture(
    files: &[(&str, &str)],
    config: IndexConfig,
    immutable: bool,
) -> (CaptureResult, SourceIndex) {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    for (path, text) in files {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    git(root.path(), &["add", "."]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    let request = CaptureRequest::new(root.path(), scope(), Revision(0));
    let captured = if immutable {
        capture_git_tree(&request, &git(root.path(), &["rev-parse", "HEAD"])).unwrap()
    } else {
        capture(&request).unwrap()
    };
    let index = Indexer::new(config)
        .unwrap()
        .build(&captured.snapshot)
        .unwrap();
    (captured, index)
}
const CHAIN: &[(&str, &str)] = &[
    ("math.ts", "export function base() { return 1; }"),
    (
        "service.ts",
        "import {base} from './math'; export function service(){return base();}",
    ),
    (
        "service.test.ts",
        "import {service} from './service'; export function test_service(){return service();}",
    ),
    ("other.ts", "export function other(){return 9;}"),
];
fn symbol(index: &SourceIndex, name: &str) -> ElementId {
    let found: Vec<_> = index
        .symbols(name)
        .into_iter()
        .filter(|n| matches!(n.kind, NodeKind::Function | NodeKind::Method))
        .collect();
    assert_eq!(found.len(), 1, "ambiguous fixture symbol {name}");
    found[0].id.clone()
}
fn request(index: &SourceIndex, changed: ElementId) -> ImpactRequest {
    ImpactRequest {
        snapshot: index.snapshot,
        generation: index.generation,
        changed: [changed].into(),
        transaction_accesses: AccessSet::default(),
        invariants: vec![],
    }
}
fn paths(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn direct_and_transitive_consumers_match_manually_established_reference() {
    let (capture, index) = fixture(CHAIN, IndexConfig::default(), true);
    assert!(capture.report.complete());
    let analyzer = ImpactAnalyzer::new(&capture, &index).unwrap();
    let base = symbol(&index, "base");
    let service = symbol(&index, "service");
    let test = symbol(&index, "test_service");
    let other = symbol(&index, "other");
    let query = request(&index, base.clone());
    let result = analyzer.analyze(&query).unwrap();
    assert_eq!(result.direct_changes, [base.clone()].into());
    assert!(result.direct_consumers.contains(&service));
    assert!(!result.direct_consumers.contains(&test));
    assert!(result.downstream.contains(&service));
    assert!(result.downstream.contains(&test));
    assert!(!result.downstream.contains(&other));
    assert!(!result.downstream.contains(&base));
    let reference = paths(&["math.ts", "service.ts", "service.test.ts"]);
    assert_eq!(result.affected_files, reference);
    let quality = measure_quality(&result.affected_files, &reference);
    assert_eq!(
        (
            quality.true_positives,
            quality.false_positives,
            quality.false_negatives
        ),
        (3, 0, 0)
    );
    assert_eq!((quality.precision, quality.recall), (Some(1.0), Some(1.0)));
    assert!(
        !result.is_complete(),
        "syntax alone cannot clear semantic coverage"
    );
    assert!(
        result
            .verification
            .iter()
            .any(|c| c.kind == VerificationKind::TargetedTest && c.path == "service.test.ts")
    );
    assert!(
        result
            .edges
            .iter()
            .all(|e| e.provenance.snapshot == index.snapshot)
    );
    result.validate_request(&index, &query).unwrap();
    assert_eq!(result, analyzer.analyze(&query).unwrap());
}

#[test]
fn cycles_terminate_without_losing_consumers_or_reclassifying_changed_roots() {
    let (capture, index) = fixture(
        &[(
            "cycle.py",
            "def a():\n    return b()\ndef b():\n    return a()\ndef test_cycle():\n    return b()\n",
        )],
        IndexConfig::default(),
        true,
    );
    let a = symbol(&index, "a");
    let b = symbol(&index, "b");
    let test = symbol(&index, "test_cycle");
    let result = ImpactAnalyzer::new(&capture, &index)
        .unwrap()
        .analyze(&request(&index, a.clone()))
        .unwrap();
    assert!(result.direct_consumers.contains(&b));
    assert!(result.downstream.contains(&b));
    assert!(result.downstream.contains(&test));
    assert!(!result.downstream.contains(&a));
    assert_eq!(result.direct_changes, [a].into());
}

#[test]
fn transaction_reads_writes_predicates_and_invariants_are_reconsidered() {
    let (capture, index) = fixture(CHAIN, IndexConfig::default(), true);
    let service = symbol(&index, "service");
    let test = symbol(&index, "test_service");
    let other = symbol(&index, "other");
    let mut query = request(&index, symbol(&index, "base"));
    query.transaction_accesses = AccessSet {
        reads: [service.clone(), other.clone()].into(),
        writes: [test.clone()].into(),
        predicates: [
            Predicate::Prefix(String::new()),
            Predicate::Prefix("no-matches:".into()),
        ]
        .into(),
        invariant_scopes: [service.clone()].into(),
    };
    query.invariants = vec![
        Invariant {
            id: InvariantId::new("consumer-exists").unwrap(),
            condition: Condition::Present(service.clone()),
        },
        Invariant {
            id: InvariantId::new("unrelated-exists").unwrap(),
            condition: Condition::Present(other),
        },
    ];
    let result = ImpactAnalyzer::new(&capture, &index)
        .unwrap()
        .analyze(&query)
        .unwrap();
    assert_eq!(result.reconsidered_reads, [service].into());
    assert_eq!(result.reconsidered_writes, [test].into());
    assert_eq!(
        result.reconsidered_predicates,
        [Predicate::Prefix(String::new())].into()
    );
    assert_eq!(
        result.reconsidered_invariants,
        [InvariantId::new("consumer-exists").unwrap()].into()
    );
}

#[test]
fn full_query_binding_distinguishes_invariant_conditions_and_empty_predicates() {
    let (capture, index) = fixture(CHAIN, IndexConfig::default(), true);
    let analyzer = ImpactAnalyzer::new(&capture, &index).unwrap();
    let base = symbol(&index, "base");
    let mut first = request(&index, base.clone());
    first.invariants.push(Invariant {
        id: InvariantId::new("value").unwrap(),
        condition: Condition::Equals {
            element: base,
            value: 1,
        },
    });
    let result = analyzer.analyze(&first).unwrap();
    let mut second = first.clone();
    if let Condition::Equals { value, .. } = &mut second.invariants[0].condition {
        *value = 2;
    }
    assert_ne!(first.digest(), second.digest());
    assert_ne!(result.id, analyzer.analyze(&second).unwrap().id);
    assert!(result.validate_request(&index, &second).is_err());
    let mut third = first.clone();
    third
        .transaction_accesses
        .predicates
        .insert(Predicate::Prefix("no-matches:".into()));
    assert_ne!(first.digest(), third.digest());
    assert_ne!(result.id, analyzer.analyze(&third).unwrap().id);
}

#[test]
fn discovered_prerequisites_remain_distinct_from_changed_and_predicted_elements() {
    let mut files = CHAIN.to_vec();
    files.retain(|(path, _)| *path != "other.ts");
    files.extend([
        ("prerequisite.ts", "export function helper(){return 3;}"),
        (
            "other.ts",
            "import {helper} from './prerequisite'; export function other(){return helper();}",
        ),
    ]);
    let (capture, index) = fixture(&files, IndexConfig::default(), true);
    let other = symbol(&index, "other");
    let helper = symbol(&index, "helper");
    let mut query = request(&index, symbol(&index, "base"));
    query.transaction_accesses.reads.insert(other.clone());
    let result = ImpactAnalyzer::new(&capture, &index)
        .unwrap()
        .analyze(&query)
        .unwrap();
    assert!(result.dependencies.contains(&helper));
    assert!(!result.dependencies.contains(&other));
    assert!(result.dependencies.is_disjoint(&result.direct_changes));
    assert!(result.dependencies.is_disjoint(&result.downstream));
    assert!(
        result
            .edges
            .iter()
            .any(|edge| edge.from == other && edge.to == helper)
    );
    assert!(!result.affected_files.contains("other.ts"));
}

#[test]
fn capture_uncertainty_and_unresolved_references_survive_the_engine_contract() {
    let (capture, index) = fixture(
        &[
            ("main.py", "def run(x):\n    return x.unknown()\n"),
            ("other.rb", "puts 'unsupported'\n"),
        ],
        IndexConfig::default(),
        false,
    );
    let analyzer = ImpactAnalyzer::new(&capture, &index).unwrap();
    let result = analyzer
        .analyze(&request(&index, symbol(&index, "run")))
        .unwrap();
    assert!(result.gaps.iter().any(|g| g.stage == "workspace-capture"));
    assert!(
        result
            .gaps
            .iter()
            .any(|g| g.path.as_deref() == Some("other.rb"))
    );
    assert!(
        result
            .gaps
            .iter()
            .any(|g| g.detail.contains("UnresolvedReference"))
    );
    let contract = analyzer.engine_contract(&capture).unwrap();
    assert_eq!(contract.snapshot(), index.snapshot);
    assert_eq!(contract.index_generation(), index.generation);
    assert_eq!(contract.gaps(), &result.gaps);
    assert!(!contract.gaps().is_empty());
}

#[test]
fn package_configuration_and_generated_inputs_propagate_to_consumers_and_checks() {
    let config = IndexConfig {
        generated_inputs: BTreeMap::from([("generated.ts".into(), paths(&["schema.json"]))]),
        ..IndexConfig::default()
    };
    let (capture, index) = fixture(
        &[
            ("schema.json", "{\"version\":1}"),
            ("generated.ts", "export function generated(){return 1;}"),
            (
                "app.ts",
                "import {generated} from './generated'; export function app(){return generated();}",
            ),
            (
                "package.json",
                "{\"name\":\"fixture\",\"dependencies\":{\"example\":\"1.0.0\"}}",
            ),
            ("tsconfig.json", "{\"compilerOptions\":{\"strict\":true}}"),
        ],
        config,
        true,
    );
    let analyzer = ImpactAnalyzer::new(&capture, &index).unwrap();
    let generated = analyzer
        .analyze(&request(&index, index.files["schema.json"].file.clone()))
        .unwrap();
    assert!(generated.affected_files.contains("generated.ts"));
    assert!(generated.affected_files.contains("app.ts"));
    assert!(
        generated
            .verification
            .iter()
            .any(|c| c.kind == VerificationKind::GeneratedInput)
    );
    for path in ["package.json", "tsconfig.json"] {
        let result = analyzer
            .analyze(&request(&index, index.files[path].file.clone()))
            .unwrap();
        assert!(result.affected_files.contains("app.ts"), "{path}");
        assert!(
            result
                .verification
                .iter()
                .any(|c| c.kind == VerificationKind::PackageBuild && c.path == path)
        );
    }
}

#[test]
fn stale_snapshots_generations_unknown_elements_and_tampering_are_rejected() {
    let (capture, index) = fixture(CHAIN, IndexConfig::default(), true);
    let analyzer = ImpactAnalyzer::new(&capture, &index).unwrap();
    let query = request(&index, symbol(&index, "base"));
    let result = analyzer.analyze(&query).unwrap();
    let mut stale = query.clone();
    stale.generation = Digest::of("test", b"stale");
    assert!(matches!(
        analyzer.analyze(&stale),
        Err(ImpactError::StaleRequest)
    ));
    stale = query.clone();
    stale.snapshot.0 = Digest::of("test", b"different-snapshot");
    assert!(matches!(
        analyzer.analyze(&stale),
        Err(ImpactError::StaleRequest)
    ));
    let mut unknown = query.clone();
    unknown.changed.insert(ElementId::new("unknown").unwrap());
    assert!(matches!(
        analyzer.analyze(&unknown),
        Err(ImpactError::UnknownElement(_))
    ));
    let mut altered = result.clone();
    altered.gaps.clear();
    assert!(altered.validate(&index).is_err());
    let mut broken = capture.clone();
    broken.report.issues.clear();
    broken.report.content_bytes += 1;
    assert!(ImpactAnalyzer::new(&broken, &index).is_err());
    let mut wrong_index = index.clone();
    wrong_index.config_digest = Digest::of("config", b"different");
    wrong_index.refresh_generation();
    assert!(result.validate(&wrong_index).is_err());
}

#[test]
fn quality_measures_expose_false_positives_false_negatives_and_undefined_denominators() {
    let measured = measure_quality(&paths(&["a", "b"]), &paths(&["a", "c", "d"]));
    assert_eq!(
        (
            measured.true_positives,
            measured.false_positives,
            measured.false_negatives
        ),
        (1, 1, 2)
    );
    assert_eq!(measured.precision, Some(0.5));
    assert_eq!(measured.recall, Some(1.0 / 3.0));
    let empty = measure_quality(&BTreeSet::new(), &BTreeSet::new());
    assert_eq!((empty.precision, empty.recall), (None, None));
}
