use celetixo_engine::credentials::CredentialPolicy;
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::persistence::*;
use celetixo_engine::repository::{RepositoryAnalysis, RepositoryAnalysisInput};
use celetixo_engine::snapshot::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use std::collections::{BTreeMap, BTreeSet};

fn d(label: &str) -> Digest {
    Digest::of("source-execution-persistence-test/v1", label.as_bytes())
}
fn check_id(label: &str) -> CheckId {
    CheckId::new(label).unwrap()
}
fn element(label: &str) -> ElementId {
    ElementId::new(label).unwrap()
}
fn invariant_id(label: &str) -> InvariantId {
    InvariantId::new(label).unwrap()
}
fn spec(name: &str) -> CheckSpec {
    CheckSpec::new(
        check_id(name),
        vec![
            "/usr/bin/check".into(),
            "--all".into(),
            "literal\nargument".into(),
        ],
        "toolchain/v1".into(),
        d("environment"),
        d("configuration"),
    )
    .unwrap()
}
fn patch_policy() -> PatchPolicy {
    PatchPolicy {
        version: 1,
        path_profile: PathProfile::PortableAsciiV1,
        symlinks: SymlinkPolicy::InternalRelativeOnly,
        max_operations: 17,
        max_payload_bytes: 4096,
        max_total_payload_bytes: 8192,
        max_result_files: 99,
        max_result_bytes: 32768,
        max_path_bytes: 255,
        max_depth: 8,
    }
}
fn resources() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 500,
        memory_bytes: 1024,
        disk_bytes: 4096,
        process_count: 4,
        file_descriptors: 32,
        stdout_bytes: 128,
        stderr_bytes: 128,
        artifact_bytes: 8192,
    }
}
fn backend() -> BackendIdentity {
    BackendIdentity {
        name: "fixture-backend".into(),
        version: "backend/v1".into(),
        image: d("image"),
        profile: d("profile"),
    }
}
const REFERENCE: &str = "credential-ref://org/workspace/repo/service";
fn definition(name: &str) -> SourceCheckDefinition {
    SourceCheckDefinition::new(SourceCheckInput {
        check: spec(name),
        working_directory: WorkingDirectory::Relative("src".into()),
        inputs: BTreeMap::from([(
            "src/main.py".into(),
            Digest::of("celetixo/source-content/v1", b"print(1)\n"),
        )]),
        outputs: BTreeSet::from(["check.out".into()]),
        toolchain: ToolchainIdentity {
            executable: "/usr/bin/check".into(),
            executable_digest: d("executable"),
            runtime_closure: d("runtime"),
            version: "toolchain/v1".into(),
            platform: "linux".into(),
            architecture: "x86_64".into(),
        },
        dependency_inputs: BTreeMap::from([(
            "src/main.py".into(),
            Digest::of("celetixo/source-content/v1", b"print(1)\n"),
        )]),
        environment: BTreeMap::from([
            ("MODE".into(), EnvironmentValue::Literal("test".into())),
            (
                "SERVICE".into(),
                EnvironmentValue::ProtectedReference(REFERENCE.into()),
            ),
        ]),
        tier: ExecutionTier::T1,
        backend: backend(),
        resources: resources(),
        network: NetworkPolicy::ScopedAllowlist {
            destinations: BTreeSet::from([
                NetworkDestination {
                    host: "example.test".into(),
                    port: 443,
                },
                NetworkDestination {
                    host: "ipv6:1".into(),
                    port: 8443,
                },
            ]),
            grant: d("network"),
            expires_at_ms: 15000,
        },
        credential_references: BTreeSet::from([REFERENCE.into()]),
        expected_artifacts: ["stdout", "stderr", "diagnostics"]
            .into_iter()
            .map(|name| ArtifactRequirement {
                name: name.into(),
                media_type: "text/plain".into(),
                max_bytes: 1024,
                required: true,
            })
            .collect(),
        depends_on: if name == "verify" {
            BTreeSet::from([check_id("prepare")])
        } else {
            BTreeSet::new()
        },
        failure_policy: if name == "optional" {
            FailurePolicy::RecordOptionalFailure
        } else {
            FailurePolicy::FailAttempt
        },
    })
    .unwrap()
}
struct Fixture {
    base: Snapshot,
    analysis: RepositoryAnalysis,
    policy: SourceExecutionPolicy,
    definitions: BTreeMap<Digest, SourceCheckDefinition>,
    binding: SourcePlanBinding,
}
impl Fixture {
    fn new() -> Self {
        let base = Snapshot::new(
            Scope {
                organization: OrganizationId::new("org").unwrap(),
                workspace: WorkspaceId::new("workspace").unwrap(),
                repository: RepositoryId::new("repo").unwrap(),
            },
            Revision(0),
            SnapshotManifest {
                base_commit: "fixture".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "celetixo/reference-source/persistence/v1".into(),
                inputs: vec![WorkspaceInput {
                    path: "src/main.py".into(),
                    layer: InputLayer::Base,
                    kind: InputKind::File,
                    mode: FileMode::Regular,
                    bytes: Some(b"print(1)\n".to_vec()),
                }],
                environment: BTreeMap::new(),
            },
            BTreeMap::new(),
        )
        .unwrap();
        let analysis = RepositoryAnalysis::new(
            &base,
            RepositoryAnalysisInput {
                index_generation: d("index"),
                analyzer_version: "fixture-analyzer/v1".into(),
                configuration: d("analysis-configuration"),
                elements: BTreeSet::from([element("module")]),
                dependencies: BTreeMap::new(),
                checks: BTreeMap::from([(element("module"), BTreeSet::from([check_id("verify")]))]),
                gaps: BTreeSet::new(),
            },
        )
        .unwrap();
        let definitions: [SourceCheckDefinition; 3] = [
            definition("verify"),
            definition("optional"),
            definition("prepare"),
        ];
        let policy = SourceExecutionPolicy::new(SourcePolicyInput {
            version: 1,
            generation: 7,
            scope: base.scope().clone(),
            snapshot: base.id(),
            analysis_artifact: analysis.digest(),
            index_generation: analysis.index_generation(),
            capture: SourceCaptureBinding {
                snapshot: base.id(),
                manifest: base.manifest().digest(),
                capture_policy: d("capture"),
                uncertainty: SourceCaptureBinding::empty_uncertainty_digest(),
                consistency: SourceCaptureConsistency::ReferenceFixture,
                gaps: BTreeSet::new(),
            },
            patch_policy: patch_policy(),
            credentials: CredentialPolicy {
                version: 1,
                reviewed_public_inputs: BTreeMap::from([(
                    "public/example.txt".into(),
                    BTreeSet::from([d("public-a"), d("public-b")]),
                )]),
            },
            paths: BTreeMap::from([
                (
                    "src/main.py".into(),
                    SourcePathRule {
                        allowed_operations: [
                            OperationKind::Create,
                            OperationKind::Replace,
                            OperationKind::Delete,
                            OperationKind::Rename,
                            OperationKind::SetMode,
                        ]
                        .into(),
                        required_writes: BTreeSet::from([element("module")]),
                        expected_base: Some(Preimage {
                            kind: InputKind::File,
                            mode: FileMode::Regular,
                            content: Digest::of("celetixo/source-content/v1", b"print(1)\n"),
                        }),
                    },
                ),
                (
                    "src/new.py".into(),
                    SourcePathRule {
                        allowed_operations: BTreeSet::from([OperationKind::Create]),
                        required_writes: BTreeSet::from([element("module")]),
                        expected_base: None,
                    },
                ),
            ]),
            checks: definitions.to_vec(),
            invariants: vec![SourceInvariantSpec {
                id: invariant_id("module-integrity"),
                scope: BTreeSet::from([element("module")]),
                contract_digest: d("invariant"),
                required_checks: BTreeSet::from([check_id("verify")]),
            }],
            always_required: BTreeSet::from([check_id("verify")]),
            approved_tiers: BTreeSet::from([
                ExecutionTier::T1,
                ExecutionTier::T2,
                ExecutionTier::T3,
            ]),
            approved_backends: BTreeSet::from([backend().digest()]),
            resource_ceiling: resources(),
            network_allowed: true,
            approved_credential_references: BTreeSet::from([REFERENCE.into()]),
            expires_at_ms: 20000,
            retention_policy: d("retention"),
        })
        .unwrap();
        let binding = SourcePlanBinding {
            scope: base.scope().clone(),
            base_snapshot: base.id(),
            analysis_artifact: analysis.digest(),
            index_generation: analysis.index_generation(),
            impact_request: d("impact-request"),
            impact_result: d("impact-result"),
            patch: SourcePatchId(d("patch")),
            operation_digest: d("operations"),
            resulting_snapshot: base.id(),
            resulting_manifest: base.manifest().digest(),
        };
        Self {
            base,
            analysis,
            policy,
            definitions: definitions.into_iter().map(|v| (v.digest(), v)).collect(),
            binding,
        }
    }
    fn requirements(&self) -> PlanRequirements {
        PlanRequirements {
            declared_checks: vec![spec("verify")],
            impact_checks: BTreeSet::from([check_id("verify")]),
            invariants: BTreeSet::from([invariant_id("module-integrity")]),
            optional_checks: BTreeSet::from([check_id("optional")]),
        }
    }
    fn restore_policy(
        &self,
        bytes: &[u8],
        limits: DomainDecodeLimits,
    ) -> Result<SourceExecutionPolicyRecord, RestoreError> {
        restore_source_execution_policy(
            &self.base,
            &self.analysis,
            7,
            &patch_policy(),
            &self.definitions,
            Digest::of(SourceExecutionPolicyRecord::FORMAT, bytes),
            bytes,
            limits,
        )
    }
    fn restore_plan(
        &self,
        bytes: &[u8],
        limits: DomainDecodeLimits,
    ) -> Result<VerificationPlanRecord, RestoreError> {
        restore_verification_plan(
            &self.policy,
            &self.binding,
            self.requirements(),
            SourcePlanId(Digest::of(VerificationPlanRecord::FORMAT, bytes)),
            bytes,
            limits,
        )
    }
}

// Independent frozen layout writer: it uses raw length prefixes and known
// fixture fields, not Canonical or any production persistence encoder. The
// CheckSpec/PatchPolicy digests below were independently calculated with Python
// hashlib/struct over these fields and the published digest framing.
#[derive(Default)]
struct Raw(Vec<u8>);
impl Raw {
    fn b(&mut self, value: &[u8]) -> &mut Self {
        self.0
            .extend_from_slice(&(value.len() as u64).to_be_bytes());
        self.0.extend_from_slice(value);
        self
    }
    fn n(&mut self, value: u64) -> &mut Self {
        self.b(&value.to_be_bytes())
    }
    fn s(&mut self, value: &str) -> &mut Self {
        self.b(value.as_bytes())
    }
    fn d(&mut self, value: Digest) -> &mut Self {
        self.b(value.as_bytes())
    }
}
fn frozen_spec(name: &str) -> Vec<u8> {
    let mut r = Raw::default();
    r.s(name)
        .n(3)
        .s("/usr/bin/check")
        .s("--all")
        .s("literal\nargument")
        .s("toolchain/v1")
        .d(d("environment"))
        .d(d("configuration"));
    r.0
}
fn frozen_patch_policy() -> Vec<u8> {
    let mut r = Raw::default();
    for n in [1, 0, 1, 17, 4096, 8192, 99, 32768, 255, 8] {
        r.n(n);
    }
    r.0
}
fn frozen_definition(name: &str) -> Vec<u8> {
    let mut r = Raw::default();
    r.d(Digest::of(CheckSpecRecord::FORMAT, &frozen_spec(name)))
        .n(1)
        .s("src")
        .n(1)
        .s("src/main.py")
        .d(Digest::of("celetixo/source-content/v1", b"print(1)\n"))
        .n(1)
        .s("check.out")
        .s("/usr/bin/check")
        .d(d("executable"))
        .d(d("runtime"))
        .s("toolchain/v1")
        .s("linux")
        .s("x86_64")
        .n(1)
        .s("src/main.py")
        .d(Digest::of("celetixo/source-content/v1", b"print(1)\n"))
        .n(2)
        .s("MODE")
        .n(0)
        .s("test")
        .s("SERVICE")
        .n(1)
        .s(REFERENCE)
        .n(1)
        .s("fixture-backend")
        .s("backend/v1")
        .d(d("image"))
        .d(d("profile"));
    for n in [1000, 500, 1024, 4096, 4, 32, 128, 128, 8192] {
        r.n(n);
    }
    r.n(1)
        .n(2)
        .s("example.test")
        .n(443)
        .s("ipv6:1")
        .n(8443)
        .d(d("network"))
        .n(15000)
        .n(1)
        .s(REFERENCE)
        .n(3);
    for name in ["diagnostics", "stderr", "stdout"] {
        r.s(name).s("text/plain").n(1024).n(1);
    }
    if name == "verify" {
        r.n(1).s("prepare");
    } else {
        r.n(0);
    }
    r.n(u64::from(name == "optional"));
    r.0
}
fn frozen_policy(f: &Fixture) -> Vec<u8> {
    let mut r = Raw::default();
    r.n(1)
        .n(7)
        .s("org")
        .s("workspace")
        .s("repo")
        .d(f.base.id().0)
        .d(f.analysis.digest())
        .d(d("index"))
        .d(f.base.id().0)
        .d(f.base.manifest().digest())
        .d(d("capture"))
        .d(SourceCaptureBinding::empty_uncertainty_digest())
        .n(1)
        .n(0)
        .d(Digest::of(
            PatchPolicyRecord::FORMAT,
            &frozen_patch_policy(),
        ));
    let mut credentials = Raw::default();
    credentials.n(1).n(1).s("public/example.txt").n(2);
    for digest in BTreeSet::from([d("public-a"), d("public-b")]) {
        credentials.d(digest);
    }
    r.b(&credentials.0).n(2).s("src/main.py").n(5);
    for n in 0..5 {
        r.n(n);
    }
    r.n(1)
        .s("module")
        .n(1)
        .n(0)
        .n(0o100644)
        .d(Digest::of("celetixo/source-content/v1", b"print(1)\n"))
        .s("src/new.py")
        .n(1)
        .n(0)
        .n(1)
        .s("module")
        .n(0)
        .n(3);
    for name in ["optional", "prepare", "verify"] {
        r.d(Digest::of(
            SourceCheckDefinitionRecord::FORMAT,
            &frozen_definition(name),
        ));
    }
    r.n(1)
        .s("module-integrity")
        .n(1)
        .s("module")
        .d(d("invariant"))
        .n(1)
        .s("verify")
        .n(1)
        .s("verify")
        .n(3)
        .n(1)
        .n(2)
        .n(3)
        .n(1)
        .d(backend().digest());
    for n in [1000, 500, 1024, 4096, 4, 32, 128, 128, 8192] {
        r.n(n);
    }
    r.n(1).n(1).s(REFERENCE).n(20000).d(d("retention"));
    r.0
}
fn frozen_plan(f: &Fixture) -> Vec<u8> {
    let mut r = Raw::default();
    r.n(1)
        .d(Digest::of(
            SourceExecutionPolicyRecord::FORMAT,
            &frozen_policy(f),
        ))
        .s("org")
        .s("workspace")
        .s("repo")
        .d(f.base.id().0)
        .d(f.analysis.digest())
        .d(d("index"))
        .d(d("impact-request"))
        .d(d("impact-result"))
        .d(d("patch"))
        .d(d("operations"))
        .d(f.base.id().0)
        .d(f.base.manifest().digest())
        .n(3);
    r.d(Digest::of(
        SourceCheckDefinitionRecord::FORMAT,
        &frozen_definition("optional"),
    ))
    .n(0)
    .n(1)
    .n(5);
    r.d(Digest::of(
        SourceCheckDefinitionRecord::FORMAT,
        &frozen_definition("prepare"),
    ))
    .n(1)
    .n(1)
    .n(4)
    .s("verify");
    r.d(Digest::of(
        SourceCheckDefinitionRecord::FORMAT,
        &frozen_definition("verify"),
    ))
    .n(1)
    .n(4)
    .n(0)
    .n(1)
    .n(2)
    .n(3)
    .s("module-integrity")
    .n(1)
    .s("module-integrity")
    .d(d("invariant"));
    r.0
}
fn fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut pos = 0;
    let mut result = Vec::new();
    while pos < bytes.len() {
        let len = u64::from_be_bytes(bytes[pos..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        result.push(bytes[pos..pos + len].to_vec());
        pos += len;
    }
    result
}
fn encoded(fields: &[Vec<u8>]) -> Vec<u8> {
    let mut r = Raw::default();
    for field in fields {
        r.b(field);
    }
    r.0
}
fn changed_number(bytes: &[u8], index: usize, value: u64) -> Vec<u8> {
    let mut values = fields(bytes);
    values[index] = value.to_be_bytes().to_vec();
    encoded(&values)
}

#[test]
fn independently_encoded_source_contracts_restore_every_field_and_preserve_published_digests() {
    assert_eq!(
        Digest::of(CheckSpecRecord::FORMAT, &frozen_spec("verify")).to_string(),
        "ddf8b33be4455eb683bfd6233c9a4ded9f1a3a2307e38de9167da48817e408bb"
    );
    assert_eq!(
        Digest::of(PatchPolicyRecord::FORMAT, &frozen_patch_policy()).to_string(),
        "af6f270a0e6f80a3e3bc0d6fcd62e8be0e8436d833fb27ea77a94d64da119910"
    );
    let check = restore_check_spec(
        spec("verify").digest(),
        &frozen_spec("verify"),
        DomainDecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(check.check(), &spec("verify"));
    let patch = restore_patch_policy(
        patch_policy().digest(),
        &frozen_patch_policy(),
        DomainDecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(patch.policy(), &patch_policy());
    let f = Fixture::new();
    for name in ["optional", "prepare", "verify"] {
        let expected = definition(name);
        let restored = restore_source_check_definition(
            &spec(name),
            expected.digest(),
            &frozen_definition(name),
            DomainDecodeLimits::default(),
        )
        .unwrap();
        assert_eq!(restored.definition(), &expected);
        assert_eq!(restored.canonical_bytes(), frozen_definition(name));
    }
    let policy = f
        .restore_policy(&frozen_policy(&f), DomainDecodeLimits::default())
        .unwrap();
    assert_eq!(policy.policy(), &f.policy);
    assert_eq!(policy.canonical_bytes(), frozen_policy(&f));
    let plan = f
        .restore_plan(&frozen_plan(&f), DomainDecodeLimits::default())
        .unwrap();
    assert_eq!(
        plan.plan(),
        &VerificationPlan::resolve(&f.policy, f.binding.clone(), f.requirements()).unwrap()
    );
    assert_eq!(plan.canonical_bytes(), frozen_plan(&f));
    plan.plan().validate_resulting_snapshot(&f.base).unwrap();
}

#[test]
fn canonical_source_records_reject_truncation_corruption_trailing_data_and_unknown_tags() {
    let f = Fixture::new();
    for name in ["optional", "prepare", "verify"] {
        let bytes = frozen_definition(name);
        for end in 0..bytes.len() {
            assert!(
                restore_source_check_definition(
                    &spec(name),
                    Digest::of(SourceCheckDefinitionRecord::FORMAT, &bytes[..end]),
                    &bytes[..end],
                    DomainDecodeLimits::default()
                )
                .is_err(),
                "{name} prefix {end}"
            );
        }
    }
    for (format, bytes) in [
        (CheckSpecRecord::FORMAT, frozen_spec("verify")),
        (PatchPolicyRecord::FORMAT, frozen_patch_policy()),
    ] {
        for end in 0..bytes.len() {
            let expected = Digest::of(format, &bytes[..end]);
            assert!(
                if format == CheckSpecRecord::FORMAT {
                    restore_check_spec(expected, &bytes[..end], DomainDecodeLimits::default())
                        .is_err()
                } else {
                    restore_patch_policy(expected, &bytes[..end], DomainDecodeLimits::default())
                        .is_err()
                },
                "{format} prefix {end}"
            );
        }
    }
    for bytes in [frozen_policy(&f), frozen_plan(&f)] {
        for end in 0..bytes.len() {
            assert!(
                if bytes == frozen_policy(&f) {
                    f.restore_policy(&bytes[..end], DomainDecodeLimits::default())
                        .is_err()
                } else {
                    f.restore_plan(&bytes[..end], DomainDecodeLimits::default())
                        .is_err()
                },
                "prefix {end}"
            );
        }
    }
    let mut trailing = frozen_policy(&f);
    trailing.extend_from_slice(&[0]);
    assert_eq!(
        f.restore_policy(&trailing, DomainDecodeLimits::default()),
        Err(RestoreError::NonCanonical)
    );
    let wrong_directory = changed_number(&frozen_definition("verify"), 1, 42);
    assert_eq!(
        restore_source_check_definition(
            &spec("verify"),
            Digest::of(SourceCheckDefinitionRecord::FORMAT, &wrong_directory),
            &wrong_directory,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::Encoding)
    );
    let version = changed_number(&frozen_plan(&f), 0, 2);
    assert_eq!(
        f.restore_plan(&version, DomainDecodeLimits::default()),
        Err(RestoreError::Encoding)
    );
    let invalid_patch = changed_number(&frozen_patch_policy(), 4, 0);
    assert!(
        restore_patch_policy(
            Digest::of(PatchPolicyRecord::FORMAT, &invalid_patch),
            &invalid_patch,
            DomainDecodeLimits::default()
        )
        .is_err()
    );
}

#[test]
fn checked_dependencies_and_external_freshness_reject_substitution() {
    let f = Fixture::new();
    let bytes = frozen_definition("verify");
    assert_eq!(
        restore_source_check_definition(
            &spec("prepare"),
            definition("verify").digest(),
            &bytes,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
    let bytes = frozen_policy(&f);
    assert_eq!(
        restore_source_execution_policy(
            &f.base,
            &f.analysis,
            8,
            &patch_policy(),
            &f.definitions,
            f.policy.digest(),
            &bytes,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
    let mut patch = patch_policy();
    patch.max_operations += 1;
    assert_eq!(
        restore_source_execution_policy(
            &f.base,
            &f.analysis,
            7,
            &patch,
            &f.definitions,
            f.policy.digest(),
            &bytes,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
    let mut definitions = f.definitions.clone();
    definitions.insert(definition("verify").digest(), definition("prepare"));
    assert_eq!(
        restore_source_execution_policy(
            &f.base,
            &f.analysis,
            7,
            &patch_policy(),
            &definitions,
            f.policy.digest(),
            &bytes,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
    let mut binding = f.binding.clone();
    binding.scope.repository = RepositoryId::new("other-repository").unwrap();
    assert_eq!(
        restore_verification_plan(
            &f.policy,
            &binding,
            f.requirements(),
            SourcePlanId(Digest::of(VerificationPlanRecord::FORMAT, &frozen_plan(&f))),
            &frozen_plan(&f),
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
    let mut requirements = f.requirements();
    requirements.impact_checks.clear();
    assert_eq!(
        restore_verification_plan(
            &f.policy,
            &f.binding,
            requirements,
            SourcePlanId(Digest::of(VerificationPlanRecord::FORMAT, &frozen_plan(&f))),
            &frozen_plan(&f),
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::NonCanonical)
    );
}

#[test]
fn rehashed_source_metadata_cannot_weaken_capture_credentials_or_check_requirements() {
    let f = Fixture::new();
    let bytes = frozen_policy(&f);
    let live = changed_number(&bytes, 12, 2);
    assert!(
        f.restore_policy(&live, DomainDecodeLimits::default())
            .is_err()
    );
    let mut values = fields(&bytes);
    values[13] = 1u64.to_be_bytes().to_vec();
    values.insert(14, b"uncaptured input".to_vec());
    assert!(
        f.restore_policy(&encoded(&values), DomainDecodeLimits::default())
            .is_err()
    );
    let mut values = fields(&bytes);
    let mut credentials = fields(&values[15]);
    credentials[5] = credentials[4].clone();
    values[15] = encoded(&credentials);
    assert_eq!(
        f.restore_policy(&encoded(&values), DomainDecodeLimits::default()),
        Err(RestoreError::NonCanonical)
    );
    let mut values = fields(&frozen_definition("verify"));
    let first = values.iter().position(|v| v == b"MODE").unwrap();
    values[first + 1] = 1u64.to_be_bytes().to_vec();
    values[first + 2] = b"arbitrary-placeholder".to_vec();
    let changed = encoded(&values);
    assert!(
        restore_source_check_definition(
            &spec("verify"),
            Digest::of(SourceCheckDefinitionRecord::FORMAT, &changed),
            &changed,
            DomainDecodeLimits::default()
        )
        .is_err()
    );
    let plan = frozen_plan(&f);
    let mut values = fields(&plan);
    let digest = definition("verify").digest();
    let position = values.iter().position(|v| v == digest.as_bytes()).unwrap();
    values[position + 1] = 0u64.to_be_bytes().to_vec();
    assert_eq!(
        f.restore_plan(&encoded(&values), DomainDecodeLimits::default()),
        Err(RestoreError::NonCanonical)
    );
}

#[test]
fn nested_and_referenced_inputs_share_byte_field_and_collection_budgets() {
    let f = Fixture::new();
    let bytes = frozen_policy(&f);
    assert_eq!(
        f.restore_policy(
            &bytes,
            DomainDecodeLimits {
                encoded_bytes: bytes.len(),
                ..DomainDecodeLimits::default()
            }
        ),
        Err(RestoreError::Limit),
        "digest-only references must charge full recovered definitions"
    );
    let mut input = f.policy.input().clone();
    input.credentials.reviewed_public_inputs.insert(
        "public/many.txt".into(),
        (0..80).map(|n| d(&format!("reviewed-{n}"))).collect(),
    );
    let policy = SourceExecutionPolicy::new(input).unwrap();
    assert_eq!(
        f.restore_policy(
            &policy.canonical_bytes(),
            DomainDecodeLimits {
                collection_entries: 60,
                ..DomainDecodeLimits::default()
            }
        ),
        Err(RestoreError::Limit)
    );
    let mut huge = spec("verify").command().to_vec();
    huge.push("x".repeat(200));
    let check = CheckSpec::new(
        check_id("verify"),
        huge,
        "toolchain/v1".into(),
        d("environment"),
        d("configuration"),
    )
    .unwrap();
    let bytes = frozen_definition("verify");
    assert_eq!(
        restore_source_check_definition(
            &check,
            Digest::of(SourceCheckDefinitionRecord::FORMAT, &bytes),
            &bytes,
            DomainDecodeLimits {
                field_bytes: 100,
                ..DomainDecodeLimits::default()
            }
        ),
        Err(RestoreError::Limit),
        "preflight external fields before hashing or cloning a substituted spec"
    );
    let spec_bytes = frozen_spec("verify");
    assert_eq!(
        restore_check_spec(
            Digest::of(CheckSpecRecord::FORMAT, &spec_bytes),
            &spec_bytes,
            DomainDecodeLimits {
                collection_entries: 2,
                ..DomainDecodeLimits::default()
            }
        ),
        Err(RestoreError::Limit)
    );
}

#[test]
fn omitted_mandatory_branches_and_invariant_fanout_are_bounded_before_plan_resolution() {
    let f = Fixture::new();
    let bytes = frozen_plan(&f);
    let mut values = fields(&bytes);
    values[14] = 0u64.to_be_bytes().to_vec();
    values.truncate(15);
    values.push(0u64.to_be_bytes().to_vec());
    let omitted = encoded(&values);
    assert_eq!(
        f.restore_plan(
            &omitted,
            DomainDecodeLimits {
                encoded_bytes: omitted.len() + 200,
                ..DomainDecodeLimits::default()
            }
        ),
        Err(RestoreError::Limit)
    );
    let mut input = f.policy.input().clone();
    input.invariants = (0..50)
        .map(|n| SourceInvariantSpec {
            id: invariant_id(&format!("invariant-{n:03}-{}", "x".repeat(150))),
            scope: BTreeSet::from([element("module")]),
            contract_digest: d("invariant"),
            required_checks: BTreeSet::from([check_id("verify")]),
        })
        .collect();
    let policy = SourceExecutionPolicy::new(input).unwrap();
    let requirements = PlanRequirements {
        invariants: policy.invariants().keys().cloned().collect(),
        ..PlanRequirements::default()
    };
    let mut values = fields(&omitted);
    values[1] = policy.digest().as_bytes().to_vec();
    let omitted = encoded(&values);
    assert_eq!(
        restore_verification_plan(
            &policy,
            &f.binding,
            requirements,
            SourcePlanId(Digest::of(VerificationPlanRecord::FORMAT, &omitted)),
            &omitted,
            DomainDecodeLimits {
                collection_entries: 150,
                ..DomainDecodeLimits::default()
            }
        ),
        Err(RestoreError::Limit)
    );
}
