//! Checked metadata reconstruction for the existing source evidence formats.
//!
//! Restoring an envelope does not retrieve its objects, authenticate its executor,
//! establish durable retention, or admit evidence. Those remain independent
//! responsibilities of the existing Engine gate and `validate_persisted`.
use super::{DomainDecodeLimits, Reader, RestoreError};
use crate::ids::{
    CheckId, Digest, PrincipalId, Scope, SnapshotId, SourceAdmissionId, SourceEvidenceId,
    SourcePlanId,
};
use crate::source_evidence::{
    SOURCE_ARTIFACT_DOMAIN, SOURCE_EVIDENCE_FORMAT, SourceArtifactReference,
    SourceAttestationLevel, SourceCheckOutcome, SourceCheckRecord, SourceCheckRecordInput,
    SourceCleanup, SourceEvidence, SourceEvidenceBinding, SourceResourceUsage, SourceTermination,
};
use crate::source_execution::ResourceLimits;
use std::collections::BTreeMap;

const CHECK_FORMAT: &str = "celetixo/source-check-record/v1";
const MAX_CHECK_BYTES: usize = 4 * 1024 * 1024;
const MAX_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;

/// Caller-supplied expected associations for a separately stored check record.
/// These are metadata expectations, not executor or artifact-store capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckRecordBinding {
    pub scope: Scope,
    pub admission: SourceAdmissionId,
    pub plan: SourcePlanId,
    pub candidate: SnapshotId,
    pub check: CheckId,
    pub retention_policy: Digest,
}

/// Restore the unchanged v1 execution object by its existing artifact address.
///
/// That object deliberately excludes its own `execution-record` reference to
/// avoid a recursive address. The returned value retains exactly the stored
/// fields; it does not invent that reference or establish complete evidence.
pub fn restore_source_check_record(
    binding: &SourceCheckRecordBinding,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceCheckRecord, RestoreError> {
    let mut r = reader(
        expected,
        SOURCE_ARTIFACT_DOMAIN,
        bytes,
        limits,
        MAX_CHECK_BYTES,
    )?;
    if r.text()? != CHECK_FORMAT {
        return Err(RestoreError::Encoding);
    }
    let record = check(&mut r)?;
    r.end()?;
    let i = record.input();
    if i.admission != binding.admission
        || i.plan != binding.plan
        || i.candidate != binding.candidate
        || i.check != binding.check
    {
        return Err(RestoreError::BindingMismatch);
    }
    check_artifacts(&record, &binding.scope, binding.retention_policy)?;
    // An execution object containing the omitted self-reference is not the
    // published format, even if that reference otherwise looks well-formed.
    if record.input().artifacts.contains_key("execution-record")
        || record.execution_bytes() != bytes
    {
        return Err(RestoreError::NonCanonical);
    }
    Ok(record)
}

/// Restore the unchanged v2 envelope against independently expected bindings.
///
/// The expected binding normally comes from `SourceEvidenceBinding::for_admission`.
/// An explicitly expected `Unattested` value remains unattested metadata; the
/// current Engine admission gate still rejects it. Object readback, plan/check
/// applicability and current authorization must still be checked by that gate.
pub fn restore_source_evidence(
    binding: &SourceEvidenceBinding,
    expected: SourceEvidenceId,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceEvidence, RestoreError> {
    let mut r = reader(
        expected.0,
        SOURCE_EVIDENCE_FORMAT,
        bytes,
        limits,
        MAX_EVIDENCE_BYTES,
    )?;
    if r.text()? != SOURCE_EVIDENCE_FORMAT {
        return Err(RestoreError::Encoding);
    }
    let admission = SourceAdmissionId(r.digest()?);
    let scope = r.scope()?;
    let executor = PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let retention_policy = r.digest()?;
    let admission_contract = r.digest()?;
    let attestation = match r.number()? {
        0 => SourceAttestationLevel::Unattested,
        1 => SourceAttestationLevel::EngineScopedExecutor,
        _ => return Err(RestoreError::Encoding),
    };
    if admission != binding.admission()
        || scope != *binding.scope()
        || executor != *binding.executor()
        || retention_policy != binding.retention_policy()
        || admission_contract != binding.admission().0
        || attestation != binding.attestation()
    {
        return Err(RestoreError::BindingMismatch);
    }
    let started_at_ms = r.number()?;
    let finished_at_ms = r.number()?;
    let mut checks = Vec::new();
    for _ in 0..r.count()? {
        let record = check(&mut r)?;
        if record.input().admission != admission {
            return Err(RestoreError::BindingMismatch);
        }
        check_artifacts(&record, &scope, retention_policy)?;
        checks.push(record);
    }
    let provenance = artifacts(&mut r)?;
    for object in provenance.values() {
        artifact_binding(object, &scope, retention_policy)?;
    }
    let diagnostics = diagnostics(&mut r)?;
    r.end()?;
    let evidence = SourceEvidence::new(
        binding.clone(),
        started_at_ms,
        finished_at_ms,
        checks,
        provenance,
        diagnostics,
    )?;
    if evidence.id() != expected || evidence.canonical_bytes() != bytes {
        return Err(RestoreError::NonCanonical);
    }
    Ok(evidence)
}

fn reader<'a>(
    expected: Digest,
    domain: &str,
    bytes: &'a [u8],
    limits: DomainDecodeLimits,
    format_limit: usize,
) -> Result<Reader<'a>, RestoreError> {
    if bytes.len() > format_limit {
        return Err(RestoreError::Limit);
    }
    Reader::new(expected, domain, bytes, limits)
}

fn check(r: &mut Reader<'_>) -> Result<SourceCheckRecord, RestoreError> {
    let admission = SourceAdmissionId(r.digest()?);
    let plan = SourcePlanId(r.digest()?);
    let candidate = SnapshotId(r.digest()?);
    let check = CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let definition = r.digest()?;
    let backend = r.digest()?;
    let executable = r.digest()?;
    let runtime_closure = r.digest()?;
    let environment = r.digest()?;
    let limits = ResourceLimits {
        wall_time_ms: r.number()?,
        cpu_time_ms: r.number()?,
        memory_bytes: r.number()?,
        disk_bytes: r.number()?,
        process_count: r.number()?,
        file_descriptors: r.number()?,
        stdout_bytes: r.number()?,
        stderr_bytes: r.number()?,
        artifact_bytes: r.number()?,
    };
    let started_at_ms = r.number()?;
    let finished_at_ms = r.number()?;
    let outcome = match r.number()? {
        0 => SourceCheckOutcome::Exited { code: integer(r)? },
        1 => SourceCheckOutcome::Signalled {
            signal: integer(r)?,
        },
        2 => SourceCheckOutcome::TimedOut,
        3 => SourceCheckOutcome::Cancelled,
        4 => SourceCheckOutcome::Expired,
        5 => SourceCheckOutcome::Revoked,
        6 => SourceCheckOutcome::OutputLimit,
        7 => SourceCheckOutcome::Infrastructure { reason: r.text()? },
        8 => SourceCheckOutcome::Skipped { reason: r.text()? },
        9 => SourceCheckOutcome::UnknownTermination { reason: r.text()? },
        _ => return Err(RestoreError::Encoding),
    };
    let usage = SourceResourceUsage {
        wall_time_ms: r.number()?,
        cpu_time_us: r.number()?,
        peak_memory_bytes: r.number()?,
    };
    let post_source_manifest = match r.number()? {
        0 => None,
        1 => Some(r.digest()?),
        _ => return Err(RestoreError::Encoding),
    };
    let termination = match r.number()? {
        0 => SourceTermination::Confirmed,
        1 => SourceTermination::Unknown { reason: r.text()? },
        _ => return Err(RestoreError::Encoding),
    };
    let cleanup = match r.number()? {
        0 => SourceCleanup::Confirmed,
        1 => SourceCleanup::Quarantined { reason: r.text()? },
        2 => SourceCleanup::Failed { reason: r.text()? },
        _ => return Err(RestoreError::Encoding),
    };
    let stdout_truncated = boolean(r)?;
    let stderr_truncated = boolean(r)?;
    let artifacts = artifacts(r)?;
    let diagnostics = diagnostics(r)?;
    SourceCheckRecord::new(SourceCheckRecordInput {
        admission,
        plan,
        candidate,
        check,
        definition,
        backend,
        executable,
        runtime_closure,
        environment,
        limits,
        started_at_ms,
        finished_at_ms,
        outcome,
        usage,
        post_source_manifest,
        termination,
        cleanup,
        stdout_truncated,
        stderr_truncated,
        artifacts,
        diagnostics,
    })
    .map_err(RestoreError::from)
}

fn integer(r: &mut Reader<'_>) -> Result<i32, RestoreError> {
    i32::try_from(r.signed()?).map_err(|_| RestoreError::Encoding)
}

fn boolean(r: &mut Reader<'_>) -> Result<bool, RestoreError> {
    match r.number()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(RestoreError::Encoding),
    }
}

fn artifacts(
    r: &mut Reader<'_>,
) -> Result<BTreeMap<String, SourceArtifactReference>, RestoreError> {
    let mut result = BTreeMap::new();
    for _ in 0..r.count()? {
        let object = SourceArtifactReference {
            scope: r.scope()?,
            name: r.text()?,
            media_type: r.text()?,
            content: r.digest()?,
            bytes: r.number()?,
            retention_policy: r.digest()?,
        };
        if result.insert(object.name.clone(), object).is_some() {
            return Err(RestoreError::NonCanonical);
        }
    }
    Ok(result)
}

fn diagnostics(r: &mut Reader<'_>) -> Result<Vec<String>, RestoreError> {
    let mut result = Vec::new();
    for _ in 0..r.count()? {
        result.push(r.text()?);
    }
    Ok(result)
}

fn check_artifacts(
    record: &SourceCheckRecord,
    scope: &Scope,
    retention: Digest,
) -> Result<(), RestoreError> {
    for object in record.input().artifacts.values() {
        artifact_binding(object, scope, retention)?;
    }
    Ok(())
}

fn artifact_binding(
    object: &SourceArtifactReference,
    scope: &Scope,
    retention: Digest,
) -> Result<(), RestoreError> {
    if object.scope != *scope || object.retention_policy != retention {
        return Err(RestoreError::BindingMismatch);
    }
    Ok(())
}
