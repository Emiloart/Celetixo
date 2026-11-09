//! Controlled finite reference fixture shared by actual OCI probes and benchmarks.
//! Exact Go bytes and real isolated tools do not assert complete Go semantics.
use celetixo_engine::access::AccessSet;
use celetixo_engine::engine::*;
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::repository::*;
use celetixo_engine::snapshot::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use celetixo_engine::transaction::TransactionIntent;
use celetixo_execution::oci::{OciBackend, OciConfiguration, PinnedExecutable, executable_digest};
use celetixo_execution::workspaces::{WorkspaceLimits, WorkspaceRegistry};
use celetixo_repository::capture::{CaptureRequest, capture_git_tree};
use celetixo_repository::harness::{ConfiguredHarness, HarnessLimits};
use celetixo_repository::impact::ImpactAnalyzer;
use celetixo_repository::index::{IndexConfig, Indexer};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
pub(crate) fn d(s: &str) -> Digest {
    Digest::of("oci-reference-fixture/v1", s.as_bytes())
}
pub(crate) fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("org").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    }
}
pub(crate) fn home() -> PathBuf {
    std::env::var_os("CELETIXO_OCI_HOME")
        .map(PathBuf::from)
        .expect("set CELETIXO_OCI_HOME to reviewed task-local runtime/toolchain directory")
}
pub(crate) fn real_backend() -> OciBackend {
    let home = home();
    let runtime = home.join("runc-1.5.2");
    let formatter = home.join("toolchain/gofmt");
    let launcher = home.join("celetixo-exec-init");
    assert_eq!(
        Digest::from_bytes(Sha256::digest(fs::read(&runtime).unwrap()).into()).to_string(),
        "599f6f94ff8c5057241eff0d54c3c74f95c34935b6457b33fe545defc61e9488"
    );
    assert_eq!(
        Digest::from_bytes(Sha256::digest(fs::read(&formatter).unwrap()).into()).to_string(),
        "0802a3784f7b2a924ccde5b34f6727ffe74e0193a1c0eea4cd773863fce4528a"
    );
    let runs = home.join("runs");
    fs::create_dir_all(&runs).unwrap();
    fs::set_permissions(&runs, fs::Permissions::from_mode(0o700)).unwrap();
    let pin = |path: PathBuf| PinnedExecutable {
        digest: executable_digest(&fs::read(&path).unwrap()),
        path,
    };
    let uid = rustix::process::getuid().as_raw();
    OciBackend::new(OciConfiguration {
        runtime: pin(runtime),
        runtime_version: "1.5.2".into(),
        systemd_inspector: pin(PathBuf::from("/usr/bin/busctl")),
        rootfs_files: [
            ("/celetixo/exec-init".into(), pin(launcher)),
            ("/toolchain/gofmt".into(), pin(formatter)),
            (
                "/lib/x86_64-linux-gnu/libc.so.6".into(),
                pin(home.join("toolchain/lib/libc.so.6")),
            ),
            (
                "/lib64/ld-linux-x86-64.so.2".into(),
                pin(home.join("toolchain/lib/ld-linux-x86-64.so.2")),
            ),
        ]
        .into(),
        private_parent: runs,
        runtime_environment: [
            ("XDG_RUNTIME_DIR".into(), format!("/run/user/{uid}")),
            (
                "DBUS_SESSION_BUS_ADDRESS".into(),
                format!("unix:path=/run/user/{uid}/bus"),
            ),
        ]
        .into(),
    })
    .unwrap()
}

pub(crate) fn workspace_registry() -> WorkspaceRegistry {
    let path = home().join("workspaces");
    match WorkspaceRegistry::create(&path, scope(), WorkspaceLimits::default()) {
        Ok(registry) => registry,
        Err(celetixo_execution::workspaces::WorkspaceError::Io(e))
            if e.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            WorkspaceRegistry::open(&path, scope(), WorkspaceLimits::default()).unwrap()
        }
        Err(error) => panic!("workspace registry: {error}"),
    }
}

pub(crate) fn limits() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 5_000,
        cpu_time_ms: 2_000,
        memory_bytes: 64 * 1024 * 1024,
        disk_bytes: 65536,
        process_count: 32,
        file_descriptors: 64,
        stdout_bytes: 65536,
        stderr_bytes: 65536,
        artifact_bytes: 256 * 1024,
    }
}

pub(crate) struct AdmittedFixture {
    pub(crate) admin: Administrator,
    pub(crate) engine: Engine,
    pub(crate) owner: Session,
    pub(crate) transaction: TransactionId,
    pub(crate) executor: SourceExecutorCapability,
    pub(crate) admission: SourceAdmission,
    pub(crate) definition: SourceCheckDefinition,
    pub(crate) base: Snapshot,
    pub(crate) patch: SourcePatch,
    pub(crate) policy: SourceExecutionPolicy,
}

pub(crate) fn admitted_command(
    backend: &OciBackend,
    candidate: &[u8],
    executable: &str,
    args: &[&str],
    limits: ResourceLimits,
) -> AdmittedFixture {
    let (store, admin) = InMemoryStore::new();
    let engine = Engine::new(store);
    let owner = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            scope(),
            Permission::ALL.into(),
        )
        .unwrap();
    let element = ElementId::new("file:main").unwrap();
    let (check_name, toolchain) = if executable == "/toolchain/gofmt" {
        ("go-syntax", "gofmt/go1.26.2")
    } else {
        ("isolation-probe", "celetixo-exec-init/v1")
    };
    let check = CheckId::new(check_name).unwrap();
    let mut command = vec![executable.into()];
    command.extend(args.iter().map(|s| (*s).to_owned()));
    let definition = SourceCheckDefinition::new(SourceCheckInput {
        check: CheckSpec::new(
            check.clone(),
            command,
            toolchain.into(),
            d("explicit-home"),
            d("go-syntax-config"),
        )
        .unwrap(),
        working_directory: WorkingDirectory::WorkspaceRoot,
        inputs: [(
            "main.go".into(),
            Digest::of("celetixo/source-content/v1", candidate),
        )]
        .into(),
        outputs: BTreeSet::new(),
        toolchain: ToolchainIdentity {
            executable: executable.into(),
            executable_digest: executable_digest(
                &fs::read(home().join(if executable == "/toolchain/gofmt" {
                    "toolchain/gofmt"
                } else {
                    "celetixo-exec-init"
                }))
                .unwrap(),
            ),
            runtime_closure: backend.rootfs_digest(),
            version: toolchain.into(),
            platform: "linux".into(),
            architecture: "x86_64".into(),
        },
        dependency_inputs: BTreeMap::new(),
        environment: [("HOME".into(), EnvironmentValue::Literal("/scratch".into()))].into(),
        tier: ExecutionTier::T1,
        backend: backend.identity().clone(),
        resources: limits,
        network: NetworkPolicy::DenyAll,
        credential_references: BTreeSet::new(),
        expected_artifacts: [
            ("stdout", "text/plain"),
            ("stderr", "text/plain"),
            ("diagnostics", "application/json"),
            ("isolation-record", "application/json"),
        ]
        .into_iter()
        .map(|(name, media)| ArtifactRequirement {
            name: name.into(),
            media_type: media.into(),
            max_bytes: 65536,
            required: true,
        })
        .collect(),
        depends_on: BTreeSet::new(),
        failure_policy: FailurePolicy::FailAttempt,
    })
    .unwrap();
    // This is a real immutable capture, including the repository's selection
    // of exact embedding-approved check definitions. No repository tool runs
    // while creating or inspecting this controlled fixture.
    let root = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(root.path())
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
    };
    git(&["init", "-q"]);
    fs::write(
        root.path().join("main.go"),
        b"package main\n\nfunc main() {}\n",
    )
    .unwrap();
    let configuration = serde_json::to_vec(&serde_json::json!({"version":1,"checks":[{"id":check.as_str(),"definition":definition.digest().to_string()}]})).unwrap();
    fs::write(root.path().join("celetixo.harness.json"), configuration).unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Execution Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "commit.gpgSign=false",
        "commit",
        "-qm",
        "controlled public source fixture",
    ]);
    let oid = git(&["rev-parse", "HEAD"]);
    let captured = capture_git_tree(
        &CaptureRequest::new(root.path(), scope(), Revision(0)),
        &oid,
    )
    .unwrap();
    assert!(captured.report.complete());
    let base = captured.snapshot.clone();
    let harness = ConfiguredHarness::from_snapshot(
        &base,
        "celetixo.harness.json",
        vec![definition.clone()],
        HarnessLimits::default(),
    )
    .unwrap();
    let structural = Indexer::new(IndexConfig::default())
        .unwrap()
        .build(&base)
        .unwrap();
    let structural_contract = ImpactAnalyzer::new(&captured, &structural)
        .unwrap()
        .engine_contract(&captured)
        .unwrap();
    assert!(
        !structural_contract.is_complete(),
        "configuration must not erase actual structural coverage gaps"
    );
    // Deliberately separate, manually established finite reference analysis for
    // these exact controlled bytes/check inputs. This does not promote the
    // actual structural contract above or assert language-wide Go semantics.
    let analysis = RepositoryAnalysis::new(
        &base,
        RepositoryAnalysisInput {
            index_generation: d("manual-finite-fixture-index"),
            analyzer_version: "manual-finite-oci-fixture/v1".into(),
            configuration: d("reviewed-finite-dependency-contract"),
            elements: [element.clone()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element.clone(), [check.clone()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    engine
        .register_repository(&owner, base.clone(), analysis.clone())
        .unwrap();
    let original = effective_workspace(&base)
        .unwrap()
        .remove("main.go")
        .unwrap();
    let policy_input = SourcePolicyInput {
        version: 1,
        generation: 1,
        scope: scope(),
        snapshot: base.id(),
        analysis_artifact: analysis.digest(),
        index_generation: analysis.index_generation(),
        capture: SourceCaptureBinding {
            snapshot: base.id(),
            manifest: base.manifest().digest(),
            capture_policy: captured.report.policy_digest,
            uncertainty: captured.report.uncertainty_digest(),
            consistency: SourceCaptureConsistency::ImmutableGitTree,
            gaps: BTreeSet::new(),
        },
        patch_policy: PatchPolicy::default(),
        credentials: celetixo_engine::credentials::CredentialPolicy::default(),
        paths: [(
            "main.go".into(),
            SourcePathRule {
                allowed_operations: [OperationKind::Replace].into(),
                required_writes: [element.clone()].into(),
                expected_base: Some(Preimage::of(&original).unwrap()),
            },
        )]
        .into(),
        checks: vec![definition.clone()],
        invariants: vec![],
        always_required: [check.clone()].into(),
        approved_tiers: [ExecutionTier::T1].into(),
        approved_backends: [backend.identity().digest()].into(),
        resource_ceiling: limits,
        network_allowed: false,
        approved_credential_references: BTreeSet::new(),
        expires_at_ms: 100_000,
        retention_policy: d("retention"),
    };
    let policy =
        SourceExecutionPolicy::new(harness.apply_to_policy(&base, policy_input).unwrap()).unwrap();
    let definition = policy.checks().get(&check).unwrap().clone();
    engine
        .install_source_policy(&owner, &scope(), None, policy.clone())
        .unwrap();
    let transaction = TransactionId::new("oci-transaction").unwrap();
    engine
        .propose(
            &owner,
            &scope(),
            transaction.clone(),
            base.id(),
            TransactionIntent {
                objective: "finite OCI source verification fixture".into(),
                accesses: AccessSet {
                    writes: [element].into(),
                    ..Default::default()
                },
                invariants: vec![],
                required_checks: vec![definition.input().check.clone()],
                depends_on: BTreeSet::new(),
            },
        )
        .unwrap();
    engine.analyze(&owner, &scope(), &transaction).unwrap();
    engine
        .reserve(&owner, &scope(), &transaction, 100_000)
        .unwrap();
    engine.execute(&owner, &scope(), &transaction).unwrap();
    let binding = engine
        .source_patch_context(&owner, &scope(), &transaction)
        .unwrap()
        .bind(
            SourceAttemptId::new(format!(
                "oci-attempt-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .unwrap(),
            "trusted-reference-fixture".into(),
            30_000,
        )
        .unwrap();
    let patch = SourcePatch::new(
        binding,
        PatchPolicy::default(),
        vec![PatchOperation::Replace {
            path: "main.go".into(),
            expected: Preimage::of(&original).unwrap(),
            entry: PatchEntry::file(candidate.to_vec(), FileMode::Regular).unwrap(),
        }],
    )
    .unwrap();
    let admission = engine
        .admit_source_attempt(
            &owner,
            &scope(),
            patch.clone(),
            IdempotencyKey::new("oci-attempt").unwrap(),
        )
        .unwrap();
    let executor = admin
        .grant_source_executor(
            PrincipalId::new("trusted-oci-executor").unwrap(),
            scope(),
            backend.identity().clone(),
            [ExecutionTier::T1].into(),
            limits,
            100_000,
        )
        .unwrap();
    AdmittedFixture {
        admin,
        engine,
        owner,
        transaction,
        executor,
        admission,
        definition,
        base,
        patch,
        policy,
    }
}
