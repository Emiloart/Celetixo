//! Structured source-domain errors for immutable historical outcomes.
//! No Debug/Display parsing and no leaked allocations for static diagnostics.
use super::*;
use crate::credentials::CredentialError;
use crate::engine::SourceAttemptError;
use crate::ids::{Canonical, CheckId, InvariantId};
use crate::repository::RepositoryAnalysisError;
use crate::source_evidence::{SourceArtifactError, SourceEvidenceError};
use crate::source_execution::SourcePlanError;
use crate::source_patch::SourcePatchError;
pub(super) fn encode_attempt(
    c: &mut Canonical,
    error: &SourceAttemptError,
) -> Result<(), RestoreError> {
    use SourceAttemptError::*;
    match error {
        PolicyChanged => c.str("PolicyChanged"),
        Expired => c.str("Expired"),
        BindingMismatch => c.str("BindingMismatch"),
        InvalidState => c.str("InvalidState"),
        AttemptActive => c.str("AttemptActive"),
        AttemptIdentity => c.str("AttemptIdentity"),
        AlreadyClaimed => c.str("AlreadyClaimed"),
        UntrustedExecutor => c.str("UntrustedExecutor"),
        UnsupportedInvariant => c.str("UnsupportedInvariant"),
        UnknownPolicyElement => c.str("UnknownPolicyElement"),
        SourceRepositoryRequired => c.str("SourceRepositoryRequired"),
        InvalidReason => c.str("InvalidReason"),
        InvalidOptions => c.str("InvalidOptions"),
        Plan(value) => {
            c.str("Plan");
            encode_plan(c, value)?;
        }
        Patch(value) => {
            c.str("Patch");
            encode_patch(c, value)?;
        }
        Credential(value) => {
            c.str("Credential");
            encode_credential(c, value)?;
        }
    };
    Ok(())
}
pub(super) fn decode_attempt(r: &mut Reader<'_>) -> Result<SourceAttemptError, RestoreError> {
    use SourceAttemptError::*;
    Ok(match r.text()?.as_str() {
        "PolicyChanged" => PolicyChanged,
        "Expired" => Expired,
        "BindingMismatch" => BindingMismatch,
        "InvalidState" => InvalidState,
        "AttemptActive" => AttemptActive,
        "AttemptIdentity" => AttemptIdentity,
        "AlreadyClaimed" => AlreadyClaimed,
        "UntrustedExecutor" => UntrustedExecutor,
        "UnsupportedInvariant" => UnsupportedInvariant,
        "UnknownPolicyElement" => UnknownPolicyElement,
        "SourceRepositoryRequired" => SourceRepositoryRequired,
        "InvalidReason" => InvalidReason,
        "InvalidOptions" => InvalidOptions,
        "Plan" => Plan(decode_plan(r)?),
        "Patch" => Patch(decode_patch(r)?),
        "Credential" => Credential(decode_credential(r)?),
        _ => return Err(RestoreError::Encoding),
    })
}
pub(super) fn encode_plan(c: &mut Canonical, error: &SourcePlanError) -> Result<(), RestoreError> {
    use SourcePlanError::*;
    match error {
        DependencyCycle => c.str("DependencyCycle"),
        BindingMismatch => c.str("BindingMismatch"),
        IncompleteCapture => c.str("IncompleteCapture"),
        PatchPolicyMismatch => c.str("PatchPolicyMismatch"),
        UnknownCacheCheck => c.str("UnknownCacheCheck"),
        InvalidPath(value) => {
            c.str("InvalidPath");
            c.str(value);
        }
        DuplicateCheck(value) => {
            c.str("DuplicateCheck");
            c.str(value.as_str());
        }
        MissingCheck(value) => {
            c.str("MissingCheck");
            c.str(value.as_str());
        }
        SubstitutedCheck(value) => {
            c.str("SubstitutedCheck");
            c.str(value.as_str());
        }
        MissingInvariant(value) => {
            c.str("MissingInvariant");
            c.str(value.as_str());
        }
        ResourceCeiling(value) => {
            c.str("ResourceCeiling");
            c.str(value.as_str());
        }
        UnapprovedBackend(value) => {
            c.str("UnapprovedBackend");
            c.str(value.as_str());
        }
        UnauthorizedPath(value) => {
            c.str("UnauthorizedPath");
            c.str(value);
        }
        PathPreimage(value) => {
            c.str("PathPreimage");
            c.str(value);
        }
        ConflictingRequirement(value) => {
            c.str("ConflictingRequirement");
            c.str(value.as_str());
        }
        CheckInputMismatch(value) => {
            c.str("CheckInputMismatch");
            c.str(value);
        }
        Credential(value) => {
            c.str("Credential");
            encode_credential(c, value)?;
        }
        InvalidPolicy(value) => {
            static_plan_invalidpolicy(value)?;
            c.str("InvalidPolicy");
            c.str(value);
        }
        InvalidCheck(value) => {
            static_plan_invalidcheck(value)?;
            c.str("InvalidCheck");
            c.str(value);
        }
    };
    Ok(())
}
pub(super) fn decode_plan(r: &mut Reader<'_>) -> Result<SourcePlanError, RestoreError> {
    use SourcePlanError::*;
    Ok(match r.text()?.as_str() {
        "DependencyCycle" => DependencyCycle,
        "BindingMismatch" => BindingMismatch,
        "IncompleteCapture" => IncompleteCapture,
        "PatchPolicyMismatch" => PatchPolicyMismatch,
        "UnknownCacheCheck" => UnknownCacheCheck,
        "InvalidPath" => InvalidPath(r.text()?),
        "DuplicateCheck" => {
            DuplicateCheck(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "MissingCheck" => {
            MissingCheck(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "SubstitutedCheck" => {
            SubstitutedCheck(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "MissingInvariant" => {
            MissingInvariant(InvariantId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "ResourceCeiling" => {
            ResourceCeiling(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "UnapprovedBackend" => {
            UnapprovedBackend(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "UnauthorizedPath" => UnauthorizedPath(r.text()?),
        "PathPreimage" => PathPreimage(r.text()?),
        "ConflictingRequirement" => {
            ConflictingRequirement(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "CheckInputMismatch" => CheckInputMismatch(r.text()?),
        "Credential" => Credential(decode_credential(r)?),
        "InvalidPolicy" => InvalidPolicy(static_plan_invalidpolicy(&r.text()?)?),
        "InvalidCheck" => InvalidCheck(static_plan_invalidcheck(&r.text()?)?),
        _ => return Err(RestoreError::Encoding),
    })
}
fn static_plan_invalidpolicy(value: &str) -> Result<&'static str, RestoreError> {
    match value {
        "credential scope" => Ok("credential scope"),
        "credential scope or grant" => Ok("credential scope or grant"),
        "empty path authorization" => Ok("empty path authorization"),
        "empty verification plan" => Ok("empty verification plan"),
        "identity or empty policy" => Ok("identity or empty policy"),
        "network widening" => Ok("network widening"),
        "patch policy" => Ok("patch policy"),
        "source invariant" => Ok("source invariant"),
        _ => Err(RestoreError::UnsupportedDomainValue),
    }
}
fn static_plan_invalidcheck(value: &str) -> Result<&'static str, RestoreError> {
    match value {
        "artifact contract" => Ok("artifact contract"),
        "cache platform" => Ok("cache platform"),
        "commands cannot run in T0" => Ok("commands cannot run in T0"),
        "credential reference" => Ok("credential reference"),
        "definition budget" => Ok("definition budget"),
        "environment key" => Ok("environment key"),
        "literal environment value" => Ok("literal environment value"),
        "network grant" => Ok("network grant"),
        "required execution logs" => Ok("required execution logs"),
        "toolchain/backend identity" => Ok("toolchain/backend identity"),
        "zero resource limit" => Ok("zero resource limit"),
        _ => Err(RestoreError::UnsupportedDomainValue),
    }
}
pub(super) fn encode_patch(
    c: &mut Canonical,
    error: &SourcePatchError,
) -> Result<(), RestoreError> {
    use SourcePatchError::*;
    match error {
        UnsupportedVersion => c.str("UnsupportedVersion"),
        InvalidPolicy => c.str("InvalidPolicy"),
        InvalidBinding => c.str("InvalidBinding"),
        Empty => c.str("Empty"),
        InvalidEncoding => c.str("InvalidEncoding"),
        PayloadDigest => c.str("PayloadDigest"),
        InvalidEntry => c.str("InvalidEntry"),
        WrongBase => c.str("WrongBase"),
        InvalidBase => c.str("InvalidBase"),
        InvalidReport => c.str("InvalidReport"),
        InvalidResult => c.str("InvalidResult"),
        InvalidPath(value) => {
            c.str("InvalidPath");
            c.str(value);
        }
        ReservedPath(value) => {
            c.str("ReservedPath");
            c.str(value);
        }
        ReservedEnvironment(value) => {
            c.str("ReservedEnvironment");
            c.str(value);
        }
        ConflictingOperations(value) => {
            c.str("ConflictingOperations");
            c.str(value);
        }
        PreimageMismatch(value) => {
            c.str("PreimageMismatch");
            c.str(value);
        }
        OccupiedDestination(value) => {
            c.str("OccupiedDestination");
            c.str(value);
        }
        NoOp(value) => {
            c.str("NoOp");
            c.str(value);
        }
        ExcludedPath(value) => {
            c.str("ExcludedPath");
            c.str(value);
        }
        PathCollision(value) => {
            c.str("PathCollision");
            c.str(value);
        }
        SymlinkDenied(value) => {
            c.str("SymlinkDenied");
            c.str(value);
        }
        UnsafeSymlink(value) => {
            c.str("UnsafeSymlink");
            c.str(value);
        }
        Limit(value) => {
            static_patch_limit(value)?;
            c.str("Limit");
            c.str(value);
        }
    };
    Ok(())
}
pub(super) fn decode_patch(r: &mut Reader<'_>) -> Result<SourcePatchError, RestoreError> {
    use SourcePatchError::*;
    Ok(match r.text()?.as_str() {
        "UnsupportedVersion" => UnsupportedVersion,
        "InvalidPolicy" => InvalidPolicy,
        "InvalidBinding" => InvalidBinding,
        "Empty" => Empty,
        "InvalidEncoding" => InvalidEncoding,
        "PayloadDigest" => PayloadDigest,
        "InvalidEntry" => InvalidEntry,
        "WrongBase" => WrongBase,
        "InvalidBase" => InvalidBase,
        "InvalidReport" => InvalidReport,
        "InvalidResult" => InvalidResult,
        "InvalidPath" => InvalidPath(r.text()?),
        "ReservedPath" => ReservedPath(r.text()?),
        "ReservedEnvironment" => ReservedEnvironment(r.text()?),
        "ConflictingOperations" => ConflictingOperations(r.text()?),
        "PreimageMismatch" => PreimageMismatch(r.text()?),
        "OccupiedDestination" => OccupiedDestination(r.text()?),
        "NoOp" => NoOp(r.text()?),
        "ExcludedPath" => ExcludedPath(r.text()?),
        "PathCollision" => PathCollision(r.text()?),
        "SymlinkDenied" => SymlinkDenied(r.text()?),
        "UnsafeSymlink" => UnsafeSymlink(r.text()?),
        "Limit" => Limit(static_patch_limit(&r.text()?)?),
        _ => return Err(RestoreError::Encoding),
    })
}
fn static_patch_limit(value: &str) -> Result<&'static str, RestoreError> {
    match value {
        "aggregate payload bytes" => Ok("aggregate payload bytes"),
        "decoded aggregate payload bytes" => Ok("decoded aggregate payload bytes"),
        "decoded payload byte overflow" => Ok("decoded payload byte overflow"),
        "decoded payload bytes" => Ok("decoded payload bytes"),
        "encoded count" => Ok("encoded count"),
        "encoded field bytes" => Ok("encoded field bytes"),
        "envelope bytes" => Ok("envelope bytes"),
        "individual payload bytes" => Ok("individual payload bytes"),
        "operation count" => Ok("operation count"),
        "payload byte overflow" => Ok("payload byte overflow"),
        "result byte overflow" => Ok("result byte overflow"),
        "result bytes" => Ok("result bytes"),
        "result file count" => Ok("result file count"),
        _ => Err(RestoreError::UnsupportedDomainValue),
    }
}
pub(super) fn encode_credential(
    c: &mut Canonical,
    error: &CredentialError,
) -> Result<(), RestoreError> {
    use CredentialError::*;
    match error {
        UnsupportedPolicy => c.str("UnsupportedPolicy"),
        InvalidReviewPath => c.str("InvalidReviewPath"),
        EmptyReview => c.str("EmptyReview"),
        SensitiveInput(value) => {
            c.str("SensitiveInput");
            c.str(value);
        }
    };
    Ok(())
}
pub(super) fn decode_credential(r: &mut Reader<'_>) -> Result<CredentialError, RestoreError> {
    use CredentialError::*;
    Ok(match r.text()?.as_str() {
        "UnsupportedPolicy" => UnsupportedPolicy,
        "InvalidReviewPath" => InvalidReviewPath,
        "EmptyReview" => EmptyReview,
        "SensitiveInput" => SensitiveInput(r.text()?),
        _ => return Err(RestoreError::Encoding),
    })
}
pub(super) fn encode_source_evidence(
    c: &mut Canonical,
    error: &SourceEvidenceError,
) -> Result<(), RestoreError> {
    use SourceEvidenceError::*;
    match error {
        InvalidMetadata => c.str("InvalidMetadata"),
        BindingMismatch => c.str("BindingMismatch"),
        CheckSet => c.str("CheckSet"),
        CheckMismatch(value) => {
            c.str("CheckMismatch");
            c.str(value.as_str());
        }
        ArtifactMismatch(value) => {
            c.str("ArtifactMismatch");
            c.str(value);
        }
        Artifact(value) => {
            c.str("Artifact");
            encode_artifact(c, value)?;
        }
    };
    Ok(())
}
pub(super) fn decode_source_evidence(
    r: &mut Reader<'_>,
) -> Result<SourceEvidenceError, RestoreError> {
    use SourceEvidenceError::*;
    Ok(match r.text()?.as_str() {
        "InvalidMetadata" => InvalidMetadata,
        "BindingMismatch" => BindingMismatch,
        "CheckSet" => CheckSet,
        "CheckMismatch" => {
            CheckMismatch(CheckId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        "ArtifactMismatch" => ArtifactMismatch(r.text()?),
        "Artifact" => Artifact(decode_artifact(r)?),
        _ => return Err(RestoreError::Encoding),
    })
}
pub(super) fn encode_artifact(
    c: &mut Canonical,
    error: &SourceArtifactError,
) -> Result<(), RestoreError> {
    use SourceArtifactError::*;
    match error {
        Scope => c.str("Scope"),
        Limit => c.str("Limit"),
        Integrity => c.str("Integrity"),
        Unavailable(value) => {
            c.str("Unavailable");
            c.str(value);
        }
        Persistence(value) => {
            c.str("Persistence");
            c.str(value);
        }
    };
    Ok(())
}
pub(super) fn decode_artifact(r: &mut Reader<'_>) -> Result<SourceArtifactError, RestoreError> {
    use SourceArtifactError::*;
    Ok(match r.text()?.as_str() {
        "Scope" => Scope,
        "Limit" => Limit,
        "Integrity" => Integrity,
        "Unavailable" => Unavailable(r.text()?),
        "Persistence" => Persistence(r.text()?),
        _ => return Err(RestoreError::Encoding),
    })
}
pub(super) fn encode_repository(
    c: &mut Canonical,
    error: &RepositoryAnalysisError,
) -> Result<(), RestoreError> {
    use RepositoryAnalysisError::*;
    match error {
        SnapshotMismatch => c.str("SnapshotMismatch"),
        StaleGeneration => c.str("StaleGeneration"),
        InvalidProvenance => c.str("InvalidProvenance"),
        UnknownElement(value) => {
            c.str("UnknownElement");
            c.str(value.as_str());
        }
    };
    Ok(())
}
pub(super) fn decode_repository(
    r: &mut Reader<'_>,
) -> Result<RepositoryAnalysisError, RestoreError> {
    use RepositoryAnalysisError::*;
    Ok(match r.text()?.as_str() {
        "SnapshotMismatch" => SnapshotMismatch,
        "StaleGeneration" => StaleGeneration,
        "InvalidProvenance" => InvalidProvenance,
        "UnknownElement" => {
            UnknownElement(ElementId::new(r.text()?).map_err(|_| RestoreError::Identity)?)
        }
        _ => return Err(RestoreError::Encoding),
    })
}

pub(super) fn encode_engine_error(
    c: &mut Canonical,
    e: &crate::engine::EngineError,
) -> Result<(), RestoreError> {
    use crate::engine::EngineError;
    match e {
        EngineError::SourceAttempt(e) => {
            c.str("SourceAttempt");
            encode_attempt(c, e)
        }
        EngineError::SourceEvidence(e) => {
            c.str("SourceEvidence");
            encode_source_evidence(c, e)
        }
        EngineError::RepositoryAnalysis(e) => {
            c.str("RepositoryAnalysis");
            encode_repository(c, e)
        }
        _ => super::outcome::encode_error(c, e),
    }
}
pub(super) fn decode_engine_error(
    r: &mut Reader<'_>,
) -> Result<crate::engine::EngineError, RestoreError> {
    use crate::engine::EngineError;
    let saved = r.offset;
    match r.text()?.as_str() {
        "SourceAttempt" => Ok(EngineError::SourceAttempt(decode_attempt(r)?)),
        "SourceEvidence" => Ok(EngineError::SourceEvidence(decode_source_evidence(r)?)),
        "RepositoryAnalysis" => Ok(EngineError::RepositoryAnalysis(decode_repository(r)?)),
        _ => {
            r.offset = saved;
            super::outcome::decode_error(r)
        }
    }
}
