//! Subordinate Linux T1 execution using an explicitly pinned rootless OCI runtime.
//!
//! The trusted embedding supplies the runtime, closed rootfs and Engine-backed
//! revalidation guard. This component grants no transaction or evidence authority.
//! Namespaces share the host kernel. Same-UID hostile host processes are outside
//! this boundary; source and tooling are copied/validated under private parents.
use crate::materialize::{CleanupOutcome, PrivateWorkspace};
use celetixo_engine::engine::{
    Engine, SourceAdmission, SourceAttemptState, SourceCleanupCapability, SourceExecutorCapability,
};
use celetixo_engine::ids::{CheckId, Digest, Scope, SnapshotId, SourcePlanId};
use celetixo_engine::source_evidence::{SourceCheckOutcome, SourceResourceUsage};
use celetixo_engine::source_execution::{
    BackendIdentity, EnvironmentValue, ExecutionTier, NetworkPolicy, SourceCheckDefinition,
    VerificationPlan, WorkingDirectory,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;

const READY: &[u8] = b"CELETIXO_EXEC_READY_V1\n";
const LAUNCHER: &str = "/celetixo/exec-init";
const MAX_TOOLING: u64 = 64 * 1024 * 1024;
const MAX_CONTROL: usize = 64 * 1024;
const POLL: Duration = Duration::from_millis(2);

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

pub fn executable_digest(bytes: &[u8]) -> Digest {
    Digest::of("celetixo/execution-file/v1", bytes)
}

fn decode_digest(value: &str) -> Result<Digest, OciError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(OciError::Journal("digest encoding differs".into()));
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| OciError::Journal("digest encoding".into()))?;
    }
    Ok(Digest::from_bytes(bytes))
}

#[derive(Debug, Clone)]
pub struct PinnedExecutable {
    pub path: PathBuf,
    pub digest: Digest,
}

/// Plain files only. Libraries/interpreters must be named individually; no host
/// directory, home, socket or ambient tool lookup is mounted into a check.
#[derive(Debug, Clone)]
pub struct OciConfiguration {
    pub runtime: PinnedExecutable,
    pub runtime_version: String,
    /// Trusted control-plane property reader; never mounted into the guest.
    pub systemd_inspector: PinnedExecutable,
    pub rootfs_files: BTreeMap<String, PinnedExecutable>,
    pub private_parent: PathBuf,
    /// Runtime control plane only, never forwarded to the OCI process.
    pub runtime_environment: BTreeMap<String, String>,
}

pub use crate::backend::{ExecutionCheckpoint, ExecutionGuard, GuardDecision};

#[derive(Debug, Error)]
pub enum OciError {
    #[error("rootless OCI execution is available only on Linux x86_64")]
    UnsupportedPlatform,
    #[error("invalid or unsupported OCI configuration: {0}")]
    Configuration(String),
    #[error("unavailable, changed or mismatched trusted executable: {0:?}")]
    Tooling(PathBuf),
    #[error("check, plan, materialization or toolchain binding differs")]
    Binding,
    #[error("requested execution capability is unsupported: {0}")]
    Unsupported(&'static str),
    #[error("Engine execution guard rejected admission: {0}")]
    Guard(String),
    #[error("execution {0} was already claimed; query or reconcile it, never launch it again")]
    AlreadyClaimed(Digest),
    #[error("owned execution journal is missing, altered or inconsistent: {0}")]
    Journal(String),
    #[error("OCI execution capacity is busy or an earlier owned run requires reconciliation")]
    Busy,
    #[error("OCI execution I/O: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone)]
pub struct OciCheckRecord {
    pub execution: Digest,
    pub journal: Digest,
    pub check: CheckId,
    pub definition: Digest,
    pub plan: SourcePlanId,
    pub snapshot: SnapshotId,
    pub manifest: Digest,
    pub backend: BackendIdentity,
    pub runtime: Digest,
    pub rootfs: Digest,
    pub launcher: Digest,
    pub isolation: Option<Vec<u8>>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub outcome: SourceCheckOutcome,
    pub usage: SourceResourceUsage,
    /// Last sampled cgroup counters; not an exact phase/lifetime peak claim.
    pub resource_observations: BTreeMap<String, String>,
    /// Observed monotonic durations. Missing keys mean the phase was not reached.
    pub phase_timings_us: BTreeMap<String, u64>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub termination_confirmed: bool,
    pub cleanup: CleanupOutcome,
    pub post_source_manifest: Option<Digest>,
    pub diagnostics: Vec<String>,
}

impl OciCheckRecord {
    /// Versioned raw execution artifact, independent of Engine evidence sealing.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let outcome = match &self.outcome {
            SourceCheckOutcome::Exited { code } => json!({"kind":"exited","code":code}),
            SourceCheckOutcome::Signalled { signal } => json!({"kind":"signalled","signal":signal}),
            SourceCheckOutcome::TimedOut => json!({"kind":"timed_out"}),
            SourceCheckOutcome::Cancelled => json!({"kind":"cancelled"}),
            SourceCheckOutcome::Expired => json!({"kind":"expired"}),
            SourceCheckOutcome::Revoked => json!({"kind":"revoked"}),
            SourceCheckOutcome::OutputLimit => json!({"kind":"output_limit"}),
            SourceCheckOutcome::Infrastructure { reason } => {
                json!({"kind":"infrastructure","reason":reason})
            }
            SourceCheckOutcome::Skipped { reason } => json!({"kind":"skipped","reason":reason}),
            SourceCheckOutcome::UnknownTermination { reason } => {
                json!({"kind":"unknown_termination","reason":reason})
            }
        };
        let cleanup = match &self.cleanup {
            CleanupOutcome::Removed => json!({"kind":"removed"}),
            CleanupOutcome::Quarantined { path, reason } => {
                json!({"kind":"quarantined","path":path,"reason":reason})
            }
        };
        serde_json::to_vec(&json!({
            "version":2,"execution":self.execution.to_string(),"journal":self.journal.to_string(),
            "check":self.check.as_str(),"definition":self.definition.to_string(),
            "plan":self.plan.to_string(),"snapshot":self.snapshot.to_string(),"manifest":self.manifest.to_string(),
            "backend":self.backend.digest().to_string(),"runtime":self.runtime.to_string(),
            "rootfs":self.rootfs.to_string(),"launcher":self.launcher.to_string(),
            "isolation":self.isolation.as_ref().map(|bytes|Digest::of("celetixo/oci-isolation/v1",bytes).to_string()),
            "started_at_ms":self.started_at_ms,"finished_at_ms":self.finished_at_ms,
            "outcome":outcome,"wall_time_ms":self.usage.wall_time_ms,"observed_cpu_time_us":self.usage.cpu_time_us,
            "observed_peak_memory_bytes":self.usage.peak_memory_bytes,"resources":self.resource_observations,
            "phase_timings_us":self.phase_timings_us,
            "stdout":Digest::of("celetixo/source-artifact/v1",&self.stdout).to_string(),
            "stderr":Digest::of("celetixo/source-artifact/v1",&self.stderr).to_string(),
            "stdout_truncated":self.stdout_truncated,"stderr_truncated":self.stderr_truncated,
            "termination_confirmed":self.termination_confirmed,"cleanup":cleanup,
            "post_source_manifest":self.post_source_manifest.map(|d|d.to_string()),"diagnostics":self.diagnostics
        })).expect("JSON value serialization")
    }
}

/// Recovery is deliberately cleanup-only. A journal never restores an Engine
/// transaction, an execution capability, or evidence admissibility after loss.
#[derive(Debug, Clone)]
pub struct OciReconciliation {
    pub execution: Digest,
    pub termination_confirmed: bool,
    pub cleanup: CleanupOutcome,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OciStoredExecution {
    pub journal: Vec<u8>,
    pub isolation: Option<Vec<u8>>,
    pub record: Option<Vec<u8>>,
    pub stdout: Option<Vec<u8>>,
    pub stderr: Option<Vec<u8>>,
}

/// Stable across response loss. A second call with these exact inputs cannot
/// create another process, including after the original controller crashed.
pub fn execution_identity(admission: &SourceAdmission, check: &SourceCheckDefinition) -> Digest {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(admission.id().0.as_bytes());
    bytes.extend_from_slice(check.digest().as_bytes());
    Digest::of("celetixo/oci-execution/v1", &bytes)
}

pub struct OciBackend {
    configuration: OciConfiguration,
    runtime: Vec<u8>,
    inspector: Vec<u8>,
    files: BTreeMap<String, Vec<u8>>,
    identity: BackendIdentity,
    host_cpus: u64,
}

impl OciBackend {
    pub fn new(configuration: OciConfiguration) -> Result<Self, OciError> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return Err(OciError::UnsupportedPlatform);
        }
        if configuration.runtime_version != "1.5.2"
            || configuration.rootfs_files.len() > 4096
            || !configuration.rootfs_files.contains_key(LAUNCHER)
            || !configuration.private_parent.is_absolute()
            || configuration.private_parent.to_str().is_none()
        {
            return Err(OciError::Configuration(
                "version, rootfs or private parent".into(),
            ));
        }
        for (key, value) in &configuration.runtime_environment {
            if !matches!(key.as_str(), "XDG_RUNTIME_DIR" | "DBUS_SESSION_BUS_ADDRESS")
                || value.contains('\0')
                || value.len() > 4096
            {
                return Err(OciError::Configuration(
                    "runtime environment allowlist".into(),
                ));
            }
        }
        let runtime = read_pinned(&configuration.runtime, 16 * 1024 * 1024)?;
        let inspector = read_pinned(&configuration.systemd_inspector, 4 * 1024 * 1024)?;
        let mut total = (runtime.len() + inspector.len()) as u64;
        let mut files = BTreeMap::new();
        for (guest, pinned) in &configuration.rootfs_files {
            if !valid_guest_file(guest) {
                return Err(OciError::Configuration("rootfs guest path".into()));
            }
            let bytes = read_pinned(pinned, MAX_TOOLING.saturating_sub(total))?;
            total = total
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| OciError::Configuration("tooling budget".into()))?;
            if total > MAX_TOOLING {
                return Err(OciError::Configuration("tooling budget".into()));
            }
            files.insert(guest.clone(), bytes);
        }
        let mut image = Vec::new();
        for (guest, bytes) in &files {
            image.extend_from_slice(&(guest.len() as u64).to_be_bytes());
            image.extend_from_slice(guest.as_bytes());
            image.extend_from_slice(executable_digest(bytes).as_bytes());
        }
        let image = Digest::of("celetixo/oci-rootfs/v1", &image);
        let mut profile = serde_json::to_vec(&seccomp_profile()).expect("profile JSON");
        profile.extend_from_slice(configuration.runtime.digest.as_bytes());
        profile.extend_from_slice(configuration.systemd_inspector.digest.as_bytes());
        profile.extend_from_slice(
            Digest::of("celetixo/oci-adapter-source/v1", include_bytes!("oci.rs")).as_bytes(),
        );
        profile.extend_from_slice(
            Digest::of(
                "celetixo/materializer-source/v1",
                include_bytes!("materialize.rs"),
            )
            .as_bytes(),
        );
        for bytes in [
            include_bytes!("workspaces.rs").as_slice(),
            include_bytes!("backend.rs").as_slice(),
            include_bytes!("oci_adapter.rs").as_slice(),
        ] {
            profile.extend_from_slice(
                Digest::of("celetixo/execution-boundary-source/v1", bytes).as_bytes(),
            );
        }
        profile.extend_from_slice(b"rootless-v3:readonly-root-source;private-namespaces;tmpfs-scratch;empty-caps;no-new-privileges;no-network;bounded-rlimits-cgroupv2;systemd-runtime-max-sigkill;ready-release;atomic-owned-journal;durable-cleanup-only-confirmation;serialized-bounded-capacity");
        profile.extend_from_slice(fs::read("/proc/sys/kernel/osrelease")?.as_slice());
        let identity = BackendIdentity {
            name: "celetixo-rootless-oci".into(),
            version: "runc-1.5.2/profile-v3".into(),
            image,
            profile: Digest::of("celetixo/oci-profile/v1", &profile),
        };
        Ok(Self {
            configuration,
            runtime,
            inspector,
            files,
            identity,
            host_cpus: std::thread::available_parallelism().map_or(1, |n| n.get() as u64),
        })
    }

    pub fn identity(&self) -> &BackendIdentity {
        &self.identity
    }
    pub fn rootfs_digest(&self) -> Digest {
        self.identity.image
    }

    pub fn query_execution(
        &self,
        engine: &Engine,
        capability: &SourceCleanupCapability,
        scope: &Scope,
        execution: Digest,
    ) -> Result<OciStoredExecution, OciError> {
        engine
            .validate_source_cleanup(capability, scope, self.identity.digest())
            .map_err(|e| OciError::Guard(e.to_string()))?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            self.query_linux(scope, execution)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            Err(OciError::UnsupportedPlatform)
        }
    }

    pub fn reconcile_execution(
        &self,
        engine: &Engine,
        capability: &SourceCleanupCapability,
        scope: &Scope,
        execution: Digest,
    ) -> Result<OciReconciliation, OciError> {
        engine
            .validate_source_cleanup(capability, scope, self.identity.digest())
            .map_err(|e| OciError::Guard(e.to_string()))?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            self.reconcile_linux(engine, capability, scope, execution)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            Err(OciError::UnsupportedPlatform)
        }
    }

    /// Reads the original immutable outcome after response loss. This cannot
    /// launch a process, restore an admission, or establish evidence authority.
    pub fn recover_execution_record(
        &self,
        engine: &Engine,
        capability: &SourceCleanupCapability,
        scope: &Scope,
        execution: Digest,
    ) -> Result<Option<OciCheckRecord>, OciError> {
        let stored = self.query_execution(engine, capability, scope, execution)?;
        let Some(bytes) = &stored.record else {
            return Ok(None);
        };
        let value: Value =
            serde_json::from_slice(bytes).map_err(|e| OciError::Journal(e.to_string()))?;
        let journal: Value = serde_json::from_slice(&stored.journal)
            .map_err(|e| OciError::Journal(e.to_string()))?;
        let number = |key: &str| {
            value[key]
                .as_u64()
                .ok_or_else(|| OciError::Journal(format!("record {key}")))
        };
        let boolean = |key: &str| {
            value[key]
                .as_bool()
                .ok_or_else(|| OciError::Journal(format!("record {key}")))
        };
        let string = |key: &str| {
            value[key]
                .as_str()
                .ok_or_else(|| OciError::Journal(format!("record {key}")))
        };
        let outcome_value = &value["outcome"];
        let reason = || {
            outcome_value["reason"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| OciError::Journal("outcome reason".into()))
        };
        let outcome = match outcome_value["kind"].as_str() {
            Some("exited") => SourceCheckOutcome::Exited {
                code: outcome_value["code"]
                    .as_i64()
                    .and_then(|code| i32::try_from(code).ok())
                    .filter(|code| (0..=255).contains(code))
                    .ok_or_else(|| OciError::Journal("exit code".into()))?,
            },
            Some("signalled") => SourceCheckOutcome::Signalled {
                signal: outcome_value["signal"]
                    .as_i64()
                    .and_then(|signal| i32::try_from(signal).ok())
                    .filter(|signal| (1..=64).contains(signal))
                    .ok_or_else(|| OciError::Journal("signal".into()))?,
            },
            Some("timed_out") => SourceCheckOutcome::TimedOut,
            Some("cancelled") => SourceCheckOutcome::Cancelled,
            Some("expired") => SourceCheckOutcome::Expired,
            Some("revoked") => SourceCheckOutcome::Revoked,
            Some("output_limit") => SourceCheckOutcome::OutputLimit,
            Some("infrastructure") => SourceCheckOutcome::Infrastructure { reason: reason()? },
            Some("skipped") => SourceCheckOutcome::Skipped { reason: reason()? },
            Some("unknown_termination") => {
                SourceCheckOutcome::UnknownTermination { reason: reason()? }
            }
            _ => return Err(OciError::Journal("outcome kind".into())),
        };
        let cleanup_value = &value["cleanup"];
        let cleanup = match cleanup_value["kind"].as_str() {
            Some("removed") => CleanupOutcome::Removed,
            Some("quarantined") => CleanupOutcome::Quarantined {
                path: PathBuf::from(
                    cleanup_value["path"]
                        .as_str()
                        .ok_or_else(|| OciError::Journal("cleanup path".into()))?,
                ),
                reason: cleanup_value["reason"]
                    .as_str()
                    .ok_or_else(|| OciError::Journal("cleanup reason".into()))?
                    .into(),
            },
            _ => return Err(OciError::Journal("cleanup kind".into())),
        };
        let observations = value["resources"]
            .as_object()
            .ok_or_else(|| OciError::Journal("resource observations".into()))?
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|value| (key.clone(), value.to_owned()))
                    .ok_or_else(|| OciError::Journal("resource observation".into()))
            })
            .collect::<Result<_, _>>()?;
        let diagnostics = value["diagnostics"]
            .as_array()
            .ok_or_else(|| OciError::Journal("diagnostics".into()))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| OciError::Journal("diagnostic".into()))
            })
            .collect::<Result<_, _>>()?;
        let phase_timings_us = value["phase_timings_us"]
            .as_object()
            .ok_or_else(|| OciError::Journal("phase timings absent".into()))?
            .iter()
            .map(|(key, value)| {
                if !matches!(
                    key.as_str(),
                    "preparation" | "startup" | "command_supervision" | "runtime_cleanup"
                ) {
                    return Err(OciError::Journal("unknown phase timing".into()));
                }
                value
                    .as_u64()
                    .map(|value| (key.clone(), value))
                    .ok_or_else(|| OciError::Journal("phase timing type".into()))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let record = OciCheckRecord {
            execution,
            journal: Digest::of("celetixo/oci-journal/v1", &stored.journal),
            check: CheckId::new(string("check")?).map_err(|e| OciError::Journal(e.to_string()))?,
            definition: decode_digest(string("definition")?)?,
            plan: SourcePlanId(decode_digest(string("plan")?)?),
            snapshot: SnapshotId(decode_digest(string("snapshot")?)?),
            manifest: decode_digest(string("manifest")?)?,
            backend: self.identity.clone(),
            runtime: self.configuration.runtime.digest,
            rootfs: self.identity.image,
            launcher: executable_digest(self.files.get(LAUNCHER).expect("launcher pinned")),
            isolation: stored.isolation,
            started_at_ms: number("started_at_ms")?,
            finished_at_ms: number("finished_at_ms")?,
            outcome,
            usage: SourceResourceUsage {
                wall_time_ms: number("wall_time_ms")?,
                cpu_time_us: number("observed_cpu_time_us")?,
                peak_memory_bytes: number("observed_peak_memory_bytes")?,
            },
            resource_observations: observations,
            phase_timings_us,
            stdout: stored
                .stdout
                .ok_or_else(|| OciError::Journal("stdout absent".into()))?,
            stderr: stored
                .stderr
                .ok_or_else(|| OciError::Journal("stderr absent".into()))?,
            stdout_truncated: boolean("stdout_truncated")?,
            stderr_truncated: boolean("stderr_truncated")?,
            termination_confirmed: boolean("termination_confirmed")?,
            cleanup,
            post_source_manifest: match value["post_source_manifest"].as_str() {
                Some(digest) => Some(decode_digest(digest)?),
                None if value["post_source_manifest"].is_null() => None,
                _ => return Err(OciError::Journal("post source manifest".into())),
            },
            diagnostics,
        };
        if record.canonical_bytes() != *bytes
            || record.finished_at_ms < record.started_at_ms
            || [
                "check",
                "definition",
                "plan",
                "snapshot",
                "manifest",
                "started_at_ms",
            ]
            .into_iter()
            .any(|key| value[key] != journal[key])
        {
            return Err(OciError::Journal(
                "canonical stored result or journal binding differs".into(),
            ));
        }
        Ok(Some(record))
    }

    pub fn execute(
        &self,
        engine: &Engine,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        workspace: &mut PrivateWorkspace,
        check: &SourceCheckDefinition,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<OciCheckRecord, OciError> {
        engine_continue(engine, executor, admission)?;
        workspace
            .validate_registered()
            .map_err(|_| OciError::Binding)?;
        let plan = admission.plan();
        if executor.backend() != self.identity()
            || admission.application_report() != workspace.report().application()
        {
            return Err(OciError::Binding);
        }
        self.validate(workspace, plan, check)?;
        guard_continue(guard)?;
        engine_continue(engine, executor, admission)?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            self.execute_linux(engine, executor, admission, workspace, check, guard)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            Err(OciError::UnsupportedPlatform)
        }
    }

    fn validate(
        &self,
        workspace: &PrivateWorkspace,
        plan: &VerificationPlan,
        check: &SourceCheckDefinition,
    ) -> Result<(), OciError> {
        let input = check.input();
        if workspace.source_path().to_str().is_none() {
            return Err(OciError::Unsupported("non-UTF8 materialization parent"));
        }
        if !plan.checks().iter().any(|c| c.definition() == check)
            || plan.binding().resulting_snapshot != workspace.report().snapshot()
            || plan.binding().resulting_manifest != workspace.report().manifest()
            || input.backend != self.identity
            || input.toolchain.runtime_closure != self.identity.image
            || self
                .files
                .get(&input.toolchain.executable)
                .map(|b| executable_digest(b))
                != Some(input.toolchain.executable_digest)
        {
            return Err(OciError::Binding);
        }
        plan.validate_resulting_snapshot(workspace.applied().snapshot())
            .map_err(|_| OciError::Binding)?;
        workspace
            .validate_contents()
            .map_err(|_| OciError::Binding)?;
        if input.tier != ExecutionTier::T1 {
            return Err(OciError::Unsupported("execution tier"));
        }
        if input.network != NetworkPolicy::DenyAll
            || !input.credential_references.is_empty()
            || input
                .environment
                .values()
                .any(|v| matches!(v, EnvironmentValue::ProtectedReference(_)))
        {
            return Err(OciError::Unsupported("network or credential grant"));
        }
        if input.environment.get("HOME") != Some(&EnvironmentValue::Literal("/scratch".into())) {
            return Err(OciError::Unsupported("explicit HOME=/scratch required"));
        }
        if input
            .environment
            .keys()
            .any(|k| k.starts_with("LD_") || matches!(k.as_str(), "PATH" | "BASH_ENV" | "ENV"))
        {
            return Err(OciError::Unsupported(
                "loader or executable search environment",
            ));
        }
        if !input.outputs.is_empty()
            || input.expected_artifacts.iter().any(|a| {
                !matches!(
                    a.name.as_str(),
                    "stdout" | "stderr" | "diagnostics" | "execution-record" | "isolation-record"
                )
            })
        {
            return Err(OciError::Unsupported("custom output artifacts or handoff"));
        }
        let r = input.resources;
        if r.process_count < 16
            || r.file_descriptors < 32
            || r.memory_bytes < 16 * 1024 * 1024
            || r.cpu_time_ms < 1000
            || r.wall_time_ms > 3_600_000
            || r.disk_bytes < 4096
            || r.stdout_bytes > 16 * 1024 * 1024
            || r.stderr_bytes > 16 * 1024 * 1024
        {
            return Err(OciError::Unsupported("resource range"));
        }
        Ok(())
    }
}

fn engine_continue(
    engine: &Engine,
    executor: &SourceExecutorCapability,
    admission: &SourceAdmission,
) -> Result<(), OciError> {
    match engine.validate_source_attempt(executor, admission) {
        Ok(SourceAttemptState::Verifying) => Ok(()),
        Ok(_) => Err(OciError::Guard("source attempt is not verifying".into())),
        Err(e) => Err(OciError::Guard(e.to_string())),
    }
}

fn guard_continue(guard: &mut dyn ExecutionGuard) -> Result<(), OciError> {
    match guard.revalidate() {
        Ok(GuardDecision::Continue) => Ok(()),
        Ok(decision) => Err(OciError::Guard(format!("{decision:?}"))),
        Err(reason) => Err(OciError::Guard(reason)),
    }
}

fn valid_guest_file(path: &str) -> bool {
    path.is_ascii()
        && path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && !path.starts_with("//")
        && path
            .split('/')
            .skip(1)
            .all(|p| !p.is_empty() && p != "." && p != "..")
        && ["/celetixo/", "/toolchain/", "/lib/", "/lib64/", "/usr/lib/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

fn read_pinned(pin: &PinnedExecutable, max: u64) -> Result<Vec<u8>, OciError> {
    if !pin.path.is_absolute() {
        return Err(OciError::Tooling(pin.path.clone()));
    }
    let mut parent = PathBuf::from("/");
    for c in pin.path.components() {
        match c {
            Component::RootDir => continue,
            Component::Normal(c) => parent.push(c),
            _ => return Err(OciError::Tooling(pin.path.clone())),
        }
        if fs::symlink_metadata(&parent)
            .map_err(|_| OciError::Tooling(pin.path.clone()))?
            .file_type()
            .is_symlink()
        {
            return Err(OciError::Tooling(pin.path.clone()));
        }
    }
    let before =
        fs::symlink_metadata(&pin.path).map_err(|_| OciError::Tooling(pin.path.clone()))?;
    if !before.is_file() || before.len() > max {
        return Err(OciError::Tooling(pin.path.clone()));
    }
    let mut bytes = Vec::new();
    File::open(&pin.path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let after = fs::symlink_metadata(&pin.path)?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || before.modified().ok() != after.modified().ok()
        || executable_digest(&bytes) != pin.digest
    {
        return Err(OciError::Tooling(pin.path.clone()));
    }
    Ok(bytes)
}

/// A versioned allowlist, deliberately lacking network, ptrace, mount, setns,
/// unshare, BPF, keyring, io_uring and process-memory access. Clone may create
/// threads/processes, but cannot request any new namespace.
pub fn seccomp_profile() -> Value {
    let names = [
        "read",
        "write",
        "close",
        "close_range",
        "fstat",
        "fstatfs",
        "statfs",
        "newfstatat",
        "stat",
        "lstat",
        "statx",
        "lseek",
        "pread64",
        "pwrite64",
        "readv",
        "writev",
        "getdents64",
        "open",
        "openat",
        "openat2",
        "readlink",
        "readlinkat",
        "access",
        "faccessat",
        "faccessat2",
        "fcntl",
        "dup",
        "dup2",
        "dup3",
        "pipe",
        "pipe2",
        "mmap",
        "mprotect",
        "munmap",
        "mremap",
        "madvise",
        "brk",
        "msync",
        "mincore",
        "rt_sigaction",
        "rt_sigprocmask",
        "rt_sigreturn",
        "sigaltstack",
        "rt_sigtimedwait",
        "getpid",
        "gettid",
        "tgkill",
        "kill",
        "fork",
        "vfork",
        "wait4",
        "waitid",
        "exit",
        "exit_group",
        "execve",
        "execveat",
        "set_tid_address",
        "set_robust_list",
        "rseq",
        "arch_prctl",
        "futex",
        "futex_waitv",
        "sched_yield",
        "sched_getaffinity",
        "sched_getparam",
        "sched_getscheduler",
        "clock_gettime",
        "clock_nanosleep",
        "gettimeofday",
        "time",
        "nanosleep",
        "getrandom",
        "uname",
        "getuid",
        "geteuid",
        "getgid",
        "getegid",
        "getgroups",
        "getcwd",
        "chdir",
        "fchdir",
        "umask",
        "getrlimit",
        "prlimit64",
        "getrusage",
        "sysinfo",
        "restart_syscall",
        "eventfd",
        "eventfd2",
        "epoll_create",
        "epoll_create1",
        "epoll_ctl",
        "epoll_wait",
        "epoll_pwait",
        "epoll_pwait2",
        "poll",
        "ppoll",
        "select",
        "pselect6",
        "ftruncate",
        "truncate",
        "fsync",
        "fdatasync",
        "mkdir",
        "mkdirat",
        "unlink",
        "unlinkat",
        "rename",
        "renameat",
        "renameat2",
    ];
    json!({"defaultAction":"SCMP_ACT_ERRNO","defaultErrnoRet":1,"architectures":["SCMP_ARCH_X86_64"],
        "syscalls":[{"names":&names[..],"action":"SCMP_ACT_ALLOW"},
        {"names":["clone"],"action":"SCMP_ACT_ALLOW","args":[{"index":0,"value":2114060288_u64,"valueTwo":0,"op":"SCMP_CMP_MASKED_EQ"}]},
        {"names":["clone3"],"action":"SCMP_ACT_ERRNO","errnoRet":38},
        {"names":["prctl"],"action":"SCMP_ACT_ALLOW","args":[{"index":0,"value":15,"op":"SCMP_CMP_EQ"}]},
        {"names":["prctl"],"action":"SCMP_ACT_ALLOW","args":[{"index":0,"value":16,"op":"SCMP_CMP_EQ"}]}]})
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn validate_private_directory(path: &Path) -> io::Result<fs::Metadata> {
        let uid = rustix::process::getuid().as_raw();
        let mut current = PathBuf::from("/");
        for part in path.components() {
            match part {
                Component::RootDir => continue,
                Component::Normal(part) => current.push(part),
                _ => return Err(io::Error::other("non-canonical private directory")),
            }
            if fs::symlink_metadata(&current)?.file_type().is_symlink() {
                return Err(io::Error::other("private directory crosses a symlink"));
            }
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o777 != 0o700 {
            return Err(io::Error::other("private directory owner/mode differs"));
        }
        Ok(metadata)
    }

    fn execution_lock(parent: &Path) -> Result<File, OciError> {
        let directory = File::open(parent)?;
        let descriptor = rustix::fs::openat(
            &directory,
            "execution.lock",
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .map_err(io::Error::from)?;
        let file = File::from(descriptor);
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(OciError::Journal(
                "execution capacity lock identity differs".into(),
            ));
        }
        rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(
            |error| {
                if error == rustix::io::Errno::WOULDBLOCK {
                    OciError::Busy
                } else {
                    OciError::Io(error.into())
                }
            },
        )?;
        Ok(file)
    }

    fn reject_unreconciled_runs(parent: &Path, current: Digest) -> Result<(), OciError> {
        if parent
            .join(format!("preparation-aborted-{current}.json"))
            .exists()
        {
            return Err(OciError::AlreadyClaimed(current));
        }
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with("celetixo-check-") && entry.path().join("prepared-claim").exists() {
                return Err(OciError::Busy);
            }
            if !name.starts_with("execution-") || name == "execution.lock" {
                continue;
            }
            if name == format!("execution-{current}") {
                return Err(OciError::AlreadyClaimed(current));
            }
            validate_private_directory(&entry.path())?;
            let bytes = read_owned(&entry.path().join("journal.json"), MAX_CONTROL as u64)?;
            let journal: Value = serde_json::from_slice(&bytes)
                .map_err(|error| OciError::Journal(error.to_string()))?;
            let root = journal_string(&journal, "root_name")?;
            if !root.starts_with("celetixo-check-")
                || !root
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err(OciError::Journal(
                    "capacity journal root name differs".into(),
                ));
            }
            if parent.join(root).exists()
                || (!entry.path().join("record.json").is_file()
                    && !entry.path().join("reconciled.json").is_file()
                    && !read_owned(
                        &entry.path().join("cleanup-confirmed.json"),
                        MAX_CONTROL as u64,
                    )
                    .is_ok_and(|marker| marker == cleanup_confirmation(&journal, &bytes)))
            {
                return Err(OciError::Busy);
            }
        }
        Ok(())
    }

    fn storage_bytes(path: &Path, entries: &mut usize) -> Result<u64, OciError> {
        *entries += 1;
        if *entries > 20_000 {
            return Err(OciError::Busy);
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(OciError::Journal(
                "capacity census crossed a symlink".into(),
            ));
        }
        if metadata.is_file() {
            return Ok(metadata.len());
        }
        if !metadata.is_dir() {
            return Err(OciError::Journal(
                "capacity census found a special file".into(),
            ));
        }
        let mut total = 0_u64;
        for entry in fs::read_dir(path)? {
            total = total
                .checked_add(storage_bytes(&entry?.path(), entries)?)
                .ok_or(OciError::Busy)?;
        }
        Ok(total)
    }

    fn durable_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = File::options().write(true).create_new(true).open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        File::open(
            path.parent()
                .ok_or_else(|| io::Error::other("file parent"))?,
        )?
        .sync_all()
    }

    fn read_owned(path: &Path, maximum: u64) -> io::Result<Vec<u8>> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
            || metadata.len() > maximum
        {
            return Err(io::Error::other(
                "owned record owner/mode/type/size differs",
            ));
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(maximum + 1)
            .read_to_end(&mut bytes)?;
        let after = fs::symlink_metadata(path)?;
        if bytes.len() as u64 != metadata.len()
            || after.dev() != metadata.dev()
            || after.ino() != metadata.ino()
            || after.len() != metadata.len()
            || after.modified().ok() != metadata.modified().ok()
        {
            return Err(io::Error::other("owned record changed while reading"));
        }
        Ok(bytes)
    }

    fn scope_json(scope: &Scope) -> Value {
        json!({"organization":scope.organization.as_str(),"workspace":scope.workspace.as_str(),"repository":scope.repository.as_str()})
    }

    fn preparation_prefix(scope: &Scope, execution: Digest) -> String {
        let scope = Digest::of(
            "celetixo/oci-owner-scope/v1",
            &serde_json::to_vec(&scope_json(scope)).expect("scope JSON"),
        );
        format!("celetixo-check-{scope}-{execution}-")
    }

    fn cleanup_confirmation(journal: &Value, bytes: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&json!({"version":1,"execution":journal["execution"],"journal":Digest::of("celetixo/oci-journal/v1",bytes).to_string(),"root_dev":journal["root_dev"],"root_ino":journal["root_ino"],"termination_confirmed":true,"runtime_deleted":true})).expect("cleanup confirmation JSON")
    }

    fn execution_claim(parent: &Path, execution: Digest) -> PathBuf {
        parent.join(format!("execution-{execution}"))
    }

    fn group_path(uid: u32, cid: &str) -> PathBuf {
        PathBuf::from(format!(
            "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/user.slice/celetixo-{cid}.scope"
        ))
    }

    fn group_empty(group: &Path) -> io::Result<bool> {
        match fs::read_to_string(group.join("cgroup.events")) {
            Ok(events) => Ok(events.lines().any(|line| line == "populated 0")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(fs::symlink_metadata(group).is_err_and(|e| e.kind() == io::ErrorKind::NotFound))
            }
            Err(error) => Err(error),
        }
    }

    fn journal_string<'a>(journal: &'a Value, key: &str) -> io::Result<&'a str> {
        journal[key]
            .as_str()
            .ok_or_else(|| io::Error::other(format!("journal {key}")))
    }

    fn owned_runtime_processes(runtime: &Path) -> io::Result<Vec<std::os::fd::OwnedFd>> {
        let executable = fs::metadata(runtime)?;
        let uid = rustix::process::getuid().as_raw();
        let mut processes = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
            else {
                continue;
            };
            let Some(pid) = rustix::process::Pid::from_raw(pid) else {
                continue;
            };
            let path = entry.path();
            if !fs::metadata(&path).is_ok_and(|metadata| metadata.uid() == uid)
                || !fs::metadata(path.join("exe")).is_ok_and(|metadata| {
                    metadata.dev() == executable.dev() && metadata.ino() == executable.ino()
                })
            {
                continue;
            }
            let handle =
                match rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::NONBLOCK) {
                    Ok(handle) => handle,
                    Err(error) if error == rustix::io::Errno::SRCH => continue,
                    Err(error) => return Err(error.into()),
                };
            // PID reuse cannot redirect the signal: it targets this pidfd, and
            // the corresponding live process still owns this private copy.
            if fs::metadata(path.join("exe")).is_ok_and(|metadata| {
                metadata.dev() == executable.dev() && metadata.ino() == executable.ino()
            }) {
                processes.push(handle);
            }
        }
        Ok(processes)
    }

    fn boot_id() -> io::Result<String> {
        let id = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let id = id.trim();
        if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err(io::Error::other("kernel boot identity differs"));
        }
        Ok(id.to_owned())
    }

    fn cleanup_retained_source(
        journal: &Value,
        claim: &Path,
        scope: &Scope,
        authorize: impl Fn() -> Result<(), OciError>,
    ) -> Result<Option<CleanupOutcome>, OciError> {
        let source_root = PathBuf::from(journal_string(journal, "source_root")?);
        if !source_root.exists() {
            return Ok(None);
        }
        let quarantine = |reason: &str| {
            Some(CleanupOutcome::Quarantined {
                path: source_root.clone(),
                reason: reason.into(),
            })
        };
        let hint = match read_owned(
            &source_root.join("execution-retention.json"),
            MAX_CONTROL as u64,
        ) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Ok(quarantine("source retention hint identity differs")),
        };
        if journal["boot_id"] != boot_id()? {
            return Ok(quarantine(
                "volatile source identity is from a different host boot",
            ));
        }
        let metadata = validate_private_directory(&source_root)?;
        if Some(metadata.dev()) != journal["source_dev"].as_u64()
            || Some(metadata.ino()) != journal["source_ino"].as_u64()
        {
            return Ok(quarantine("retained source parent identity differs"));
        }
        let parsed_hint: Value =
            serde_json::from_slice(&hint).map_err(|e| OciError::Journal(e.to_string()))?;
        if parsed_hint["version"] != 1
            || parsed_hint["journal"] != claim.join("journal.json").to_string_lossy().as_ref()
        {
            return Ok(quarantine("source retention belongs to different work"));
        }
        authorize()?;
        let again = validate_private_directory(&source_root)?;
        if again.dev() != metadata.dev() || again.ino() != metadata.ino() {
            return Ok(quarantine("source parent changed before cleanup"));
        }
        let source = crate::workspaces::RetainedSource {
            path: &source_root,
            scope,
            admission: journal_string(journal, "admission")?,
            candidate: journal_string(journal, "snapshot")?,
            manifest: journal_string(journal, "manifest")?,
            latch: &hint,
        };
        crate::workspaces::WorkspaceRegistry::cleanup_retained(source, || {
            authorize().map_err(|e| io::Error::other(e.to_string()))
        })
        .map(Some)
        .map_err(|e| OciError::Journal(e.to_string()))
    }

    impl OciBackend {
        fn owned_journal(
            &self,
            scope: &Scope,
            execution: Digest,
        ) -> Result<(PathBuf, Vec<u8>, Value), OciError> {
            validate_private_directory(&self.configuration.private_parent)?;
            let claim = execution_claim(&self.configuration.private_parent, execution);
            validate_private_directory(&claim)?;
            let bytes = read_owned(&claim.join("journal.json"), MAX_CONTROL as u64)?;
            let journal: Value =
                serde_json::from_slice(&bytes).map_err(|e| OciError::Journal(e.to_string()))?;
            let keys = [
                "admission",
                "backend",
                "boot_id",
                "check",
                "cid",
                "config",
                "definition",
                "execution",
                "gid",
                "manifest",
                "plan",
                "root_dev",
                "root_ino",
                "root_name",
                "rootfs",
                "runtime",
                "scope",
                "snapshot",
                "source_dev",
                "source_ino",
                "source_root",
                "source_attempt",
                "started_at_ms",
                "transaction",
                "transaction_attempt",
                "uid",
                "version",
                "wall_time_ms",
            ];
            let object = journal
                .as_object()
                .ok_or_else(|| OciError::Journal("object".into()))?;
            if object.len() != keys.len()
                || keys.iter().any(|key| !object.contains_key(*key))
                || serde_json::to_vec(&journal).expect("journal JSON") != bytes
                || journal["version"] != 1
                || journal["execution"] != execution.to_string()
                || journal["backend"] != self.identity.digest().to_string()
                || journal["runtime"] != self.configuration.runtime.digest.to_string()
                || journal["rootfs"] != self.identity.image.to_string()
                || journal["scope"] != scope_json(scope)
                || journal["uid"] != rustix::process::getuid().as_raw()
                || journal["gid"] != rustix::process::getgid().as_raw()
            {
                return Err(OciError::Journal(
                    "canonical identity/scope/profile differs".into(),
                ));
            }
            let root_name = journal_string(&journal, "root_name")?;
            if !root_name.starts_with(&preparation_prefix(scope, execution))
                || root_name.len() > 192
                || !root_name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || journal["cid"] != root_name.to_ascii_lowercase()
                || journal["root_dev"].as_u64().is_none()
                || journal["root_ino"].as_u64().is_none()
            {
                return Err(OciError::Journal(
                    "owned resource name/identity differs".into(),
                ));
            }
            let config = read_owned(&claim.join("config.json"), MAX_CONTROL as u64)?;
            if journal["config"] != Digest::of("celetixo/oci-config/v1", &config).to_string() {
                return Err(OciError::Journal("configuration digest differs".into()));
            }
            Ok((claim, bytes, journal))
        }

        pub(super) fn query_linux(
            &self,
            scope: &Scope,
            execution: Digest,
        ) -> Result<OciStoredExecution, OciError> {
            let (claim, journal, _) = self.owned_journal(scope, execution)?;
            let optional = |name: &str, maximum: u64| match read_owned(&claim.join(name), maximum) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(OciError::Io(e)),
            };
            let record = optional("record.json", MAX_CONTROL as u64)?;
            let isolation = optional("isolation.json", MAX_CONTROL as u64)?;
            let stdout = optional("stdout", 16 * 1024 * 1024)?;
            let stderr = optional("stderr", 16 * 1024 * 1024)?;
            if let Some(bytes) = &record {
                let value: Value =
                    serde_json::from_slice(bytes).map_err(|e| OciError::Journal(e.to_string()))?;
                if value["execution"] != execution.to_string()
                    || isolation
                        .as_ref()
                        .map(|bytes| Digest::of("celetixo/oci-isolation/v1", bytes).to_string())
                        .as_ref()
                        != value["isolation"].as_str().map(str::to_owned).as_ref()
                    || value["journal"]
                        != Digest::of("celetixo/oci-journal/v1", &journal).to_string()
                    || stdout
                        .as_ref()
                        .map(|b| Digest::of("celetixo/source-artifact/v1", b).to_string())
                        .as_ref()
                        != value["stdout"].as_str().map(str::to_owned).as_ref()
                    || stderr
                        .as_ref()
                        .map(|b| Digest::of("celetixo/source-artifact/v1", b).to_string())
                        .as_ref()
                        != value["stderr"].as_str().map(str::to_owned).as_ref()
                {
                    return Err(OciError::Journal("stored result/artifacts differ".into()));
                }
            }
            Ok(OciStoredExecution {
                journal,
                isolation,
                record,
                stdout,
                stderr,
            })
        }

        fn reconcile_preparation(
            &self,
            engine: &Engine,
            capability: &SourceCleanupCapability,
            scope: &Scope,
            execution: Digest,
        ) -> Result<Option<OciReconciliation>, OciError> {
            let parent = &self.configuration.private_parent;
            if execution_claim(parent, execution).exists() {
                return Ok(None);
            }
            let prefix = preparation_prefix(scope, execution);
            let marker = parent.join(format!("preparation-aborted-{execution}.json"));
            let confirmation = serde_json::to_vec(&json!({"version":1,"execution":execution.to_string(),"scope":scope_json(scope),"backend":self.identity.digest().to_string(),"launch_claim_published":false,"cleanup_only":true})).expect("preparation cleanup JSON");
            let mut found = false;
            if marker.exists() && read_owned(&marker, MAX_CONTROL as u64)? != confirmation {
                return Err(OciError::Journal(
                    "preparation cleanup marker differs".into(),
                ));
            }
            for entry in fs::read_dir(parent)? {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if !name.starts_with(&prefix) {
                    continue;
                }
                let root = entry.path();
                let metadata = validate_private_directory(&root)?;
                let cid = name.to_ascii_lowercase();
                if !group_empty(&group_path(rustix::process::getuid().as_raw(), &cid))? {
                    return Ok(Some(OciReconciliation {
                        execution,
                        termination_confirmed: false,
                        cleanup: CleanupOutcome::Quarantined {
                            path: root,
                            reason:
                                "unpublished preparation unexpectedly has an active process group"
                                    .into(),
                        },
                        diagnostics: vec![
                            "launch state is inconsistent; resources retained".into(),
                        ],
                    }));
                }
                engine
                    .validate_source_cleanup(capability, scope, self.identity.digest())
                    .map_err(|error| OciError::Guard(error.to_string()))?;
                if !marker.exists() {
                    durable_file(&marker, &confirmation)?;
                }
                let again = validate_private_directory(&root)?;
                if again.dev() != metadata.dev() || again.ino() != metadata.ino() {
                    return Err(OciError::Journal("preparation root changed".into()));
                }
                // Launch is possible only after the complete prepared claim is
                // atomically moved to execution-<id>. Its absence, the scoped
                // private name and the held capacity lock establish that this
                // preparation never became runnable, even if its JSON is partial.
                engine
                    .validate_source_cleanup(capability, scope, self.identity.digest())
                    .map_err(|error| OciError::Guard(error.to_string()))?;
                fs::remove_dir_all(&root)?;
                File::open(parent)?.sync_all()?;
                found = true;
            }
            if found || marker.exists() {
                Ok(Some(OciReconciliation {execution,termination_confirmed:true,cleanup:CleanupOutcome::Removed,diagnostics:vec!["unpublished preparation removed; same execution identity remains non-runnable; no check result recovered".into()]}))
            } else {
                Ok(None)
            }
        }

        pub(super) fn reconcile_linux(
            &self,
            engine: &Engine,
            capability: &SourceCleanupCapability,
            scope: &Scope,
            execution: Digest,
        ) -> Result<OciReconciliation, OciError> {
            validate_private_directory(&self.configuration.private_parent)?;
            let _capacity = execution_lock(&self.configuration.private_parent)?;
            if let Some(prepared) =
                self.reconcile_preparation(engine, capability, scope, execution)?
            {
                return Ok(prepared);
            }
            let (claim, journal_bytes, journal) = self.owned_journal(scope, execution)?;
            let authorize = || {
                engine
                    .validate_source_cleanup(capability, scope, self.identity.digest())
                    .map_err(|e| OciError::Guard(e.to_string()))
            };
            let root = self
                .configuration
                .private_parent
                .join(journal_string(&journal, "root_name")?);
            let cid = journal_string(&journal, "cid")?;
            let uid = rustix::process::getuid().as_raw();
            let group = group_path(uid, cid);
            let quarantine = |reason: String| OciReconciliation {
                execution,
                termination_confirmed: false,
                cleanup: CleanupOutcome::Quarantined {
                    path: root.clone(),
                    reason: reason.clone(),
                },
                diagnostics: vec![reason],
            };
            if !root.exists() {
                let stored = self.query_linux(scope, execution)?;
                let reconciled = read_owned(&claim.join("reconciled.json"), MAX_CONTROL as u64)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .is_some_and(|record| {
                        record["execution"] == execution.to_string()
                            && record["termination_confirmed"] == true
                            && record["cleanup_only"] == true
                    });
                let cleanup_confirmed =
                    read_owned(&claim.join("cleanup-confirmed.json"), MAX_CONTROL as u64)
                        .is_ok_and(|bytes| bytes == cleanup_confirmation(&journal, &journal_bytes));
                if group_empty(&group)?
                    && (stored.record.is_some() || reconciled || cleanup_confirmed)
                {
                    let source_cleanup =
                        cleanup_retained_source(&journal, &claim, scope, authorize)?;
                    return Ok(OciReconciliation {
                        execution,
                        termination_confirmed: true,
                        cleanup: source_cleanup.unwrap_or(CleanupOutcome::Removed),
                        diagnostics: vec![
                            "resources already removed; immutable prior record retained".into(),
                        ],
                    });
                }
                return Ok(quarantine(
                    "owned root missing without a terminal record".into(),
                ));
            }
            let metadata = validate_private_directory(&root)?;
            if Some(metadata.dev()) != journal["root_dev"].as_u64()
                || Some(metadata.ino()) != journal["root_ino"].as_u64()
            {
                return Err(OciError::Journal("owned root was replaced".into()));
            }
            let runtime = root.join("runtime");
            if read_pinned(
                &PinnedExecutable {
                    path: runtime.clone(),
                    digest: self.configuration.runtime.digest,
                },
                16 * 1024 * 1024,
            )? != self.runtime
            {
                return Err(OciError::Tooling(runtime));
            }
            // Stop the trusted runtime controller and any still-starting init
            // re-execs first. Otherwise an interrupted startup could create its
            // cgroup after an earlier empty-group observation. A unique copied
            // runtime inode and pidfds avoid signalling unrelated PID reuse.
            let stop_runtime = Instant::now();
            loop {
                let processes = owned_runtime_processes(&runtime)?;
                if processes.is_empty() {
                    break;
                }
                for process in processes {
                    authorize()?;
                    match rustix::process::pidfd_send_signal(
                        &process,
                        rustix::process::Signal::KILL,
                    ) {
                        Ok(()) => {}
                        Err(error) if error == rustix::io::Errno::SRCH => {}
                        Err(error) => return Err(OciError::Io(error.into())),
                    }
                }
                if stop_runtime.elapsed() > Duration::from_secs(3) {
                    return Ok(quarantine("owned runtime controllers did not stop".into()));
                }
                std::thread::sleep(POLL);
            }
            if !group_empty(&group)? {
                let group_meta = fs::symlink_metadata(&group)?;
                if !group_meta.is_dir() || group_meta.uid() != uid {
                    return Err(OciError::Journal("cgroup owner/type differs".into()));
                }
                authorize()?;
                fs::write(group.join("cgroup.kill"), b"1")?;
            }
            let stop = Instant::now();
            while !group_empty(&group)? && stop.elapsed() < Duration::from_secs(3) {
                authorize()?;
                std::thread::sleep(POLL);
            }
            if !group_empty(&group)? {
                return Ok(quarantine("process tree remains populated".into()));
            }
            let state = root.join("state");
            if state.join(cid).exists() {
                authorize()?;
                let (code, _, stderr) = bounded_control(
                    self.runtime_command(&runtime, &state)
                        .args(["delete", "--force", cid]),
                    Duration::from_secs(3),
                )?;
                if code != Some(0) {
                    return Ok(quarantine(format!(
                        "runtime delete did not confirm cleanup: {}",
                        String::from_utf8_lossy(&stderr)
                    )));
                }
            }
            authorize()?;
            if !group_empty(&group)? || state.join(cid).exists() {
                return Ok(quarantine("resources remain after runtime deletion".into()));
            }
            let confirmation = serde_json::to_vec(&json!({"version":1,"execution":execution.to_string(),"termination_confirmed":true,"cleanup_only":true,"operator":capability.principal().as_str()})).expect("reconciliation JSON");
            let confirmation_path = claim.join("reconciled.json");
            if !confirmation_path.exists() {
                durable_file(&confirmation_path, &confirmation)?;
            }
            authorize()?;
            let again = validate_private_directory(&root)?;
            if again.dev() != metadata.dev() || again.ino() != metadata.ino() {
                return Err(OciError::Journal(
                    "owned root changed before cleanup".into(),
                ));
            }
            fs::remove_dir_all(&root)?;
            File::open(&self.configuration.private_parent)?.sync_all()?;
            let source_cleanup = cleanup_retained_source(&journal, &claim, scope, authorize)?;
            Ok(OciReconciliation { execution, termination_confirmed: true, cleanup: source_cleanup.unwrap_or(CleanupOutcome::Removed), diagnostics: vec!["owned resources stopped and removed; no execution or evidence authority restored".into()] })
        }

        fn runtime_command(&self, runtime: &Path, state: &Path) -> Command {
            let mut c = Command::new(runtime);
            c.env_clear()
                .envs(&self.configuration.runtime_environment)
                .arg("--root")
                .arg(state)
                .arg("--systemd-cgroup");
            c
        }

        fn verify_scope_deadline(
            &self,
            inspector: &Path,
            cid: &str,
            wall_time_ms: u64,
        ) -> io::Result<String> {
            let unit = format!("celetixo-{cid}.scope");
            let mut object = String::from("/org/freedesktop/systemd1/unit/");
            for byte in unit.bytes() {
                if byte.is_ascii_alphanumeric() {
                    object.push(char::from(byte));
                } else {
                    object.push_str(&format!("_{byte:02x}"));
                }
            }
            let (code, bytes, stderr) = bounded_control(
                Command::new(inspector)
                    .env_clear()
                    .envs(&self.configuration.runtime_environment)
                    .args([
                        "--user",
                        "--timeout=2",
                        "get-property",
                        "org.freedesktop.systemd1",
                    ])
                    .arg(object)
                    .args([
                        "org.freedesktop.systemd1.Scope",
                        "RuntimeMaxUSec",
                        "TimeoutStopUSec",
                        "KillMode",
                        "KillSignal",
                        "SendSIGKILL",
                        "FinalKillSignal",
                    ]),
                Duration::from_secs(3),
            )?;
            let actual = String::from_utf8(bytes)
                .map_err(|_| io::Error::other("systemd property encoding"))?;
            let expected = format!(
                "t {}\nt 100000\ns \"control-group\"\ni 9\nb true\ni 9\n",
                wall_time_ms.saturating_mul(1000)
            );
            if code != Some(0) || actual != expected {
                return Err(io::Error::other(format!(
                    "systemd deadline/termination properties differ: {actual:?}; {}",
                    String::from_utf8_lossy(&stderr)
                )));
            }
            Ok(actual)
        }

        pub(super) fn execute_linux(
            &self,
            engine: &Engine,
            executor: &SourceExecutorCapability,
            admission: &SourceAdmission,
            workspace: &mut PrivateWorkspace,
            check: &SourceCheckDefinition,
            guard: &mut dyn ExecutionGuard,
        ) -> Result<OciCheckRecord, OciError> {
            let plan = admission.plan();
            let preparation_started = Instant::now();
            let uid = rustix::process::getuid().as_raw();
            let gid = rustix::process::getgid().as_raw();
            if uid == 0 {
                return Err(OciError::Unsupported("host root execution"));
            }
            validate_private_directory(&self.configuration.private_parent)?;
            let _capacity = execution_lock(&self.configuration.private_parent)?;
            reject_unreconciled_runs(
                &self.configuration.private_parent,
                execution_identity(admission, check),
            )?;
            let tooling = (self.runtime.len()
                + self.inspector.len()
                + self.files.values().map(Vec::len).sum::<usize>())
                as u64;
            let retained = storage_bytes(&self.configuration.private_parent, &mut 0)?;
            let reserved = check
                .input()
                .resources
                .stdout_bytes
                .saturating_add(check.input().resources.stderr_bytes)
                .saturating_add((MAX_CONTROL * 8) as u64);
            // One provisioned closure plus one private copied closure, bounded
            // retained records and the complete admitted output allowance.
            if tooling
                .saturating_mul(2)
                .saturating_add(retained)
                .saturating_add(reserved)
                > MAX_TOOLING
            {
                return Err(OciError::Busy);
            }
            engine_continue(engine, executor, admission)?;
            let execution = execution_identity(admission, check);
            let prefix = preparation_prefix(&admission.binding().scope, execution);
            let temp = tempfile::Builder::new()
                .prefix(&prefix)
                .tempdir_in(&self.configuration.private_parent)?;
            let root = temp.path().to_path_buf();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
            let root_identity = fs::symlink_metadata(&root)?;
            let prepared_claim = root.join("prepared-claim");
            fs::create_dir(&prepared_claim)?;
            fs::set_permissions(&prepared_claim, fs::Permissions::from_mode(0o700))?;
            let cid = root
                .file_name()
                .and_then(|p| p.to_str())
                .ok_or(OciError::Unsupported("non-UTF8 private path"))?
                .to_ascii_lowercase();
            let runtime = root.join("runtime");
            write_executable(&runtime, &self.runtime)?;
            let inspector = root.join("systemd-inspector");
            write_executable(&inspector, &self.inspector)?;
            let state = root.join("state");
            let pidfile = root.join("worker.pid");
            let fsroot = root.join("rootfs");
            fs::create_dir(&fsroot)?;
            for directory in ["proc", "dev", "dev/pts", "dev/shm", "sys", "src", "scratch"] {
                fs::create_dir_all(fsroot.join(directory))?;
            }
            for (guest, bytes) in &self.files {
                let path = fsroot.join(&guest[1..]);
                fs::create_dir_all(path.parent().expect("guest parent"))?;
                write_executable(&path, bytes)?;
            }
            let input = check.input();
            let limits = input.resources;
            let cwd = match &input.working_directory {
                WorkingDirectory::WorkspaceRoot => "/src".into(),
                WorkingDirectory::Relative(path) => format!("/src/{path}"),
            };
            let env: Vec<_> = input
                .environment
                .iter()
                .map(|(k, v)| match v {
                    EnvironmentValue::Literal(v) => format!("{k}={v}"),
                    EnvironmentValue::ProtectedReference(_) => unreachable!("preflight rejected"),
                })
                .collect();
            let mut args = vec![LAUNCHER.to_string(), "--".into()];
            args.extend_from_slice(input.check.command());
            let config = json!({"ociVersion":"1.3.0","hostname":"celetixo","root":{"path":"rootfs","readonly":true},
                "annotations":{
                    "org.systemd.property.RuntimeMaxUSec":format!("uint64 {}",limits.wall_time_ms.saturating_mul(1000)),
                    "org.systemd.property.TimeoutStopUSec":"uint64 100000",
                    "org.systemd.property.KillMode":"'control-group'",
                    "org.systemd.property.KillSignal":"int32 9",
                    "org.systemd.property.SendSIGKILL":"true",
                    "org.systemd.property.FinalKillSignal":"int32 9",
                    "org.systemd.property.CollectMode":"'inactive-or-failed'"},
                "process":{"terminal":false,"user":{"uid":0,"gid":0,"additionalGids":[]},"args":args,"env":env,"cwd":cwd,
                    "noNewPrivileges":true,"capabilities":{"bounding":[],"effective":[],"permitted":[],"inheritable":[],"ambient":[]},
                    "rlimits":[{"type":"RLIMIT_NOFILE","soft":limits.file_descriptors,"hard":limits.file_descriptors},
                    {"type":"RLIMIT_CORE","soft":0,"hard":0},{"type":"RLIMIT_FSIZE","soft":limits.disk_bytes,"hard":limits.disk_bytes},
                    {"type":"RLIMIT_CPU","soft":limits.cpu_time_ms.div_ceil(1000),"hard":limits.cpu_time_ms.div_ceil(1000)}]},
                "mounts":[{"destination":"/proc","type":"proc","source":"proc","options":["nosuid","nodev","noexec","ro"]},
                    {"destination":"/dev","type":"tmpfs","source":"tmpfs","options":["nosuid","noexec","mode=755","size=65536"]},
                    {"destination":"/dev/pts","type":"devpts","source":"devpts","options":["nosuid","noexec","newinstance","ptmxmode=0666","mode=0620"]},
                    {"destination":"/dev/shm","type":"tmpfs","source":"shm","options":["nosuid","noexec","nodev","mode=700","size=4096"]},
                    {"destination":"/scratch","type":"tmpfs","source":"tmpfs","options":["nosuid","noexec","nodev","mode=700",format!("size={}",limits.disk_bytes)]},
                    {"destination":"/src","type":"bind","source":workspace.source_path(),"options":["rbind","rprivate","ro","nosuid","nodev","noexec"]}],
                "linux":{"uidMappings":[{"containerID":0,"hostID":uid,"size":1}],"gidMappings":[{"containerID":0,"hostID":gid,"size":1}],
                    "namespaces":[{"type":"pid"},{"type":"ipc"},{"type":"uts"},{"type":"mount"},{"type":"cgroup"},{"type":"user"},{"type":"network"}],
                    "cgroupsPath":format!("user.slice:celetixo:{cid}"),"resources":{"cpu":{"period":100000,"quota":25000},
                        "memory":{"limit":limits.memory_bytes,"swap":limits.memory_bytes},"pids":{"limit":limits.process_count},"unified":{"memory.oom.group":"1"}},
                    "maskedPaths":["/proc/acpi","/proc/asound","/proc/kcore","/proc/keys","/proc/timer_list","/proc/scsi"],
                    "readonlyPaths":["/dev","/dev/pts","/dev/shm","/proc/sys","/proc/irq","/proc/bus","/proc/sysrq-trigger"],"seccomp":seccomp_profile()}});
            let config_bytes = serde_json::to_vec(&config).expect("OCI JSON");
            durable_file(&root.join("config.json"), &config_bytes)?;
            guard_continue(guard)?;
            engine_continue(engine, executor, admission)?;
            workspace
                .validate_contents()
                .map_err(|_| OciError::Binding)?;
            let started_at_ms = engine
                .source_execution_time(executor, admission)
                .map_err(|e| OciError::Guard(e.to_string()))?;
            let source_root = workspace.source_path().parent().ok_or(OciError::Binding)?;
            let source_identity = validate_private_directory(source_root)?;
            let claim = execution_claim(&self.configuration.private_parent, execution);
            let journal_bytes = serde_json::to_vec(&json!({
                "version":1,"execution":execution.to_string(),"scope":scope_json(&admission.binding().scope),
                "backend":self.identity.digest().to_string(),"runtime":self.configuration.runtime.digest.to_string(),
                "rootfs":self.identity.image.to_string(),"config":Digest::of("celetixo/oci-config/v1",&config_bytes).to_string(),
                "root_name":root.file_name().expect("owned root name").to_str().expect("UTF8 checked"),
                "root_dev":root_identity.dev(),"root_ino":root_identity.ino(),"uid":uid,"gid":gid,"cid":cid,
                "source_root":source_root,"source_dev":source_identity.dev(),"source_ino":source_identity.ino(),"boot_id":boot_id()?,
                "admission":admission.id().to_string(),"transaction":admission.binding().transaction.as_str(),
                "transaction_attempt":admission.binding().transaction_attempt,"source_attempt":admission.binding().source_attempt.as_str(),
                "check":check.id().as_str(),"definition":check.digest().to_string(),"plan":plan.id().to_string(),
                "snapshot":workspace.report().snapshot().to_string(),"manifest":workspace.report().manifest().to_string(),
                "started_at_ms":started_at_ms,"wall_time_ms":limits.wall_time_ms
            })).expect("journal JSON");
            durable_file(&prepared_claim.join("config.json"), &config_bytes)?;
            durable_file(&prepared_claim.join("journal.json"), &journal_bytes)?;
            guard
                .checkpoint(ExecutionCheckpoint::BeforeClaimPublication)
                .map_err(OciError::Guard)?;
            rustix::fs::renameat_with(
                rustix::fs::CWD,
                &prepared_claim,
                rustix::fs::CWD,
                &claim,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(|error| {
                if error == rustix::io::Errno::EXIST {
                    OciError::AlreadyClaimed(execution)
                } else {
                    OciError::Io(error.into())
                }
            })?;
            // Once the complete launch claim is visible, no error or unwind may
            // erase its root before explicit reconciliation.
            let root = temp.keep();
            File::open(&self.configuration.private_parent)?.sync_all()?;
            File::open(&root)?.sync_all()?;
            workspace
                .retain_for_execution(&claim.join("journal.json"))
                .map_err(|error| OciError::Journal(error.to_string()))?;
            guard
                .checkpoint(ExecutionCheckpoint::AfterClaimPublication)
                .map_err(OciError::Guard)?;
            // After a durable launch claim exists, even an unwind must retain
            // resources for explicit reconciliation. No TempDir destructor may
            // remove a filesystem still referenced by a live process.
            let mut phase_timings_us =
                BTreeMap::from([("preparation".into(), elapsed_us(preparation_started))]);
            let started = Instant::now();
            let mut child = self
                .runtime_command(&runtime, &state)
                .arg("run")
                .arg("--keep")
                .arg("--bundle")
                .arg(&root)
                .arg("--pid-file")
                .arg(&pidfile)
                .arg(&cid)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            let mut stdout = child.stdout.take().expect("piped stdout");
            let mut stderr = child.stderr.take().expect("piped stderr");
            if let Err(error) = nonblocking(&stdout).and_then(|()| nonblocking(&stderr)) {
                child.stdin.take();
                let _ = child.kill();
                let _ = child.try_wait();
                workspace.quarantine_in_place(format!(
                    "output setup failed; runtime resources retained at {root:?}"
                ));
                return Err(OciError::Io(error));
            }
            let mut wire = Vec::new();
            let mut out = Vec::new();
            let mut err = Vec::new();
            let mut out_eof = false;
            let mut err_eof = false;
            let mut cgroup = None;
            let mut released = false;
            let mut released_at = None;
            let mut publication_wait_polls = 0_u64;
            let mut forced = None;
            let mut killed = false;
            let mut truncated_out = false;
            let mut truncated_err = false;
            let mut diagnostics = Vec::new();
            let mut observations = BTreeMap::new();
            let mut isolation = None;
            let mut cpu = 0;
            let mut peak = 0;
            let mut sampled = started;
            let mut status = None;
            loop {
                for (stream, dest, eof, maximum, truncated) in [
                    (
                        &mut stdout as &mut dyn Read,
                        &mut wire,
                        &mut out_eof,
                        limits.stdout_bytes.saturating_add(READY.len() as u64),
                        &mut truncated_out,
                    ),
                    (
                        &mut stderr as &mut dyn Read,
                        &mut err,
                        &mut err_eof,
                        limits.stderr_bytes,
                        &mut truncated_err,
                    ),
                ] {
                    let mut buffer = [0_u8; 8192];
                    for _ in 0..8 {
                        match stream.read(&mut buffer) {
                            Ok(0) => {
                                *eof = true;
                                break;
                            }
                            Ok(n) => {
                                let remaining = maximum.saturating_sub(dest.len() as u64) as usize;
                                dest.extend_from_slice(&buffer[..n.min(remaining)]);
                                if n > remaining {
                                    *truncated = true;
                                    forced.get_or_insert(SourceCheckOutcome::OutputLimit);
                                }
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) => {
                                diagnostics.push(format!("output read: {e}"));
                                forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                                    reason: "output read failed".into(),
                                });
                                *eof = true;
                                break;
                            }
                        }
                    }
                }
                match guard.revalidate() {
                    Ok(GuardDecision::Continue) => {}
                    Ok(d) => {
                        forced.get_or_insert(match d {
                            GuardDecision::Cancelled => SourceCheckOutcome::Cancelled,
                            GuardDecision::Expired => SourceCheckOutcome::Expired,
                            GuardDecision::Revoked => SourceCheckOutcome::Revoked,
                            GuardDecision::Continue => unreachable!(),
                        });
                    }
                    Err(e) => {
                        forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                            reason: format!("execution guard: {e}"),
                        });
                    }
                }
                if let Err(e) = engine_continue(engine, executor, admission) {
                    forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                        reason: format!("Engine revalidation: {e}"),
                    });
                }
                if started.elapsed() >= Duration::from_millis(limits.wall_time_ms) {
                    forced.get_or_insert(SourceCheckOutcome::TimedOut);
                }
                if !released && forced.is_none() && wire.len() >= READY.len() {
                    if !wire.starts_with(READY) {
                        forced = Some(SourceCheckOutcome::Infrastructure {
                            reason: "trusted launcher handshake differs".into(),
                        });
                    } else {
                        let publication = guard
                            .checkpoint(ExecutionCheckpoint::BeforeControlValidation)
                            .map_err(io::Error::other)
                            .and_then(|()| pid_file_published(&pidfile, uid));
                        match publication {
                            Ok(false) => {
                                publication_wait_polls = publication_wait_polls.saturating_add(1);
                                observations.insert(
                                    "pid_publication_wait_polls".into(),
                                    publication_wait_polls.to_string(),
                                );
                            }
                            Err(error) => {
                                forced = Some(SourceCheckOutcome::Infrastructure {
                                    reason: format!("PID publication: {error}"),
                                })
                            }
                            Ok(true) => {
                                match verify_controls(&pidfile, &cid, uid, limits).and_then(
                                    |path| {
                                        let properties = self.verify_scope_deadline(
                                            &inspector,
                                            &cid,
                                            limits.wall_time_ms,
                                        )?;
                                        let bytes = control_record(&pidfile, &path, &properties)?;
                                        durable_file(&claim.join("isolation.json"), &bytes)?;
                                        isolation = Some(bytes);
                                        observations
                                            .insert("systemd.scope.properties".into(), properties);
                                        Ok(path)
                                    },
                                ) {
                                    Ok(path) => {
                                        cgroup = Some(path);
                                        guard_continue(guard)
                                            .and_then(|()| {
                                                engine_continue(engine, executor, admission)
                                            })
                                            .map_err(|e| OciError::Guard(e.to_string()))
                                            .and_then(|()| {
                                                workspace
                                                    .validate_contents()
                                                    .map_err(|_| OciError::Binding)
                                            })
                                            .and_then(|()| {
                                                child
                                                    .stdin
                                                    .as_mut()
                                                    .ok_or(OciError::Unsupported(
                                                        "missing release pipe",
                                                    ))?
                                                    .write_all(&[1])
                                                    .map_err(OciError::Io)
                                            })
                                            .unwrap_or_else(|e| {
                                                forced = Some(SourceCheckOutcome::Infrastructure {
                                                    reason: e.to_string(),
                                                });
                                            });
                                        child.stdin.take();
                                        released = forced.is_none();
                                        if released {
                                            phase_timings_us
                                                .insert("startup".into(), elapsed_us(started));
                                            released_at = Some(Instant::now());
                                        }
                                    }
                                    Err(e) => {
                                        forced = Some(SourceCheckOutcome::Infrastructure {
                                            reason: format!("effective controls: {e}"),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(path) = &cgroup {
                    match sample(path, &mut observations) {
                        Ok(Some((usage, memory))) => {
                            cpu = usage;
                            peak = peak.max(memory);
                            sampled = Instant::now();
                            if cpu >= limits.cpu_time_ms.saturating_mul(1000) {
                                forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                                    reason: "aggregate CPU budget exhausted".into(),
                                });
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                                reason: format!("resource sampling: {e}"),
                            });
                        }
                    }
                }
                if forced.is_some() && !killed {
                    child.stdin.take();
                    let result = bounded_control(
                        self.runtime_command(&runtime, &state)
                            .arg("kill")
                            .arg("--all")
                            .arg(&cid)
                            .arg("KILL"),
                        Duration::from_secs(3),
                    );
                    match result {
                        Ok((Some(0), _, _message)) => {}
                        Ok((_, _, message)) => diagnostics.push(format!(
                            "kill request: {}",
                            String::from_utf8_lossy(&message)
                        )),
                        Err(e) => diagnostics.push(format!("kill request: {e}")),
                    }
                    killed = true;
                }
                if status.is_none() {
                    match child.try_wait() {
                        Ok(s) => status = s,
                        Err(e) => {
                            forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                                reason: format!("runtime wait: {e}"),
                            });
                        }
                    }
                }
                if status.is_some() && out_eof && err_eof {
                    break;
                }
                if started.elapsed()
                    > Duration::from_millis(limits.wall_time_ms)
                        .saturating_add(Duration::from_secs(8))
                {
                    diagnostics.push("runtime or process termination was not established".into());
                    let _ = child.kill();
                    let _ = child.try_wait();
                    break;
                }
                std::thread::sleep(POLL);
            }
            if wire.starts_with(READY) {
                out.extend_from_slice(&wire[READY.len()..]);
            } else if !wire.is_empty() {
                diagnostics.push("unrecognized launcher output".into());
            }
            let owned_group = group_path(uid, &cid);
            let termination_confirmed =
                status.is_some() && group_empty(&owned_group).unwrap_or(false);
            let elapsed = started.elapsed();
            if let Some(released_at) = released_at {
                phase_timings_us.insert("command_supervision".into(), elapsed_us(released_at));
            }
            let conservative_cpu = cpu.saturating_add(
                (sampled.elapsed().as_micros().min(u128::from(u64::MAX)) as u64)
                    .saturating_mul(self.host_cpus),
            );
            if conservative_cpu > limits.cpu_time_ms.saturating_mul(1000) {
                forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                    reason: "CPU upper bound exceeds the admitted budget".into(),
                });
            }
            if let Err(e) = engine_continue(engine, executor, admission) {
                forced.get_or_insert(SourceCheckOutcome::Infrastructure {
                    reason: format!("final Engine revalidation: {e}"),
                });
            }
            let mut outcome = if !termination_confirmed {
                SourceCheckOutcome::UnknownTermination {
                    reason: "process-tree termination unconfirmed".into(),
                }
            } else if let Some(reason) = forced {
                reason
            } else if !released {
                SourceCheckOutcome::Infrastructure {
                    reason: "command was not released".into(),
                }
            } else {
                match status.and_then(|s| s.code()) {
                    Some(code) if code < 128 => SourceCheckOutcome::Exited { code },
                    Some(code) => SourceCheckOutcome::Infrastructure {
                        reason: format!(
                            "OCI runtime exit {code} ambiguously encodes a signal or command exit"
                        ),
                    },
                    None => SourceCheckOutcome::Infrastructure {
                        reason: "runtime controller terminated by a signal".into(),
                    },
                }
            };
            let mut post_source_manifest = None;
            if workspace.validate_contents().is_ok() {
                post_source_manifest = Some(workspace.report().manifest());
            } else {
                diagnostics.push("post-check source validation failed".into());
                outcome = SourceCheckOutcome::Infrastructure {
                    reason: "post-check source differs".into(),
                };
            }
            let cleanup_started = Instant::now();
            let delete = bounded_control(
                self.runtime_command(&runtime, &state)
                    .arg("delete")
                    .arg("--force")
                    .arg(&cid),
                Duration::from_secs(3),
            );
            let root_unchanged = fs::symlink_metadata(&root).is_ok_and(|m| {
                m.dev() == root_identity.dev() && m.ino() == root_identity.ino() && m.is_dir()
            });
            let mut safe_cleanup = termination_confirmed
                && root_unchanged
                && delete.as_ref().is_ok_and(|(code, _, _)| *code == Some(0));
            if safe_cleanup {
                let journal: Value =
                    serde_json::from_slice(&journal_bytes).expect("constructed journal");
                if let Err(error) = durable_file(
                    &claim.join("cleanup-confirmed.json"),
                    &cleanup_confirmation(&journal, &journal_bytes),
                ) {
                    safe_cleanup = false;
                    diagnostics.push(format!("durable cleanup confirmation: {error}"));
                    outcome = SourceCheckOutcome::Infrastructure {
                        reason: "cleanup confirmation could not be persisted".into(),
                    };
                }
            }
            let cleanup = if safe_cleanup {
                match fs::remove_dir_all(&root) {
                    Ok(()) => CleanupOutcome::Removed,
                    Err(e) => CleanupOutcome::Quarantined {
                        path: root.clone(),
                        reason: e.to_string(),
                    },
                }
            } else {
                let reason =
                    "termination, runtime cleanup or private directory identity is uncertain"
                        .to_string();
                workspace.quarantine_in_place(reason.clone());
                CleanupOutcome::Quarantined {
                    path: root.clone(),
                    reason,
                }
            };
            if !matches!(cleanup, CleanupOutcome::Removed) {
                diagnostics.push("execution resources retained for reconciliation".into());
            } else if let Err(error) = workspace.release_after_execution() {
                diagnostics.push(format!("source retention latch: {error}"));
                workspace.quarantine_in_place(error.to_string());
                outcome = SourceCheckOutcome::Infrastructure {
                    reason: "source cleanup retention could not be released".into(),
                };
            }
            phase_timings_us.insert("runtime_cleanup".into(), elapsed_us(cleanup_started));
            let record = OciCheckRecord {
                execution,
                journal: Digest::of("celetixo/oci-journal/v1", &journal_bytes),
                check: check.id().clone(),
                definition: check.digest(),
                plan: plan.id(),
                snapshot: workspace.report().snapshot(),
                manifest: workspace.report().manifest(),
                backend: self.identity.clone(),
                runtime: self.configuration.runtime.digest,
                rootfs: self.identity.image,
                launcher: executable_digest(self.files.get(LAUNCHER).expect("launcher pinned")),
                isolation,
                started_at_ms,
                finished_at_ms: engine
                    .source_execution_time(executor, admission)
                    .unwrap_or(started_at_ms),
                outcome,
                usage: SourceResourceUsage {
                    wall_time_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
                    cpu_time_us: cpu,
                    peak_memory_bytes: peak,
                },
                resource_observations: observations,
                phase_timings_us,
                stdout: out,
                stderr: err,
                stdout_truncated: truncated_out,
                stderr_truncated: truncated_err,
                termination_confirmed,
                cleanup,
                post_source_manifest,
                diagnostics,
            };
            guard
                .checkpoint(ExecutionCheckpoint::BeforeRecordPublication)
                .map_err(OciError::Guard)?;
            durable_file(&claim.join("stdout"), &record.stdout)?;
            durable_file(&claim.join("stderr"), &record.stderr)?;
            durable_file(&claim.join("record.json"), &record.canonical_bytes())?;
            guard
                .checkpoint(ExecutionCheckpoint::AfterRecordPublication)
                .map_err(OciError::Guard)?;
            Ok(record)
        }
    }

    fn write_executable(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut f = File::options().write(true).create_new(true).open(path)?;
        f.write_all(bytes)?;
        f.set_permissions(fs::Permissions::from_mode(0o555))?;
        f.sync_all()
    }
    fn nonblocking(fd: &impl std::os::fd::AsFd) -> io::Result<()> {
        let flags = rustix::fs::fcntl_getfl(fd)?;
        rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?;
        Ok(())
    }

    fn bounded_control(
        command: &mut Command,
        timeout: Duration,
    ) -> io::Result<(Option<i32>, Vec<u8>, Vec<u8>)> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut out = child.stdout.take().expect("control stdout");
        let mut err = child.stderr.take().expect("control stderr");
        nonblocking(&out)?;
        nonblocking(&err)?;
        let mut a = Vec::new();
        let mut b = Vec::new();
        let start = Instant::now();
        loop {
            for (r, v) in [
                (&mut out as &mut dyn Read, &mut a),
                (&mut err as &mut dyn Read, &mut b),
            ] {
                let mut buf = [0; 4096];
                match r.read(&mut buf) {
                    Ok(n) => {
                        if v.len() + n > MAX_CONTROL {
                            child.kill()?;
                            let _ = child.wait();
                            return Err(io::Error::other("control output exceeded"));
                        }
                        v.extend_from_slice(&buf[..n]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e),
                }
            }
            if let Some(status) = child.try_wait()? {
                return Ok((status.code(), a, b));
            }
            if start.elapsed() > timeout {
                child.kill()?;
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "runtime control timed out",
                ));
            }
            std::thread::sleep(POLL);
        }
    }

    // runc publishes its PID file after Container.Run returns. A trusted
    // launcher can announce readiness first. Only an absent final PID marker
    // is pending; invalid markers and all failed control checks still fail.
    fn pid_file_published(path: &Path, uid: u32) -> io::Result<bool> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.nlink() != 1
            || !(1..=16).contains(&metadata.len())
        {
            return Err(io::Error::other(
                "PID publication marker identity or size differs",
            ));
        }
        Ok(true)
    }

    fn verify_controls(
        pidfile: &Path,
        cid: &str,
        uid: u32,
        limits: celetixo_engine::source_execution::ResourceLimits,
    ) -> io::Result<PathBuf> {
        let pid = fs::read_to_string(pidfile)?
            .trim()
            .parse::<u32>()
            .map_err(|_| io::Error::other("PID syntax"))?;
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let raw = fs::read_to_string(proc.join("cgroup"))?;
        let rel = raw
            .strip_prefix("0::/")
            .and_then(|p| p.strip_suffix('\n'))
            .ok_or_else(|| io::Error::other("cgroup v2 binding"))?;
        let expected = format!(
            "user.slice/user-{uid}.slice/user@{uid}.service/user.slice/celetixo-{cid}.scope"
        );
        if rel != expected {
            return Err(io::Error::other("cgroup path differs"));
        }
        let group = PathBuf::from("/sys/fs/cgroup").join(rel);
        for (name, expected) in [
            ("cpu.max", "25000 100000".into()),
            ("memory.max", limits.memory_bytes.to_string()),
            ("memory.swap.max", "0".into()),
            ("pids.max", limits.process_count.to_string()),
            ("memory.oom.group", "1".into()),
        ] {
            if fs::read_to_string(group.join(name))?.trim() != expected {
                return Err(io::Error::other(format!("effective {name} differs")));
            }
        }
        for name in ["user", "pid", "mnt", "net", "ipc", "uts", "cgroup"] {
            if fs::read_link(proc.join("ns").join(name))?
                == fs::read_link(Path::new("/proc/self/ns").join(name))?
            {
                return Err(io::Error::other("namespace was not isolated"));
            }
        }
        let status = fs::read_to_string(proc.join("status"))?;
        for field in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
            if !status.lines().any(|l| {
                l.starts_with(&format!("{field}:"))
                    && l.split_whitespace().nth(1) == Some("0000000000000000")
            }) {
                return Err(io::Error::other("capability set is not empty"));
            }
        }
        if !status
            .lines()
            .any(|l| l.starts_with("NoNewPrivs:") && l.split_whitespace().nth(1) == Some("1"))
            || !status
                .lines()
                .any(|l| l.starts_with("Seccomp:") && l.split_whitespace().nth(1) == Some("2"))
        {
            return Err(io::Error::other("privilege/seccomp controls absent"));
        }
        let actual = fs::read_to_string(proc.join("limits"))?;
        if !actual.lines().any(|l| {
            l.strip_prefix("Max open files").is_some_and(|r| {
                r.split_whitespace()
                    .take(2)
                    .all(|n| n == limits.file_descriptors.to_string())
            })
        }) {
            return Err(io::Error::other("file descriptor limit differs"));
        }
        for (label, bound) in [
            ("Max cpu time", limits.cpu_time_ms.div_ceil(1000)),
            ("Max file size", limits.disk_bytes),
            ("Max core file size", 0),
        ] {
            if !actual.lines().any(|line| {
                line.strip_prefix(label).is_some_and(|suffix| {
                    let values: Vec<_> = suffix.split_whitespace().take(2).collect();
                    values.len() == 2 && values.iter().all(|value| *value == bound.to_string())
                })
            }) {
                return Err(io::Error::other(format!("effective {label} differs")));
            }
        }
        let mounts = fs::read_to_string(proc.join("mountinfo"))?;
        for (point, flags) in [
            ("/", vec!["ro"]),
            ("/src", vec!["ro", "nosuid", "nodev", "noexec"]),
            ("/scratch", vec!["rw", "nosuid", "nodev", "noexec"]),
        ] {
            if !mounts.lines().any(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                fields.get(4) == Some(&point)
                    && fields.get(5).is_some_and(|actual| {
                        flags
                            .iter()
                            .all(|flag| actual.split(',').any(|value| value == *flag))
                    })
            }) {
                return Err(io::Error::other(format!(
                    "effective {point} mount flags differ"
                )));
            }
        }
        Ok(group)
    }

    fn control_record(pidfile: &Path, cgroup: &Path, systemd: &str) -> io::Result<Vec<u8>> {
        let pid = fs::read_to_string(pidfile)?
            .trim()
            .parse::<u32>()
            .map_err(|_| io::Error::other("PID syntax"))?;
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let mut records = BTreeMap::new();
        for name in [
            "status",
            "limits",
            "mountinfo",
            "cgroup",
            "uid_map",
            "gid_map",
        ] {
            records.insert(format!("proc/{name}"), fs::read_to_string(proc.join(name))?);
        }
        for name in [
            "cpu.max",
            "memory.max",
            "memory.swap.max",
            "pids.max",
            "memory.oom.group",
        ] {
            records.insert(
                format!("cgroup/{name}"),
                fs::read_to_string(cgroup.join(name))?,
            );
        }
        for name in ["user", "pid", "mnt", "net", "ipc", "uts", "cgroup"] {
            records.insert(
                format!("namespace/{name}"),
                fs::read_link(proc.join("ns").join(name))?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        records.insert("systemd.scope.properties".into(), systemd.into());
        let bytes = serde_json::to_vec(&json!({"version":1,"effective_controls":records}))
            .expect("isolation JSON");
        if bytes.len() > MAX_CONTROL {
            return Err(io::Error::other("effective control record exceeds bound"));
        }
        Ok(bytes)
    }

    fn sample(
        path: &Path,
        observations: &mut BTreeMap<String, String>,
    ) -> io::Result<Option<(u64, u64)>> {
        for name in [
            "cpu.stat",
            "memory.peak",
            "memory.events",
            "pids.current",
            "pids.peak",
        ] {
            match fs::read_to_string(path.join(name)) {
                Ok(s) => {
                    observations.insert(name.into(), s);
                }
                Err(e) if matches!(e.raw_os_error(), Some(2 | 19)) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
        let cpu = observations
            .get("cpu.stat")
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("usage_usec ")))
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::other("CPU counter syntax"))?;
        let memory = observations
            .get("memory.peak")
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| io::Error::other("memory counter syntax"))?;
        Ok(Some((cpu, memory)))
    }
    #[cfg(test)]
    mod publication_tests {
        use super::*;
        #[test]
        fn missing_pid_marker_waits_but_invalid_and_aliased_markers_fail() {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("worker.pid");
            let uid = rustix::process::getuid().as_raw();
            assert!(!pid_file_published(&path, uid).unwrap());
            fs::write(&path, b"12345").unwrap();
            assert!(pid_file_published(&path, uid).unwrap());
            assert!(pid_file_published(&path, uid.saturating_add(1)).is_err());
            fs::hard_link(&path, root.path().join("alias")).unwrap();
            assert!(pid_file_published(&path, uid).is_err());
            fs::remove_file(root.path().join("alias")).unwrap();
            fs::write(&path, b"").unwrap();
            assert!(pid_file_published(&path, uid).is_err());
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink("absent", &path).unwrap();
            assert!(pid_file_published(&path, uid).is_err());
        }
    }
}
