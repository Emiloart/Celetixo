use celetixo_engine::access::*;
use celetixo_engine::engine::{CommitRequest, Engine, EngineError, InMemoryStore, Permission};
use celetixo_engine::evidence::*;
use celetixo_engine::ids::*;
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::patch::{Change, Patch};
use celetixo_engine::persistence::*;
use celetixo_engine::snapshot::*;
use celetixo_engine::transaction::*;
use std::collections::{BTreeMap, BTreeSet};

fn d(value: &str) -> Digest {
    Digest::of("lifecycle-codec-test/v1", value.as_bytes())
}
fn element(value: &str) -> ElementId {
    ElementId::new(value).unwrap()
}
fn base() -> Snapshot {
    Snapshot::new(
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
            provenance: "lifecycle-codec-test/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        BTreeMap::from([(element("a"), 1), (element("b"), 2)]),
    )
    .unwrap()
}
fn accesses() -> AccessSet {
    AccessSet {
        reads: BTreeSet::from([element("a"), element("b")]),
        writes: BTreeSet::from([element("a")]),
        invariant_scopes: BTreeSet::from([element("b")]),
        predicates: BTreeSet::from([
            Predicate::Prefix("a".into()),
            Predicate::Prefix("future:".into()),
        ]),
    }
}
fn check() -> CheckSpec {
    CheckSpec::new(
        CheckId::new("verify").unwrap(),
        vec!["test-command".into(), "--flag".into()],
        "test-toolchain/v1".into(),
        d("environment"),
        d("configuration"),
    )
    .unwrap()
}
fn intent() -> TransactionIntent {
    TransactionIntent {
        objective: "reference intent with all native fields".into(),
        accesses: accesses(),
        invariants: vec![
            Invariant {
                id: InvariantId::new("equals").unwrap(),
                condition: Condition::Equals {
                    element: element("a"),
                    value: i64::MIN,
                },
            },
            Invariant {
                id: InvariantId::new("present").unwrap(),
                condition: Condition::Present(element("b")),
            },
            Invariant {
                id: InvariantId::new("sum").unwrap(),
                condition: Condition::SumAtMost {
                    elements: BTreeSet::from([element("a"), element("b")]),
                    maximum: i64::MAX,
                },
            },
        ],
        required_checks: vec![check()],
        depends_on: BTreeSet::from([
            TransactionId::new("previous:a").unwrap(),
            TransactionId::new("previous:b").unwrap(),
        ]),
    }
}
fn patch() -> Patch {
    Patch::new(
        base().id(),
        BTreeMap::from([
            (
                element("a"),
                Change {
                    expected: Some(1),
                    value: Some(i64::MAX),
                },
            ),
            (
                element("b"),
                Change {
                    expected: Some(2),
                    value: None,
                },
            ),
            (
                element("new"),
                Change {
                    expected: None,
                    value: Some(i64::MIN),
                },
            ),
        ]),
    )
    .unwrap()
}
fn evidence(result: CheckResult) -> VerificationEvidence {
    VerificationEvidence::new(
        EvidenceBindings {
            transaction: TransactionId::new("tx").unwrap(),
            attempt: u64::MAX,
            base_snapshot: base().id(),
            candidate_snapshot: SnapshotId(d("candidate")),
            patch: patch().id(),
            analysis: d("analysis"),
        },
        check(),
        result,
        ExecutionProvenance {
            verifier: PrincipalId::new("trusted-verifier").unwrap(),
            runner: "reference/v1".into(),
            run_id: "run".into(),
            started_at: 0,
            finished_at: u64::MAX,
        },
        vec!["first diagnostic".into(), "second diagnostic".into()],
        vec![
            Artifact::new(
                "stdout",
                "application/octet-stream",
                b"stdout\0\xff".to_vec(),
            )
            .unwrap(),
            Artifact::new(
                "diagnostics",
                "application/octet-stream",
                b"diagnostics\0\xff".to_vec(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}
fn fields(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let count = u64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize;
        result.push(bytes[offset + 8..offset + 8 + count].to_vec());
        offset += count + 8;
    }
    result
}
fn pack(fields: &[Vec<u8>]) -> Vec<u8> {
    fields
        .iter()
        .flat_map(|field| [(field.len() as u64).to_be_bytes().to_vec(), field.clone()].concat())
        .collect()
}

fn unhex(hex: &str) -> Vec<u8> {
    hex.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn request() -> CommitRequest {
    CommitRequest {
        transaction: TransactionId::new("tx").unwrap(),
        attempt: u64::MAX,
        authorization_grant: u64::MAX,
        fence: u64::MAX,
        base_snapshot: base().id(),
        candidate_snapshot: SnapshotId(d("candidate")),
        patch: patch().id(),
        analysis: d("analysis"),
        intent: NativeIntentRecord::new(&base(), intent()).unwrap().digest(),
        evidence: BTreeMap::from([(CheckId::new("verify").unwrap(), EvidenceId(d("evidence")))]),
        key: IdempotencyKey::new("outside-semantic-identity").unwrap(),
    }
}

#[test]
fn independently_encoded_frozen_layouts_survive_checked_reconstruction() {
    let native = NativeIntentRecord::new(&base(), intent()).unwrap();
    let impact = ImpactAnalysisRecord::new(
        &base(),
        ImpactAnalysis::new(
            AnalysisStamp {
                snapshot: base().id(),
                index_generation: u64::MAX,
                analyzer: "reference".into(),
                version: "v1".into(),
                configuration: d("configuration"),
            },
            accesses(),
            Completeness::Incomplete {
                reasons: vec!["unresolved".into(), "unsupported".into()],
            },
        ),
    )
    .unwrap();
    let evidence = evidence(CheckResult::Passed { exit_code: 0 });
    let verification = VerificationEvidenceRecord::new(&base(), evidence.clone()).unwrap();
    let request = request();
    let principal = PrincipalId::new("worker").unwrap();
    let native_bytes = unhex(include_str!("fixtures/native-intent-record-v1.hex"));
    let impact_bytes = unhex(include_str!("fixtures/impact-analysis-record-v1.hex"));
    let evidence_bytes = unhex(include_str!("fixtures/verification-evidence-record-v1.hex"));
    let request_bytes = unhex(include_str!("fixtures/commit-request-v1.hex"));
    assert_eq!(native.canonical_bytes(), native_bytes);
    assert_eq!(impact.canonical_bytes(), impact_bytes);
    assert_eq!(verification.canonical_bytes(), evidence_bytes);
    assert_eq!(
        request.canonical_bytes(base().scope(), &principal),
        request_bytes
    );
    let limits = DomainDecodeLimits::default();
    assert_eq!(
        restore_native_intent(&base(), native.digest(), &native_bytes, limits).unwrap(),
        native
    );
    assert_eq!(
        restore_impact_record(&base(), u64::MAX, impact.digest(), &impact_bytes, limits).unwrap(),
        impact
    );
    assert_eq!(
        restore_verification_record(
            &base(),
            evidence.id(),
            verification.digest(),
            &evidence_bytes,
            limits
        )
        .unwrap(),
        verification
    );
    assert_eq!(
        restore_commit_request(
            base().scope(),
            &principal,
            request.key.clone(),
            request.digest(base().scope(), &principal),
            &request_bytes,
            limits
        )
        .unwrap(),
        request
    );
}

#[test]
fn commit_request_reconstruction_checks_context_order_tags_and_budgets() {
    let request = request();
    let scope = base().scope().clone();
    let principal = PrincipalId::new("worker").unwrap();
    let bytes = request.canonical_bytes(&scope, &principal);
    let decode = |bytes: &[u8], limits| {
        restore_commit_request(
            &scope,
            &principal,
            request.key.clone(),
            Digest::of(CommitRequest::FORMAT, bytes),
            bytes,
            limits,
        )
    };
    for end in 0..bytes.len() {
        assert!(decode(&bytes[..end], DomainDecodeLimits::default()).is_err());
    }
    let mut wrong_scope = scope.clone();
    wrong_scope.repository = RepositoryId::new("wrong").unwrap();
    for (scope, principal) in [
        (&wrong_scope, &principal),
        (&scope, &PrincipalId::new("wrong").unwrap()),
    ] {
        assert_eq!(
            restore_commit_request(
                scope,
                principal,
                request.key.clone(),
                request.digest(
                    &base().scope().clone(),
                    &PrincipalId::new("worker").unwrap()
                ),
                &bytes,
                DomainDecodeLimits::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
    }
    for limits in [
        DomainDecodeLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(decode(&bytes, limits), Err(RestoreError::Limit));
    }
    // Scope(3), principal, transaction, attempt, authorization grant, fence.
    for field in [6, 7] {
        let mut modified = fields(&bytes);
        modified[field] = 0u64.to_be_bytes().to_vec();
        assert_eq!(
            decode(&pack(&modified), DomainDecodeLimits::default()),
            Err(RestoreError::InvalidDomainValue)
        );
    }
    let mut duplicate = fields(&bytes);
    duplicate[13] = 2u64.to_be_bytes().to_vec();
    let duplicate_entry = [duplicate[14].clone(), duplicate[15].clone()];
    duplicate.extend(duplicate_entry);
    assert_eq!(
        decode(&pack(&duplicate), DomainDecodeLimits::default()),
        Err(RestoreError::NonCanonical)
    );
    let mut reordered = fields(&bytes);
    reordered[13] = 2u64.to_be_bytes().to_vec();
    reordered.extend([b"a-before-verify".to_vec(), d("second").as_bytes().to_vec()]);
    assert_eq!(
        decode(&pack(&reordered), DomainDecodeLimits::default()),
        Err(RestoreError::NonCanonical)
    );
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        decode(&trailing, DomainDecodeLimits::default()),
        Err(RestoreError::NonCanonical)
    );
}

#[test]
fn access_and_reference_patch_preserve_v1_identity_and_full_value_range() {
    let access = accesses();
    assert_eq!(
        restore_access_set(
            access.digest(),
            &access.canonical_bytes(),
            DomainDecodeLimits::default()
        )
        .unwrap(),
        access
    );
    let patch = patch();
    let restored = restore_reference_patch(
        &base(),
        patch.id(),
        &patch.canonical_bytes(),
        DomainDecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(restored, patch);
    assert_eq!(
        restored.apply(base().state()).unwrap(),
        BTreeMap::from([(element("a"), i64::MAX), (element("new"), i64::MIN)])
    );
    let wrong = base()
        .successor(Revision(1), base().state().clone())
        .unwrap();
    assert_eq!(
        restore_reference_patch(
            &wrong,
            patch.id(),
            &patch.canonical_bytes(),
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::BindingMismatch)
    );
}

#[test]
fn native_intent_roundtrip_preserves_every_native_field_and_constructor_rules() {
    let record = NativeIntentRecord::new(&base(), intent()).unwrap();
    assert_eq!(
        restore_native_intent(
            &base(),
            record.digest(),
            &record.canonical_bytes(),
            DomainDecodeLimits::default()
        )
        .unwrap(),
        record
    );
    let mut duplicate = intent();
    duplicate.required_checks.push(check());
    assert_eq!(
        NativeIntentRecord::new(&base(), duplicate),
        Err(RestoreError::InvalidDomainValue)
    );
    let mut duplicate = intent();
    duplicate.invariants.push(duplicate.invariants[0].clone());
    assert_eq!(
        NativeIntentRecord::new(&base(), duplicate),
        Err(RestoreError::InvalidDomainValue)
    );
    let mut empty = intent();
    empty.objective = " ".into();
    assert_eq!(
        NativeIntentRecord::new(&base(), empty),
        Err(RestoreError::InvalidDomainValue)
    );
}

#[test]
fn impact_records_retain_complete_and_incomplete_states_and_require_external_generation() {
    for completeness in [
        Completeness::Complete,
        Completeness::Incomplete { reasons: vec![] },
        Completeness::Incomplete {
            reasons: vec!["unresolved".into(), "unsupported".into()],
        },
    ] {
        let impact = ImpactAnalysis::new(
            AnalysisStamp {
                snapshot: base().id(),
                index_generation: u64::MAX,
                analyzer: "reference".into(),
                version: "v1".into(),
                configuration: d("configuration"),
            },
            accesses(),
            completeness,
        );
        let record = ImpactAnalysisRecord::new(&base(), impact.clone()).unwrap();
        let restored = restore_impact_record(
            &base(),
            u64::MAX,
            record.digest(),
            &record.canonical_bytes(),
            DomainDecodeLimits::default(),
        )
        .unwrap();
        assert_eq!(restored.analysis(), &impact);
        assert_eq!(restored.analysis().digest(), impact.digest());
        assert_eq!(
            restore_impact_record(
                &base(),
                1,
                record.digest(),
                &record.canonical_bytes(),
                DomainDecodeLimits::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
    }
}

#[test]
fn evidence_envelopes_retain_original_evidence_identity_and_all_binary_artifacts() {
    for result in [
        CheckResult::Passed { exit_code: 0 },
        CheckResult::Failed { exit_code: -1 },
        CheckResult::Failed {
            exit_code: i32::MAX,
        },
        CheckResult::Incomplete {
            reason: "missing capture".into(),
        },
    ] {
        let evidence = evidence(result);
        let record = VerificationEvidenceRecord::new(&base(), evidence.clone()).unwrap();
        let restored = restore_verification_record(
            &base(),
            evidence.id(),
            record.digest(),
            &record.canonical_bytes(),
            DomainDecodeLimits::default(),
        )
        .unwrap();
        assert_eq!(restored.evidence(), &evidence);
        assert_ne!(record.digest(), evidence.id().0);
        assert_eq!(
            restore_verification_record(
                &base(),
                EvidenceId(d("forged")),
                record.digest(),
                &record.canonical_bytes(),
                DomainDecodeLimits::default()
            ),
            Err(RestoreError::BindingMismatch)
        );
    }
}

#[test]
fn every_truncated_prefix_rehashed_malformed_and_resource_amplification_input_rejects() {
    let intent = NativeIntentRecord::new(&base(), intent()).unwrap();
    let evidence = evidence(CheckResult::Passed { exit_code: 0 });
    let evidence_record = VerificationEvidenceRecord::new(&base(), evidence.clone()).unwrap();
    for (domain, bytes) in [
        (NativeIntentRecord::FORMAT, intent.canonical_bytes()),
        (
            VerificationEvidenceRecord::FORMAT,
            evidence_record.canonical_bytes(),
        ),
    ] {
        for end in 0..bytes.len() {
            let truncated = &bytes[..end];
            let expected = Digest::of(domain, truncated);
            let result = if domain == NativeIntentRecord::FORMAT {
                restore_native_intent(&base(), expected, truncated, DomainDecodeLimits::default())
                    .map(|_| ())
            } else {
                restore_verification_record(
                    &base(),
                    evidence.id(),
                    expected,
                    truncated,
                    DomainDecodeLimits::default(),
                )
                .map(|_| ())
            };
            assert!(result.is_err(), "accepted prefix {end} in {domain}");
        }
        let mut malformed = bytes.clone();
        malformed[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        let expected = Digest::of(domain, &malformed);
        let result = if domain == NativeIntentRecord::FORMAT {
            restore_native_intent(&base(), expected, &malformed, DomainDecodeLimits::default())
                .map(|_| ())
        } else {
            restore_verification_record(
                &base(),
                evidence.id(),
                expected,
                &malformed,
                DomainDecodeLimits::default(),
            )
            .map(|_| ())
        };
        assert_eq!(result, Err(RestoreError::Limit));
    }
    for limits in [
        DomainDecodeLimits {
            field_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            encoded_bytes: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            collection_entries: 1,
            ..Default::default()
        },
    ] {
        assert_eq!(
            restore_native_intent(&base(), intent.digest(), &intent.canonical_bytes(), limits),
            Err(RestoreError::Limit)
        );
        assert_eq!(
            restore_verification_record(
                &base(),
                evidence.id(),
                evidence_record.digest(),
                &evidence_record.canonical_bytes(),
                limits
            ),
            Err(RestoreError::Limit)
        );
    }
}

#[test]
fn noncanonical_access_patch_and_evidence_order_cannot_be_hidden_by_rehashing() {
    let mut access = fields(&accesses().canonical_bytes());
    access.swap(1, 2);
    let modified = pack(&access);
    assert_eq!(
        restore_access_set(
            Digest::of("celetixo/access-set/v1", &modified),
            &modified,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::NonCanonical)
    );
    let mut patch = fields(&patch().canonical_bytes());
    // Three change records have different optional-value sizes; exchange entire
    // records while preserving valid field types and a correct content address.
    let a = patch[2..7].to_vec();
    let b = patch[7..11].to_vec();
    patch.splice(2..11, b.into_iter().chain(a));
    let modified = pack(&patch);
    assert_eq!(
        restore_reference_patch(
            &base(),
            PatchId(Digest::of("celetixo/patch/v1", &modified)),
            &modified,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::NonCanonical)
    );
    let evidence = evidence(CheckResult::Passed { exit_code: 0 });
    let record = VerificationEvidenceRecord::new(&base(), evidence.clone()).unwrap();
    let mut encoded = fields(&record.canonical_bytes());
    let count = encoded.len();
    let first = encoded[count - 6..count - 3].to_vec();
    let second = encoded[count - 3..].to_vec();
    encoded.splice(count - 6..count, second.into_iter().chain(first));
    let modified = pack(&encoded);
    assert_eq!(
        restore_verification_record(
            &base(),
            evidence.id(),
            Digest::of(VerificationEvidenceRecord::FORMAT, &modified),
            &modified,
            DomainDecodeLimits::default()
        ),
        Err(RestoreError::NonCanonical)
    );
}

#[test]
fn restored_evidence_data_does_not_replace_fresh_scoped_verifier_authority() {
    let base = base();
    let scope = base.scope();
    let (store, admin) = InMemoryStore::new();
    let engine = Engine::new(store);
    let worker = admin
        .grant(
            PrincipalId::new("worker").unwrap(),
            scope.clone(),
            Permission::ALL
                .into_iter()
                .filter(|p| *p != Permission::Verify)
                .collect(),
        )
        .unwrap();
    let verifier = admin
        .grant(
            PrincipalId::new("trusted-verifier").unwrap(),
            scope.clone(),
            BTreeSet::from([Permission::Verify]),
        )
        .unwrap();
    engine
        .register(
            &worker,
            base.clone(),
            EngineeringModel {
                generation: 1,
                analyzer: "finite-reference".into(),
                version: "v1".into(),
                configuration: d("model"),
                dependencies: BTreeMap::new(),
                completeness: Completeness::Complete,
            },
        )
        .unwrap();
    let id = TransactionId::new("verified-reference").unwrap();
    let intent = TransactionIntent {
        objective: "restore facts, then independently authorize".into(),
        accesses: AccessSet {
            writes: BTreeSet::from([element("a")]),
            ..Default::default()
        },
        invariants: vec![],
        required_checks: vec![check()],
        depends_on: BTreeSet::new(),
    };
    let record = NativeIntentRecord::new(&base, intent).unwrap();
    let restored = restore_native_intent(
        &base,
        record.digest(),
        &record.canonical_bytes(),
        DomainDecodeLimits::default(),
    )
    .unwrap();
    engine
        .propose(
            &worker,
            scope,
            id.clone(),
            base.id(),
            restored.intent().clone(),
        )
        .unwrap();
    engine.analyze(&worker, scope, &id).unwrap();
    engine.reserve(&worker, scope, &id, 100).unwrap();
    engine.execute(&worker, scope, &id).unwrap();
    let patch = Patch::new(
        base.id(),
        BTreeMap::from([(
            element("a"),
            Change {
                expected: Some(1),
                value: Some(3),
            },
        )]),
    )
    .unwrap();
    let restored = restore_reference_patch(
        &base,
        patch.id(),
        &patch.canonical_bytes(),
        DomainDecodeLimits::default(),
    )
    .unwrap();
    engine.submit_patch(&worker, scope, &id, restored).unwrap();
    let evidence = VerificationEvidence::new(
        engine.bindings(&worker, scope, &id).unwrap(),
        check(),
        CheckResult::Passed { exit_code: 0 },
        ExecutionProvenance {
            verifier: verifier.principal().clone(),
            runner: "fixture".into(),
            run_id: "run".into(),
            started_at: 0,
            finished_at: 0,
        },
        vec![],
        vec![Artifact::new("stdout", "text/plain", b"ok".to_vec()).unwrap()],
    )
    .unwrap();
    let record = VerificationEvidenceRecord::new(&base, evidence.clone()).unwrap();
    let restored = restore_verification_record(
        &base,
        evidence.id(),
        record.digest(),
        &record.canonical_bytes(),
        DomainDecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(
        engine.record_evidence(&worker, scope, &id, restored.evidence().clone()),
        Err(EngineError::Unauthorized)
    );
    engine
        .record_evidence(&verifier, scope, &id, restored.evidence().clone())
        .unwrap();
    admin.revoke(&verifier).unwrap();
    assert_eq!(
        engine.commit(
            &worker,
            scope,
            &id,
            IdempotencyKey::new("rejected-revoked-evidence").unwrap()
        ),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(engine.current(&worker, scope).unwrap(), base);
}
