#![cfg(unix)]

use celetixo_engine::access::AccessSet;
use celetixo_engine::ids::{OrganizationId, RepositoryId, Revision, Scope, WorkspaceId};
use celetixo_repository::capture::{CaptureRequest, capture_git_tree};
use celetixo_repository::impact::{ImpactAnalyzer, ImpactRequest, measure_quality};
use celetixo_repository::index::{IndexConfig, Indexer, IssueKind};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Deserialize)]
struct Reference {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    changed_path: String,
    changed_symbol: String,
    affected_files: BTreeSet<String>,
}

fn materialize(template: &Path, root: &Path, source: &Path) {
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            materialize(template, root, &entry.path());
        } else {
            assert!(entry.file_type().unwrap().is_file());
            let target = root.join(
                entry
                    .path()
                    .strip_prefix(template)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace("__UNIT__", "unit000"),
            );
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(
                target,
                fs::read_to_string(entry.path())
                    .unwrap()
                    .replace("__UNIT__", "unit000"),
            )
            .unwrap();
        }
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Impact reference fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn manual_file_impact_reference_reports_the_known_dynamic_false_negative() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/benchmark");
    let reference: Reference =
        serde_json::from_slice(&fs::read(fixture.join("reference.json")).unwrap()).unwrap();
    assert_eq!(reference.cases.len(), 5);
    let repository = tempfile::tempdir().unwrap();
    let template = fixture.join("template");
    materialize(&template, repository.path(), &template);
    fs::write(repository.path().join("src/lib.rs"), "pub mod unit000;\n").unwrap();
    git(repository.path(), &["init", "-q"]);
    git(repository.path(), &["add", "."]);
    git(
        repository.path(),
        &["commit", "-qm", "manual impact reference v1"],
    );
    let commit = git(repository.path(), &["rev-parse", "HEAD"]);
    let capture = capture_git_tree(
        &CaptureRequest::new(
            repository.path(),
            Scope {
                organization: OrganizationId::new("quality-org").unwrap(),
                workspace: WorkspaceId::new("quality-ws").unwrap(),
                repository: RepositoryId::new("quality-repository").unwrap(),
            },
            Revision(0),
        ),
        &commit,
    )
    .unwrap();
    assert!(capture.report.complete());
    let index = Indexer::new(IndexConfig::default())
        .unwrap()
        .build(&capture.snapshot)
        .unwrap();
    let analyzer = ImpactAnalyzer::new(&capture, &index).unwrap();
    let mut true_positives = 0;
    let mut false_negatives = 0;
    for case in reference.cases {
        let roots = index
            .symbols(&case.changed_symbol)
            .into_iter()
            .filter(|node| node.path == case.changed_path)
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 1, "{}", case.name);
        let result = analyzer
            .analyze(&ImpactRequest {
                snapshot: index.snapshot,
                generation: index.generation,
                changed: BTreeSet::from([roots[0].id.clone()]),
                transaction_accesses: AccessSet::default(),
                invariants: vec![],
            })
            .unwrap();
        result.validate(&index).unwrap();
        assert!(
            !result.is_complete(),
            "structural quality cannot certify completeness: {}",
            case.name
        );
        assert!(
            index
                .issues
                .iter()
                .any(|issue| issue.path == case.changed_path
                    && issue.kind == IssueKind::SemanticAnalysisUnavailable)
        );
        let mut expected_predictions = case.affected_files.clone();
        let expected_misses = if case.name == "python-base-with-dynamic-consumer" {
            assert!(expected_predictions.remove("python/unit000/dynamic.py"));
            1
        } else {
            0
        };
        assert_eq!(result.affected_files, expected_predictions, "{}", case.name);
        let quality = measure_quality(&result.affected_files, &case.affected_files);
        assert_eq!(quality.false_positives, 0, "{}", case.name);
        assert_eq!(quality.false_negatives, expected_misses, "{}", case.name);
        assert_eq!(quality.precision, Some(1.0), "{}", case.name);
        assert_eq!(
            quality.recall,
            Some(if expected_misses == 0 { 1.0 } else { 0.75 }),
            "{}",
            case.name
        );
        true_positives += quality.true_positives;
        false_negatives += quality.false_negatives;
    }
    assert_eq!(true_positives, 15);
    assert_eq!(false_negatives, 1);
}
