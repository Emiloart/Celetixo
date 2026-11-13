//! Normalized-record recovery tests. This backend is an atomic fault probe,
//! not evidence of PostgreSQL durability or a live execution adapter.
use celetixo_engine::source_evidence::{SourceArtifactError, SourceArtifactReader};
use celetixo_engine::{
    engine::{authority::*, *},
    ids::*,
    source_execution::*,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("admin-recovery").unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    }
}
fn principal() -> PrincipalId {
    PrincipalId::new("worker").unwrap()
}
fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::new(s).unwrap()
}
fn d(s: &str) -> Digest {
    Digest::of("admin-recovery/v1", s.as_bytes())
}
fn backend_id() -> BackendIdentity {
    BackendIdentity {
        name: "metadata-only".into(),
        version: "v1".into(),
        image: d("image"),
        profile: d("profile"),
    }
}
fn resources() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 1000,
        memory_bytes: 4096,
        disk_bytes: 8192,
        process_count: 4,
        file_descriptors: 32,
        stdout_bytes: 1024,
        stderr_bytes: 1024,
        artifact_bytes: 4096,
    }
}
fn allocations() -> Vec<(&'static str, AdministrativeMutation)> {
    vec![
        (
            "session",
            AdministrativeMutation::GrantSession {
                principal: principal(),
                permissions: Permission::ALL.into(),
            },
        ),
        (
            "executor",
            AdministrativeMutation::GrantExecutor {
                principal: principal(),
                backend: backend_id(),
                tiers: [ExecutionTier::T1].into(),
                resources: resources(),
                expires_at_ms: 10_000,
            },
        ),
        (
            "cleanup",
            AdministrativeMutation::GrantCleanup {
                principal: principal(),
                backend: d("profile"),
                expires_at_ms: 10_000,
            },
        ),
        (
            "store",
            AdministrativeMutation::RegisterArtifactStore {
                adapter: SourceArtifactAdapter {
                    identity: SourceArtifactAdapterId::new("configured-reader").unwrap(),
                    configuration: d("configuration"),
                },
                retention: d("retention"),
            },
        ),
    ]
}
fn revocations() -> Vec<(&'static str, AdministrativeMutation)> {
    vec![
        (
            "revoke-session",
            AdministrativeMutation::RevokeSession {
                grant: 1,
                principal: principal(),
            },
        ),
        (
            "revoke-executor",
            AdministrativeMutation::RevokeExecutor {
                grant: 1,
                principal: principal(),
                backend: backend_id(),
            },
        ),
        (
            "revoke-cleanup",
            AdministrativeMutation::RevokeCleanup {
                grant: 1,
                principal: principal(),
                backend: d("profile"),
            },
        ),
        (
            "revoke-store",
            AdministrativeMutation::RevokeArtifactStore { registration: 1 },
        ),
    ]
}
struct Backend {
    records: Mutex<SourceRecords>,
    fault: AtomicU8,
}
impl Backend {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            records: Mutex::new(SourceRecords::empty(scope())),
            fault: AtomicU8::new(0),
        })
    }
}
impl AuthorityBackend for Backend {
    fn access(
        &self,
        access: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure> {
        let update = matches!(
            access.mode(),
            AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
        );
        let fault = self.fault.load(Ordering::SeqCst);
        if update && fault == 1 {
            return Err(AuthorityFailure::Unavailable);
        }
        let mut records = self.records.lock().unwrap();
        if records.base.scope != *access.scope() {
            return Err(AuthorityFailure::ScopeDenied);
        }
        let mut state = AuthorityState::from_source_records(records.clone(), Default::default())
            .map_err(|_| AuthorityFailure::Integrity)?;
        evaluate(&mut state)?;
        if update {
            if fault == 2 {
                return Err(AuthorityFailure::Unavailable);
            }
            *records = state
                .source_records(Default::default())
                .map_err(|_| AuthorityFailure::Integrity)?;
            if fault == 3 {
                return Err(AuthorityFailure::CommitIndeterminate);
            }
        }
        Ok(())
    }
}

#[test]
fn all_administrative_original_results_survive_revocation_and_a_fresh_embedding() {
    let backend = Backend::new();
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let missing = revocations()[1].1.clone();
    let rejected = admin
        .mutate(&scope(), key("missing"), missing.clone())
        .unwrap();
    assert_eq!(rejected.outcome, Err(EngineError::Unauthorized));
    let mut originals = vec![("missing", missing, rejected)];
    for (name, request) in allocations() {
        let result = admin.mutate(&scope(), key(name), request.clone()).unwrap();
        assert!(matches!(
            result.outcome,
            Ok(MutationValue::Allocation { id: 1, .. })
        ));
        originals.push((name, request, result));
    }
    let worker = admin
        .authenticate_existing_session(&scope(), 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.mutate(
            &worker,
            &scope(),
            key("self-grant"),
            MutationRequest::Administrative(Box::new(allocations()[0].1.clone()))
        ),
        Err(EngineError::Unauthorized)
    );
    assert!(
        engine
            .mutation_outcome(&worker, &scope(), &key("self-grant"))
            .unwrap()
            .is_none()
    );
    let old = admin
        .mutate(&scope(), key("missing"), originals[0].1.clone())
        .unwrap();
    assert!(old.replayed);
    assert_eq!(old.outcome, Err(EngineError::Unauthorized));
    assert!(backend.records.lock().unwrap().executors[&1].active);
    for (name, request) in revocations() {
        let result = admin.mutate(&scope(), key(name), request.clone()).unwrap();
        assert!(matches!(
            result.outcome,
            Ok(MutationValue::Revocation { id: 1, .. })
        ));
        originals.push((name, request, result));
    }
    let before = backend.records.lock().unwrap().clone();
    let (_, admin) = Engine::with_authority_backend(backend.clone());
    for (name, request, original) in originals {
        let replay = admin.mutate(&scope(), key(name), request).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, original.outcome);
        assert_eq!(replay.recorded_at_ms, original.recorded_at_ms);
        assert_eq!(
            admin
                .mutation_outcome(&scope(), &key(name))
                .unwrap()
                .unwrap()
                .outcome,
            original.outcome
        );
    }
    assert_eq!(*backend.records.lock().unwrap(), before);
    assert_eq!(before.base.session_counter, 1);
    assert_eq!(before.executor_counter, 1);
    assert_eq!(before.cleanup_counter, 1);
    assert_eq!(before.artifact_counter, 1);
    assert!(!before.base.grants[&1].active);
    assert!(!before.executors[&1].active);
    assert!(!before.cleanup[&1].active);
    assert!(!before.artifact_stores[&1].active);
    assert_eq!(
        admin
            .authenticate_existing_session(&scope(), 1, |_, _| Ok(()))
            .err(),
        Some(EngineError::Unauthorized)
    );
    assert!(before.base.repository.is_none());
    assert_eq!(before.base.mutations.len(), 9);
    for record in before.base.mutations.values() {
        assert!(record.request.transaction().is_none());
        assert!(record.resolved_attempt.is_none());
        assert!(record.administration.is_some());
    }
}

#[test]
fn allocation_faults_roll_back_or_reconcile_one_original_id_for_every_resource() {
    for (name, request) in allocations() {
        let backend = Backend::new();
        let (_, admin) = Engine::with_authority_backend(backend.clone());
        let before = backend.records.lock().unwrap().clone();
        for fault in [1, 2] {
            backend.fault.store(fault, Ordering::SeqCst);
            assert_eq!(
                admin.mutate(&scope(), key(name), request.clone()),
                Err(EngineError::Authority(AuthorityFailure::Unavailable))
            );
            assert_eq!(*backend.records.lock().unwrap(), before);
        }
        backend.fault.store(3, Ordering::SeqCst);
        assert_eq!(
            admin.mutate(&scope(), key(name), request.clone()),
            Err(EngineError::Authority(
                AuthorityFailure::CommitIndeterminate
            ))
        );
        backend.fault.store(0, Ordering::SeqCst);
        let before = backend.records.lock().unwrap().clone();
        let (_, fresh) = Engine::with_authority_backend(backend.clone());
        let result = fresh.mutate(&scope(), key(name), request).unwrap();
        assert!(result.replayed);
        assert_eq!(
            result.outcome,
            Ok(MutationValue::Allocation {
                resource: allocations()
                    .into_iter()
                    .find(|(n, _)| *n == name)
                    .unwrap()
                    .1
                    .resource(),
                id: 1
            })
        );
        assert_eq!(*backend.records.lock().unwrap(), before);
    }
}

#[test]
fn administrator_and_session_keys_are_distinct_and_changed_admin_inputs_cannot_reuse_a_key() {
    let backend = Backend::new();
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let request = allocations()[0].1.clone();
    let original = admin
        .mutate(&scope(), key("shared"), request.clone())
        .unwrap();
    let worker = admin
        .authenticate_existing_session(&scope(), 1, |_, _| Ok(()))
        .unwrap();
    let result = engine
        .mutate(
            &worker,
            &scope(),
            key("shared"),
            MutationRequest::Analyze {
                transaction: TransactionId::new("missing").unwrap(),
            },
        )
        .unwrap();
    assert_eq!(result.outcome, Err(EngineError::NotFound));
    assert_eq!(
        admin
            .mutate(&scope(), key("shared"), request)
            .unwrap()
            .outcome,
        original.outcome
    );
    assert_eq!(
        engine
            .mutation_outcome(&worker, &scope(), &key("shared"))
            .unwrap()
            .unwrap()
            .outcome,
        result.outcome
    );
    assert_eq!(
        admin.mutate(
            &scope(),
            key("shared"),
            AdministrativeMutation::GrantSession {
                principal: principal(),
                permissions: BTreeSet::new()
            }
        ),
        Err(EngineError::IdempotencyConflict)
    );
    assert_eq!(backend.records.lock().unwrap().base.mutations.len(), 2);
    let mut foreign = scope();
    foreign.repository = RepositoryId::new("foreign").unwrap();
    assert_eq!(
        admin.mutate(&foreign, key("shared"), allocations()[0].1.clone()),
        Err(EngineError::Authority(AuthorityFailure::ScopeDenied))
    );
}

#[test]
fn invalid_original_options_and_corrupt_administrative_context_fail_closed() {
    let backend = Backend::new();
    let (_, admin) = Engine::with_authority_backend(backend.clone());
    let bad = AdministrativeMutation::GrantExecutor {
        principal: principal(),
        backend: backend_id(),
        tiers: [ExecutionTier::T0].into(),
        resources: resources(),
        expires_at_ms: 100,
    };
    let rejected = admin.mutate(&scope(), key("invalid"), bad.clone()).unwrap();
    assert_eq!(
        rejected.outcome,
        Err(EngineError::SourceAttempt(
            SourceAttemptError::UntrustedExecutor
        ))
    );
    let expired = AdministrativeMutation::GrantCleanup {
        principal: principal(),
        backend: d("profile"),
        expires_at_ms: 0,
    };
    assert_eq!(
        admin
            .mutate(&scope(), key("expired"), expired)
            .unwrap()
            .outcome,
        Err(EngineError::InvalidTime)
    );
    admin
        .mutate(&scope(), key("grant"), allocations()[0].1.clone())
        .unwrap();
    let records = backend.records.lock().unwrap().clone();
    for variant in 0..4 {
        let mut corrupt = records.clone();
        let (key, record) = corrupt
            .base
            .mutations
            .iter_mut()
            .find(|(_, r)| matches!(r.outcome, Ok(MutationValue::Allocation { .. })))
            .unwrap();
        assert_eq!(key.actor, MutationActor::EmbeddingAdministrator);
        match variant {
            0 => record.administration = None,
            1 => record.administration.as_mut().unwrap().counter_before = 99,
            2 => {
                record.outcome = Ok(MutationValue::Allocation {
                    resource: AdministrativeResource::Executor,
                    id: 1,
                })
            }
            _ => {
                record.outcome = Ok(MutationValue::Allocation {
                    resource: AdministrativeResource::Session,
                    id: 2,
                })
            }
        }
        assert!(
            AuthorityState::from_source_records(corrupt, Default::default()).is_err(),
            "accepted corruption {variant}"
        );
    }
    let (_, fresh) = Engine::with_authority_backend(backend.clone());
    assert_eq!(
        fresh.mutate(&scope(), key("invalid"), bad).unwrap().outcome,
        rejected.outcome
    );
}

#[test]
fn historical_grants_cannot_restore_expired_authority_or_reattach_a_revoked_reader() {
    struct Reader;
    impl SourceArtifactReader for Reader {
        fn read(&self, _: &Scope, _: Digest, _: u64) -> Result<Vec<u8>, SourceArtifactError> {
            Err(SourceArtifactError::Unavailable(
                "metadata fixture has no artifacts".into(),
            ))
        }
        fn retention_policy(&self) -> Digest {
            d("retention")
        }
    }
    let backend = Backend::new();
    let (_, admin) = Engine::with_authority_backend(backend.clone());
    let originals: Vec<_> = allocations()
        .into_iter()
        .map(|(name, request)| {
            let reply = admin.mutate(&scope(), key(name), request.clone()).unwrap();
            (name, request, reply)
        })
        .collect();
    let reader = Arc::new(Reader);
    let _capability = admin
        .resolve_existing_source_artifact_store(&scope(), 1, |_, _| Ok(reader.clone()))
        .unwrap();
    assert_eq!(Arc::strong_count(&reader), 2);
    admin
        .mutate(
            &scope(),
            key("revoke-store"),
            AdministrativeMutation::RevokeArtifactStore { registration: 1 },
        )
        .unwrap();
    assert_eq!(Arc::strong_count(&reader), 1);
    backend.records.lock().unwrap().base.clock_ms = 20_000;
    let (_, fresh) = Engine::with_authority_backend(backend.clone());
    for (name, request, expected) in originals {
        let replay = fresh.mutate(&scope(), key(name), request).unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, expected.outcome);
    }
    assert_eq!(
        fresh
            .authenticate_existing_source_executor(&scope(), 1, |_, _| Ok(()))
            .err(),
        Some(EngineError::Unauthorized)
    );
    assert_eq!(
        fresh
            .authenticate_existing_source_cleanup(&scope(), 1, |_, _| Ok(()))
            .err(),
        Some(EngineError::Unauthorized)
    );
    assert_eq!(
        fresh
            .resolve_existing_source_artifact_store(&scope(), 1, |_, _| Ok(reader.clone()))
            .err(),
        Some(EngineError::Unauthorized)
    );
    assert_eq!(Arc::strong_count(&reader), 1);
    assert_eq!(backend.records.lock().unwrap().artifact_counter, 1);
}
