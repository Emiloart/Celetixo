//! Independent admin layouts retain raw rejected options without capabilities.
use super::*;
use celetixo_engine::source_execution::{BackendIdentity, ExecutionTier, ResourceLimits};

fn admin_key() -> MutationKey {
    MutationKey {
        actor: MutationActor::EmbeddingAdministrator,
        key: IdempotencyKey::new("operation").unwrap(),
    }
}
fn principal() -> PrincipalId {
    PrincipalId::new("p").unwrap()
}
fn backend() -> BackendIdentity {
    BackendIdentity {
        name: "reference".into(),
        version: "v1".into(),
        image: Digest::of("test/v1", b"image"),
        profile: Digest::of("test/v1", b"profile"),
    }
}
fn resources() -> ResourceLimits {
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
fn records() -> Vec<MutationRecord> {
    use AdministrativeMutation as A;
    let requests = vec![
        A::GrantSession {
            principal: principal(),
            permissions: [Permission::Read, Permission::Verify].into(),
        },
        A::RevokeSession {
            grant: 4,
            principal: principal(),
        },
        A::GrantExecutor {
            principal: principal(),
            backend: backend(),
            tiers: [ExecutionTier::T1, ExecutionTier::T2].into(),
            resources: resources(),
            expires_at_ms: 100,
        },
        A::RevokeExecutor {
            grant: 4,
            principal: principal(),
            backend: backend(),
        },
        A::GrantCleanup {
            principal: principal(),
            backend: backend().profile,
            expires_at_ms: 100,
        },
        A::RevokeCleanup {
            grant: 4,
            principal: principal(),
            backend: backend().profile,
        },
        A::RegisterArtifactStore {
            adapter: SourceArtifactAdapter {
                identity: SourceArtifactAdapterId::new("configured").unwrap(),
                configuration: backend().image,
            },
            retention: backend().profile,
        },
        A::RevokeArtifactStore { registration: 4 },
    ];
    requests
        .into_iter()
        .enumerate()
        .map(|(index, request)| {
            let revoke = index % 2 == 1;
            let resource = request.resource();
            MutationRecord {
                request: MutationRequest::Administrative(Box::new(request)),
                recorded_at_ms: 9,
                resolved_attempt: None,
                state_before: None,
                lease_events_before: 0,
                source_execution: None,
                source_commit: None,
                source_revalidation: None,
                administration: Some(AdministrativeContext {
                    counter_before: if revoke { 4 } else { 3 },
                    target_active_before: if revoke { Some(true) } else { None },
                }),
                outcome: Ok(if revoke {
                    MutationValue::Revocation { resource, id: 4 }
                } else {
                    MutationValue::Allocation { resource, id: 4 }
                }),
            }
        })
        .collect()
}

#[test]
fn independent_administrator_header_has_no_fabricated_principal_grant_or_transaction() {
    let request = records().remove(0).request;
    let mut bytes = Vec::new();
    for text in ["o", "w", "r", "EmbeddingAdministrator", "GrantSession", "p"] {
        field(&mut bytes, text.as_bytes());
    }
    number(&mut bytes, 2);
    field(&mut bytes, b"Read");
    field(&mut bytes, b"Verify");
    assert_eq!(
        request.canonical_bytes(&scope(), &MutationActor::EmbeddingAdministrator),
        bytes
    );
    let mut actor = Vec::new();
    field(&mut actor, b"EmbeddingAdministrator");
    assert_eq!(
        MutationActor::EmbeddingAdministrator.digest(),
        Digest::of("celetixo/engine-mutation-actor/v1", &actor)
    );
    assert_ne!(
        MutationActor::EmbeddingAdministrator.digest(),
        super::actor().digest()
    );
}

#[test]
fn all_admin_requests_and_original_contexts_roundtrip_with_bounded_typed_outcomes() {
    for record in records() {
        let key = admin_key();
        let input = record.request.canonical_bytes(&scope(), &key.actor);
        let request = &record.request;
        assert!(request.transaction().is_none());
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &key.actor,
                request.digest(&scope(), &key.actor),
                &input,
                Default::default()
            )
            .unwrap(),
            *request
        );
        for end in 0..input.len() {
            let prefix = &input[..end];
            assert!(
                restore_mutation_request(
                    &scope(),
                    &key.actor,
                    Digest::of(MutationRequest::FORMAT, prefix),
                    prefix,
                    Default::default()
                )
                .is_err()
            );
        }
        assert!(
            restore_mutation_request(
                &scope(),
                &super::actor(),
                request.digest(&scope(), &key.actor),
                &input,
                Default::default()
            )
            .is_err()
        );
        let value = MutationOutcomeRecord::new(scope(), key.clone(), record.clone()).unwrap();
        let bytes = value.canonical_bytes();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key,
                request,
                value.digest(),
                &bytes,
                Default::default()
            )
            .unwrap(),
            value
        );
        assert!(value.recovery_size().unwrap().bytes >= input.len() + bytes.len());
        for end in 0..bytes.len() {
            let prefix = &bytes[..end];
            assert!(
                restore_mutation_outcome(
                    &scope(),
                    &key,
                    request,
                    Digest::of(MutationOutcomeRecord::FORMAT, prefix),
                    prefix,
                    Default::default()
                )
                .is_err()
            );
        }
        for limits in [
            DomainDecodeLimits {
                collection_entries: 0,
                ..Default::default()
            },
            DomainDecodeLimits {
                field_bytes: 1,
                ..Default::default()
            },
            DomainDecodeLimits {
                encoded_bytes: input.len() - 1,
                ..Default::default()
            },
        ] {
            assert!(
                restore_mutation_request(
                    &scope(),
                    &key.actor,
                    request.digest(&scope(), &key.actor),
                    &input,
                    limits
                )
                .is_err()
            );
        }
        let mut invalid = record.clone();
        invalid.administration = None;
        assert!(MutationOutcomeRecord::new(scope(), key.clone(), invalid).is_err());
        let mut invalid = record;
        invalid.resolved_attempt = Some(0);
        invalid.state_before = Some(TransactionState::Proposed);
        assert!(MutationOutcomeRecord::new(scope(), key, invalid).is_err());
    }
}

#[test]
fn rejected_original_grant_options_keep_invalid_tiers_resources_and_unresolved_selectors() {
    let mut samples = Vec::new();
    for (request, error) in [
        (
            AdministrativeMutation::GrantExecutor {
                principal: principal(),
                backend: backend(),
                tiers: [ExecutionTier::T0].into(),
                resources: resources(),
                expires_at_ms: 100,
            },
            EngineError::SourceAttempt(SourceAttemptError::UntrustedExecutor),
        ),
        (
            AdministrativeMutation::GrantCleanup {
                principal: principal(),
                backend: backend().profile,
                expires_at_ms: 0,
            },
            EngineError::InvalidTime,
        ),
        (
            AdministrativeMutation::RevokeSession {
                grant: 0,
                principal: principal(),
            },
            EngineError::Unauthorized,
        ),
        (
            AdministrativeMutation::RevokeExecutor {
                grant: 999,
                principal: principal(),
                backend: backend(),
            },
            EngineError::Unauthorized,
        ),
    ] {
        samples.push(MutationRecord {
            request: MutationRequest::Administrative(Box::new(request)),
            recorded_at_ms: 9,
            resolved_attempt: None,
            state_before: None,
            lease_events_before: 0,
            source_execution: None,
            source_commit: None,
            source_revalidation: None,
            administration: Some(AdministrativeContext {
                counter_before: 0,
                target_active_before: None,
            }),
            outcome: Err(error),
        });
    }
    let mut record = records().remove(2);
    let MutationRequest::Administrative(request) = &mut record.request else {
        unreachable!()
    };
    let AdministrativeMutation::GrantExecutor { resources, .. } = request.as_mut() else {
        unreachable!()
    };
    resources.memory_bytes = 0;
    record.outcome = Err(EngineError::SourceAttempt(SourceAttemptError::Plan(
        resources.validate().unwrap_err(),
    )));
    samples.push(record);
    for record in samples {
        let key = admin_key();
        let request = record.request.clone();
        let input = request.canonical_bytes(&scope(), &key.actor);
        assert_eq!(
            restore_mutation_request(
                &scope(),
                &key.actor,
                request.digest(&scope(), &key.actor),
                &input,
                Default::default()
            )
            .unwrap(),
            request
        );
        let value = MutationOutcomeRecord::new(scope(), key.clone(), record).unwrap();
        assert_eq!(
            restore_mutation_outcome(
                &scope(),
                &key,
                &request,
                value.digest(),
                &value.canonical_bytes(),
                Default::default()
            )
            .unwrap(),
            value
        );
    }
}
