//! Pure source metadata codecs. These fixtures neither execute checks nor
//! establish executor authority, artifact persistence, or evidence admission.
use celetixo_engine::access::AccessSet;
use celetixo_engine::credentials::CredentialPolicy;
use celetixo_engine::engine::{Engine, InMemoryStore, Permission, SourceAdmission};
use celetixo_engine::evidence::CheckSpec;
use celetixo_engine::ids::*;
use celetixo_engine::persistence::{
    DomainDecodeLimits, RestoreError, SourceCheckRecordBinding, restore_source_check_record,
    restore_source_evidence,
};
use celetixo_engine::repository::{RepositoryAnalysis, RepositoryAnalysisInput};
use celetixo_engine::snapshot::*;
use celetixo_engine::source_evidence::*;
use celetixo_engine::source_execution::*;
use celetixo_engine::source_patch::*;
use celetixo_engine::transaction::TransactionIntent;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};

fn d(byte: u8) -> Digest {
    Digest::from_bytes([byte; 32])
}
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("org").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repo").unwrap(),
    }
}
fn cid() -> CheckId {
    CheckId::new("verify").unwrap()
}
fn limits() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1,
        cpu_time_ms: 2,
        memory_bytes: 3,
        disk_bytes: 4,
        process_count: 5,
        file_descriptors: 6,
        stdout_bytes: 7,
        stderr_bytes: 8,
        artifact_bytes: 9,
    }
}
fn binding() -> SourceCheckRecordBinding {
    SourceCheckRecordBinding {
        scope: scope(),
        admission: SourceAdmissionId(d(1)),
        plan: SourcePlanId(d(2)),
        candidate: SnapshotId(d(3)),
        check: cid(),
        retention_policy: d(9),
    }
}
fn input() -> SourceCheckRecordInput {
    SourceCheckRecordInput {
        admission: SourceAdmissionId(d(1)),
        plan: SourcePlanId(d(2)),
        candidate: SnapshotId(d(3)),
        check: cid(),
        definition: d(4),
        backend: d(5),
        executable: d(6),
        runtime_closure: d(7),
        environment: d(8),
        limits: limits(),
        started_at_ms: 10,
        finished_at_ms: u64::MAX,
        outcome: SourceCheckOutcome::UnknownTermination {
            reason: "runtime result unavailable".into(),
        },
        usage: SourceResourceUsage {
            wall_time_ms: u64::MAX,
            cpu_time_us: 12,
            peak_memory_bytes: 13,
        },
        post_source_manifest: Some(d(10)),
        termination: SourceTermination::Unknown {
            reason: "termination uncertain".into(),
        },
        cleanup: SourceCleanup::Quarantined {
            reason: "source retained".into(),
        },
        stdout_truncated: true,
        stderr_truncated: false,
        artifacts: ["diagnostics", "stderr", "stdout"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    name.into(),
                    SourceArtifactReference {
                        scope: scope(),
                        name: name.into(),
                        media_type: "text/plain".into(),
                        content: d(11 + index as u8),
                        bytes: u64::MAX,
                        retention_policy: d(9),
                    },
                )
            })
            .collect(),
        diagnostics: vec!["first\nline".into(), "résumé".into()],
    }
}

fn number(value: u64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}
fn pack(fields: &[Vec<u8>]) -> Vec<u8> {
    fields
        .iter()
        .flat_map(|field| [(field.len() as u64).to_be_bytes().to_vec(), field.clone()].concat())
        .collect()
}
fn fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let count = usize::try_from(u64::from_be_bytes(
            bytes[offset..offset + 8].try_into().unwrap(),
        ))
        .unwrap();
        result.push(bytes[offset + 8..offset + 8 + count].to_vec());
        offset += count + 8;
    }
    result
}

/// This literal field order is independent of the production encoder/decoder.
/// Its content address is frozen below so accidental format changes cannot
/// silently make the encoder and decoder agree on a different wire layout.
fn frozen_check_fields() -> Vec<Vec<u8>> {
    let mut result = vec![
        b"celetixo/source-check-record/v1".to_vec(),
        vec![1; 32],
        vec![2; 32],
        vec![3; 32],
        b"verify".to_vec(),
        vec![4; 32],
        vec![5; 32],
        vec![6; 32],
        vec![7; 32],
        vec![8; 32],
        number(1),
        number(2),
        number(3),
        number(4),
        number(5),
        number(6),
        number(7),
        number(8),
        number(9),
        number(10),
        number(u64::MAX),
        number(9),
        b"runtime result unavailable".to_vec(),
        number(u64::MAX),
        number(12),
        number(13),
        number(1),
        vec![10; 32],
        number(1),
        b"termination uncertain".to_vec(),
        number(1),
        b"source retained".to_vec(),
        number(1),
        number(0),
        number(3),
    ];
    for (name, content) in [("diagnostics", 11), ("stderr", 12), ("stdout", 13)] {
        result.extend([
            b"org".to_vec(),
            b"workspace".to_vec(),
            b"repo".to_vec(),
            name.as_bytes().to_vec(),
            b"text/plain".to_vec(),
            vec![content; 32],
            number(u64::MAX),
            vec![9; 32],
        ]);
    }
    result.extend([
        number(2),
        b"first\nline".to_vec(),
        "résumé".as_bytes().to_vec(),
    ]);
    result
}

#[test]
fn frozen_execution_layout_preserves_every_nonrecursive_field() {
    let record = SourceCheckRecord::new(input()).unwrap();
    let bytes = pack(&frozen_check_fields());
    let address = Digest::of(SOURCE_ARTIFACT_DOMAIN, &bytes);
    assert_eq!(
        address.to_string(),
        "edd6480abdbbde55695b6e749900ea665c8d627c08d64a737cf2fa8fd978cc1a"
    );
    assert_eq!(record.execution_bytes(), bytes);
    assert_eq!(
        restore_source_check_record(&binding(), address, &bytes, DomainDecodeLimits::default())
            .unwrap(),
        record
    );
    assert!(!record.input().artifacts.contains_key("execution-record"));
}

#[test]
fn execution_outcome_termination_cleanup_and_manifest_variants_survive() {
    let outcomes = [
        SourceCheckOutcome::Exited { code: 255 },
        SourceCheckOutcome::Signalled { signal: 64 },
        SourceCheckOutcome::TimedOut,
        SourceCheckOutcome::Cancelled,
        SourceCheckOutcome::Expired,
        SourceCheckOutcome::Revoked,
        SourceCheckOutcome::OutputLimit,
        SourceCheckOutcome::Infrastructure {
            reason: "unavailable".into(),
        },
        SourceCheckOutcome::Skipped {
            reason: "dependency failed".into(),
        },
        SourceCheckOutcome::UnknownTermination {
            reason: "process status missing".into(),
        },
    ];
    for (index, outcome) in outcomes.into_iter().enumerate() {
        let mut value = input();
        value.outcome = outcome;
        value.post_source_manifest = if index % 2 == 0 { None } else { Some(d(10)) };
        value.termination = if index % 2 == 0 {
            SourceTermination::Confirmed
        } else {
            SourceTermination::Unknown {
                reason: "unknown".into(),
            }
        };
        value.cleanup = match index % 3 {
            0 => SourceCleanup::Confirmed,
            1 => SourceCleanup::Failed {
                reason: "cleanup failed".into(),
            },
            _ => SourceCleanup::Quarantined {
                reason: "retained".into(),
            },
        };
        value.stdout_truncated = index % 2 == 0;
        value.stderr_truncated = index % 2 != 0;
        let record = SourceCheckRecord::new(value).unwrap();
        let bytes = record.execution_bytes();
        assert_eq!(
            restore_source_check_record(
                &binding(),
                Digest::of(SOURCE_ARTIFACT_DOMAIN, &bytes),
                &bytes,
                DomainDecodeLimits::default()
            )
            .unwrap(),
            record
        );
    }
}

#[test]
fn check_decoder_rejects_noncanonical_invalid_and_misbound_metadata() {
    let original = frozen_check_fields();
    let restore = |changed: &[Vec<u8>]| {
        let bytes = pack(changed);
        restore_source_check_record(
            &binding(),
            Digest::of(SOURCE_ARTIFACT_DOMAIN, &bytes),
            &bytes,
            DomainDecodeLimits::default(),
        )
    };
    for (index, replacement, expected) in [
        (
            0,
            b"celetixo/source-check-record/v2".to_vec(),
            RestoreError::Encoding,
        ),
        (1, vec![90; 32], RestoreError::BindingMismatch),
        (2, vec![90; 32], RestoreError::BindingMismatch),
        (3, vec![90; 32], RestoreError::BindingMismatch),
        (4, b"another-check".to_vec(), RestoreError::BindingMismatch),
        (4, b"bad check".to_vec(), RestoreError::Identity),
        (5, vec![4; 31], RestoreError::Encoding),
        (10, vec![0; 7], RestoreError::Encoding),
        (21, number(10), RestoreError::Encoding),
        (26, number(2), RestoreError::Encoding),
        (28, number(2), RestoreError::Encoding),
        (30, number(3), RestoreError::Encoding),
        (32, number(2), RestoreError::Encoding),
        (33, number(2), RestoreError::Encoding),
        (35, b"other-org".to_vec(), RestoreError::BindingMismatch),
        (42, vec![90; 32], RestoreError::BindingMismatch),
        (60, vec![0xff], RestoreError::Encoding),
    ] {
        let mut changed = original.clone();
        changed[index] = replacement;
        assert_eq!(restore(&changed), Err(expected), "field {index}");
    }
    let mut changed = original.clone();
    changed[10] = number(0);
    assert!(matches!(
        restore(&changed),
        Err(RestoreError::SourceEvidence(
            SourceEvidenceError::InvalidMetadata
        ))
    ));
    let mut changed = original.clone();
    changed[20] = number(9);
    assert!(matches!(
        restore(&changed),
        Err(RestoreError::SourceEvidence(
            SourceEvidenceError::InvalidMetadata
        ))
    ));
    // A known status with an unrepresentable i32 must not truncate into success.
    let mut changed = original.clone();
    changed[21] = number(0);
    changed[22] = i64::MAX.to_be_bytes().to_vec();
    assert_eq!(restore(&changed), Err(RestoreError::Encoding));
    // This valid i32 is still outside the existing exit-status value contract.
    changed[22] = (-1_i64).to_be_bytes().to_vec();
    assert!(matches!(
        restore(&changed),
        Err(RestoreError::SourceEvidence(
            SourceEvidenceError::InvalidMetadata
        ))
    ));
    for (tag, value) in [(0, 256_i64), (1, 0), (1, 65)] {
        let mut changed = original.clone();
        changed[21] = number(tag);
        changed[22] = value.to_be_bytes().to_vec();
        assert!(matches!(
            restore(&changed),
            Err(RestoreError::SourceEvidence(
                SourceEvidenceError::InvalidMetadata
            ))
        ));
    }
    let mut changed = original.clone();
    changed[59] = number(3);
    changed.push(b"bad\0diagnostic".to_vec());
    assert!(matches!(
        restore(&changed),
        Err(RestoreError::SourceEvidence(
            SourceEvidenceError::InvalidMetadata
        ))
    ));
    // Duplicate and reordered artifact names must not be silently normalized.
    let mut changed = original.clone();
    changed[46] = b"diagnostics".to_vec();
    assert_eq!(restore(&changed), Err(RestoreError::NonCanonical));
    let mut changed = original.clone();
    let first = changed[35..43].to_vec();
    let second = changed[43..51].to_vec();
    changed[35..43].clone_from_slice(&second);
    changed[43..51].clone_from_slice(&first);
    assert_eq!(restore(&changed), Err(RestoreError::NonCanonical));
    // A self-reference in the execution object is not part of its v1 encoding.
    let mut changed = original.clone();
    changed[38] = b"execution-record".to_vec();
    assert_eq!(restore(&changed), Err(RestoreError::NonCanonical));
    let mut changed = original.clone();
    changed.push(b"trailing".to_vec());
    assert_eq!(restore(&changed), Err(RestoreError::NonCanonical));
}

#[test]
fn check_decoder_checks_digest_truncation_and_all_explicit_budgets() {
    let bytes = pack(&frozen_check_fields());
    let address = Digest::of(SOURCE_ARTIFACT_DOMAIN, &bytes);
    assert_eq!(
        restore_source_check_record(&binding(), d(99), &bytes, DomainDecodeLimits::default()),
        Err(RestoreError::DigestMismatch)
    );
    for end in 0..bytes.len() {
        let truncated = &bytes[..end];
        assert!(
            restore_source_check_record(
                &binding(),
                Digest::of(SOURCE_ARTIFACT_DOMAIN, truncated),
                truncated,
                DomainDecodeLimits::default()
            )
            .is_err()
        );
    }
    let exact = DomainDecodeLimits {
        encoded_bytes: bytes.len(),
        collection_entries: 5,
        field_bytes: 32,
    };
    assert!(restore_source_check_record(&binding(), address, &bytes, exact).is_ok());
    for limited in [
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..exact
        },
        DomainDecodeLimits {
            collection_entries: 4,
            ..exact
        },
        DomainDecodeLimits {
            field_bytes: 31,
            ..exact
        },
        DomainDecodeLimits {
            collection_entries: 0,
            ..exact
        },
        DomainDecodeLimits {
            field_bytes: 0,
            ..exact
        },
    ] {
        assert_eq!(
            restore_source_check_record(&binding(), address, &bytes, limited),
            Err(RestoreError::Limit)
        );
    }
    let mut malicious = frozen_check_fields();
    malicious[34] = number(u64::MAX);
    let malicious = pack(&malicious);
    assert_eq!(
        restore_source_check_record(
            &binding(),
            Digest::of(SOURCE_ARTIFACT_DOMAIN, &malicious),
            &malicious,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::Limit)
    );
    let too_large = vec![0; 4 * 1024 * 1024 + 1];
    assert_eq!(
        restore_source_check_record(&binding(), d(99), &too_large, DomainDecodeLimits::default()),
        Err(RestoreError::Limit)
    );
}

/// Obtain a real immutable admission through the existing Engine path. No new
/// constructor or codec is allowed to manufacture an admission capability.
fn admission() -> SourceAdmission {
    let (store, admin) = InMemoryStore::new();
    let engine = Engine::new(store);
    let session = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            scope(),
            Permission::ALL.into(),
        )
        .unwrap();
    let element = ElementId::new("file:main").unwrap();
    let base = Snapshot::new(
        scope(),
        Revision(0),
        SnapshotManifest {
            base_commit: "fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "celetixo/reference-source/codec-fixture-v1".into(),
            inputs: vec![WorkspaceInput {
                path: "main.rs".into(),
                layer: InputLayer::Base,
                kind: InputKind::File,
                mode: FileMode::Regular,
                bytes: Some(b"old".to_vec()),
            }],
            environment: BTreeMap::new(),
        },
        State::new(),
    )
    .unwrap();
    let analysis = RepositoryAnalysis::new(
        &base,
        RepositoryAnalysisInput {
            index_generation: d(20),
            analyzer_version: "finite-reference/v1".into(),
            configuration: d(21),
            elements: [element.clone()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element.clone(), [cid()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    engine
        .register_repository(&session, base.clone(), analysis.clone())
        .unwrap();
    let backend = BackendIdentity {
        name: "metadata-fixture".into(),
        version: "v1".into(),
        image: d(22),
        profile: d(23),
    };
    let check = SourceCheckDefinition::new(SourceCheckInput {
        check: CheckSpec::new(
            cid(),
            vec!["/tools/reference".into()],
            "fixture/v1".into(),
            d(24),
            d(25),
        )
        .unwrap(),
        working_directory: WorkingDirectory::WorkspaceRoot,
        inputs: [(
            "main.rs".into(),
            Digest::of("celetixo/source-content/v1", b"new"),
        )]
        .into(),
        outputs: BTreeSet::new(),
        toolchain: ToolchainIdentity {
            executable: "/tools/reference".into(),
            executable_digest: d(26),
            runtime_closure: d(27),
            version: "fixture/v1".into(),
            platform: "fixture".into(),
            architecture: "fixture".into(),
        },
        dependency_inputs: BTreeMap::new(),
        environment: BTreeMap::new(),
        tier: ExecutionTier::T1,
        backend: backend.clone(),
        resources: limits(),
        network: NetworkPolicy::DenyAll,
        credential_references: BTreeSet::new(),
        expected_artifacts: ["stdout", "stderr", "diagnostics"]
            .into_iter()
            .map(|name| ArtifactRequirement {
                name: name.into(),
                media_type: "text/plain".into(),
                max_bytes: 1,
                required: true,
            })
            .collect(),
        depends_on: BTreeSet::new(),
        failure_policy: FailurePolicy::FailAttempt,
    })
    .unwrap();
    let policy = SourceExecutionPolicy::new(SourcePolicyInput {
        version: 1,
        generation: 1,
        scope: scope(),
        snapshot: base.id(),
        analysis_artifact: analysis.digest(),
        index_generation: analysis.index_generation(),
        capture: SourceCaptureBinding {
            snapshot: base.id(),
            manifest: base.manifest().digest(),
            capture_policy: d(28),
            uncertainty: SourceCaptureBinding::empty_uncertainty_digest(),
            consistency: SourceCaptureConsistency::ReferenceFixture,
            gaps: BTreeSet::new(),
        },
        patch_policy: PatchPolicy::default(),
        credentials: CredentialPolicy::default(),
        paths: [(
            "main.rs".into(),
            SourcePathRule {
                allowed_operations: [OperationKind::Replace].into(),
                required_writes: [element.clone()].into(),
                expected_base: Some(Preimage::of(&base.manifest().inputs[0]).unwrap()),
            },
        )]
        .into(),
        checks: vec![check.clone()],
        invariants: vec![],
        always_required: BTreeSet::new(),
        approved_tiers: [ExecutionTier::T1].into(),
        approved_backends: [backend.digest()].into(),
        resource_ceiling: limits(),
        network_allowed: false,
        approved_credential_references: BTreeSet::new(),
        expires_at_ms: 10_000,
        retention_policy: d(9),
    })
    .unwrap();
    engine
        .install_source_policy(&session, &scope(), None, policy)
        .unwrap();
    let transaction = TransactionId::new("transaction").unwrap();
    engine
        .propose(
            &session,
            &scope(),
            transaction.clone(),
            base.id(),
            TransactionIntent {
                objective: "source metadata fixture".into(),
                accesses: AccessSet {
                    writes: [element].into(),
                    ..Default::default()
                },
                invariants: vec![],
                required_checks: vec![check.input().check.clone()],
                depends_on: BTreeSet::new(),
            },
        )
        .unwrap();
    engine.analyze(&session, &scope(), &transaction).unwrap();
    engine
        .reserve(&session, &scope(), &transaction, 1_000)
        .unwrap();
    engine.execute(&session, &scope(), &transaction).unwrap();
    let binding = engine
        .source_patch_context(&session, &scope(), &transaction)
        .unwrap()
        .bind(
            SourceAttemptId::new("source-attempt").unwrap(),
            "metadata-fixture".into(),
            500,
        )
        .unwrap();
    let patch = SourcePatch::new(
        binding,
        PatchPolicy::default(),
        vec![PatchOperation::Replace {
            path: "main.rs".into(),
            expected: Preimage::of(&base.manifest().inputs[0]).unwrap(),
            entry: PatchEntry::file(b"new".to_vec(), FileMode::Regular).unwrap(),
        }],
    )
    .unwrap();
    engine
        .admit_source_attempt(
            &session,
            &scope(),
            patch,
            IdempotencyKey::new("admission").unwrap(),
        )
        .unwrap()
}

fn envelope_fixture(admission: &SourceAdmission) -> SourceEvidence {
    let binding =
        SourceEvidenceBinding::for_admission(admission, PrincipalId::new("executor").unwrap());
    let mut value = input();
    value.admission = admission.id();
    value.plan = admission.plan().id();
    value.candidate = admission.resulting_snapshot();
    value.started_at_ms = 10;
    value.finished_at_ms = 20;
    // The self-reference is excluded only from the standalone execution object.
    let record_bytes = SourceCheckRecord::new(value.clone())
        .unwrap()
        .execution_bytes();
    value.artifacts.insert(
        "execution-record".into(),
        SourceArtifactReference::for_bytes(
            scope(),
            "execution-record".into(),
            "application/vnd.celetixo.source-check-v1".into(),
            &record_bytes,
            d(9),
        ),
    );
    SourceEvidence::new(
        binding,
        5,
        25,
        vec![SourceCheckRecord::new(value).unwrap()],
        BTreeMap::from([
            (
                "execution-admission".into(),
                SourceArtifactReference::for_bytes(
                    scope(),
                    "execution-admission".into(),
                    "application/octet-stream".into(),
                    &admission.canonical_bytes(),
                    d(9),
                ),
            ),
            (
                "verification-plan".into(),
                SourceArtifactReference::for_bytes(
                    scope(),
                    "verification-plan".into(),
                    "application/octet-stream".into(),
                    &admission.plan().canonical_bytes(),
                    d(9),
                ),
            ),
        ]),
        vec!["evidence diagnostic\nsecond line".into()],
    )
    .unwrap()
}

#[test]
fn v2_envelope_restoration_keeps_self_references_provenance_and_attestation() {
    let admission = admission();
    let evidence = envelope_fixture(&admission);
    let mut expected = vec![
        b"celetixo/source-evidence/v2".to_vec(),
        admission.id().0.as_bytes().to_vec(),
        b"org".to_vec(),
        b"workspace".to_vec(),
        b"repo".to_vec(),
        b"executor".to_vec(),
        vec![9; 32],
        admission.id().0.as_bytes().to_vec(),
        number(1),
        number(5),
        number(25),
        number(1),
    ];
    let mut check = frozen_check_fields();
    check[1] = admission.id().0.as_bytes().to_vec();
    check[2] = admission.plan().id().0.as_bytes().to_vec();
    check[3] = admission.resulting_snapshot().0.as_bytes().to_vec();
    check[20] = number(20);
    check.remove(0);
    // The complete envelope embeds the extra execution-record object, sorted
    // between diagnostics and stderr; its preimage excludes that reference.
    check[33] = number(4);
    let object = &evidence.checks()[&cid()].input().artifacts["execution-record"];
    let object_fields = vec![
        b"org".to_vec(),
        b"workspace".to_vec(),
        b"repo".to_vec(),
        b"execution-record".to_vec(),
        b"application/vnd.celetixo.source-check-v1".to_vec(),
        object.content.as_bytes().to_vec(),
        number(object.bytes),
        vec![9; 32],
    ];
    check.splice(42..42, object_fields);
    expected.extend(check);
    expected.push(number(2));
    for (name, object) in evidence.provenance() {
        expected.extend([
            b"org".to_vec(),
            b"workspace".to_vec(),
            b"repo".to_vec(),
            name.as_bytes().to_vec(),
            b"application/octet-stream".to_vec(),
            object.content.as_bytes().to_vec(),
            number(object.bytes),
            vec![9; 32],
        ]);
    }
    expected.extend([number(1), b"evidence diagnostic\nsecond line".to_vec()]);
    let bytes = evidence.canonical_bytes();
    assert_eq!(bytes, pack(&expected));
    assert_eq!(
        restore_source_evidence(
            evidence.binding(),
            evidence.id(),
            &bytes,
            DomainDecodeLimits::default()
        )
        .unwrap(),
        evidence
    );
    assert!(
        evidence.checks()[&cid()]
            .input()
            .artifacts
            .contains_key("execution-record")
    );
    let unattested_binding = evidence
        .binding()
        .clone()
        .with_attestation(SourceAttestationLevel::Unattested);
    let unattested = SourceEvidence::new(
        unattested_binding.clone(),
        5,
        25,
        evidence.checks().values().cloned().collect(),
        evidence.provenance().clone(),
        vec![],
    )
    .unwrap();
    let bytes = unattested.canonical_bytes();
    assert_eq!(
        restore_source_evidence(
            &unattested_binding,
            unattested.id(),
            &bytes,
            DomainDecodeLimits::default()
        )
        .unwrap(),
        unattested
    );
    assert_eq!(
        restore_source_evidence(
            evidence.binding(),
            unattested.id(),
            &bytes,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
}

#[test]
fn envelope_decoder_rejects_binding_substitution_and_noncanonical_sets() {
    let admission = admission();
    let evidence = envelope_fixture(&admission);
    let original = fields(&evidence.canonical_bytes());
    let restore = |changed: &[Vec<u8>]| {
        let bytes = pack(changed);
        restore_source_evidence(
            evidence.binding(),
            SourceEvidenceId(Digest::of(SOURCE_EVIDENCE_FORMAT, &bytes)),
            &bytes,
            DomainDecodeLimits::default(),
        )
    };
    for (index, replacement, expected) in [
        (
            0,
            b"celetixo/source-evidence/v1".to_vec(),
            RestoreError::Encoding,
        ),
        (1, vec![90; 32], RestoreError::BindingMismatch),
        (2, b"other-org".to_vec(), RestoreError::BindingMismatch),
        (
            3,
            b"other-workspace".to_vec(),
            RestoreError::BindingMismatch,
        ),
        (4, b"other-repo".to_vec(), RestoreError::BindingMismatch),
        (5, b"other-executor".to_vec(), RestoreError::BindingMismatch),
        (5, b"invalid executor".to_vec(), RestoreError::Identity),
        (6, vec![90; 32], RestoreError::BindingMismatch),
        (7, vec![90; 32], RestoreError::BindingMismatch),
        (8, number(2), RestoreError::Encoding),
        (8, number(0), RestoreError::BindingMismatch),
        (12, vec![90; 32], RestoreError::BindingMismatch),
        (46, b"other-org".to_vec(), RestoreError::BindingMismatch),
        (53, vec![90; 32], RestoreError::BindingMismatch),
    ] {
        let mut changed = original.clone();
        changed[index] = replacement;
        assert_eq!(restore(&changed), Err(expected), "field {index}");
    }
    let mut changed = original.clone();
    changed[10] = number(4);
    assert!(matches!(
        restore(&changed),
        Err(RestoreError::SourceEvidence(
            SourceEvidenceError::InvalidMetadata
        ))
    ));
    let mut changed = original.clone();
    changed[11] = number(2);
    changed.splice(81..81, original[12..81].iter().cloned());
    assert!(matches!(
        restore(&changed),
        Err(RestoreError::SourceEvidence(SourceEvidenceError::CheckSet))
    ));
    // The second provenance object must not overwrite the first on decoding.
    let mut changed = original.clone();
    let provenance_count = 12 + 34 + 4 * 8 + 3;
    assert_eq!(
        u64::from_be_bytes(changed[provenance_count].clone().try_into().unwrap()),
        2
    );
    changed[provenance_count + 12] = b"execution-admission".to_vec();
    assert_eq!(restore(&changed), Err(RestoreError::NonCanonical));
    let mut changed = original.clone();
    let first = changed[provenance_count + 1..provenance_count + 9].to_vec();
    let second = changed[provenance_count + 9..provenance_count + 17].to_vec();
    changed[provenance_count + 1..provenance_count + 9].clone_from_slice(&second);
    changed[provenance_count + 9..provenance_count + 17].clone_from_slice(&first);
    assert_eq!(restore(&changed), Err(RestoreError::NonCanonical));
    let mut changed = original;
    changed.push(b"trailing".to_vec());
    assert_eq!(restore(&changed), Err(RestoreError::NonCanonical));
}

#[test]
fn envelope_check_order_must_be_canonical_and_distinct() {
    let admission = admission();
    let evidence = envelope_fixture(&admission);
    let first = evidence.checks()[&cid()].clone();
    let mut second = first.input().clone();
    second.check = CheckId::new("another").unwrap();
    let evidence = SourceEvidence::new(
        evidence.binding().clone(),
        5,
        25,
        vec![first, SourceCheckRecord::new(second).unwrap()],
        evidence.provenance().clone(),
        vec![],
    )
    .unwrap();
    let bytes = evidence.canonical_bytes();
    assert_eq!(
        restore_source_evidence(
            evidence.binding(),
            evidence.id(),
            &bytes,
            DomainDecodeLimits::default()
        )
        .unwrap(),
        evidence
    );
    let mut changed = fields(&bytes);
    assert_eq!(changed[15], b"another");
    assert_eq!(changed[84], b"verify");
    let first = changed[12..81].to_vec();
    let second = changed[81..150].to_vec();
    changed[12..81].clone_from_slice(&second);
    changed[81..150].clone_from_slice(&first);
    let bytes = pack(&changed);
    assert_eq!(
        restore_source_evidence(
            evidence.binding(),
            SourceEvidenceId(Digest::of(SOURCE_EVIDENCE_FORMAT, &bytes)),
            &bytes,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::NonCanonical)
    );
}

struct MissingObjects(AtomicUsize);
impl SourceArtifactReader for MissingObjects {
    fn read(&self, _: &Scope, _: Digest, _: u64) -> Result<Vec<u8>, SourceArtifactError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(SourceArtifactError::Unavailable(
            "fixture object absent".into(),
        ))
    }
    fn retention_policy(&self) -> Digest {
        d(9)
    }
}

#[test]
fn metadata_restore_never_establishes_artifact_durability_or_admission() {
    let admission = admission();
    let evidence = envelope_fixture(&admission);
    let bytes = evidence.canonical_bytes();
    let restored = restore_source_evidence(
        evidence.binding(),
        evidence.id(),
        &bytes,
        DomainDecodeLimits::default(),
    )
    .unwrap();
    let envelope = SourceArtifactReference::for_bytes(
        scope(),
        "sealed-evidence".into(),
        SOURCE_EVIDENCE_MEDIA_TYPE.into(),
        &bytes,
        d(9),
    );
    let reader = MissingObjects(AtomicUsize::new(0));
    assert_eq!(reader.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        restored.validate_persisted(
            &admission,
            evidence.binding().executor(),
            &envelope,
            &reader,
            &BTreeMap::new()
        ),
        Err(SourceEvidenceError::Artifact(
            SourceArtifactError::Unavailable("fixture object absent".into())
        ))
    );
    assert_eq!(reader.0.load(Ordering::SeqCst), 1);
    // Changing expected executor is rejected before any object I/O, even after
    // exact metadata decoding. A principal string does not restore a grant.
    assert_eq!(
        restored.validate_persisted(
            &admission,
            &PrincipalId::new("another-executor").unwrap(),
            &envelope,
            &reader,
            &BTreeMap::new()
        ),
        Err(SourceEvidenceError::BindingMismatch)
    );
    assert_eq!(reader.0.load(Ordering::SeqCst), 1);
}

#[test]
fn envelope_decoder_enforces_digest_format_and_aggregate_nested_budgets() {
    let admission = admission();
    let evidence = envelope_fixture(&admission);
    let bytes = evidence.canonical_bytes();
    assert_eq!(
        restore_source_evidence(
            evidence.binding(),
            SourceEvidenceId(d(99)),
            &bytes,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::DigestMismatch)
    );
    for end in 0..bytes.len() {
        let truncated = &bytes[..end];
        assert!(
            restore_source_evidence(
                evidence.binding(),
                SourceEvidenceId(Digest::of(SOURCE_EVIDENCE_FORMAT, truncated)),
                truncated,
                DomainDecodeLimits::default()
            )
            .is_err()
        );
    }
    // One check + four check artifacts + two check diagnostics + two provenance
    // objects + one envelope diagnostic share one collection-entry budget.
    let exact = DomainDecodeLimits {
        encoded_bytes: bytes.len(),
        collection_entries: 10,
        field_bytes: 128,
    };
    assert!(restore_source_evidence(evidence.binding(), evidence.id(), &bytes, exact).is_ok());
    for limited in [
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..exact
        },
        DomainDecodeLimits {
            collection_entries: 9,
            ..exact
        },
        DomainDecodeLimits {
            field_bytes: 31,
            ..exact
        },
    ] {
        assert_eq!(
            restore_source_evidence(evidence.binding(), evidence.id(), &bytes, limited),
            Err(RestoreError::Limit)
        );
    }
    let mut malicious = fields(&bytes);
    malicious[11] = number(u64::MAX);
    let malicious = pack(&malicious);
    assert_eq!(
        restore_source_evidence(
            evidence.binding(),
            SourceEvidenceId(Digest::of(SOURCE_EVIDENCE_FORMAT, &malicious)),
            &malicious,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::Limit)
    );
    let oversized = vec![0; 16 * 1024 * 1024 + 1];
    assert_eq!(
        restore_source_evidence(
            evidence.binding(),
            SourceEvidenceId(d(99)),
            &oversized,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::Limit)
    );
}
