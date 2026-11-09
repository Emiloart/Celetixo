//! Immutable source execution records. Records confer no verification authority.
//!
//! The Engine authenticates a registered executor, revalidates admission and reads
//! every object through its trusted, scoped storage adapter before retaining a result.
use crate::engine::SourceAdmission;
use crate::ids::{
    Canonical, CheckId, Digest, PrincipalId, Scope, SnapshotId, SourceAdmissionId,
    SourceEvidenceId, SourcePlanId,
};
use crate::source_execution::{FailurePolicy, ResourceLimits, VerificationPlan};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const SOURCE_ARTIFACT_DOMAIN: &str = "celetixo/source-artifact/v1";
const MAX_RECORD_METADATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_EVIDENCE_METADATA_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceArtifactError {
    #[error("artifact is unavailable: {0}")]
    Unavailable(String),
    #[error("artifact scope is not authorized")]
    Scope,
    #[error("artifact exceeds its retrieval limit")]
    Limit,
    #[error("artifact content does not match its address")]
    Integrity,
    #[error("artifact persistence or retention failed: {0}")]
    Persistence(String),
}

/// Installed by the trusted embedding, never supplied by a worker at evidence delivery.
/// A local adapter must confirm file and directory synchronization before publication.
pub trait SourceArtifactReader: Send + Sync {
    fn read(
        &self,
        scope: &Scope,
        address: Digest,
        max_bytes: u64,
    ) -> Result<Vec<u8>, SourceArtifactError>;
    fn retention_policy(&self) -> Digest;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceArtifactReference {
    pub scope: Scope,
    pub name: String,
    pub media_type: String,
    pub content: Digest,
    pub bytes: u64,
    pub retention_policy: Digest,
}

impl SourceArtifactReference {
    pub fn for_bytes(
        scope: Scope,
        name: String,
        media_type: String,
        bytes: &[u8],
        retention_policy: Digest,
    ) -> Self {
        Self {
            scope,
            name,
            media_type,
            content: Digest::of(SOURCE_ARTIFACT_DOMAIN, bytes),
            bytes: bytes.len() as u64,
            retention_policy,
        }
    }
    fn encode(&self, c: &mut Canonical) {
        encode_scope(c, &self.scope);
        c.str(&self.name);
        c.str(&self.media_type);
        c.digest(self.content);
        c.u64(self.bytes);
        c.digest(self.retention_policy);
    }
}

/// The v2 envelope makes result attestation explicit. Check records and artifact
/// addresses retain their independent v1 formats; v1 evidence is never relabelled.
pub const SOURCE_EVIDENCE_FORMAT: &str = "celetixo/source-evidence/v2";
pub const SOURCE_EVIDENCE_MEDIA_TYPE: &str = "application/vnd.celetixo.source-evidence-v2";

/// Result-provenance strength, independent of check success and physical
/// isolation. No hardware or remote-worker attestation is implemented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAttestationLevel {
    Unattested,
    /// The Engine must separately authenticate the current scoped executor
    /// capability. An enum value or principal string is never that capability.
    EngineScopedExecutor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidenceBinding {
    admission: SourceAdmissionId,
    scope: Scope,
    executor: PrincipalId,
    retention_policy: Digest,
    /// Canonical admission identity transitively binds the complete patch, transaction,
    /// authorization, lease, analysis, candidate, policy and verification plan.
    admission_contract: Digest,
    attestation: SourceAttestationLevel,
}

impl SourceEvidenceBinding {
    /// Construct expected metadata, not authentication. The Engine separately
    /// verifies the actual executor capability before and after artifact I/O.
    pub fn for_admission(admission: &SourceAdmission, executor: PrincipalId) -> Self {
        Self {
            admission: admission.id(),
            scope: admission.binding().scope.clone(),
            executor,
            retention_policy: admission.retention_policy(),
            admission_contract: admission.id().0,
            attestation: SourceAttestationLevel::EngineScopedExecutor,
        }
    }
    pub fn admission(&self) -> SourceAdmissionId {
        self.admission
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn executor(&self) -> &PrincipalId {
        &self.executor
    }
    pub fn retention_policy(&self) -> Digest {
        self.retention_policy
    }
    pub fn attestation(&self) -> SourceAttestationLevel {
        self.attestation
    }
    /// Declared metadata only. The current Engine gate rejects any level other
    /// than its authenticated scoped-executor contract, including Unattested.
    pub fn with_attestation(mut self, attestation: SourceAttestationLevel) -> Self {
        self.attestation = attestation;
        self
    }
    fn encode(&self, c: &mut Canonical) {
        c.digest(self.admission.0);
        encode_scope(c, &self.scope);
        c.str(self.executor.as_str());
        c.digest(self.retention_policy);
        c.digest(self.admission_contract);
        c.u64(match self.attestation {
            SourceAttestationLevel::Unattested => 0,
            SourceAttestationLevel::EngineScopedExecutor => 1,
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceCheckOutcome {
    Exited { code: i32 },
    Signalled { signal: i32 },
    TimedOut,
    Cancelled,
    Expired,
    Revoked,
    OutputLimit,
    Infrastructure { reason: String },
    Skipped { reason: String },
    UnknownTermination { reason: String },
}

impl SourceCheckOutcome {
    fn encode(&self, c: &mut Canonical) {
        match self {
            Self::Exited { code } => {
                c.u64(0);
                c.i64(i64::from(*code));
            }
            Self::Signalled { signal } => {
                c.u64(1);
                c.i64(i64::from(*signal));
            }
            Self::TimedOut => c.u64(2),
            Self::Cancelled => c.u64(3),
            Self::Expired => c.u64(4),
            Self::Revoked => c.u64(5),
            Self::OutputLimit => c.u64(6),
            Self::Infrastructure { reason } => {
                c.u64(7);
                c.str(reason);
            }
            Self::Skipped { reason } => {
                c.u64(8);
                c.str(reason);
            }
            Self::UnknownTermination { reason } => {
                c.u64(9);
                c.str(reason);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceCleanup {
    Confirmed,
    Quarantined { reason: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceTermination {
    Confirmed,
    Unknown { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceResourceUsage {
    pub wall_time_ms: u64,
    pub cpu_time_us: u64,
    pub peak_memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckRecordInput {
    pub admission: SourceAdmissionId,
    pub plan: SourcePlanId,
    pub candidate: SnapshotId,
    pub check: CheckId,
    pub definition: Digest,
    pub backend: Digest,
    pub executable: Digest,
    pub runtime_closure: Digest,
    pub environment: Digest,
    pub limits: ResourceLimits,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub outcome: SourceCheckOutcome,
    pub usage: SourceResourceUsage,
    pub post_source_manifest: Option<Digest>,
    pub termination: SourceTermination,
    pub cleanup: SourceCleanup,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    /// Always includes stdout, stderr, diagnostics and execution-record; policy
    /// may require more. Missing mandatory artifacts yield incomplete evidence.
    pub artifacts: BTreeMap<String, SourceArtifactReference>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckRecord {
    input: SourceCheckRecordInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceEvidenceError {
    #[error("invalid or oversized source evidence metadata")]
    InvalidMetadata,
    #[error("source evidence binding differs from current Engine admission")]
    BindingMismatch,
    #[error("source evidence contains missing, duplicate or unplanned checks")]
    CheckSet,
    #[error("source check configuration differs from its authorized plan: {0}")]
    CheckMismatch(CheckId),
    #[error("artifact identity, retention or scope differs from its requirement: {0}")]
    ArtifactMismatch(String),
    #[error("source artifact retrieval failed: {0}")]
    Artifact(#[from] SourceArtifactError),
}

impl SourceCheckRecord {
    pub fn new(input: SourceCheckRecordInput) -> Result<Self, SourceEvidenceError> {
        input
            .limits
            .validate()
            .map_err(|_| SourceEvidenceError::InvalidMetadata)?;
        if input.finished_at_ms < input.started_at_ms
            || input.artifacts.len() > 4096
            || input.diagnostics.len() > 4096
            || input
                .diagnostics
                .iter()
                .any(|s| s.len() > 64 * 1024 || s.contains('\0'))
            || input.artifacts.iter().any(|(name, object)| {
                name != &object.name
                    || !valid_text(name, 4096)
                    || !valid_text(&object.media_type, 1024)
            })
        {
            return Err(SourceEvidenceError::InvalidMetadata);
        }
        let metadata_bytes = input
            .diagnostics
            .iter()
            .map(|s| s.len().saturating_add(8))
            .chain(input.artifacts.values().map(|o| {
                o.name
                    .len()
                    .saturating_add(o.media_type.len())
                    .saturating_add(o.scope.organization.as_str().len())
                    .saturating_add(o.scope.workspace.as_str().len())
                    .saturating_add(o.scope.repository.as_str().len())
                    .saturating_add(256)
            }))
            .fold(1024_usize, usize::saturating_add);
        if metadata_bytes > MAX_RECORD_METADATA_BYTES {
            return Err(SourceEvidenceError::InvalidMetadata);
        }
        match &input.outcome {
            SourceCheckOutcome::Exited { code } if !(0..=255).contains(code) => {
                return Err(SourceEvidenceError::InvalidMetadata);
            }
            SourceCheckOutcome::Signalled { signal } if !(1..=64).contains(signal) => {
                return Err(SourceEvidenceError::InvalidMetadata);
            }
            SourceCheckOutcome::Infrastructure { reason }
            | SourceCheckOutcome::Skipped { reason }
            | SourceCheckOutcome::UnknownTermination { reason }
                if !valid_text(reason, 64 * 1024) =>
            {
                return Err(SourceEvidenceError::InvalidMetadata);
            }
            _ => {}
        }
        if let SourceTermination::Unknown { reason } = &input.termination
            && !valid_text(reason, 64 * 1024)
        {
            return Err(SourceEvidenceError::InvalidMetadata);
        }
        match &input.cleanup {
            SourceCleanup::Quarantined { reason } | SourceCleanup::Failed { reason }
                if !valid_text(reason, 64 * 1024) =>
            {
                return Err(SourceEvidenceError::InvalidMetadata);
            }
            _ => {}
        }
        let result = Self { input };
        let mut encoded = Canonical::default();
        result.encode(&mut encoded, true);
        if encoded.into_bytes().len() > MAX_RECORD_METADATA_BYTES {
            return Err(SourceEvidenceError::InvalidMetadata);
        }
        Ok(result)
    }
    pub fn input(&self) -> &SourceCheckRecordInput {
        &self.input
    }

    /// Stored separately and compared byte-for-byte by the Engine. Excludes its
    /// own object reference to avoid a recursive content address.
    pub fn execution_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str("celetixo/source-check-record/v1");
        self.encode(&mut c, false);
        c.into_bytes()
    }
    fn encode(&self, c: &mut Canonical, include_record: bool) {
        let i = &self.input;
        c.digest(i.admission.0);
        c.digest(i.plan.0);
        c.digest(i.candidate.0);
        c.str(i.check.as_str());
        c.digest(i.definition);
        c.digest(i.backend);
        c.digest(i.executable);
        c.digest(i.runtime_closure);
        c.digest(i.environment);
        encode_limits(c, i.limits);
        c.u64(i.started_at_ms);
        c.u64(i.finished_at_ms);
        i.outcome.encode(c);
        c.u64(i.usage.wall_time_ms);
        c.u64(i.usage.cpu_time_us);
        c.u64(i.usage.peak_memory_bytes);
        match i.post_source_manifest {
            Some(d) => {
                c.u64(1);
                c.digest(d);
            }
            None => c.u64(0),
        }
        match &i.termination {
            SourceTermination::Confirmed => c.u64(0),
            SourceTermination::Unknown { reason } => {
                c.u64(1);
                c.str(reason);
            }
        }
        match &i.cleanup {
            SourceCleanup::Confirmed => c.u64(0),
            SourceCleanup::Quarantined { reason } => {
                c.u64(1);
                c.str(reason);
            }
            SourceCleanup::Failed { reason } => {
                c.u64(2);
                c.str(reason);
            }
        }
        c.u64(u64::from(i.stdout_truncated));
        c.u64(u64::from(i.stderr_truncated));
        let objects = i
            .artifacts
            .iter()
            .filter(|(name, _)| include_record || name.as_str() != "execution-record")
            .collect::<Vec<_>>();
        c.u64(objects.len() as u64);
        for (_, object) in objects {
            object.encode(c);
        }
        c.u64(i.diagnostics.len() as u64);
        for diagnostic in &i.diagnostics {
            c.str(diagnostic);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidence {
    id: SourceEvidenceId,
    binding: SourceEvidenceBinding,
    started_at_ms: u64,
    finished_at_ms: u64,
    checks: BTreeMap<CheckId, SourceCheckRecord>,
    provenance: BTreeMap<String, SourceArtifactReference>,
    diagnostics: Vec<String>,
}

impl SourceEvidence {
    pub fn new(
        binding: SourceEvidenceBinding,
        started_at_ms: u64,
        finished_at_ms: u64,
        checks: Vec<SourceCheckRecord>,
        provenance: BTreeMap<String, SourceArtifactReference>,
        diagnostics: Vec<String>,
    ) -> Result<Self, SourceEvidenceError> {
        if finished_at_ms < started_at_ms
            || checks.is_empty()
            || checks.len() > 4096
            // A plan can contain 4096 checks. Each has an exact canonical
            // definition in addition to the nine global provenance objects.
            || provenance.len() > 4096 + 9
            || provenance.iter().any(|(name, object)| {
                name != &object.name
                    || !valid_text(name, 4096)
                    || !valid_text(&object.media_type, 1024)
            })
            || diagnostics.len() > 4096
            || diagnostics
                .iter()
                .any(|s| s.len() > 64 * 1024 || s.contains('\0'))
        {
            return Err(SourceEvidenceError::InvalidMetadata);
        }
        let mut records = BTreeMap::new();
        let mut metadata_bytes = diagnostics
            .iter()
            .map(|s| s.len().saturating_add(8))
            .chain(provenance.values().map(|o| {
                o.name
                    .len()
                    .saturating_add(o.media_type.len())
                    .saturating_add(o.scope.organization.as_str().len())
                    .saturating_add(o.scope.workspace.as_str().len())
                    .saturating_add(o.scope.repository.as_str().len())
                    .saturating_add(256)
            }))
            .fold(1024_usize, usize::saturating_add);
        for check in checks {
            let mut encoded = Canonical::default();
            check.encode(&mut encoded, true);
            metadata_bytes = metadata_bytes.saturating_add(encoded.into_bytes().len());
            if metadata_bytes > MAX_EVIDENCE_METADATA_BYTES {
                return Err(SourceEvidenceError::InvalidMetadata);
            }
            if check.input.started_at_ms < started_at_ms
                || check.input.finished_at_ms > finished_at_ms
            {
                return Err(SourceEvidenceError::InvalidMetadata);
            }
            if records.insert(check.input.check.clone(), check).is_some() {
                return Err(SourceEvidenceError::CheckSet);
            }
        }
        let mut result = Self {
            id: SourceEvidenceId(Digest::of("uninitialized", b"")),
            binding,
            started_at_ms,
            finished_at_ms,
            checks: records,
            provenance,
            diagnostics,
        };
        let bytes = result.canonical_bytes();
        if bytes.len() > MAX_EVIDENCE_METADATA_BYTES {
            return Err(SourceEvidenceError::InvalidMetadata);
        }
        result.id = SourceEvidenceId(Digest::of(SOURCE_EVIDENCE_FORMAT, &bytes));
        Ok(result)
    }
    pub fn id(&self) -> SourceEvidenceId {
        self.id
    }
    pub fn binding(&self) -> &SourceEvidenceBinding {
        &self.binding
    }
    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }
    pub fn finished_at_ms(&self) -> u64 {
        self.finished_at_ms
    }
    pub fn checks(&self) -> &BTreeMap<CheckId, SourceCheckRecord> {
        &self.checks
    }
    pub fn provenance(&self) -> &BTreeMap<String, SourceArtifactReference> {
        &self.provenance
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(SOURCE_EVIDENCE_FORMAT);
        self.binding.encode(&mut c);
        c.u64(self.started_at_ms);
        c.u64(self.finished_at_ms);
        c.u64(self.checks.len() as u64);
        for record in self.checks.values() {
            record.encode(&mut c, true);
        }
        c.u64(self.provenance.len() as u64);
        for object in self.provenance.values() {
            object.encode(&mut c);
        }
        c.u64(self.diagnostics.len() as u64);
        for diagnostic in &self.diagnostics {
            c.str(diagnostic);
        }
        c.into_bytes()
    }

    /// This pure/storage verification grants no authority. The Engine must validate
    /// current admission and executor capabilities before and after the storage I/O.
    pub fn validate_persisted(
        &self,
        admission: &SourceAdmission,
        executor: &PrincipalId,
        envelope: &SourceArtifactReference,
        reader: &dyn SourceArtifactReader,
        expected_provenance: &BTreeMap<String, Vec<u8>>,
    ) -> Result<SourceEvidenceAssessment, SourceEvidenceError> {
        if self.binding != SourceEvidenceBinding::for_admission(admission, executor.clone())
            || self.started_at_ms < admission.admitted_at_ms()
            || self.finished_at_ms > admission.expires_at_ms()
            || reader.retention_policy() != self.binding.retention_policy
        {
            return Err(SourceEvidenceError::BindingMismatch);
        }
        let canonical = self.canonical_bytes();
        if envelope.name != "sealed-evidence" || envelope.media_type != SOURCE_EVIDENCE_MEDIA_TYPE {
            return Err(SourceEvidenceError::ArtifactMismatch(envelope.name.clone()));
        }
        if read_verified_object(envelope, &self.binding, reader, canonical.len() as u64)?
            != canonical
        {
            return Err(SourceArtifactError::Integrity.into());
        }
        if self.provenance.keys().collect::<BTreeSet<_>>()
            != expected_provenance.keys().collect::<BTreeSet<_>>()
        {
            return Err(SourceEvidenceError::ArtifactMismatch(
                "provenance object set".into(),
            ));
        }
        for (name, bytes) in expected_provenance {
            let object = &self.provenance[name];
            if read_verified_object(object, &self.binding, reader, bytes.len() as u64)? != *bytes {
                return Err(SourceArtifactError::Integrity.into());
            }
        }
        self.validate_checks(admission.plan(), admission.resulting_manifest(), reader)
    }

    fn validate_checks(
        &self,
        plan: &VerificationPlan,
        manifest: Digest,
        reader: &dyn SourceArtifactReader,
    ) -> Result<SourceEvidenceAssessment, SourceEvidenceError> {
        let expected = plan
            .checks()
            .iter()
            .map(|c| c.definition().id().clone())
            .collect::<BTreeSet<_>>();
        if self.checks.keys().cloned().collect::<BTreeSet<_>>() != expected {
            return Err(SourceEvidenceError::CheckSet);
        }
        let mut failed = BTreeSet::new();
        let mut incomplete = BTreeSet::new();
        for planned in plan.checks() {
            let definition = planned.definition();
            let policy = definition.input();
            let record = &self.checks[definition.id()];
            let i = record.input();
            if i.admission != self.binding.admission
                || i.plan != plan.id()
                || i.candidate != plan.binding().resulting_snapshot
                || i.definition != definition.digest()
                || i.backend != policy.backend.digest()
                || i.executable != policy.toolchain.executable_digest
                || i.runtime_closure != policy.toolchain.runtime_closure
                || i.environment != policy.check.environment()
                || i.limits != policy.resources
            {
                return Err(SourceEvidenceError::CheckMismatch(i.check.clone()));
            }
            let mut total = 0_u64;
            let allowed = policy
                .expected_artifacts
                .iter()
                .map(|r| r.name.as_str())
                .chain(["stdout", "stderr", "diagnostics", "execution-record"])
                .collect::<BTreeSet<_>>();
            if i.artifacts
                .keys()
                .any(|name| !allowed.contains(name.as_str()))
            {
                return Err(SourceEvidenceError::ArtifactMismatch(
                    "unplanned check artifact".into(),
                ));
            }
            for object in i.artifacts.values() {
                total = total
                    .checked_add(object.bytes)
                    .ok_or(SourceEvidenceError::InvalidMetadata)?;
                let bytes = read_verified_object(
                    object,
                    &self.binding,
                    reader,
                    policy.resources.artifact_bytes,
                )?;
                if object.name == "execution-record"
                    && (object.media_type != "application/vnd.celetixo.source-check-v1"
                        || bytes != record.execution_bytes())
                {
                    return Err(SourceArtifactError::Integrity.into());
                }
            }
            if total > policy.resources.artifact_bytes {
                return Err(SourceArtifactError::Limit.into());
            }
            for name in ["stdout", "stderr", "diagnostics", "execution-record"] {
                if !i.artifacts.contains_key(name) {
                    incomplete.insert(format!("{}: missing {name}", i.check));
                }
            }
            for (name, limit) in [
                ("stdout", policy.resources.stdout_bytes),
                ("stderr", policy.resources.stderr_bytes),
            ] {
                if i.artifacts.get(name).is_some_and(|o| o.bytes > limit) {
                    return Err(SourceArtifactError::Limit.into());
                }
            }
            for requirement in &policy.expected_artifacts {
                match i.artifacts.get(&requirement.name) {
                    Some(object)
                        if object.media_type != requirement.media_type
                            || object.bytes > requirement.max_bytes =>
                    {
                        return Err(SourceEvidenceError::ArtifactMismatch(object.name.clone()));
                    }
                    None if requirement.required => {
                        incomplete.insert(format!("{}: missing {}", i.check, requirement.name));
                    }
                    _ => {}
                }
            }
            if i.post_source_manifest != Some(manifest)
                || i.termination != SourceTermination::Confirmed
                || i.cleanup != SourceCleanup::Confirmed
                || i.stdout_truncated
                || i.stderr_truncated
                || i.usage.wall_time_ms > policy.resources.wall_time_ms
                || i.usage.cpu_time_us > policy.resources.cpu_time_ms.saturating_mul(1000)
                || i.usage.peak_memory_bytes > policy.resources.memory_bytes
                || i.finished_at_ms - i.started_at_ms > policy.resources.wall_time_ms
            {
                incomplete.insert(format!(
                    "{}: integrity, termination, cleanup, output or resource completeness",
                    i.check
                ));
            }
            for dependency in &policy.depends_on {
                let prerequisite = self
                    .checks
                    .get(dependency)
                    .ok_or(SourceEvidenceError::CheckSet)?
                    .input();
                if prerequisite.finished_at_ms > i.started_at_ms
                    || (i.outcome == SourceCheckOutcome::Exited { code: 0 }
                        && prerequisite.outcome != SourceCheckOutcome::Exited { code: 0 })
                {
                    incomplete.insert(format!(
                        "{}: prerequisite {dependency} was not complete before execution",
                        i.check
                    ));
                }
            }
            match &i.outcome {
                SourceCheckOutcome::Exited { code: 0 } => {}
                SourceCheckOutcome::Exited { .. } | SourceCheckOutcome::Signalled { .. } => {
                    if planned.required() || policy.failure_policy == FailurePolicy::FailAttempt {
                        failed.insert(i.check.clone());
                    }
                }
                _ => {
                    incomplete.insert(format!("{}: execution did not complete", i.check));
                }
            }
        }
        Ok(if !incomplete.is_empty() {
            SourceEvidenceAssessment::Incomplete {
                reasons: incomplete,
            }
        } else if !failed.is_empty() {
            SourceEvidenceAssessment::Failed { checks: failed }
        } else {
            SourceEvidenceAssessment::Passed
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceEvidenceAssessment {
    Passed,
    Failed { checks: BTreeSet<CheckId> },
    Incomplete { reasons: BTreeSet<String> },
}

/// The bytes used by the caller are the same bytes whose size and digest were
/// checked. A second retrieval could observe different storage contents.
fn read_verified_object(
    object: &SourceArtifactReference,
    binding: &SourceEvidenceBinding,
    reader: &dyn SourceArtifactReader,
    limit: u64,
) -> Result<Vec<u8>, SourceEvidenceError> {
    if object.scope != binding.scope || object.retention_policy != binding.retention_policy {
        return Err(SourceEvidenceError::ArtifactMismatch(object.name.clone()));
    }
    if object.bytes > limit || object.bytes > 512 * 1024 * 1024 {
        return Err(SourceArtifactError::Limit.into());
    }
    let bytes = reader.read(&binding.scope, object.content, object.bytes)?;
    if bytes.len() as u64 != object.bytes
        || Digest::of(SOURCE_ARTIFACT_DOMAIN, &bytes) != object.content
    {
        return Err(SourceArtifactError::Integrity.into());
    }
    Ok(bytes)
}
fn valid_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn encode_scope(c: &mut Canonical, scope: &Scope) {
    c.str(scope.organization.as_str());
    c.str(scope.workspace.as_str());
    c.str(scope.repository.as_str());
}
fn encode_limits(c: &mut Canonical, limits: ResourceLimits) {
    for v in [
        limits.wall_time_ms,
        limits.cpu_time_ms,
        limits.memory_bytes,
        limits.disk_bytes,
        limits.process_count,
        limits.file_descriptors,
        limits.stdout_bytes,
        limits.stderr_bytes,
        limits.artifact_bytes,
    ] {
        c.u64(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{OrganizationId, RepositoryId, WorkspaceId};
    fn d(s: &str) -> Digest {
        Digest::of("source-evidence-fixture", s.as_bytes())
    }
    fn scope() -> Scope {
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        }
    }
    fn binding() -> SourceEvidenceBinding {
        SourceEvidenceBinding {
            admission: SourceAdmissionId(d("admission")),
            scope: scope(),
            executor: PrincipalId::new("trusted-executor").unwrap(),
            retention_policy: d("retention"),
            admission_contract: d("admission"),
            attestation: SourceAttestationLevel::EngineScopedExecutor,
        }
    }
    fn input(id: &str) -> SourceCheckRecordInput {
        SourceCheckRecordInput {
            admission: SourceAdmissionId(d("admission")),
            plan: SourcePlanId(d("plan")),
            candidate: SnapshotId(d("candidate")),
            check: CheckId::new(id).unwrap(),
            definition: d("definition"),
            backend: d("backend"),
            executable: d("executable"),
            runtime_closure: d("closure"),
            environment: d("environment"),
            limits: ResourceLimits {
                wall_time_ms: 100,
                cpu_time_ms: 100,
                memory_bytes: 1000,
                disk_bytes: 1000,
                process_count: 4,
                file_descriptors: 16,
                stdout_bytes: 100,
                stderr_bytes: 100,
                artifact_bytes: 1000,
            },
            started_at_ms: 10,
            finished_at_ms: 20,
            outcome: SourceCheckOutcome::Exited { code: 0 },
            usage: SourceResourceUsage {
                wall_time_ms: 10,
                cpu_time_us: 100,
                peak_memory_bytes: 500,
            },
            post_source_manifest: Some(d("manifest")),
            termination: SourceTermination::Confirmed,
            cleanup: SourceCleanup::Confirmed,
            stdout_truncated: false,
            stderr_truncated: false,
            artifacts: BTreeMap::from([(
                "stdout".into(),
                SourceArtifactReference::for_bytes(
                    scope(),
                    "stdout".into(),
                    "application/octet-stream".into(),
                    b"output",
                    d("retention"),
                ),
            )]),
            diagnostics: vec![],
        }
    }
    #[test]
    fn seal_order_is_deterministic_and_context_outcome_and_object_changes_are_bound() {
        let checks = vec![
            SourceCheckRecord::new(input("b")).unwrap(),
            SourceCheckRecord::new(input("a")).unwrap(),
        ];
        let record =
            SourceEvidence::new(binding(), 10, 20, checks.clone(), BTreeMap::new(), vec![])
                .unwrap();
        let mut reversed = checks.clone();
        reversed.reverse();
        assert_eq!(
            record,
            SourceEvidence::new(binding(), 10, 20, reversed, BTreeMap::new(), vec![]).unwrap()
        );
        let mut changed_binding = binding();
        changed_binding.executor = PrincipalId::new("other-executor").unwrap();
        assert_ne!(
            record.id(),
            SourceEvidence::new(
                changed_binding,
                10,
                20,
                checks.clone(),
                BTreeMap::new(),
                vec![]
            )
            .unwrap()
            .id()
        );
        for variant in 0..6 {
            let mut changed = input("a");
            match variant {
                0 => changed.outcome = SourceCheckOutcome::TimedOut,
                1 => changed.stdout_truncated = true,
                2 => changed.post_source_manifest = None,
                3 => {
                    changed.cleanup = SourceCleanup::Quarantined {
                        reason: "unconfirmed termination".into(),
                    }
                }
                4 => changed.runtime_closure = d("other-runtime"),
                _ => {
                    changed
                        .artifacts
                        .get_mut("stdout")
                        .unwrap()
                        .scope
                        .repository = RepositoryId::new("other").unwrap()
                }
            }
            let changed_checks = vec![checks[0].clone(), SourceCheckRecord::new(changed).unwrap()];
            assert_ne!(
                record.id(),
                SourceEvidence::new(binding(), 10, 20, changed_checks, BTreeMap::new(), vec![])
                    .unwrap()
                    .id()
            );
        }
    }
    #[test]
    fn execution_object_has_no_recursive_address_and_still_binds_every_other_object() {
        let initial = SourceCheckRecord::new(input("a")).unwrap();
        let bytes = initial.execution_bytes();
        let mut with_record = initial.input().clone();
        with_record.artifacts.insert(
            "execution-record".into(),
            SourceArtifactReference::for_bytes(
                scope(),
                "execution-record".into(),
                "application/vnd.celetixo.source-check-v1".into(),
                &bytes,
                d("retention"),
            ),
        );
        assert_eq!(
            SourceCheckRecord::new(with_record.clone())
                .unwrap()
                .execution_bytes(),
            bytes
        );
        with_record.artifacts.get_mut("stdout").unwrap().content = d("changed-output");
        assert_ne!(
            SourceCheckRecord::new(with_record)
                .unwrap()
                .execution_bytes(),
            bytes
        );
    }
    #[test]
    fn invalid_status_times_duplicates_and_artifact_labels_reject() {
        for variant in 0..5 {
            let mut invalid = input("a");
            match variant {
                0 => invalid.finished_at_ms = 0,
                1 => invalid.outcome = SourceCheckOutcome::Exited { code: 256 },
                2 => invalid.outcome = SourceCheckOutcome::Infrastructure { reason: "".into() },
                3 => invalid.artifacts.get_mut("stdout").unwrap().name = "renamed".into(),
                _ => invalid.termination = SourceTermination::Unknown { reason: "".into() },
            }
            assert!(SourceCheckRecord::new(invalid).is_err());
        }
        let check = SourceCheckRecord::new(input("a")).unwrap();
        assert_eq!(
            SourceEvidence::new(
                binding(),
                10,
                20,
                vec![check.clone(), check],
                BTreeMap::new(),
                vec![]
            ),
            Err(SourceEvidenceError::CheckSet)
        );
    }
    #[test]
    fn aggregate_metadata_is_bounded_before_sealing() {
        let mut oversized = input("large");
        oversized.diagnostics = vec!["x".repeat(64 * 1024); 65];
        assert_eq!(
            SourceCheckRecord::new(oversized),
            Err(SourceEvidenceError::InvalidMetadata)
        );
        let mut checks = Vec::new();
        for index in 0..17 {
            let mut value = input(&format!("check:{index}"));
            value.diagnostics = vec!["x".repeat(64 * 1024); 16];
            checks.push(SourceCheckRecord::new(value).unwrap());
        }
        assert_eq!(
            SourceEvidence::new(binding(), 10, 20, checks, BTreeMap::new(), vec![]),
            Err(SourceEvidenceError::InvalidMetadata)
        );
    }

    #[test]
    fn a_large_plan_can_seal_all_check_definition_references() {
        let checks = (0..64)
            .map(|index| SourceCheckRecord::new(input(&format!("check:{index}"))).unwrap())
            .collect();
        let provenance = (0..64)
            .map(|index| format!("check-definition/check:{index}"))
            .chain((0..9).map(|index| format!("global:{index}")))
            .map(|name| {
                let object = SourceArtifactReference::for_bytes(
                    scope(),
                    name.clone(),
                    "application/vnd.celetixo.provenance-v1".into(),
                    b"finite canonical fixture",
                    d("retention"),
                );
                (name, object)
            })
            .collect();
        let evidence = SourceEvidence::new(binding(), 10, 20, checks, provenance, vec![]).unwrap();
        assert_eq!(evidence.checks().len(), 64);
        assert_eq!(evidence.provenance().len(), 73);
    }
}
