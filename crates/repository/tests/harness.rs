//! Repository configuration selects public, finite reference definitions here.
//! These tests do not execute the named toolchains or claim language harnesses.
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::repository::{RepositoryAnalysis, RepositoryAnalysisInput};
use celetixo_engine::snapshot::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use celetixo_repository::harness::*;
use std::collections::{BTreeMap, BTreeSet};

const CONFIG: &str = "celetixo.harness.json";
const ROOTS: [&str; 4] = ["typescript.check", "python.check", "go.check", "rust.check"];

fn d(value: &str) -> Digest {
    Digest::of("harness-reference-fixture/v1", value.as_bytes())
}
fn id(value: &str) -> CheckId {
    CheckId::new(value).unwrap()
}
fn element() -> ElementId {
    ElementId::new("file:main").unwrap()
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("harness-org").unwrap(),
        workspace: WorkspaceId::new("harness-workspace").unwrap(),
        repository: RepositoryId::new("harness-repository").unwrap(),
    }
}
fn resources() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 500,
        memory_bytes: 16 * 1024 * 1024,
        disk_bytes: 1024 * 1024,
        process_count: 8,
        file_descriptors: 64,
        stdout_bytes: 4096,
        stderr_bytes: 4096,
        artifact_bytes: 65_536,
    }
}
fn backend() -> BackendIdentity {
    BackendIdentity {
        name: "reference-only-no-execution".into(),
        version: "v1".into(),
        image: d("image"),
        profile: d("profile"),
    }
}
fn definition(name: &str, dependencies: &[&str]) -> SourceCheckDefinition {
    SourceCheckDefinition::new(SourceCheckInput {
        check: CheckSpec::new(
            id(name),
            vec!["/tools/reference-check".into(), name.into()],
            "reference-tools/v1".into(),
            d("environment"),
            d("configuration"),
        )
        .unwrap(),
        working_directory: WorkingDirectory::WorkspaceRoot,
        inputs: BTreeMap::new(),
        outputs: BTreeSet::new(),
        toolchain: ToolchainIdentity {
            executable: "/tools/reference-check".into(),
            executable_digest: d("executable"),
            runtime_closure: d("runtime"),
            version: "reference-tools/v1".into(),
            platform: "linux".into(),
            architecture: "x86_64".into(),
        },
        dependency_inputs: BTreeMap::new(),
        environment: BTreeMap::new(),
        tier: ExecutionTier::T1,
        backend: backend(),
        resources: resources(),
        network: NetworkPolicy::DenyAll,
        credential_references: BTreeSet::new(),
        expected_artifacts: ["stdout", "stderr", "diagnostics"]
            .into_iter()
            .map(|name| ArtifactRequirement {
                name: name.into(),
                media_type: "text/plain".into(),
                max_bytes: 4096,
                required: true,
            })
            .collect(),
        depends_on: dependencies.iter().map(|value| id(value)).collect(),
        failure_policy: FailurePolicy::FailAttempt,
    })
    .unwrap()
}
fn registry() -> Vec<SourceCheckDefinition> {
    std::iter::once(definition("shared.static", &[]))
        .chain(ROOTS.map(|name| definition(name, &["shared.static"])))
        .collect()
}
fn configuration(checks: &[SourceCheckDefinition], selected: &[&str]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "checks": selected.iter().map(|name| {
            let check = checks.iter().find(|check| check.id() == &id(name)).unwrap();
            serde_json::json!({"id": name, "definition": check.digest().to_string()})
        }).collect::<Vec<_>>()
    }))
    .unwrap()
}
fn snapshot(bytes: &[u8]) -> Snapshot {
    Snapshot::new(
        scope(),
        Revision(0),
        SnapshotManifest {
            base_commit: "finite-harness-reference".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "celetixo/reference-source/harness-v1".into(),
            inputs: [
                WorkspaceInput {
                    path: CONFIG.into(),
                    layer: InputLayer::Base,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(bytes.to_vec()),
                },
                WorkspaceInput {
                    path: "src/main.rs".into(),
                    layer: InputLayer::Base,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(b"fn main() {}\n".to_vec()),
                },
            ]
            .into(),
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap()
}
fn analysis(snapshot: &Snapshot) -> RepositoryAnalysis {
    RepositoryAnalysis::new(
        snapshot,
        RepositoryAnalysisInput {
            index_generation: d("index"),
            analyzer_version: "finite-reference-only/v1".into(),
            configuration: d("analysis-configuration"),
            elements: [element()].into(),
            dependencies: BTreeMap::new(),
            checks: BTreeMap::new(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap()
}
fn policy(snapshot: &Snapshot, checks: Vec<SourceCheckDefinition>) -> SourcePolicyInput {
    let analysis = analysis(snapshot);
    SourcePolicyInput {
        version: 1,
        generation: 1,
        scope: scope(),
        snapshot: snapshot.id(),
        analysis_artifact: analysis.digest(),
        index_generation: analysis.index_generation(),
        capture: SourceCaptureBinding {
            snapshot: snapshot.id(),
            manifest: snapshot.manifest().digest(),
            capture_policy: d("capture-policy"),
            uncertainty: SourceCaptureBinding::empty_uncertainty_digest(),
            consistency: SourceCaptureConsistency::ReferenceFixture,
            gaps: BTreeSet::new(),
        },
        patch_policy: PatchPolicy::default(),
        credentials: Default::default(),
        paths: [(
            CONFIG.into(),
            SourcePathRule {
                allowed_operations: [OperationKind::Replace].into(),
                required_writes: [element()].into(),
                expected_base: Some(
                    Preimage::of(&effective_workspace(snapshot).unwrap()[CONFIG]).unwrap(),
                ),
            },
        )]
        .into(),
        checks,
        invariants: vec![],
        always_required: [id("shared.static")].into(),
        approved_tiers: [ExecutionTier::T1].into(),
        approved_backends: [backend().digest()].into(),
        resource_ceiling: resources(),
        network_allowed: false,
        approved_credential_references: BTreeSet::new(),
        expires_at_ms: 10_000,
        retention_policy: d("retention"),
    }
}
fn load(
    snapshot: &Snapshot,
    checks: Vec<SourceCheckDefinition>,
) -> Result<ConfiguredHarness, HarnessError> {
    ConfiguredHarness::from_snapshot(snapshot, CONFIG, checks, HarnessLimits::default())
}
fn plan_binding(
    base: &Snapshot,
    candidate: &Snapshot,
    policy: &SourcePolicyInput,
) -> SourcePlanBinding {
    SourcePlanBinding {
        scope: scope(),
        base_snapshot: base.id(),
        analysis_artifact: policy.analysis_artifact,
        index_generation: policy.index_generation,
        impact_request: d("impact-request"),
        impact_result: d("impact-result"),
        patch: SourcePatchId(d("patch")),
        operation_digest: d("operations"),
        resulting_snapshot: candidate.id(),
        resulting_manifest: candidate.manifest().digest(),
    }
}

#[test]
fn configuration_selects_approved_roots_and_deterministic_dependency_closure() {
    let checks = registry();
    let base = snapshot(&configuration(&checks, &ROOTS));
    let harness = load(&base, checks.clone()).unwrap();
    assert_eq!(harness.selected_roots(), &ROOTS.map(id).into());
    assert_eq!(
        harness.derived_prerequisites(),
        &[id("shared.static")].into()
    );
    assert_eq!(harness.definitions().len(), 5);
    assert_eq!(harness.definitions()[0].id(), &id("shared.static"));
    let mut reversed = checks.clone();
    reversed.reverse();
    assert_eq!(harness, load(&base, reversed).unwrap());
    for configured in harness.definitions() {
        let approved = checks
            .iter()
            .find(|check| check.id() == configured.id())
            .unwrap();
        assert_eq!(
            configured.input().check.command(),
            approved.input().check.command()
        );
        assert_eq!(configured.input().toolchain, approved.input().toolchain);
        assert_eq!(configured.input().tier, approved.input().tier);
        assert_eq!(configured.input().resources, approved.input().resources);
        assert_eq!(configured.input().network, approved.input().network);
        assert_eq!(
            configured.input().credential_references,
            approved.input().credential_references
        );
        assert_ne!(
            configured.input().check.configuration(),
            approved.input().check.configuration()
        );
        assert_eq!(
            configured.input().inputs[CONFIG],
            harness.configuration_content()
        );
    }
    assert_eq!(base, snapshot(&configuration(&checks, &ROOTS)));
}

#[test]
fn strict_json_rejects_unknown_duplicate_missing_and_unsupported_fields() {
    let check = definition("unit", &[]);
    let digest = check.digest().to_string();
    let valid_selection = format!(r#"{{"id":"unit","definition":"{digest}"}}"#);
    for bytes in [
        format!(r#"{{"version":1,"checks":[{valid_selection}],"command":["/bin/sh"]}}"#),
        format!(r#"{{"version":1,"version":1,"checks":[{valid_selection}]}}"#),
        format!(r#"{{"version":1,"checks":[{valid_selection}],"checks":[]}}"#),
        format!(
            r#"{{"version":1,"checks":[{{"id":"unit","id":"unit","definition":"{digest}"}}]}}"#
        ),
        format!(
            r#"{{"version":1,"checks":[{{"id":"unit","definition":"{digest}","resources":{{}}}}]}}"#
        ),
        r#"{"version":1,"checks":[{"id":"unit"}]}"#.into(),
        r#"{"version":1.0,"checks":[]}"#.into(),
        r#"{"version":1,"checks":{}}"#.into(),
        r#"{"version":1,"checks":[]} trailing"#.into(),
    ] {
        assert!(matches!(
            load(&snapshot(bytes.as_bytes()), vec![check.clone()]),
            Err(HarnessError::InvalidJson { .. })
        ));
    }
    assert!(matches!(
        load(&snapshot(b"\xff"), vec![check.clone()]),
        Err(HarnessError::InvalidJson { .. })
    ));
    assert_eq!(
        load(&snapshot(b"{\"version\":2,\"checks\":[]}"), vec![check]),
        Err(HarnessError::UnsupportedVersion)
    );
}

#[test]
fn configuration_cannot_invent_substitute_or_duplicate_approved_checks() {
    let check = definition("unit", &[]);
    let bytes = configuration(std::slice::from_ref(&check), &["unit"]);
    assert_eq!(
        load(&snapshot(&bytes), vec![]),
        Err(HarnessError::EmptySelection)
    );
    assert_eq!(
        load(&snapshot(&bytes), vec![definition("other", &[])]),
        Err(HarnessError::MissingCheck(id("unit")))
    );
    let mut changed = check.input().clone();
    changed.resources.wall_time_ms += 1;
    assert_eq!(
        load(
            &snapshot(&bytes),
            vec![SourceCheckDefinition::new(changed).unwrap()]
        ),
        Err(HarnessError::DefinitionMismatch(id("unit")))
    );
    assert_eq!(
        load(&snapshot(&bytes), vec![check.clone(), check.clone()]),
        Err(HarnessError::DuplicateCheck(id("unit")))
    );
    let duplicate = configuration(std::slice::from_ref(&check), &["unit", "unit"]);
    assert_eq!(
        load(&snapshot(&duplicate), vec![check.clone()]),
        Err(HarnessError::DuplicateCheck(id("unit")))
    );
    for definition in ["xyz".into(), "A".repeat(64), "0".repeat(63)] {
        let bytes = serde_json::to_vec(
            &serde_json::json!({"version":1,"checks":[{"id":"unit","definition":definition}]}),
        )
        .unwrap();
        assert_eq!(
            load(&snapshot(&bytes), vec![check.clone()]),
            Err(HarnessError::InvalidSelection)
        );
    }
    assert_eq!(
        load(&snapshot(b"{\"version\":1,\"checks\":[]}"), vec![check]),
        Err(HarnessError::EmptySelection)
    );
}

#[test]
fn missing_dependencies_and_cycles_cannot_produce_a_harness() {
    let root = definition("root", &["missing"]);
    let bytes = configuration(std::slice::from_ref(&root), &["root"]);
    assert_eq!(
        load(&snapshot(&bytes), vec![root]),
        Err(HarnessError::MissingCheck(id("missing")))
    );
    let cycle = vec![definition("a", &["b"]), definition("b", &["a"])];
    let bytes = configuration(&cycle, &["a"]);
    assert_eq!(
        load(&snapshot(&bytes), cycle),
        Err(HarnessError::DependencyCycle)
    );
}

#[test]
fn configuration_and_registry_budgets_fail_explicitly() {
    let checks = registry();
    let bytes = configuration(&checks, &ROOTS);
    let base = snapshot(&bytes);
    for (limits, expected) in [
        (
            HarnessLimits {
                max_config_bytes: bytes.len() - 1,
                ..Default::default()
            },
            "configuration bytes",
        ),
        (
            HarnessLimits {
                max_selected_checks: 1,
                ..Default::default()
            },
            "selected checks",
        ),
        (
            HarnessLimits {
                max_registry_checks: 1,
                ..Default::default()
            },
            "registry checks",
        ),
        (
            HarnessLimits {
                max_registry_bytes: 1,
                ..Default::default()
            },
            "registry bytes",
        ),
        (
            HarnessLimits {
                max_dependency_edges: 1,
                ..Default::default()
            },
            "dependency edges",
        ),
        (
            HarnessLimits {
                max_config_bytes: 0,
                ..Default::default()
            },
            "nonzero limits",
        ),
    ] {
        assert_eq!(
            ConfiguredHarness::from_snapshot(&base, CONFIG, checks.clone(), limits),
            Err(HarnessError::Limit(expected))
        );
    }
}

#[test]
fn only_effective_regular_configuration_bytes_are_read() {
    let checks = registry();
    let bytes = configuration(&checks, &ROOTS);
    let base = snapshot(&bytes);
    for path in ["../outside", "/outside", "dir\\config.json", ".git/config"] {
        assert_eq!(
            ConfiguredHarness::from_snapshot(&base, path, checks.clone(), HarnessLimits::default()),
            Err(HarnessError::InvalidPath)
        );
    }
    assert_eq!(
        ConfiguredHarness::from_snapshot(
            &base,
            "absent.json",
            checks.clone(),
            HarnessLimits::default()
        ),
        Err(HarnessError::MissingConfiguration)
    );
    let mut manifest = base.manifest().clone();
    let input = manifest
        .inputs
        .iter_mut()
        .find(|input| input.path == CONFIG)
        .unwrap();
    input.kind = InputKind::Symlink;
    input.mode = FileMode::Symlink;
    let other = Snapshot::new(scope(), Revision(0), manifest, BTreeMap::new()).unwrap();
    assert_eq!(
        load(&other, checks.clone()),
        Err(HarnessError::MissingConfiguration)
    );
    let mut manifest = base.manifest().clone();
    manifest
        .inputs
        .iter_mut()
        .find(|input| input.path == CONFIG)
        .unwrap()
        .mode = FileMode::Executable;
    let executable = Snapshot::new(scope(), Revision(0), manifest, BTreeMap::new()).unwrap();
    assert!(load(&executable, checks.clone()).is_ok());
    let mut manifest = base.manifest().clone();
    manifest.inputs.push(WorkspaceInput {
        path: CONFIG.into(),
        layer: InputLayer::Overlay,
        kind: InputKind::File,
        mode: FileMode::Regular,
        bytes: Some(b"invalid overlay".to_vec()),
    });
    let changed = Snapshot::new(scope(), Revision(0), manifest, BTreeMap::new()).unwrap();
    assert!(matches!(
        load(&changed, checks),
        Err(HarnessError::InvalidJson { .. })
    ));
}

#[test]
fn raw_configuration_bytes_change_harness_check_policy_and_plan_identities() {
    let checks = registry();
    let bytes = configuration(&checks, &ROOTS);
    let base = snapshot(&bytes);
    let mut spaced = bytes.clone();
    spaced.extend_from_slice(b"\n ");
    let other = snapshot(&spaced);
    let first = load(&base, checks.clone()).unwrap();
    let second = load(&other, checks.clone()).unwrap();
    assert_eq!(first.selected_roots(), second.selected_roots());
    assert_ne!(
        first.configuration_content(),
        second.configuration_content()
    );
    assert_ne!(first.digest(), second.digest());
    for (first, second) in first.definitions().iter().zip(second.definitions()) {
        assert_ne!(first.digest(), second.digest());
        assert_ne!(
            first.input().check.configuration(),
            second.input().check.configuration()
        );
    }
    let first_input = first
        .apply_to_policy(&base, policy(&base, checks.clone()))
        .unwrap();
    let second_input = second
        .apply_to_policy(&other, policy(&other, checks))
        .unwrap();
    let first_policy = SourceExecutionPolicy::new(first_input.clone()).unwrap();
    let second_policy = SourceExecutionPolicy::new(second_input.clone()).unwrap();
    assert_ne!(first_policy.digest(), second_policy.digest());
    let first_plan = VerificationPlan::resolve(
        &first_policy,
        plan_binding(&base, &base, &first_input),
        PlanRequirements::default(),
    )
    .unwrap();
    let second_plan = VerificationPlan::resolve(
        &second_policy,
        plan_binding(&other, &other, &second_input),
        PlanRequirements::default(),
    )
    .unwrap();
    assert_ne!(first_plan.id(), second_plan.id());
    first_plan.validate_resulting_snapshot(&base).unwrap();
}

#[test]
fn policy_application_preserves_authority_and_enforces_existing_mandatory_requirements() {
    let checks = registry();
    let base = snapshot(&configuration(&checks, &ROOTS));
    let harness = load(&base, checks.clone()).unwrap();
    let mut input = policy(&base, checks.clone());
    input.invariants.push(SourceInvariantSpec {
        id: InvariantId::new("contract").unwrap(),
        scope: [element()].into(),
        contract_digest: d("contract"),
        required_checks: [id("rust.check")].into(),
    });
    let original = input.clone();
    let configured = harness.apply_to_policy(&base, input).unwrap();
    assert_eq!(configured.capture, original.capture);
    assert_eq!(configured.paths, original.paths);
    assert_eq!(configured.invariants, original.invariants);
    assert_eq!(configured.approved_backends, original.approved_backends);
    assert_eq!(configured.resource_ceiling, original.resource_ceiling);
    assert_eq!(configured.network_allowed, original.network_allowed);
    assert_eq!(
        configured.approved_credential_references,
        original.approved_credential_references
    );
    assert_eq!(
        configured.always_required,
        ROOTS
            .map(id)
            .into_iter()
            .chain([id("shared.static")])
            .collect()
    );
    assert_eq!(
        harness.apply_to_policy(&base, configured.clone()).unwrap(),
        configured
    );
    let limited_base = snapshot(&configuration(&checks, &["go.check"]));
    let limited = load(&limited_base, checks.clone()).unwrap();
    let mut mandatory = policy(&limited_base, checks.clone());
    mandatory.always_required.insert(id("rust.check"));
    assert_eq!(
        limited.apply_to_policy(&limited_base, mandatory),
        Err(HarnessError::MissingMandatory(id("rust.check")))
    );
    let mut mandatory = policy(&limited_base, checks);
    mandatory.invariants = original.invariants;
    assert_eq!(
        limited.apply_to_policy(&limited_base, mandatory),
        Err(HarnessError::MissingMandatory(id("rust.check")))
    );
}

#[test]
fn policy_scope_snapshot_substitutions_and_conflicting_config_inputs_reject() {
    let checks = registry();
    let base = snapshot(&configuration(&checks, &ROOTS));
    let harness = load(&base, checks.clone()).unwrap();
    for variant in 0..3 {
        let mut input = policy(&base, checks.clone());
        match variant {
            0 => input.scope.organization = OrganizationId::new("foreign").unwrap(),
            1 => input.snapshot = SnapshotId(d("foreign-snapshot")),
            _ => input.capture.manifest = d("foreign-manifest"),
        }
        assert_eq!(
            harness.apply_to_policy(&base, input),
            Err(HarnessError::BindingMismatch)
        );
    }
    let mut input = policy(&base, checks.clone());
    let mut changed = input.checks[0].input().clone();
    changed.resources.wall_time_ms -= 1;
    input.checks[0] = SourceCheckDefinition::new(changed).unwrap();
    assert_eq!(
        harness.apply_to_policy(&base, input),
        Err(HarnessError::PolicyDefinitionMismatch(id("shared.static")))
    );
    let mut changed = definition("unit", &[]).input().clone();
    changed.inputs.insert(CONFIG.into(), d("different-input"));
    let changed = SourceCheckDefinition::new(changed).unwrap();
    let base = snapshot(&configuration(std::slice::from_ref(&changed), &["unit"]));
    assert_eq!(
        load(&base, vec![changed]),
        Err(HarnessError::ConflictingConfigurationInput(id("unit")))
    );
}

#[test]
fn configured_plan_retains_declared_impact_and_invariant_check_requirements() {
    let checks = registry();
    let base = snapshot(&configuration(&checks, &ROOTS));
    let harness = load(&base, checks.clone()).unwrap();
    let mut input = policy(&base, checks);
    let invariant = InvariantId::new("source-contract").unwrap();
    input.invariants.push(SourceInvariantSpec {
        id: invariant.clone(),
        scope: [element()].into(),
        contract_digest: d("source-contract"),
        required_checks: [id("rust.check")].into(),
    });
    let configured = harness.apply_to_policy(&base, input).unwrap();
    let installed = SourceExecutionPolicy::new(configured.clone()).unwrap();
    let declared = installed.checks()[&id("go.check")].input().check.clone();
    let plan = VerificationPlan::resolve(
        &installed,
        plan_binding(&base, &base, &configured),
        PlanRequirements {
            declared_checks: vec![declared],
            impact_checks: [id("python.check")].into(),
            invariants: [invariant.clone()].into(),
            optional_checks: BTreeSet::new(),
        },
    )
    .unwrap();
    assert!(plan.checks().iter().all(PlannedCheck::required));
    for (name, rationale) in [
        ("go.check", CheckRationale::Declared),
        ("python.check", CheckRationale::Impact),
        ("rust.check", CheckRationale::Invariant(invariant)),
    ] {
        assert!(
            plan.checks()
                .iter()
                .find(|check| check.definition().id() == &id(name))
                .unwrap()
                .rationales()
                .contains(&rationale)
        );
    }
    plan.validate_resulting_snapshot(&base).unwrap();
}

#[test]
fn candidate_config_changes_fail_exact_plan_inputs_instead_of_reusing_a_harness() {
    let checks = registry();
    let bytes = configuration(&checks, &ROOTS);
    let base = snapshot(&bytes);
    let harness = load(&base, checks.clone()).unwrap();
    let input = harness
        .apply_to_policy(&base, policy(&base, checks))
        .unwrap();
    let installed = SourceExecutionPolicy::new(input.clone()).unwrap();
    let patch = SourcePatch::new(
        PatchBinding {
            scope: scope(),
            principal: PrincipalId::new("human").unwrap(),
            transaction: TransactionId::new("transaction").unwrap(),
            transaction_attempt: 1,
            source_attempt: SourceAttemptId::new("attempt").unwrap(),
            base_snapshot: base.id(),
            base_revision: base.revision(),
            analysis_artifact: input.analysis_artifact,
            index_generation: input.index_generation,
            impact_request: d("request"),
            impact_result: d("impact"),
            producer: "finite harness reference".into(),
            created_at_ms: 0,
            expires_at_ms: 1000,
        },
        PatchPolicy::default(),
        vec![PatchOperation::Replace {
            path: CONFIG.into(),
            expected: Preimage::of(&effective_workspace(&base).unwrap()[CONFIG]).unwrap(),
            entry: PatchEntry::file([bytes, b"\n".to_vec()].concat(), FileMode::Regular).unwrap(),
        }],
    )
    .unwrap();
    let application = patch.apply(&base).unwrap();
    let candidate = application.snapshot();
    let plan = VerificationPlan::resolve(
        &installed,
        plan_binding(&base, candidate, &input),
        PlanRequirements::default(),
    )
    .unwrap();
    assert_eq!(
        plan.validate_resulting_snapshot(candidate),
        Err(SourcePlanError::CheckInputMismatch(CONFIG.into()))
    );
    assert_eq!(
        harness.apply_to_policy(candidate, input),
        Err(HarnessError::BindingMismatch)
    );
    assert_eq!(
        base.id(),
        snapshot(&configuration(&registry(), &ROOTS)).id()
    );
}

#[cfg(unix)]
#[test]
fn immutable_multilanguage_capture_selects_checks_without_erasing_structural_coverage_gaps() {
    use celetixo_engine::access::AccessSet;
    use celetixo_engine::engine::{
        Engine, EngineError, InMemoryStore, Permission, SourceAttemptError,
    };
    use celetixo_engine::transaction::TransactionIntent;
    use celetixo_repository::capture::{CaptureRequest, capture_git_tree};
    use celetixo_repository::impact::ImpactAnalyzer;
    use celetixo_repository::index::{IndexConfig, Indexer};
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
        String::from_utf8(output.stdout).unwrap().trim().into()
    }
    let checks = registry();
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    fs::write(root.path().join(CONFIG), configuration(&checks, &ROOTS)).unwrap();
    for (path, source) in [
        (
            "main.ts",
            "export function answer(): number { return 42; }\n",
        ),
        ("main.py", "def answer():\n    return 42\n"),
        ("main.go", "package main\nfunc answer() int { return 42 }\n"),
        ("main.rs", "fn answer() -> i32 { 42 }\n"),
    ] {
        fs::write(root.path().join(path), source).unwrap();
    }
    git(root.path(), &["add", "."]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Harness Fixture",
            "-c",
            "user.email=harness@example.invalid",
            "commit",
            "-qm",
            "public harness fixture",
        ],
    );
    let oid = git(root.path(), &["rev-parse", "HEAD"]);
    let capture = capture_git_tree(
        &CaptureRequest::new(root.path(), scope(), Revision(0)),
        &oid,
    )
    .unwrap();
    assert!(capture.report.complete());
    let base = &capture.snapshot;
    let harness = load(base, checks.clone()).unwrap();
    let index = Indexer::new(IndexConfig::default())
        .unwrap()
        .build(base)
        .unwrap();
    let analysis = ImpactAnalyzer::new(&capture, &index)
        .unwrap()
        .engine_contract(&capture)
        .unwrap();
    assert!(!analysis.is_complete());
    let guard = analysis.elements().iter().next().unwrap().clone();
    let mut input = policy(base, checks);
    input.analysis_artifact = analysis.digest();
    input.index_generation = analysis.index_generation();
    input.capture = SourceCaptureBinding {
        snapshot: base.id(),
        manifest: base.manifest().digest(),
        capture_policy: capture.report.policy_digest,
        uncertainty: capture.report.uncertainty_digest(),
        consistency: SourceCaptureConsistency::ImmutableGitTree,
        gaps: BTreeSet::new(),
    };
    input.paths.get_mut(CONFIG).unwrap().required_writes = [guard.clone()].into();
    let configured = harness.apply_to_policy(base, input).unwrap();
    let configured = SourceExecutionPolicy::new(configured).unwrap();
    let policy_input = configured.input().clone();
    configured.validate_snapshot(base).unwrap();
    for change_git_reason in [true, false] {
        let mut manifest = base.manifest().clone();
        if change_git_reason {
            manifest
                .inclusion_policy
                .excluded_paths
                .insert(".git".into(), "unreviewed omission".into());
        } else {
            manifest.inclusion_policy.excluded_paths.insert(
                "missing-source.rs".into(),
                "source excluded from execution".into(),
            );
        }
        let changed = Snapshot::new(scope(), Revision(0), manifest, BTreeMap::new()).unwrap();
        let mut rebound = policy_input.clone();
        rebound.snapshot = changed.id();
        rebound.capture.snapshot = changed.id();
        rebound.capture.manifest = changed.manifest().digest();
        // This pure check deliberately retains the old semantic addresses.
        // Even with matching capture addresses, an unknown exclusion cannot
        // pass capture validation; Engine analysis checks are additional gates.
        assert_eq!(
            SourceExecutionPolicy::new(rebound)
                .unwrap()
                .validate_snapshot(&changed),
            Err(SourcePlanError::IncompleteCapture)
        );
    }
    let (store, admin) = InMemoryStore::new();
    let engine = Engine::new(store);
    let owner = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            scope(),
            Permission::ALL.into(),
        )
        .unwrap();
    engine
        .register_repository(&owner, base.clone(), analysis)
        .unwrap();
    assert_eq!(
        engine.install_source_policy(&owner, &scope(), None, configured),
        Err(EngineError::IncompleteAnalysis)
    );
    let tx = TransactionId::new("real-structural-configured-harness").unwrap();
    engine
        .propose(
            &owner,
            &scope(),
            tx.clone(),
            base.id(),
            TransactionIntent {
                objective: "configured checks cannot erase semantic gaps".into(),
                accesses: AccessSet {
                    writes: [guard].into(),
                    ..Default::default()
                },
                invariants: vec![],
                required_checks: vec![policy_input.checks[0].input().check.clone()],
                depends_on: BTreeSet::new(),
            },
        )
        .unwrap();
    assert_eq!(
        engine.analyze(&owner, &scope(), &tx),
        Err(EngineError::IncompleteAnalysis)
    );
    assert!(engine.source_patch_context(&owner, &scope(), &tx).is_err());
    // Incomplete analysis left the transaction Proposed. A direct attempt
    // request cannot skip that failed analysis and enter execution.
    let patch = SourcePatch::new(
        PatchBinding {
            scope: scope(),
            principal: PrincipalId::new("human").unwrap(),
            transaction: tx,
            transaction_attempt: 1,
            source_attempt: SourceAttemptId::new("blocked-real-attempt").unwrap(),
            base_snapshot: base.id(),
            base_revision: base.revision(),
            analysis_artifact: policy_input.analysis_artifact,
            index_generation: policy_input.index_generation,
            impact_request: d("unadmitted-request"),
            impact_result: d("unadmitted-impact"),
            producer: "public fixture".into(),
            created_at_ms: 0,
            expires_at_ms: 1000,
        },
        PatchPolicy::default(),
        vec![PatchOperation::Replace {
            path: CONFIG.into(),
            expected: policy_input.paths[CONFIG].expected_base.unwrap(),
            entry: PatchEntry::file(
                [configuration(&registry(), &ROOTS), b"\n".to_vec()].concat(),
                FileMode::Regular,
            )
            .unwrap(),
        }],
    )
    .unwrap();
    assert_eq!(
        engine.admit_source_attempt(
            &owner,
            &scope(),
            patch,
            IdempotencyKey::new("blocked-real-attempt").unwrap()
        ),
        Err(EngineError::SourceAttempt(SourceAttemptError::InvalidState))
    );
    assert!(
        engine
            .source_attempt(
                &owner,
                &scope(),
                &SourceAttemptId::new("blocked-real-attempt").unwrap()
            )
            .is_err()
    );
    assert_eq!(engine.current(&owner, &scope()).unwrap(), *base);
}
