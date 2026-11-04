//! Immutable verification records. Constructing a record grants no Engine authority.
//!
//! The trusted verification boundary must authenticate the runner before retaining these
//! records. Retained bytes are process-local evidence in the reference store, not a claim
//! of durable object-storage persistence or an isolated external command executor.
use crate::ids::{
    Canonical, CheckId, Digest, EvidenceId, PatchId, PrincipalId, SnapshotId, TransactionId,
};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EvidenceError {
    #[error("verification command must have a nonempty executable and no NUL bytes")]
    InvalidCommand,
    #[error("{0} must be nonempty and contain no NUL bytes")]
    InvalidMetadata(&'static str),
    #[error("verification finish precedes its start")]
    InvalidTimeRange,
    #[error(
        "passed evidence requires exit code zero and failed evidence requires a nonzero exit code"
    )]
    InvalidExitStatus,
    #[error("incomplete evidence requires a reason")]
    MissingIncompleteReason,
    #[error("verification evidence requires retained artifacts")]
    MissingArtifacts,
    #[error("artifact names must be unique")]
    DuplicateArtifacts,
}

fn valid_metadata(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains('\0')
}

/// Exact required check identity, argument vector, toolchain, configuration and environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckSpec {
    id: CheckId,
    command: Vec<String>,
    toolchain: String,
    environment: Digest,
    configuration: Digest,
}
impl CheckSpec {
    pub fn new(
        id: CheckId,
        command: Vec<String>,
        toolchain: String,
        environment: Digest,
        configuration: Digest,
    ) -> Result<Self, EvidenceError> {
        if command.first().is_none_or(|arg| !valid_metadata(arg))
            || command.iter().any(|arg| arg.contains('\0'))
        {
            return Err(EvidenceError::InvalidCommand);
        }
        if !valid_metadata(&toolchain) {
            return Err(EvidenceError::InvalidMetadata("toolchain"));
        }
        Ok(Self {
            id,
            command,
            toolchain,
            environment,
            configuration,
        })
    }
    pub fn id(&self) -> &CheckId {
        &self.id
    }
    pub fn command(&self) -> &[String] {
        &self.command
    }
    pub fn toolchain(&self) -> &str {
        &self.toolchain
    }
    pub fn environment(&self) -> Digest {
        self.environment
    }
    pub fn configuration(&self) -> Digest {
        self.configuration
    }
    fn encode(&self, out: &mut Canonical) {
        out.str(self.id.as_str());
        out.u64(self.command.len() as u64);
        for arg in &self.command {
            out.str(arg);
        }
        out.str(&self.toolchain);
        out.digest(self.environment);
        out.digest(self.configuration);
    }
    pub fn digest(&self) -> Digest {
        let mut out = Canonical::default();
        self.encode(&mut out);
        out.finish("celetixo/check-spec/v1")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBindings {
    pub transaction: TransactionId,
    /// Immutable, monotonic execution/revalidation attempt owned by the Engine.
    pub attempt: u64,
    pub base_snapshot: SnapshotId,
    pub candidate_snapshot: SnapshotId,
    pub patch: PatchId,
    /// Full impact-analysis digest, including stamp, dependency set and completeness.
    pub analysis: Digest,
}
impl EvidenceBindings {
    fn encode(&self, out: &mut Canonical) {
        out.str(self.transaction.as_str());
        out.u64(self.attempt);
        out.digest(self.base_snapshot.0);
        out.digest(self.candidate_snapshot.0);
        out.digest(self.patch.0);
        out.digest(self.analysis);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    Passed { exit_code: i32 },
    Failed { exit_code: i32 },
    Incomplete { reason: String },
}
impl CheckResult {
    fn validate(&self) -> Result<(), EvidenceError> {
        match self {
            Self::Passed { exit_code: 0 } => Ok(()),
            Self::Passed { .. } | Self::Failed { exit_code: 0 } => {
                Err(EvidenceError::InvalidExitStatus)
            }
            Self::Failed { .. } => Ok(()),
            Self::Incomplete { reason } if valid_metadata(reason) => Ok(()),
            Self::Incomplete { .. } => Err(EvidenceError::MissingIncompleteReason),
        }
    }
    fn encode(&self, out: &mut Canonical) {
        match self {
            Self::Passed { exit_code } => {
                out.str("passed");
                out.i64(i64::from(*exit_code));
            }
            Self::Failed { exit_code } => {
                out.str("failed");
                out.i64(i64::from(*exit_code));
            }
            Self::Incomplete { reason } => {
                out.str("incomplete");
                out.str(reason);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProvenance {
    /// Authenticated verifier identity; Engine insertion also checks verifier authorization.
    pub verifier: PrincipalId,
    /// Runner implementation and version, e.g. `reference-evaluator/v1`.
    pub runner: String,
    pub run_id: String,
    pub started_at: u64,
    pub finished_at: u64,
}
impl ExecutionProvenance {
    fn validate(&self) -> Result<(), EvidenceError> {
        if !valid_metadata(&self.runner) {
            return Err(EvidenceError::InvalidMetadata("runner"));
        }
        if !valid_metadata(&self.run_id) {
            return Err(EvidenceError::InvalidMetadata("run id"));
        }
        if self.finished_at < self.started_at {
            return Err(EvidenceError::InvalidTimeRange);
        }
        Ok(())
    }
    fn encode(&self, out: &mut Canonical) {
        out.str(self.verifier.as_str());
        out.str(&self.runner);
        out.str(&self.run_id);
        out.u64(self.started_at);
        out.u64(self.finished_at);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    name: String,
    media_type: String,
    content: Vec<u8>,
    digest: Digest,
}
impl Artifact {
    pub fn new(
        name: impl Into<String>,
        media_type: impl Into<String>,
        content: Vec<u8>,
    ) -> Result<Self, EvidenceError> {
        let name = name.into();
        let media_type = media_type.into();
        if !valid_metadata(&name) {
            return Err(EvidenceError::InvalidMetadata("artifact name"));
        }
        if !valid_metadata(&media_type) {
            return Err(EvidenceError::InvalidMetadata("artifact media type"));
        }
        let mut out = Canonical::default();
        out.str(&name);
        out.str(&media_type);
        out.bytes(&content);
        let digest = out.finish("celetixo/verification-artifact/v1");
        Ok(Self {
            name,
            media_type,
            content,
            digest,
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn media_type(&self) -> &str {
        &self.media_type
    }
    pub fn content(&self) -> &[u8] {
        &self.content
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationEvidence {
    id: EvidenceId,
    bindings: EvidenceBindings,
    check: CheckSpec,
    result: CheckResult,
    provenance: ExecutionProvenance,
    diagnostics: Vec<String>,
    artifacts: Vec<Artifact>,
}
impl VerificationEvidence {
    pub fn new(
        bindings: EvidenceBindings,
        check: CheckSpec,
        result: CheckResult,
        provenance: ExecutionProvenance,
        diagnostics: Vec<String>,
        mut artifacts: Vec<Artifact>,
    ) -> Result<Self, EvidenceError> {
        result.validate()?;
        provenance.validate()?;
        if artifacts.is_empty() {
            return Err(EvidenceError::MissingArtifacts);
        }
        let names: BTreeSet<_> = artifacts.iter().map(|artifact| artifact.name()).collect();
        if names.len() != artifacts.len() {
            return Err(EvidenceError::DuplicateArtifacts);
        }
        // Artifact delivery order is immaterial; diagnostic order is meaningful.
        artifacts.sort_by(|left, right| left.name.cmp(&right.name));
        let mut out = Canonical::default();
        bindings.encode(&mut out);
        check.encode(&mut out);
        result.encode(&mut out);
        provenance.encode(&mut out);
        out.u64(diagnostics.len() as u64);
        for diagnostic in &diagnostics {
            out.str(diagnostic);
        }
        out.u64(artifacts.len() as u64);
        for artifact in &artifacts {
            out.digest(artifact.digest());
        }
        let id = EvidenceId(out.finish("celetixo/verification-evidence/v1"));
        Ok(Self {
            id,
            bindings,
            check,
            result,
            provenance,
            diagnostics,
            artifacts,
        })
    }
    pub fn id(&self) -> EvidenceId {
        self.id
    }
    pub fn bindings(&self) -> &EvidenceBindings {
        &self.bindings
    }
    pub fn check(&self) -> &CheckSpec {
        &self.check
    }
    pub fn result(&self) -> &CheckResult {
        &self.result
    }
    pub fn provenance(&self) -> &ExecutionProvenance {
        &self.provenance
    }
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
    pub fn artifacts(&self) -> &[Artifact] {
        &self.artifacts
    }
    pub fn passed(&self) -> bool {
        matches!(self.result, CheckResult::Passed { exit_code: 0 })
    }
    pub fn matches(&self, bindings: &EvidenceBindings, check: &CheckSpec) -> bool {
        &self.bindings == bindings && &self.check == check
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(s: &str) -> Digest {
        Digest::of("test", s.as_bytes())
    }
    fn bindings() -> EvidenceBindings {
        EvidenceBindings {
            transaction: TransactionId::new("tx:1").unwrap(),
            attempt: 1,
            base_snapshot: SnapshotId(d("base")),
            candidate_snapshot: SnapshotId(d("candidate")),
            patch: PatchId(d("patch")),
            analysis: d("analysis"),
        }
    }
    fn check() -> CheckSpec {
        CheckSpec::new(
            CheckId::new("unit").unwrap(),
            vec!["reference-evaluator".into(), "test".into()],
            "reference/v1".into(),
            d("env"),
            d("config"),
        )
        .unwrap()
    }
    fn provenance() -> ExecutionProvenance {
        ExecutionProvenance {
            verifier: PrincipalId::new("verifier").unwrap(),
            runner: "reference/v1".into(),
            run_id: "run:1".into(),
            started_at: 2,
            finished_at: 3,
        }
    }
    fn artifacts() -> Vec<Artifact> {
        vec![Artifact::new("stdout", "text/plain", b"1 test passed".to_vec()).unwrap()]
    }
    fn record(bindings: EvidenceBindings, check: CheckSpec) -> VerificationEvidence {
        VerificationEvidence::new(
            bindings,
            check,
            CheckResult::Passed { exit_code: 0 },
            provenance(),
            vec![],
            artifacts(),
        )
        .unwrap()
    }
    #[test]
    fn exact_bindings_reject_each_stale_or_mismatched_input() {
        let record = record(bindings(), check());
        assert!(record.passed());
        assert!(record.matches(&bindings(), &check()));
        for changed in [
            EvidenceBindings {
                transaction: TransactionId::new("tx:2").unwrap(),
                ..bindings()
            },
            EvidenceBindings {
                attempt: 2,
                ..bindings()
            },
            EvidenceBindings {
                base_snapshot: SnapshotId(d("base:2")),
                ..bindings()
            },
            EvidenceBindings {
                candidate_snapshot: SnapshotId(d("candidate:2")),
                ..bindings()
            },
            EvidenceBindings {
                patch: PatchId(d("patch:2")),
                ..bindings()
            },
            EvidenceBindings {
                analysis: d("analysis:2"),
                ..bindings()
            },
        ] {
            assert!(!record.matches(&changed, &check()));
            assert_ne!(record.id(), self::record(changed, check()).id());
        }
        let changed = CheckSpec::new(
            CheckId::new("unit").unwrap(),
            vec!["reference-evaluator".into(), "test".into()],
            "reference/v1".into(),
            d("env:2"),
            d("config"),
        )
        .unwrap();
        assert!(!record.matches(&bindings(), &changed));
    }
    #[test]
    fn check_digest_includes_command_configuration_environment_and_toolchain() {
        let original = check();
        for changed in [
            CheckSpec::new(
                CheckId::new("other").unwrap(),
                original.command().to_vec(),
                original.toolchain().into(),
                original.environment(),
                original.configuration(),
            )
            .unwrap(),
            CheckSpec::new(
                original.id().clone(),
                vec!["reference-evaluator".into()],
                original.toolchain().into(),
                original.environment(),
                original.configuration(),
            )
            .unwrap(),
            CheckSpec::new(
                original.id().clone(),
                original.command().to_vec(),
                "reference/v2".into(),
                original.environment(),
                original.configuration(),
            )
            .unwrap(),
            CheckSpec::new(
                original.id().clone(),
                original.command().to_vec(),
                original.toolchain().into(),
                d("env:2"),
                original.configuration(),
            )
            .unwrap(),
            CheckSpec::new(
                original.id().clone(),
                original.command().to_vec(),
                original.toolchain().into(),
                original.environment(),
                d("config:2"),
            )
            .unwrap(),
        ] {
            assert_ne!(original.digest(), changed.digest());
        }
    }
    #[test]
    fn evidence_digest_includes_diagnostics_artifacts_provenance_and_outcome() {
        let initial = record(bindings(), check());
        let changed_diagnostic = VerificationEvidence::new(
            bindings(),
            check(),
            CheckResult::Passed { exit_code: 0 },
            provenance(),
            vec!["warning".into()],
            artifacts(),
        )
        .unwrap();
        let changed_artifact = VerificationEvidence::new(
            bindings(),
            check(),
            CheckResult::Passed { exit_code: 0 },
            provenance(),
            vec![],
            vec![Artifact::new("stdout", "text/plain", b"different".to_vec()).unwrap()],
        )
        .unwrap();
        let changed_provenance = VerificationEvidence::new(
            bindings(),
            check(),
            CheckResult::Passed { exit_code: 0 },
            ExecutionProvenance {
                run_id: "run:2".into(),
                ..provenance()
            },
            vec![],
            artifacts(),
        )
        .unwrap();
        let failed = VerificationEvidence::new(
            bindings(),
            check(),
            CheckResult::Failed { exit_code: 1 },
            provenance(),
            vec![],
            artifacts(),
        )
        .unwrap();
        for other in [
            changed_diagnostic,
            changed_artifact,
            changed_provenance,
            failed.clone(),
        ] {
            assert_ne!(initial.id(), other.id());
        }
        assert!(!failed.passed());
        assert_eq!(initial.artifacts()[0].content(), b"1 test passed");
    }
    #[test]
    fn inconsistent_status_missing_capture_and_invalid_provenance_fail_closed() {
        for result in [
            CheckResult::Passed { exit_code: 1 },
            CheckResult::Failed { exit_code: 0 },
            CheckResult::Incomplete { reason: "".into() },
        ] {
            assert!(
                VerificationEvidence::new(
                    bindings(),
                    check(),
                    result,
                    provenance(),
                    vec![],
                    artifacts()
                )
                .is_err()
            );
        }
        assert_eq!(
            VerificationEvidence::new(
                bindings(),
                check(),
                CheckResult::Passed { exit_code: 0 },
                provenance(),
                vec![],
                vec![]
            ),
            Err(EvidenceError::MissingArtifacts)
        );
        assert_eq!(
            VerificationEvidence::new(
                bindings(),
                check(),
                CheckResult::Passed { exit_code: 0 },
                ExecutionProvenance {
                    started_at: 5,
                    ..provenance()
                },
                vec![],
                artifacts()
            ),
            Err(EvidenceError::InvalidTimeRange)
        );
        let mut duplicate = artifacts();
        duplicate.extend(artifacts());
        assert_eq!(
            VerificationEvidence::new(
                bindings(),
                check(),
                CheckResult::Passed { exit_code: 0 },
                provenance(),
                vec![],
                duplicate
            ),
            Err(EvidenceError::DuplicateArtifacts)
        );
    }
}
