//! Policy and plan identities do not grant authority. Fixtures here are an
//! explicitly finite reference model, not complete language analysis.
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::snapshot::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use std::collections::{BTreeMap, BTreeSet};

fn d(value: &str) -> Digest {
    Digest::of("source-plan-fixture/v1", value.as_bytes())
}
fn id(value: &str) -> CheckId {
    CheckId::new(value).unwrap()
}
fn element(value: &str) -> ElementId {
    ElementId::new(value).unwrap()
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("organization").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    }
}
fn limits() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 500,
        memory_bytes: 64 * 1024 * 1024,
        disk_bytes: 1024 * 1024,
        process_count: 8,
        file_descriptors: 64,
        stdout_bytes: 4096,
        stderr_bytes: 4096,
        artifact_bytes: 16_384,
    }
}
fn backend() -> BackendIdentity {
    BackendIdentity {
        name: "reference-only-backend".into(),
        version: "v1".into(),
        image: d("image"),
        profile: d("profile"),
    }
}
fn definition(name: &str, dependencies: &[&str]) -> SourceCheckDefinition {
    SourceCheckDefinition::new(SourceCheckInput {
        check: CheckSpec::new(
            id(name),
            vec!["/tools/check".into(), name.into()],
            "fixture-toolchain/v1".into(),
            d("environment"),
            d("configuration"),
        )
        .unwrap(),
        working_directory: WorkingDirectory::WorkspaceRoot,
        inputs: [("src/main.rs".into(), d("main-content"))].into(),
        outputs: ["test-result.json".into()].into(),
        toolchain: ToolchainIdentity {
            executable: "/tools/check".into(),
            executable_digest: d("check-executable"),
            runtime_closure: d("runtime-closure"),
            version: "fixture-toolchain/v1".into(),
            platform: "linux".into(),
            architecture: "x86_64".into(),
        },
        dependency_inputs: [("Cargo.lock".into(), d("lockfile-content"))].into(),
        environment: [("LANG".into(), EnvironmentValue::Literal("C".into()))].into(),
        tier: ExecutionTier::T1,
        backend: backend(),
        resources: limits(),
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
        depends_on: dependencies.iter().map(|name| id(name)).collect(),
        failure_policy: FailurePolicy::FailAttempt,
    })
    .unwrap()
}
fn snapshot() -> Snapshot {
    Snapshot::new(
        scope(),
        Revision(0),
        SnapshotManifest {
            base_commit: "finite-reference-source".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "celetixo/reference-source/v1".into(),
            inputs: vec![WorkspaceInput {
                path: "src/main.rs".into(),
                layer: InputLayer::Base,
                kind: InputKind::File,
                mode: FileMode::Regular,
                bytes: Some(b"fn main() {}\n".to_vec()),
            }],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap()
}
fn policy(checks: Vec<SourceCheckDefinition>) -> SourceExecutionPolicy {
    let snapshot = snapshot();
    SourceExecutionPolicy::new(SourcePolicyInput {
        version: 1,
        generation: 1,
        scope: scope(),
        snapshot: snapshot.id(),
        analysis_artifact: d("analysis"),
        index_generation: d("index"),
        capture: SourceCaptureBinding {
            snapshot: snapshot.id(),
            manifest: snapshot.manifest().digest(),
            capture_policy: d("reference-capture-policy"),
            uncertainty: d("reference-uncertainty"),
            consistency: SourceCaptureConsistency::ReferenceFixture,
            gaps: BTreeSet::new(),
        },
        patch_policy: PatchPolicy::default(),
        credentials: Default::default(),
        paths: [(
            "src/main.rs".into(),
            SourcePathRule {
                allowed_operations: [OperationKind::Replace].into(),
                required_writes: [element("file:main")].into(),
                expected_base: Some(Preimage::of(&snapshot.manifest().inputs[0]).unwrap()),
            },
        )]
        .into(),
        checks,
        invariants: vec![],
        always_required: BTreeSet::new(),
        approved_tiers: [ExecutionTier::T1].into(),
        approved_backends: [backend().digest()].into(),
        resource_ceiling: ResourceLimits {
            wall_time_ms: 5000,
            cpu_time_ms: 2500,
            memory_bytes: 128 * 1024 * 1024,
            disk_bytes: 10 * 1024 * 1024,
            process_count: 16,
            file_descriptors: 128,
            stdout_bytes: 16_384,
            stderr_bytes: 16_384,
            artifact_bytes: 65_536,
        },
        network_allowed: false,
        approved_credential_references: BTreeSet::new(),
        expires_at_ms: 100_000,
        retention_policy: d("retention"),
    })
    .unwrap()
}
fn binding() -> SourcePlanBinding {
    SourcePlanBinding {
        scope: scope(),
        base_snapshot: snapshot().id(),
        analysis_artifact: d("analysis"),
        index_generation: d("index"),
        impact_request: d("impact-request"),
        impact_result: d("impact-result"),
        patch: SourcePatchId(d("patch")),
        operation_digest: d("operations"),
        resulting_snapshot: SnapshotId(d("candidate")),
        resulting_manifest: d("candidate-manifest"),
    }
}
fn requirements(policy: &SourceExecutionPolicy, name: &str) -> PlanRequirements {
    PlanRequirements {
        declared_checks: vec![policy.checks()[&id(name)].input().check.clone()],
        ..Default::default()
    }
}
fn patch(policy: &SourceExecutionPolicy, operation: PatchOperation) -> SourcePatch {
    SourcePatch::new(
        PatchBinding {
            scope: scope(),
            principal: PrincipalId::new("human").unwrap(),
            transaction: TransactionId::new("transaction").unwrap(),
            transaction_attempt: 0,
            source_attempt: SourceAttemptId::new("source-attempt").unwrap(),
            base_snapshot: snapshot().id(),
            base_revision: Revision(0),
            analysis_artifact: d("analysis"),
            index_generation: d("index"),
            impact_request: d("impact-request"),
            impact_result: d("impact-result"),
            created_at_ms: 1,
            expires_at_ms: 100,
            producer: "reference-test-fixture".into(),
        },
        policy.input().patch_policy.clone(),
        vec![operation],
    )
    .unwrap()
}

#[test]
fn planner_unions_mandatory_sources_and_orders_dependency_closure() {
    let policy = policy(vec![
        definition("unit", &["compile"]),
        definition("compile", &["prepare"]),
        definition("prepare", &[]),
        definition("lint", &[]),
        definition("optional", &[]),
    ]);
    let mut input = policy.input().clone();
    input.always_required.insert(id("lint"));
    input.invariants.push(SourceInvariantSpec {
        id: InvariantId::new("behavior").unwrap(),
        scope: [element("file:main")].into(),
        contract_digest: d("behavior-contract"),
        required_checks: [id("unit")].into(),
    });
    let policy = SourceExecutionPolicy::new(input).unwrap();
    let plan = VerificationPlan::resolve(
        &policy,
        binding(),
        PlanRequirements {
            declared_checks: vec![policy.checks()[&id("unit")].input().check.clone()],
            impact_checks: [id("compile")].into(),
            invariants: [InvariantId::new("behavior").unwrap()].into(),
            optional_checks: [id("optional")].into(),
        },
    )
    .unwrap();
    let ordered: Vec<_> = plan
        .checks()
        .iter()
        .map(|check| check.definition().id().as_str())
        .collect();
    assert_eq!(ordered, ["lint", "optional", "prepare", "compile", "unit"]);
    assert!(plan.checks().iter().filter(|c| c.required()).count() == 4);
    let unit = plan.checks().last().unwrap();
    assert!(unit.rationales().contains(&CheckRationale::Declared));
    assert!(unit.rationales().contains(&CheckRationale::Invariant(
        InvariantId::new("behavior").unwrap()
    )));
    assert_eq!(plan.invariant_contracts().len(), 1);
}

#[test]
fn definition_delivery_order_is_irrelevant_and_requirement_provenance_is_bound() {
    let a = policy(vec![
        definition("unit", &["compile"]),
        definition("compile", &[]),
    ]);
    let b = policy(vec![
        definition("compile", &[]),
        definition("unit", &["compile"]),
    ]);
    assert_eq!(a, b);
    let first = VerificationPlan::resolve(&a, binding(), requirements(&a, "unit")).unwrap();
    let second = VerificationPlan::resolve(&b, binding(), requirements(&b, "unit")).unwrap();
    assert_eq!(first, second);
    let mut request = requirements(&a, "unit");
    request.impact_checks.insert(id("unit"));
    let same_checks_new_rationale = VerificationPlan::resolve(&a, binding(), request).unwrap();
    assert_ne!(first.id(), same_checks_new_rationale.id());
    assert_eq!(
        first.checks().len(),
        same_checks_new_rationale.checks().len()
    );
}

#[test]
fn optional_dependency_is_promoted_when_also_required_transitively() {
    let policy = policy(vec![
        definition("unit", &["shared"]),
        definition("optional", &["shared"]),
        definition("shared", &[]),
    ]);
    let mut request = requirements(&policy, "unit");
    request.optional_checks.insert(id("optional"));
    let plan = VerificationPlan::resolve(&policy, binding(), request).unwrap();
    let shared = plan
        .checks()
        .iter()
        .find(|check| check.definition().id() == &id("shared"))
        .unwrap();
    assert!(shared.required());
    assert_eq!(shared.rationales().len(), 2);
}

#[test]
fn missing_substituted_duplicate_and_unsupported_requirements_fail_explicitly() {
    let policy = policy(vec![definition("unit", &[])]);
    let missing = PlanRequirements {
        impact_checks: [id("missing")].into(),
        ..Default::default()
    };
    assert_eq!(
        VerificationPlan::resolve(&policy, binding(), missing),
        Err(SourcePlanError::MissingCheck(id("missing")))
    );
    let mut substituted = requirements(&policy, "unit");
    substituted.declared_checks[0] = CheckSpec::new(
        id("unit"),
        vec!["/tools/other".into()],
        "fixture-toolchain/v1".into(),
        d("environment"),
        d("configuration"),
    )
    .unwrap();
    assert_eq!(
        VerificationPlan::resolve(&policy, binding(), substituted),
        Err(SourcePlanError::SubstitutedCheck(id("unit")))
    );
    let mut duplicate = requirements(&policy, "unit");
    duplicate
        .declared_checks
        .push(duplicate.declared_checks[0].clone());
    assert_eq!(
        VerificationPlan::resolve(&policy, binding(), duplicate),
        Err(SourcePlanError::DuplicateCheck(id("unit")))
    );
    let missing_invariant = PlanRequirements {
        invariants: [InvariantId::new("missing").unwrap()].into(),
        ..Default::default()
    };
    assert_eq!(
        VerificationPlan::resolve(&policy, binding(), missing_invariant),
        Err(SourcePlanError::MissingInvariant(
            InvariantId::new("missing").unwrap()
        ))
    );
    assert!(VerificationPlan::resolve(&policy, binding(), PlanRequirements::default()).is_err());
}

#[test]
fn policy_rejects_dormant_missing_dependencies_cycles_and_duplicate_definitions() {
    let valid = policy(vec![definition("unit", &[])]);
    for (checks, expected) in [
        (
            vec![definition("unit", &["missing"])],
            SourcePlanError::MissingCheck(id("missing")),
        ),
        (
            vec![definition("unit", &["lint"]), definition("lint", &["unit"])],
            SourcePlanError::DependencyCycle,
        ),
        (
            vec![definition("unit", &[]), definition("unit", &[])],
            SourcePlanError::DuplicateCheck(id("unit")),
        ),
    ] {
        let mut input = valid.input().clone();
        input.checks = checks;
        assert_eq!(SourceExecutionPolicy::new(input), Err(expected));
    }
}

#[test]
fn every_resource_dimension_must_fit_and_zero_limits_are_invalid() {
    let valid = policy(vec![definition("unit", &[])]);
    let original = definition("unit", &[]).input().clone();
    for dimension in 0..9 {
        let mut input = original.clone();
        let ceiling = valid.input().resource_ceiling;
        match dimension {
            0 => input.resources.wall_time_ms = ceiling.wall_time_ms + 1,
            1 => input.resources.cpu_time_ms = ceiling.cpu_time_ms + 1,
            2 => input.resources.memory_bytes = ceiling.memory_bytes + 1,
            3 => input.resources.disk_bytes = ceiling.disk_bytes + 1,
            4 => input.resources.process_count = ceiling.process_count + 1,
            5 => input.resources.file_descriptors = ceiling.file_descriptors + 1,
            6 => input.resources.stdout_bytes = ceiling.stdout_bytes + 1,
            7 => input.resources.stderr_bytes = ceiling.stderr_bytes + 1,
            _ => input.resources.artifact_bytes = ceiling.artifact_bytes + 1,
        }
        let mut policy_input = valid.input().clone();
        policy_input.checks = vec![SourceCheckDefinition::new(input).unwrap()];
        assert_eq!(
            SourceExecutionPolicy::new(policy_input),
            Err(SourcePlanError::ResourceCeiling(id("unit")))
        );
    }
    let mut invalid = original;
    invalid.resources.process_count = 0;
    assert!(SourceCheckDefinition::new(invalid).is_err());
}

#[test]
fn command_t0_host_lookup_escaping_paths_and_missing_execution_logs_fail() {
    let original = definition("unit", &[]).input().clone();
    for variant in 0..7 {
        let mut changed = original.clone();
        match variant {
            0 => changed.tier = ExecutionTier::T0,
            1 => changed.toolchain.executable = "check".into(),
            2 => changed.working_directory = WorkingDirectory::Relative("../outside".into()),
            3 => {
                changed.outputs.insert(".git/config".into());
            }
            4 => changed.expected_artifacts.retain(|a| a.name != "stderr"),
            5 => {
                changed.environment.insert(
                    "API_TOKEN".into(),
                    EnvironmentValue::Literal("raw-value".into()),
                );
            }
            _ => changed
                .expected_artifacts
                .push(changed.expected_artifacts[0].clone()),
        };
        assert!(
            SourceCheckDefinition::new(changed).is_err(),
            "variant {variant}"
        );
    }
}

#[test]
fn backend_network_and_credential_grants_cannot_be_widened() {
    let valid = policy(vec![definition("unit", &[])]);
    let original = valid.checks()[&id("unit")].input().clone();
    for variant in 0..4 {
        let mut changed = original.clone();
        match variant {
            0 => changed.tier = ExecutionTier::T2,
            1 => changed.backend.image = d("another-image"),
            2 => {
                changed.network = NetworkPolicy::ScopedAllowlist {
                    destinations: [NetworkDestination {
                        host: "example.test".into(),
                        port: 443,
                    }]
                    .into(),
                    grant: d("grant"),
                    expires_at_ms: 100,
                }
            }
            _ => {
                changed
                    .credential_references
                    .insert("credential-ref://organization/workspace/repository/test".into());
            }
        }
        let mut input = valid.input().clone();
        input.checks = vec![SourceCheckDefinition::new(changed).unwrap()];
        assert!(
            SourceExecutionPolicy::new(input).is_err(),
            "variant {variant}"
        );
    }
    let reference = "credential-ref://organization/workspace/repository/test";
    let mut changed = original;
    changed.environment.insert(
        "API_TOKEN".into(),
        EnvironmentValue::ProtectedReference(reference.into()),
    );
    let mut input = valid.input().clone();
    input.checks = vec![SourceCheckDefinition::new(changed).unwrap()];
    input
        .approved_credential_references
        .insert(reference.into());
    assert!(SourceExecutionPolicy::new(input.clone()).is_ok());
    input.scope.workspace = WorkspaceId::new("foreign").unwrap();
    assert!(SourceExecutionPolicy::new(input).is_err());
}

#[test]
fn live_incomplete_and_relabelled_capture_cannot_become_complete_by_policy() {
    let valid = policy(vec![definition("unit", &[])]);
    valid.validate_snapshot(&snapshot()).unwrap();
    for variant in 0..3 {
        let mut changed = valid.input().clone();
        match variant {
            0 => changed.capture.consistency = SourceCaptureConsistency::LiveObserved,
            1 => {
                changed
                    .capture
                    .gaps
                    .insert("missing generated input".into());
            }
            _ => changed.capture.consistency = SourceCaptureConsistency::ImmutableGitTree,
        }
        match SourceExecutionPolicy::new(changed) {
            Err(SourcePlanError::IncompleteCapture) => {}
            Ok(policy) => assert_eq!(
                policy.validate_snapshot(&snapshot()),
                Err(SourcePlanError::IncompleteCapture)
            ),
            other => panic!("unexpected capture result: {other:?}"),
        }
    }
}

#[test]
fn path_rules_bind_exact_preimages_operations_guards_and_new_paths() {
    let valid = policy(vec![definition("unit", &[])]);
    let base = snapshot();
    let replace = patch(
        &valid,
        PatchOperation::Replace {
            path: "src/main.rs".into(),
            expected: Preimage::of(&base.manifest().inputs[0]).unwrap(),
            entry: PatchEntry::file(
                b"fn main() { println!(\"ok\"); }\n".to_vec(),
                FileMode::Regular,
            )
            .unwrap(),
        },
    );
    valid
        .validate_patch_paths(&replace, &base, &[element("file:main")].into())
        .unwrap();
    assert!(matches!(
        valid.validate_patch_paths(&replace, &base, &BTreeSet::new()),
        Err(SourcePlanError::UnauthorizedPath(_))
    ));
    let create = patch(
        &valid,
        PatchOperation::Create {
            path: "src/new.rs".into(),
            entry: PatchEntry::file(vec![], FileMode::Regular).unwrap(),
        },
    );
    assert!(matches!(
        valid.validate_patch_paths(&create, &base, &[element("package:src")].into()),
        Err(SourcePlanError::UnauthorizedPath(_))
    ));
    let mut input = valid.input().clone();
    input.paths.insert(
        "src/new.rs".into(),
        SourcePathRule {
            allowed_operations: [OperationKind::Create].into(),
            required_writes: [element("package:src")].into(),
            expected_base: None,
        },
    );
    let approved = SourceExecutionPolicy::new(input).unwrap();
    approved
        .validate_patch_paths(&create, &base, &[element("package:src")].into())
        .unwrap();
    let mut input = valid.input().clone();
    input
        .paths
        .get_mut("src/main.rs")
        .unwrap()
        .expected_base
        .as_mut()
        .unwrap()
        .content = d("wrong-content");
    assert!(matches!(
        SourceExecutionPolicy::new(input)
            .unwrap()
            .validate_patch_paths(&replace, &base, &[element("file:main")].into()),
        Err(SourcePlanError::PathPreimage(_))
    ));
}

#[test]
fn patch_policy_cannot_widen_symlinks_or_payload_limits() {
    let valid = policy(vec![definition("unit", &[])]);
    let base = snapshot();
    let ordinary = patch(
        &valid,
        PatchOperation::Delete {
            path: "src/main.rs".into(),
            expected: Preimage::of(&base.manifest().inputs[0]).unwrap(),
        },
    );
    let mut widened = ordinary.policy().clone();
    widened.symlinks = SymlinkPolicy::InternalRelativeOnly;
    let widened = SourcePatch::new(
        ordinary.binding().clone(),
        widened,
        ordinary.operations().to_vec(),
    )
    .unwrap();
    assert_eq!(
        valid.validate_patch_paths(&widened, &base, &[element("file:main")].into()),
        Err(SourcePlanError::PatchPolicyMismatch)
    );
}

#[test]
fn full_check_identity_binds_all_execution_relevant_dimensions() {
    let original = definition("unit", &[]);
    for variant in 0..19 {
        let mut changed = original.input().clone();
        match variant {
            0 => {
                changed.check = CheckSpec::new(
                    id("unit"),
                    vec!["/tools/check".into(), "new-arg".into()],
                    "fixture-toolchain/v1".into(),
                    d("environment"),
                    d("configuration"),
                )
                .unwrap()
            }
            1 => changed.working_directory = WorkingDirectory::Relative("src".into()),
            2 => {
                changed
                    .inputs
                    .insert("src/main.rs".into(), d("changed-source"));
            }
            3 => {
                changed.outputs.insert("another-output".into());
            }
            4 => changed.toolchain.executable_digest = d("new-executable"),
            5 => changed.toolchain.runtime_closure = d("new-runtime"),
            6 => changed.toolchain.platform = "another-platform".into(),
            7 => changed.toolchain.architecture = "aarch64".into(),
            8 => {
                changed
                    .dependency_inputs
                    .insert("Cargo.lock".into(), d("new-lockfile"));
            }
            9 => {
                changed
                    .environment
                    .insert("LANG".into(), EnvironmentValue::Literal("C.UTF-8".into()));
            }
            10 => changed.tier = ExecutionTier::T2,
            11 => changed.backend.version = "v2".into(),
            12 => changed.backend.image = d("new-image"),
            13 => changed.backend.profile = d("new-profile"),
            14 => changed.resources.wall_time_ms += 1,
            15 => {
                changed.network = NetworkPolicy::ScopedAllowlist {
                    destinations: [NetworkDestination {
                        host: "example.test".into(),
                        port: 443,
                    }]
                    .into(),
                    grant: d("network"),
                    expires_at_ms: 100,
                }
            }
            16 => {
                changed
                    .credential_references
                    .insert("credential-ref://organization/workspace/repository/credential".into());
            }
            17 => changed.expected_artifacts[0].max_bytes += 1,
            _ => changed.failure_policy = FailurePolicy::RecordOptionalFailure,
        }
        let changed = SourceCheckDefinition::new(changed).unwrap();
        assert_ne!(original.digest(), changed.digest(), "variant {variant}");
    }
}

#[test]
fn plan_and_cache_identity_bind_result_patch_analysis_policy_and_platform() {
    let policy = policy(vec![definition("unit", &[])]);
    let original =
        VerificationPlan::resolve(&policy, binding(), requirements(&policy, "unit")).unwrap();
    let context = SourceCacheContext {
        platform: "linux".into(),
        architecture: "x86_64".into(),
        external_inputs: [("compiler-support".into(), d("support"))].into(),
    };
    let original_cache = SourceCacheKey::for_check(&original, &id("unit"), &context).unwrap();
    assert!(!original_cache.reuse_enabled());
    for variant in 0..6 {
        let mut changed = binding();
        match variant {
            0 => changed.impact_request = d("changed-request"),
            1 => changed.impact_result = d("changed-impact"),
            2 => changed.patch = SourcePatchId(d("changed-patch")),
            3 => changed.operation_digest = d("changed-operations"),
            4 => changed.resulting_snapshot = SnapshotId(d("changed-result")),
            _ => changed.resulting_manifest = d("changed-manifest"),
        }
        let plan =
            VerificationPlan::resolve(&policy, changed, requirements(&policy, "unit")).unwrap();
        assert_ne!(original.id(), plan.id());
        assert_ne!(
            original_cache,
            SourceCacheKey::for_check(&plan, &id("unit"), &context).unwrap()
        );
    }
    for variant in 0..3 {
        let mut changed = context.clone();
        match variant {
            0 => changed.platform = "other".into(),
            1 => changed.architecture = "aarch64".into(),
            _ => {
                changed
                    .external_inputs
                    .insert("compiler-support".into(), d("changed-support"));
            }
        }
        assert_ne!(
            original_cache,
            SourceCacheKey::for_check(&original, &id("unit"), &changed).unwrap()
        );
    }
    let mut changed = policy.input().clone();
    changed.generation += 1;
    let changed = SourceExecutionPolicy::new(changed).unwrap();
    let new_plan =
        VerificationPlan::resolve(&changed, binding(), requirements(&changed, "unit")).unwrap();
    assert_ne!(original.id(), new_plan.id());
    assert_ne!(
        original_cache,
        SourceCacheKey::for_check(&new_plan, &id("unit"), &context).unwrap()
    );
}

#[test]
fn stale_scope_snapshot_and_analysis_cannot_build_a_plan() {
    let policy = policy(vec![definition("unit", &[])]);
    for variant in 0..4 {
        let mut changed = binding();
        match variant {
            0 => changed.scope.workspace = WorkspaceId::new("foreign").unwrap(),
            1 => changed.base_snapshot = SnapshotId(d("other-snapshot")),
            2 => changed.analysis_artifact = d("other-artifact"),
            _ => changed.index_generation = d("other-index"),
        }
        assert_eq!(
            VerificationPlan::resolve(&policy, changed, requirements(&policy, "unit")),
            Err(SourcePlanError::BindingMismatch)
        );
    }
}

#[test]
fn candidate_input_digests_and_working_directory_are_independently_checked() {
    let initial = policy(vec![definition("unit", &[])]);
    let base = snapshot();
    let accepted_patch = patch(
        &initial,
        PatchOperation::Replace {
            path: "src/main.rs".into(),
            expected: Preimage::of(&base.manifest().inputs[0]).unwrap(),
            entry: PatchEntry::file(b"fn main() { /* changed */ }\n".to_vec(), FileMode::Regular)
                .unwrap(),
        },
    );
    let application = accepted_patch.apply(&base).unwrap();
    let candidate = application.snapshot();
    let content = candidate
        .manifest()
        .inputs
        .iter()
        .find(|entry| entry.path == "src/main.rs")
        .unwrap()
        .bytes
        .as_deref()
        .unwrap();
    let mut check = initial.checks()[&id("unit")].input().clone();
    check.inputs = [(
        "src/main.rs".into(),
        Digest::of("celetixo/source-content/v1", content),
    )]
    .into();
    check.dependency_inputs.clear();
    check.working_directory = WorkingDirectory::Relative("src".into());
    let mut policy_input = initial.input().clone();
    policy_input.checks = vec![SourceCheckDefinition::new(check.clone()).unwrap()];
    let approved = SourceExecutionPolicy::new(policy_input.clone()).unwrap();
    let mut candidate_binding = binding();
    candidate_binding.patch = accepted_patch.id();
    candidate_binding.operation_digest = accepted_patch.operation_digest();
    candidate_binding.resulting_snapshot = candidate.id();
    candidate_binding.resulting_manifest = candidate.manifest().digest();
    let plan = VerificationPlan::resolve(
        &approved,
        candidate_binding.clone(),
        requirements(&approved, "unit"),
    )
    .unwrap();
    plan.validate_resulting_snapshot(candidate).unwrap();
    assert_eq!(
        plan.validate_resulting_snapshot(&base),
        Err(SourcePlanError::BindingMismatch)
    );
    for variant in 0..3 {
        let mut invalid = check.clone();
        match variant {
            0 => {
                invalid.inputs.insert(
                    "src/main.rs".into(),
                    Digest::of(
                        "celetixo/source-content/v1",
                        base.manifest().inputs[0].bytes.as_deref().unwrap(),
                    ),
                );
            }
            1 => {
                invalid
                    .dependency_inputs
                    .insert("missing.lock".into(), d("missing"));
            }
            _ => invalid.working_directory = WorkingDirectory::Relative("missing".into()),
        }
        let mut changed = policy_input.clone();
        changed.checks = vec![SourceCheckDefinition::new(invalid).unwrap()];
        let changed = SourceExecutionPolicy::new(changed).unwrap();
        let plan = VerificationPlan::resolve(
            &changed,
            candidate_binding.clone(),
            requirements(&changed, "unit"),
        )
        .unwrap();
        assert!(matches!(
            plan.validate_resulting_snapshot(candidate),
            Err(SourcePlanError::CheckInputMismatch(_))
        ));
    }
}

#[test]
fn literal_environment_material_is_screened_even_under_an_ordinary_key() {
    let original = definition("unit", &[]);
    for material in [
        format!("{}{}", "ghp_", "x".repeat(36)),
        [
            "postgres://user:",
            "realpassword",
            "@example.invalid/database",
        ]
        .concat(),
        format!("-----BEGIN {}-----\n{}", "PRIVATE KEY", "A".repeat(64)),
    ] {
        let mut input = original.input().clone();
        input.environment.insert(
            "BUILD_OPTION".into(),
            EnvironmentValue::Literal(material.clone()),
        );
        let error = SourceCheckDefinition::new(input).unwrap_err();
        assert!(!error.to_string().contains(&material));
    }
    let valid = policy(vec![original]);
    let mut invalid = valid.input().clone();
    invalid.credentials.version = 2;
    assert!(matches!(
        SourceExecutionPolicy::new(invalid),
        Err(SourcePlanError::Credential(_))
    ));
}

#[test]
fn credential_environment_keys_require_scoped_references_even_without_a_known_value_pattern() {
    let original = definition("unit", &[]);
    for key in [
        "APIKEY",
        "PASSWD",
        "KEY",
        "AWS_ACCESS_KEY",
        "DATABASE_PASSWORD",
        "AUTH_TOKEN",
        "CLIENT_SECRET",
        "SERVICE_CREDENTIAL",
    ] {
        let mut input = original.input().clone();
        input.environment.insert(
            key.into(),
            EnvironmentValue::Literal("opaque-private-value".into()),
        );
        assert_eq!(
            SourceCheckDefinition::new(input),
            Err(SourcePlanError::InvalidCheck("literal environment value")),
            "literal credential value accepted under {key}"
        );
    }
    // Only the shape is admitted here. Policy installation independently checks
    // scope and the explicit grant before this reference can become runnable.
    let mut input = original.input().clone();
    input.environment.insert(
        "APIKEY".into(),
        EnvironmentValue::ProtectedReference(
            "credential-ref://organization/workspace/repository/test".into(),
        ),
    );
    SourceCheckDefinition::new(input).unwrap();
}
