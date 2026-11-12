//! Exact persisted source decisions, not RPC responses or capability tokens.
use celetixo_engine::credentials::CredentialError;
use celetixo_engine::engine::{EngineError, SourceAttemptError};
use celetixo_engine::ids::*;
use celetixo_engine::persistence::*;
use celetixo_engine::repository::RepositoryAnalysisError;
use celetixo_engine::source_evidence::{SourceArtifactError, SourceEvidenceError};
use celetixo_engine::source_execution::SourcePlanError;
use celetixo_engine::source_patch::SourcePatchError;
fn context() -> SourceOutcomeContext {
    SourceOutcomeContext {
        scope: Scope {
            organization: OrganizationId::new("o").unwrap(),
            workspace: WorkspaceId::new("w").unwrap(),
            repository: RepositoryId::new("r").unwrap(),
        },
        transaction: TransactionId::new("tx").unwrap(),
        target_attempt: 0,
        key: IdempotencyKey::new("k").unwrap(),
        request: Digest::from_bytes([1; 32]),
    }
}
fn check_id() -> CheckId {
    CheckId::new("check").unwrap()
}
fn invariant() -> InvariantId {
    InvariantId::new("invariant").unwrap()
}
fn errors() -> Vec<EngineError> {
    let mut errors = vec![
        EngineError::SourceAttempt(SourceAttemptError::PolicyChanged),
        EngineError::SourceAttempt(SourceAttemptError::Expired),
        EngineError::SourceAttempt(SourceAttemptError::BindingMismatch),
        EngineError::SourceAttempt(SourceAttemptError::InvalidState),
        EngineError::SourceAttempt(SourceAttemptError::AttemptActive),
        EngineError::SourceAttempt(SourceAttemptError::AttemptIdentity),
        EngineError::SourceAttempt(SourceAttemptError::AlreadyClaimed),
        EngineError::SourceAttempt(SourceAttemptError::UntrustedExecutor),
        EngineError::SourceAttempt(SourceAttemptError::UnsupportedInvariant),
        EngineError::SourceAttempt(SourceAttemptError::UnknownPolicyElement),
        EngineError::SourceAttempt(SourceAttemptError::SourceRepositoryRequired),
        EngineError::SourceAttempt(SourceAttemptError::InvalidReason),
        EngineError::SourceAttempt(SourceAttemptError::InvalidOptions),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::DependencyCycle)),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::BindingMismatch)),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::IncompleteCapture)),
        EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::PatchPolicyMismatch,
        )),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::UnknownCacheCheck)),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::DuplicateCheck(
            check_id(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::MissingCheck(
            check_id(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::SubstitutedCheck(
            check_id(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::ResourceCeiling(
            check_id(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::UnapprovedBackend(check_id()),
        )),
        EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::ConflictingRequirement(check_id()),
        )),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::MissingInvariant(
            invariant(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPath(
            "path\0with\ntrivia".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::UnauthorizedPath(
            "path\0with\ntrivia".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::PathPreimage(
            "path\0with\ntrivia".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::CheckInputMismatch("path\0with\ntrivia".into()),
        )),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "credential scope",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "credential scope or grant",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "empty path authorization",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "empty verification plan",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "identity or empty policy",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "network widening",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "patch policy",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "source invariant",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "artifact contract",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "cache platform",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "commands cannot run in T0",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "credential reference",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "definition budget",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "environment key",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "literal environment value",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "network grant",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "required execution logs",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "toolchain/backend identity",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidCheck(
            "zero resource limit",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(
            SourcePatchError::UnsupportedVersion,
        )),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidPolicy)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidBinding)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Empty)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidEncoding)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::PayloadDigest)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidEntry)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::WrongBase)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidBase)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidReport)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidResult)),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::InvalidPath(
            "path\0binary".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::ReservedPath(
            "path\0binary".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(
            SourcePatchError::ReservedEnvironment("path\0binary".into()),
        )),
        EngineError::SourceAttempt(SourceAttemptError::Patch(
            SourcePatchError::ConflictingOperations("path\0binary".into()),
        )),
        EngineError::SourceAttempt(SourceAttemptError::Patch(
            SourcePatchError::PreimageMismatch("path\0binary".into()),
        )),
        EngineError::SourceAttempt(SourceAttemptError::Patch(
            SourcePatchError::OccupiedDestination("path\0binary".into()),
        )),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::NoOp(
            "path\0binary".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::ExcludedPath(
            "path\0binary".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::PathCollision(
            "path\0binary".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::SymlinkDenied(
            "path\0binary".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::UnsafeSymlink(
            "path\0binary".into(),
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "aggregate payload bytes",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "decoded aggregate payload bytes",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "decoded payload byte overflow",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "decoded payload bytes",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "encoded count",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "encoded field bytes",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "envelope bytes",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "individual payload bytes",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "operation count",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "payload byte overflow",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "result byte overflow",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "result bytes",
        ))),
        EngineError::SourceAttempt(SourceAttemptError::Patch(SourcePatchError::Limit(
            "result file count",
        ))),
    ];
    for error in [
        CredentialError::UnsupportedPolicy,
        CredentialError::InvalidReviewPath,
        CredentialError::EmptyReview,
        CredentialError::SensitiveInput("logical-path\0only".into()),
    ] {
        errors.push(EngineError::SourceAttempt(SourceAttemptError::Credential(
            error.clone(),
        )));
        errors.push(EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::Credential(error),
        )));
    }
    for error in [
        SourceEvidenceError::InvalidMetadata,
        SourceEvidenceError::BindingMismatch,
        SourceEvidenceError::CheckSet,
        SourceEvidenceError::CheckMismatch(check_id()),
        SourceEvidenceError::ArtifactMismatch("artifact\0name".into()),
    ] {
        errors.push(EngineError::SourceEvidence(error));
    }
    for error in [
        SourceArtifactError::Scope,
        SourceArtifactError::Limit,
        SourceArtifactError::Integrity,
        SourceArtifactError::Unavailable("unavailable\0".into()),
        SourceArtifactError::Persistence("storage\0interrupted".into()),
    ] {
        errors.push(EngineError::SourceEvidence(SourceEvidenceError::Artifact(
            error,
        )));
    }
    for error in [
        RepositoryAnalysisError::SnapshotMismatch,
        RepositoryAnalysisError::StaleGeneration,
        RepositoryAnalysisError::InvalidProvenance,
        RepositoryAnalysisError::UnknownElement(ElementId::new("element").unwrap()),
    ] {
        errors.push(EngineError::RepositoryAnalysis(error));
    }
    errors
}
#[test]
fn all_source_error_families_preserve_exact_tags_values_and_binary_strings() {
    for error in errors() {
        let c = context();
        let record = SourceOutcomeRecord::new(c.clone(), Err(error.clone())).unwrap();
        let restored = restore_source_outcome(
            &c,
            record.digest(),
            &record.canonical_bytes(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(restored, record);
        assert_eq!(restored.outcome(), &Err(error));
    }
}
fn field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}
#[test]
fn independently_encoded_accepted_and_nested_rejection_layouts_are_frozen() {
    for accepted in [false, true] {
        let mut bytes = Vec::new();
        for text in ["o", "w", "r", "tx"] {
            field(&mut bytes, text.as_bytes());
        }
        field(&mut bytes, &0u64.to_be_bytes());
        field(&mut bytes, b"k");
        field(&mut bytes, &[1; 32]);
        let expected = if accepted {
            field(&mut bytes, b"Accepted");
            field(&mut bytes, &[2; 32]);
            Ok(SourceAdmissionId(Digest::from_bytes([2; 32])))
        } else {
            for text in ["Rejected", "SourceAttempt", "Plan", "MissingCheck", "check"] {
                field(&mut bytes, text.as_bytes());
            }
            Err(EngineError::SourceAttempt(SourceAttemptError::Plan(
                SourcePlanError::MissingCheck(check_id()),
            )))
        };
        let record = restore_source_outcome(
            &context(),
            Digest::of(SourceOutcomeRecord::FORMAT, &bytes),
            &bytes,
            Default::default(),
        )
        .unwrap();
        assert_eq!(record.outcome(), &expected);
        assert_eq!(record.canonical_bytes(), bytes);
        let expected_digest = if accepted {
            "bc3218018cd9fd7647d3c5527a741b1d756a4c20800eab64c59f5faa4b0ad597"
        } else {
            "8b599e46c8acb20e052afe49d76ed34cf2371b7a192872574922203c64650dd3"
        };
        assert_eq!(record.digest().to_string(), expected_digest);
    }
}
#[test]
fn malformed_rehashed_errors_context_and_budgets_cannot_be_hidden_by_valid_addresses() {
    let c = context();
    let record = SourceOutcomeRecord::new(
        c.clone(),
        Err(EngineError::SourceAttempt(SourceAttemptError::Plan(
            SourcePlanError::MissingCheck(check_id()),
        ))),
    )
    .unwrap();
    let bytes = record.canonical_bytes();
    for end in 0..bytes.len() {
        let prefix = &bytes[..end];
        assert!(
            restore_source_outcome(
                &c,
                Digest::of(SourceOutcomeRecord::FORMAT, prefix),
                prefix,
                Default::default()
            )
            .is_err()
        );
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert_eq!(
        restore_source_outcome(
            &c,
            Digest::of(SourceOutcomeRecord::FORMAT, &extra),
            &extra,
            Default::default()
        ),
        Err(RestoreError::NonCanonical)
    );
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 31,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            restore_source_outcome(&c, record.digest(), &bytes, limits),
            Err(RestoreError::Limit)
        );
    }
    let mut other = c.clone();
    other.target_attempt = 999;
    assert_eq!(
        restore_source_outcome(&other, record.digest(), &bytes, Default::default()),
        Err(RestoreError::BindingMismatch)
    );
    let mut other = c.clone();
    other.key = IdempotencyKey::new("other").unwrap();
    assert_eq!(
        restore_source_outcome(&other, record.digest(), &bytes, Default::default()),
        Err(RestoreError::BindingMismatch)
    );
    let mut bad = bytes.clone();
    let offset = bad
        .windows(b"MissingCheck".len())
        .position(|w| w == b"MissingCheck")
        .unwrap();
    bad[offset] = b'X';
    assert_eq!(
        restore_source_outcome(
            &c,
            Digest::of(SourceOutcomeRecord::FORMAT, &bad),
            &bad,
            Default::default()
        ),
        Err(RestoreError::Encoding)
    );
    for e in [
        EngineError::StoreUnavailable,
        EngineError::ResponseLost,
        EngineError::Authority(
            celetixo_engine::engine::authority::AuthorityFailure::CommitIndeterminate,
        ),
        EngineError::SourceAttempt(SourceAttemptError::Plan(SourcePlanError::InvalidPolicy(
            "unpublished arbitrary static",
        ))),
    ] {
        assert_eq!(
            SourceOutcomeRecord::new(c.clone(), Err(e)),
            Err(RestoreError::UnsupportedDomainValue)
        );
    }
}
