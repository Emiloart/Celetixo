//! These pure tests establish contract rejection, not sandbox containment.
use celetixo_engine::ids::Digest;
use celetixo_execution::oci::{
    OciBackend, OciConfiguration, OciError, PinnedExecutable, executable_digest, seccomp_profile,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn pin(path: &Path, bytes: &[u8]) -> PinnedExecutable {
    fs::write(path, bytes).unwrap();
    PinnedExecutable {
        path: path.into(),
        digest: executable_digest(bytes),
    }
}

fn configuration(parent: &Path) -> OciConfiguration {
    OciConfiguration {
        runtime: pin(
            &parent.join("runtime"),
            b"trusted test data; never executed",
        ),
        runtime_version: "1.5.2".into(),
        systemd_inspector: pin(&parent.join("inspector"), b"inspector fixture"),
        rootfs_files: [
            (
                "/celetixo/exec-init".into(),
                pin(&parent.join("launcher"), b"launcher fixture"),
            ),
            (
                "/toolchain/gofmt".into(),
                pin(&parent.join("formatter"), b"formatter fixture"),
            ),
        ]
        .into(),
        private_parent: parent.into(),
        runtime_environment: BTreeMap::new(),
    }
}

#[test]
fn missing_or_changed_runtime_never_becomes_an_available_backend() {
    let directory = tempfile::tempdir().unwrap();
    let config = configuration(directory.path());
    fs::remove_file(&config.runtime.path).unwrap();
    assert!(matches!(
        OciBackend::new(config.clone()),
        Err(OciError::Tooling(_))
    ));
    fs::write(&config.runtime.path, b"substituted executable").unwrap();
    assert!(matches!(OciBackend::new(config), Err(OciError::Tooling(_))));
}

#[test]
fn runtime_version_and_rootfs_are_explicit_pins() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = configuration(directory.path());
    config.runtime_version = "1.2.2".into();
    assert!(matches!(
        OciBackend::new(config),
        Err(OciError::Configuration(_))
    ));
    let mut config = configuration(directory.path());
    config.rootfs_files.remove("/celetixo/exec-init");
    assert!(matches!(
        OciBackend::new(config),
        Err(OciError::Configuration(_))
    ));
}

#[test]
fn rootfs_path_traversal_and_reserved_mount_paths_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    for path in [
        "/toolchain/../outside",
        "/toolchain//alias",
        "/src/tool",
        "/scratch/tool",
        "/toolchain/drive:alias",
        "toolchain/relative",
    ] {
        let mut config = configuration(directory.path());
        config
            .rootfs_files
            .insert(path.into(), config.runtime.clone());
        assert!(
            matches!(OciBackend::new(config), Err(OciError::Configuration(_))),
            "{path}"
        );
    }
}

#[test]
fn runtime_environment_cannot_load_code_or_import_credentials() {
    let directory = tempfile::tempdir().unwrap();
    for key in ["LD_PRELOAD", "HOME", "PATH", "GITHUB_TOKEN"] {
        let mut config = configuration(directory.path());
        config
            .runtime_environment
            .insert(key.into(), "untrusted".into());
        assert!(matches!(
            OciBackend::new(config),
            Err(OciError::Configuration(_))
        ));
    }
}

#[test]
fn content_identity_is_independent_of_asset_storage_paths() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let a = OciBackend::new(configuration(first.path())).unwrap();
    let b = OciBackend::new(configuration(second.path())).unwrap();
    assert_eq!(a.identity(), b.identity());
    let mut changed = configuration(second.path());
    changed.rootfs_files.insert(
        "/toolchain/gofmt".into(),
        pin(&second.path().join("formatter"), b"new toolchain"),
    );
    let changed = OciBackend::new(changed).unwrap();
    assert_ne!(a.identity().image, changed.identity().image);
    assert_ne!(a.identity().digest(), changed.identity().digest());
}

#[test]
fn seccomp_policy_is_an_allowlist_with_namespace_clone_restrictions() {
    let profile = seccomp_profile();
    assert_eq!(profile["defaultAction"], "SCMP_ACT_ERRNO");
    let calls = profile["syscalls"].as_array().unwrap();
    let unconditional = calls
        .iter()
        .filter(|c| c["action"] == "SCMP_ACT_ALLOW" && c.get("args").is_none())
        .flat_map(|c| {
            c["names"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|s| s.as_str())
        })
        .collect::<Vec<_>>();
    for forbidden in [
        "mount",
        "unshare",
        "setns",
        "ptrace",
        "bpf",
        "keyctl",
        "socket",
        "connect",
        "io_uring_setup",
        "clone",
    ] {
        assert!(!unconditional.contains(&forbidden), "{forbidden}");
    }
    assert!(
        calls
            .iter()
            .any(|c| c["names"] == serde_json::json!(["clone"])
                && c["args"][0]["value"] == 0x7e020000_u64
                && c["args"][0]["valueTwo"] == 0)
    );
}

#[cfg(unix)]
#[test]
fn tooling_symlinks_are_not_followed() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = configuration(directory.path());
    let alias = directory.path().join("runtime-alias");
    std::os::unix::fs::symlink(&config.runtime.path, &alias).unwrap();
    config.runtime.path = alias;
    assert!(matches!(OciBackend::new(config), Err(OciError::Tooling(_))));
}

#[test]
fn executable_addresses_are_domain_separated() {
    assert_ne!(
        executable_digest(b"same bytes"),
        Digest::of("celetixo/source-content/v1", b"same bytes")
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod support;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod real {
    use super::support::*;
    use super::*;
    use celetixo_engine::engine::*;
    use celetixo_engine::ids::*;
    use celetixo_engine::snapshot::Snapshot;
    use celetixo_engine::source_execution::{ResourceLimits, SourceCheckDefinition};
    use celetixo_execution::materialize::{CleanupOutcome, PrivateWorkspace};
    use celetixo_execution::oci::{ExecutionGuard, GuardDecision};
    use std::path::PathBuf;
    use std::time::Instant;

    struct Fixture {
        admin: Administrator,
        engine: Engine,
        owner: Session,
        transaction: TransactionId,
        executor: SourceExecutorCapability,
        admission: SourceAdmission,
        workspace: PrivateWorkspace,
        definition: SourceCheckDefinition,
        base: Snapshot,
    }
    fn fixture(backend: &OciBackend, candidate: &[u8], args: &[&str]) -> Fixture {
        fixture_command(backend, candidate, "/toolchain/gofmt", args, limits())
    }
    fn fixture_command(
        backend: &OciBackend,
        candidate: &[u8],
        executable: &str,
        args: &[&str],
        limits: ResourceLimits,
    ) -> Fixture {
        let AdmittedFixture {
            admin,
            engine,
            owner,
            transaction,
            executor,
            admission,
            definition,
            base,
            patch,
            policy: _,
        } = admitted_command(backend, candidate, executable, args, limits);
        engine.claim_source_attempt(&executor, &admission).unwrap();
        let workspace = PrivateWorkspace::materialize(&base, &patch).unwrap();
        engine
            .mark_source_patch_applied(&executor, &admission)
            .unwrap();
        engine
            .begin_source_verification(&executor, &admission)
            .unwrap();
        Fixture {
            admin,
            engine,
            owner,
            transaction,
            executor,
            admission,
            workspace,
            definition,
            base,
        }
    }
    struct Clock<'a> {
        admin: &'a Administrator,
        start: Instant,
        previous: u64,
    }
    impl ExecutionGuard for Clock<'_> {
        fn revalidate(&mut self) -> Result<GuardDecision, String> {
            let now = self.start.elapsed().as_millis() as u64;
            self.admin
                .advance_time(now.saturating_sub(self.previous))
                .map_err(|e| e.to_string())?;
            self.previous = now;
            Ok(GuardDecision::Continue)
        }
    }

    fn run(fixture: &mut Fixture, backend: &OciBackend) -> celetixo_execution::oci::OciCheckRecord {
        let mut clock = Clock {
            admin: &fixture.admin,
            start: Instant::now(),
            previous: 0,
        };
        let record = backend
            .execute(
                &fixture.engine,
                &fixture.executor,
                &fixture.admission,
                &mut fixture.workspace,
                &fixture.definition,
                &mut clock,
            )
            .unwrap();
        fixture.workspace.validate_contents().unwrap();
        assert!(record.termination_confirmed, "{record:?}");
        assert_eq!(record.cleanup, CleanupOutcome::Removed, "{record:?}");
        record
    }

    #[test]
    #[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
    fn real_rootless_oci_boundaries_resources_and_reconciliation() {
        use celetixo_engine::source_evidence::SourceCheckOutcome;
        use celetixo_execution::oci::execution_identity;
        let backend = real_backend();
        let sentinel = tempfile::Builder::new()
            .prefix("celetixo-host-sentinel-")
            .tempfile()
            .unwrap();
        fs::write(
            sentinel.path(),
            b"test-only host sentinel; no personal data",
        )
        .unwrap();
        let source = b"package main\n\nfunc main() { println(7) }\n";
        for mode in ["boundaries", "disk", "fds", "pids"] {
            let mut arguments = vec!["--probe", mode];
            if mode == "boundaries" {
                arguments.push(sentinel.path().to_str().unwrap());
            }
            let mut f = fixture_command(
                &backend,
                source,
                "/celetixo/exec-init",
                &arguments,
                limits(),
            );
            let record = run(&mut f, &backend);
            assert_eq!(
                record.outcome,
                SourceCheckOutcome::Exited { code: 0 },
                "mode={mode}; stderr={}; {record:?}",
                String::from_utf8_lossy(&record.stderr)
            );
            assert!(!record.stdout.is_empty(), "mode={mode}");
            let recovery = f
                .admin
                .grant_source_cleanup(
                    PrincipalId::new("recovery-operator").unwrap(),
                    scope(),
                    backend.identity().digest(),
                    100_000,
                )
                .unwrap();
            let stored = backend
                .query_execution(&f.engine, &recovery, &scope(), record.execution)
                .unwrap();
            assert_eq!(stored.record, Some(record.canonical_bytes()));
            assert_eq!(stored.stdout, Some(record.stdout.clone()));
            assert_eq!(stored.stderr, Some(record.stderr.clone()));
            let prior = record.canonical_bytes();
            let mut clock = Clock {
                admin: &f.admin,
                start: Instant::now(),
                previous: 0,
            };
            assert!(
                matches!(backend.execute(&f.engine, &f.executor, &f.admission, &mut f.workspace, &f.definition, &mut clock), Err(OciError::AlreadyClaimed(id)) if id == execution_identity(&f.admission, &f.definition))
            );
            let clean = backend
                .reconcile_execution(&f.engine, &recovery, &scope(), record.execution)
                .unwrap();
            assert!(clean.termination_confirmed);
            assert_eq!(clean.cleanup, CleanupOutcome::Removed);
            assert_eq!(
                backend
                    .query_execution(&f.engine, &recovery, &scope(), record.execution)
                    .unwrap()
                    .record,
                Some(prior)
            );
            f.admin.revoke_source_cleanup(&recovery).unwrap();
            assert!(matches!(
                backend.reconcile_execution(&f.engine, &recovery, &scope(), record.execution),
                Err(OciError::Guard(_))
            ));
        }
        for mode in ["memory", "output", "children", "cpu"] {
            let mut resource = limits();
            if mode == "output" {
                resource.stdout_bytes = 4096;
            }
            if mode == "children" {
                resource.wall_time_ms = 1200;
            }
            if mode == "cpu" {
                resource.wall_time_ms = 8000;
                resource.cpu_time_ms = 1000;
            }
            let mut f = fixture_command(
                &backend,
                source,
                "/celetixo/exec-init",
                &["--probe", mode],
                resource,
            );
            let record = run(&mut f, &backend);
            assert!(
                !matches!(record.outcome, SourceCheckOutcome::Exited { code: 0 }),
                "mode={mode}; {record:?}"
            );
            if mode == "output" {
                assert_eq!(record.outcome, SourceCheckOutcome::OutputLimit);
                assert!(record.stdout_truncated);
            }
            if mode == "children" {
                assert_eq!(record.outcome, SourceCheckOutcome::TimedOut);
                assert!(String::from_utf8_lossy(&record.stdout).contains("child-pid="));
            }
            if mode == "memory" {
                assert!(
                    record
                        .resource_observations
                        .get("memory.events")
                        .is_some_and(|value| value.lines().any(|line| line
                            .strip_prefix("oom_kill ")
                            .is_some_and(|count| count.parse::<u64>().unwrap() > 0))),
                    "OOM observation absent: {record:?}"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
    fn real_rootless_oci_cancellation_stops_children() {
        use celetixo_engine::source_evidence::SourceCheckOutcome;
        let backend = real_backend();
        let mut f = fixture_command(
            &backend,
            b"package main\n\nfunc main() { println(7) }\n",
            "/celetixo/exec-init",
            &["--probe", "children"],
            limits(),
        );
        struct Cancel<'a> {
            clock: Clock<'a>,
            engine: &'a Engine,
            owner: &'a Session,
            transaction: &'a TransactionId,
            done: bool,
        }
        impl ExecutionGuard for Cancel<'_> {
            fn revalidate(&mut self) -> Result<GuardDecision, String> {
                self.clock.revalidate()?;
                if self.clock.start.elapsed().as_millis() > 1200 {
                    if !self.done {
                        self.engine
                            .abort(
                                self.owner,
                                &scope(),
                                self.transaction,
                                "bounded real OCI cancellation probe".into(),
                            )
                            .map_err(|e| e.to_string())?;
                        self.done = true;
                    }
                    return Ok(GuardDecision::Cancelled);
                }
                Ok(GuardDecision::Continue)
            }
        }
        let mut guard = Cancel {
            clock: Clock {
                admin: &f.admin,
                start: Instant::now(),
                previous: 0,
            },
            engine: &f.engine,
            owner: &f.owner,
            transaction: &f.transaction,
            done: false,
        };
        let record = backend
            .execute(
                &f.engine,
                &f.executor,
                &f.admission,
                &mut f.workspace,
                &f.definition,
                &mut guard,
            )
            .unwrap();
        assert_eq!(record.outcome, SourceCheckOutcome::Cancelled, "{record:?}");
        assert!(record.termination_confirmed);
        assert_eq!(record.cleanup, CleanupOutcome::Removed);
        assert!(String::from_utf8_lossy(&record.stdout).contains("child-pid="));
        f.workspace.validate_contents().unwrap();
    }

    fn launched_pid(claim: &Path) -> Option<u32> {
        let journal: serde_json::Value =
            serde_json::from_slice(&fs::read(claim.join("journal.json")).ok()?).ok()?;
        let root = home().join("runs").join(journal["root_name"].as_str()?);
        let pid = fs::read_to_string(root.join("worker.pid"))
            .ok()?
            .trim()
            .parse::<u32>()
            .ok()?;
        let command = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let arguments: Vec<_> = command
            .split(|byte| *byte == 0)
            .filter(|argument| !argument.is_empty())
            .collect();
        (arguments == [b"/celetixo/exec-init".as_slice(), b"--probe", b"children"]).then_some(pid)
    }

    #[test]
    #[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
    fn real_rootless_oci_unwind_retains_and_cleanup_only_grant_recovers() {
        use celetixo_execution::oci::execution_identity;
        let backend = real_backend();
        let mut f = fixture_command(
            &backend,
            b"package main\n\nfunc main() { println(7) }\n",
            "/celetixo/exec-init",
            &["--probe", "children"],
            limits(),
        );
        let execution = execution_identity(&f.admission, &f.definition);
        let claim = home().join("runs").join(format!("execution-{execution}"));
        let source_root = f.workspace.source_path().parent().unwrap().to_path_buf();
        struct Panic<'a> {
            clock: Clock<'a>,
            claim: PathBuf,
        }
        impl ExecutionGuard for Panic<'_> {
            fn revalidate(&mut self) -> Result<GuardDecision, String> {
                self.clock.revalidate()?;
                if launched_pid(&self.claim).is_some() {
                    panic!("controlled supervision unwind after actual command launch");
                }
                Ok(GuardDecision::Continue)
            }
        }
        let mut guard = Panic {
            clock: Clock {
                admin: &f.admin,
                start: Instant::now(),
                previous: 0,
            },
            claim: claim.clone(),
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            backend.execute(
                &f.engine,
                &f.executor,
                &f.admission,
                &mut f.workspace,
                &f.definition,
                &mut guard,
            )
        }));
        assert!(
            result.is_err(),
            "the real command never reached the intended unwind point"
        );
        f.workspace.validate_contents().unwrap();
        drop(f);
        assert!(source_root.join("source/main.go").is_file());
        assert!(source_root.join("execution-retention.json").is_file());
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
        let cleanup = admin
            .grant_source_cleanup(
                PrincipalId::new("fresh-recovery-operator").unwrap(),
                scope(),
                backend.identity().digest(),
                10_000,
            )
            .unwrap();
        let stored = backend
            .query_execution(&engine, &cleanup, &scope(), execution)
            .unwrap();
        assert!(stored.record.is_none());
        assert!(stored.isolation.is_some());
        let recovered = backend
            .reconcile_execution(&engine, &cleanup, &scope(), execution)
            .unwrap();
        assert!(recovered.termination_confirmed, "{recovered:?}");
        assert_eq!(recovered.cleanup, CleanupOutcome::Removed);
        assert!(!source_root.exists());
        assert_eq!(
            backend
                .reconcile_execution(&engine, &cleanup, &scope(), execution)
                .unwrap()
                .cleanup,
            CleanupOutcome::Removed
        );
        assert!(
            backend
                .query_execution(&engine, &cleanup, &scope(), execution)
                .unwrap()
                .record
                .is_none()
        );
    }

    #[test]
    #[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
    fn real_rootless_oci_controller_loss_preserves_independent_deadline() {
        use celetixo_execution::oci::execution_identity;
        let backend = real_backend();
        if let Some(metadata) = std::env::var_os("CELETIXO_OCI_CONTROLLER_CHILD") {
            let mut resources = limits();
            resources.wall_time_ms = 1200;
            let mut f = fixture_command(
                &backend,
                b"package main\n\nfunc main() { println(7) }\n",
                "/celetixo/exec-init",
                &["--probe", "children"],
                resources,
            );
            let execution = execution_identity(&f.admission, &f.definition);
            fs::write(metadata, serde_json::to_vec(&serde_json::json!({"execution":execution.to_string(),"source_root":f.workspace.source_path().parent().unwrap()})).unwrap()).unwrap();
            let _ = run(&mut f, &backend);
            panic!("parent did not inject the intended controller loss");
        }
        let metadata = tempfile::Builder::new()
            .prefix("celetixo-controller-child-")
            .tempfile()
            .unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "real::real_rootless_oci_controller_loss_preserves_independent_deadline",
                "--nocapture",
            ])
            .env("CELETIXO_OCI_CONTROLLER_CHILD", metadata.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let (execution, source_root, claim, journal, pid) = loop {
            assert!(start.elapsed().as_secs() < 10, "real child never launched");
            if let Ok(bytes) = fs::read(metadata.path())
                && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
                && let (Some(id), Some(source)) =
                    (value["execution"].as_str(), value["source_root"].as_str())
            {
                let mut digest = [0_u8; 32];
                assert_eq!(id.len(), 64);
                for (index, byte) in digest.iter_mut().enumerate() {
                    *byte = u8::from_str_radix(&id[index * 2..index * 2 + 2], 16).unwrap();
                }
                let execution = Digest::from_bytes(digest);
                let claim = home().join("runs").join(format!("execution-{execution}"));
                if let Some(pid) = launched_pid(&claim) {
                    let journal: serde_json::Value =
                        serde_json::from_slice(&fs::read(claim.join("journal.json")).unwrap())
                            .unwrap();
                    assert!(claim.join("isolation.json").is_file());
                    break (execution, PathBuf::from(source), claim, journal, pid);
                }
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("child exited before injection: {status}");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        let cid = journal["cid"].as_str().unwrap();
        let uid = rustix::process::getuid().as_raw();
        let group = PathBuf::from(format!(
            "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/user.slice/celetixo-{cid}.scope"
        ));
        let deadline_wait = Instant::now();
        loop {
            let empty = match fs::read_to_string(group.join("cgroup.events")) {
                Ok(events) => events.lines().any(|line| line == "populated 0"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => !group.exists(),
                Err(error) => panic!("scope observation failed: {error}"),
            };
            if empty {
                break;
            }
            assert!(
                deadline_wait.elapsed().as_millis() < 3500,
                "systemd deadline did not kill the orphaned process tree"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let observed_ms = deadline_wait.elapsed().as_millis();
        assert!(source_root.join("execution-retention.json").is_file());
        assert!(source_root.join("source/main.go").is_file());
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
        let cleanup = admin
            .grant_source_cleanup(
                PrincipalId::new("post-loss-recovery").unwrap(),
                scope(),
                backend.identity().digest(),
                10_000,
            )
            .unwrap();
        let stored = backend
            .query_execution(&engine, &cleanup, &scope(), execution)
            .unwrap();
        assert!(stored.record.is_none());
        assert!(stored.isolation.is_some());
        let recovery = backend
            .reconcile_execution(&engine, &cleanup, &scope(), execution)
            .unwrap();
        assert!(recovery.termination_confirmed, "{recovery:?}");
        assert_eq!(recovery.cleanup, CleanupOutcome::Removed);
        assert!(!source_root.exists());
        fs::write(claim.join("controller-loss-proof.json"),serde_json::to_vec(&serde_json::json!({"version":1,"execution":execution.to_string(),"controller_killed":true,"actual_command_pid":pid,"admitted_wall_ms":1200,"observed_orphan_stop_after_controller_loss_ms":observed_ms,"cleanup_only_after_engine_loss":true,"source_retained_until_reconciliation":true})).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "requires reviewed signed runc1.5.2, pinned ELF entrypoint plus explicit loader/libc closure, pinned cached Go1.26.2 formatter and delegated Linux cgroupv2"]
    fn real_rootless_oci_readonly_gofmt_and_intrinsic_authority() {
        use celetixo_engine::source_evidence::SourceCheckOutcome;
        let backend = real_backend();
        let candidate = b"package main\n\nfunc main() { println(1) }\n";
        let mut f = fixture(&backend, candidate, &["-e", "/src/main.go"]);
        let base = f.base.clone();
        let mut clock = Clock {
            admin: &f.admin,
            start: Instant::now(),
            previous: 0,
        };
        let record = backend
            .execute(
                &f.engine,
                &f.executor,
                &f.admission,
                &mut f.workspace,
                &f.definition,
                &mut clock,
            )
            .unwrap();
        fs::write(
            home().join("current-oci-gofmt-record.json"),
            record.canonical_bytes(),
        )
        .unwrap();
        assert_eq!(
            record.outcome,
            SourceCheckOutcome::Exited { code: 0 },
            "{record:?}"
        );
        assert!(record.termination_confirmed);
        assert_eq!(record.cleanup, CleanupOutcome::Removed);
        assert!(!record.stdout_truncated && !record.stderr_truncated);
        assert!(record.stderr.is_empty());
        assert!(record.stdout.starts_with(b"package main\n"));
        assert_eq!(
            record.post_source_manifest,
            Some(f.workspace.report().manifest())
        );
        f.workspace.validate_contents().unwrap();
        assert_eq!(
            celetixo_engine::snapshot::effective_workspace(&base).unwrap()["main.go"]
                .bytes
                .as_deref(),
            Some(&b"package main\n\nfunc main() {}\n"[..])
        );
        let mut revoked = fixture(&backend, candidate, &["-e", "/src/main.go"]);
        revoked
            .admin
            .revoke_source_executor(&revoked.executor)
            .unwrap();
        let mut clock = Clock {
            admin: &revoked.admin,
            start: Instant::now(),
            previous: 0,
        };
        assert!(matches!(
            backend.execute(
                &revoked.engine,
                &revoked.executor,
                &revoked.admission,
                &mut revoked.workspace,
                &revoked.definition,
                &mut clock
            ),
            Err(OciError::Guard(_))
        ));
        revoked.workspace.validate_contents().unwrap();
        let mut invalid = fixture(&backend, b"package main\nfunc {\n", &["-e", "/src/main.go"]);
        let mut clock = Clock {
            admin: &invalid.admin,
            start: Instant::now(),
            previous: 0,
        };
        let record = backend
            .execute(
                &invalid.engine,
                &invalid.executor,
                &invalid.admission,
                &mut invalid.workspace,
                &invalid.definition,
                &mut clock,
            )
            .unwrap();
        assert!(!matches!(
            record.outcome,
            SourceCheckOutcome::Exited { code: 0 }
        ));
        assert!(!record.stderr.is_empty());
        assert!(record.termination_confirmed);
        invalid.workspace.validate_contents().unwrap();
    }

    #[test]
    #[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
    fn real_patch_to_isolated_checks_persisted_evidence_and_engine_admission() {
        use celetixo_engine::source_evidence::{SourceArtifactReader, SourceEvidenceAssessment};
        use celetixo_engine::transaction::TransactionState;
        use celetixo_execution::artifacts::{ArtifactStoreLimits, LocalSourceArtifactStore};
        use celetixo_execution::coordinator::{
            ExecutionStage, MonotonicEngineClock, SourceRunCoordinator, SourceRunRequest,
        };
        use std::sync::Arc;
        let backend = real_backend();
        for (candidate, fault, passed) in [
            (
                b"package main\nfunc answer() int { return 42 }\n".as_slice(),
                SourceEvidenceFault::None,
                true,
            ),
            (
                b"package main\nfunc {\n".as_slice(),
                SourceEvidenceFault::None,
                false,
            ),
            (
                b"package main\nfunc answer() int { return 42 }\n".as_slice(),
                SourceEvidenceFault::AfterRetention,
                true,
            ),
        ] {
            let f = admitted_command(
                &backend,
                candidate,
                "/toolchain/gofmt",
                &["-e", "/src/main.go"],
                limits(),
            );
            let directory = tempfile::tempdir().unwrap();
            let store_path = directory.path().join("immutable-evidence");
            let store_limits = ArtifactStoreLimits {
                max_object_bytes: 1024 * 1024,
                max_total_bytes: 4 * 1024 * 1024,
                max_objects: 256,
            };
            let artifacts = Arc::new(
                LocalSourceArtifactStore::create(
                    &store_path,
                    scope(),
                    f.admission.retention_policy(),
                    store_limits,
                )
                .unwrap(),
            );
            let storage = f
                .admin
                .register_source_artifact_store(scope(), artifacts.clone())
                .unwrap();
            let mut clock = MonotonicEngineClock::new(f.admin.clone());
            let (envelope, bytes) = {
                let coordinator = SourceRunCoordinator {
                    engine: &f.engine,
                    executor: &f.executor,
                    backend: &backend,
                    artifacts: artifacts.as_ref(),
                    storage: &storage,
                };
                let request = || SourceRunRequest {
                    base: &f.base,
                    patch: &f.patch,
                    policy: &f.policy,
                    admission: &f.admission,
                };
                let (receipt, sealed) = if fault == SourceEvidenceFault::None {
                    let result = coordinator.execute(request(), &mut clock).unwrap();
                    assert_eq!(result.cleanup, CleanupOutcome::Removed);
                    (result.receipt, result.sealed)
                } else {
                    let failure = coordinator
                        .execute_with_evidence_fault(request(), &mut clock, fault)
                        .unwrap_err();
                    assert_eq!(failure.stage, ExecutionStage::EvidenceAdmission);
                    assert_eq!(failure.cleanup, Some(CleanupOutcome::Removed));
                    // Response loss does not terminalize or rerun an already
                    // retained result. Redelivery obtains its original receipt.
                    assert!(failure.terminal_recording.is_none());
                    let sealed = failure.sealed.unwrap();
                    (coordinator.deliver(&f.admission, &sealed).unwrap(), sealed)
                };
                assert_eq!(
                    receipt.assessment,
                    if passed {
                        SourceEvidenceAssessment::Passed
                    } else {
                        SourceEvidenceAssessment::Failed {
                            checks: [f.definition.id().clone()].into(),
                        }
                    }
                );
                assert_eq!(coordinator.deliver(&f.admission, &sealed).unwrap(), receipt);
                assert!(coordinator.execute(request(), &mut clock).is_err());
                assert_eq!(f.engine.current(&f.owner, &scope()).unwrap(), f.base);
                assert_eq!(
                    f.engine
                        .source_attempt(&f.owner, &scope(), &f.admission.binding().source_attempt)
                        .unwrap()
                        .state,
                    if passed {
                        SourceAttemptState::EvidenceSealed
                    } else {
                        SourceAttemptState::ExecutionFailed
                    }
                );
                assert_eq!(
                    f.engine
                        .transaction(&f.owner, &scope(), &f.transaction)
                        .unwrap()
                        .state,
                    if passed {
                        TransactionState::Validating
                    } else {
                        TransactionState::Executing
                    }
                );
                assert_eq!(
                    f.engine.commit(
                        &f.owner,
                        &scope(),
                        &f.transaction,
                        IdempotencyKey::new("no-source-publication").unwrap()
                    ),
                    Err(EngineError::SourceMutationUnsupported)
                );
                let raw = &sealed.evidence.checks()[f.definition.id()]
                    .input()
                    .artifacts["isolation-record"];
                let isolation = artifacts.read(&scope(), raw.content, raw.bytes).unwrap();
                let isolation: serde_json::Value = serde_json::from_slice(&isolation).unwrap();
                assert!(isolation["effective_isolation"].is_object());
                (sealed.envelope, sealed.evidence.canonical_bytes())
            };
            let retention = f.admission.retention_policy();
            drop(clock);
            drop(storage);
            drop(f);
            drop(artifacts);
            let reopened =
                LocalSourceArtifactStore::open(&store_path, scope(), retention, store_limits)
                    .unwrap();
            assert_eq!(
                reopened
                    .read(&scope(), envelope.content, envelope.bytes)
                    .unwrap(),
                bytes
            );
        }
    }

    #[test]
    #[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
    fn real_publication_faults_reconcile_without_reexecuting_claimed_checks() {
        use celetixo_engine::source_evidence::SourceCheckOutcome;
        use celetixo_execution::oci::{ExecutionCheckpoint, execution_identity};
        struct Fault<'a> {
            clock: Clock<'a>,
            point: ExecutionCheckpoint,
        }
        impl ExecutionGuard for Fault<'_> {
            fn revalidate(&mut self) -> Result<GuardDecision, String> {
                self.clock.revalidate()
            }
            fn checkpoint(&mut self, point: ExecutionCheckpoint) -> Result<(), String> {
                if point == self.point {
                    Err("controlled publication response loss".into())
                } else {
                    Ok(())
                }
            }
        }
        let backend = real_backend();
        for point in [
            ExecutionCheckpoint::AfterClaimPublication,
            ExecutionCheckpoint::BeforeRecordPublication,
            ExecutionCheckpoint::AfterRecordPublication,
        ] {
            let mut f = fixture(
                &backend,
                b"package main\nfunc answer() int { return 42 }\n",
                &["-e", "/src/main.go"],
            );
            let execution = execution_identity(&f.admission, &f.definition);
            let mut guard = Fault {
                clock: Clock {
                    admin: &f.admin,
                    start: Instant::now(),
                    previous: 0,
                },
                point,
            };
            assert!(matches!(
                backend.execute(
                    &f.engine,
                    &f.executor,
                    &f.admission,
                    &mut f.workspace,
                    &f.definition,
                    &mut guard
                ),
                Err(OciError::Guard(_))
            ));
            let mut clock = Clock {
                admin: &f.admin,
                start: Instant::now(),
                previous: 0,
            };
            assert!(matches!(
                backend.execute(
                    &f.engine,
                    &f.executor,
                    &f.admission,
                    &mut f.workspace,
                    &f.definition,
                    &mut clock
                ),
                Err(OciError::AlreadyClaimed(_))
            ));
            // Drop the interrupted controller's source owner before using the
            // distinct cleanup-only recovery capability.
            drop(f);
            let (store, admin) = InMemoryStore::new();
            let engine = Engine::new(store);
            let cleanup = admin
                .grant_source_cleanup(
                    PrincipalId::new("publication-recovery").unwrap(),
                    scope(),
                    backend.identity().digest(),
                    10_000,
                )
                .unwrap();
            let recovered = backend
                .reconcile_execution(&engine, &cleanup, &scope(), execution)
                .unwrap();
            assert!(recovered.termination_confirmed, "{recovered:?}");
            assert_eq!(recovered.cleanup, CleanupOutcome::Removed);
            let record = backend
                .recover_execution_record(&engine, &cleanup, &scope(), execution)
                .unwrap();
            assert_eq!(
                record.is_some(),
                point == ExecutionCheckpoint::AfterRecordPublication
            );
            if let Some(record) = record {
                assert_eq!(record.outcome, SourceCheckOutcome::Exited { code: 0 });
                assert!(record.termination_confirmed);
            }
        }
    }

    #[test]
    #[ignore = "requires reviewed pinned OCI tooling and delegated Linux cgroupv2"]
    fn real_readiness_waits_for_pid_publication_and_missing_pid_cannot_run_a_check() {
        use celetixo_engine::source_evidence::SourceCheckOutcome;
        use celetixo_execution::oci::{ExecutionCheckpoint, execution_identity};
        struct Delay<'a> {
            clock: Clock<'a>,
            claim: PathBuf,
            held: Option<(PathBuf, PathBuf)>,
            withheld: bool,
            restored: bool,
            remaining: u8,
            forever: bool,
        }
        impl ExecutionGuard for Delay<'_> {
            fn revalidate(&mut self) -> Result<GuardDecision, String> {
                self.clock.revalidate()
            }
            fn checkpoint(&mut self, point: ExecutionCheckpoint) -> Result<(), String> {
                if point != ExecutionCheckpoint::BeforeControlValidation {
                    return Ok(());
                }
                if let Some((marker, held)) = self.held.as_ref() {
                    if !self.forever && self.remaining == 0 {
                        fs::rename(held, marker).map_err(|e| e.to_string())?;
                        self.held = None;
                        self.restored = true;
                    } else {
                        self.remaining = self.remaining.saturating_sub(1);
                    }
                } else if !self.withheld {
                    let journal: serde_json::Value = serde_json::from_slice(
                        &fs::read(self.claim.join("journal.json")).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    let root_name = journal["root_name"]
                        .as_str()
                        .ok_or("owned root name missing")?;
                    let marker = home().join("runs").join(root_name).join("worker.pid");
                    if marker.try_exists().map_err(|e| e.to_string())? {
                        let held = marker.with_file_name("withheld-worker.pid");
                        fs::rename(&marker, &held).map_err(|e| e.to_string())?;
                        self.held = Some((marker, held));
                        self.withheld = true;
                    }
                }
                Ok(())
            }
        }
        let backend = real_backend();
        for forever in [false, true] {
            let mut resources = limits();
            resources.wall_time_ms = if forever { 1_000 } else { 5_000 };
            let mut f = fixture_command(
                &backend,
                b"package main\nfunc answer() int { return 42 }\n",
                "/toolchain/gofmt",
                &["-e", "/src/main.go"],
                resources,
            );
            let execution = execution_identity(&f.admission, &f.definition);
            let claim = home().join("runs").join(format!("execution-{execution}"));
            let mut guard = Delay {
                clock: Clock {
                    admin: &f.admin,
                    start: Instant::now(),
                    previous: 0,
                },
                claim,
                held: None,
                withheld: false,
                restored: false,
                remaining: 2,
                forever,
            };
            let record = backend
                .execute(
                    &f.engine,
                    &f.executor,
                    &f.admission,
                    &mut f.workspace,
                    &f.definition,
                    &mut guard,
                )
                .unwrap();
            assert!(guard.withheld);
            assert!(record.termination_confirmed, "{record:?}");
            assert_eq!(record.cleanup, CleanupOutcome::Removed);
            f.workspace.validate_contents().unwrap();
            assert!(
                record.resource_observations["pid_publication_wait_polls"]
                    .parse::<u64>()
                    .unwrap()
                    >= 2
            );
            if forever {
                assert!(!guard.restored);
                assert_ne!(record.outcome, SourceCheckOutcome::Exited { code: 0 });
                assert!(record.stdout.is_empty());
                assert!(record.isolation.is_none());
            } else {
                assert!(guard.restored);
                assert_eq!(
                    record.outcome,
                    SourceCheckOutcome::Exited { code: 0 },
                    "{record:?}"
                );
                assert!(record.isolation.is_some());
            }
        }
    }
}
