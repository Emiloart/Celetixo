//! Exact admin requests against the real restricted PostgreSQL source profile.
use super::*;
use celetixo_postgres::AuthorityStage;
use std::sync::atomic::{AtomicBool, Ordering};

fn expiry() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
        + 600_000
}
fn backend_id() -> BackendIdentity {
    BackendIdentity {
        name: "reference-not-executed".into(),
        version: "v1".into(),
        image: d("image"),
        profile: d("profile"),
    }
}
fn resources() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 500,
        memory_bytes: 4096,
        disk_bytes: 8192,
        process_count: 4,
        file_descriptors: 32,
        stdout_bytes: 1024,
        stderr_bytes: 1024,
        artifact_bytes: 4096,
    }
}
fn principal() -> PrincipalId {
    PrincipalId::new("admin-worker").unwrap()
}
fn assert_replay_unchanged(mut before: SourceRecords, after: SourceRecords) {
    // A replay observes and publishes current database time, while retaining
    // every original decision, allocation and resource state.
    assert!(after.base.clock_ms >= before.base.clock_ms);
    before.base.clock_ms = after.base.clock_ms;
    assert_eq!(before, after);
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
                expires_at_ms: expiry(),
            },
        ),
        (
            "cleanup",
            AdministrativeMutation::GrantCleanup {
                principal: principal(),
                backend: d("profile"),
                expires_at_ms: expiry(),
            },
        ),
        (
            "store",
            AdministrativeMutation::RegisterArtifactStore {
                adapter: SourceArtifactAdapter {
                    identity: SourceArtifactAdapterId::new("configured-reference").unwrap(),
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
fn faulting(stage: AuthorityStage) -> (Administrator, Arc<AtomicBool>) {
    let armed = Arc::new(AtomicBool::new(false));
    let active = armed.clone();
    let backend = Arc::new(authority().with_checkpoint(Arc::new(move |point| {
        if active.load(Ordering::SeqCst) && point == stage {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (_, admin) = Engine::with_authority_backend(backend);
    (admin, armed)
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn administrative_original_results_survive_revocation_and_fresh_sql_instances_without_a_repository()
{
    let selected = scope("admin-original-replies");
    authorize_source(&mut owner(), &selected);
    let backend = Arc::new(authority());
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let missing = revocations()[1].1.clone();
    let denied = admin
        .mutate(&selected, key("missing"), missing.clone())
        .unwrap();
    assert_eq!(denied.outcome, Err(EngineError::Unauthorized));
    let mut originals = vec![("missing", missing, denied)];
    for (operation, request) in allocations() {
        let reply = admin
            .mutate(&selected, key(operation), request.clone())
            .unwrap();
        assert!(matches!(
            reply.outcome,
            Ok(MutationValue::Allocation { id: 1, .. })
        ));
        let mut changed = request.clone();
        match &mut changed {
            AdministrativeMutation::GrantSession { principal, .. }
            | AdministrativeMutation::GrantExecutor { principal, .. }
            | AdministrativeMutation::GrantCleanup { principal, .. } => {
                *principal = PrincipalId::new("substituted-principal").unwrap();
            }
            AdministrativeMutation::RegisterArtifactStore { retention, .. } => {
                *retention = d("substituted-retention");
            }
            _ => unreachable!(),
        }
        assert_eq!(
            admin.mutate(&selected, key(operation), changed),
            Err(EngineError::IdempotencyConflict)
        );
        originals.push((operation, request, reply));
    }
    let worker = admin
        .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        engine.mutate(
            &worker,
            &selected,
            key("self-grant"),
            MutationRequest::Administrative(Box::new(originals[1].1.clone()))
        ),
        Err(EngineError::Unauthorized)
    );
    let missing_replay = admin
        .mutate(&selected, key("missing"), originals[0].1.clone())
        .unwrap();
    assert!(missing_replay.replayed);
    assert_eq!(missing_replay.outcome, Err(EngineError::Unauthorized));
    assert!(backend.inspect_source(&selected).unwrap().records.executors[&1].active);
    for (operation, request) in revocations() {
        let reply = admin
            .mutate(&selected, key(operation), request.clone())
            .unwrap();
        assert!(matches!(
            reply.outcome,
            Ok(MutationValue::Revocation { id: 1, .. })
        ));
        originals.push((operation, request, reply));
    }
    let before = backend.inspect_source(&selected).unwrap();
    let (_, fresh) = Engine::with_authority_backend(Arc::new(authority()));
    for (operation, request, expected) in originals {
        let reply = fresh.mutate(&selected, key(operation), request).unwrap();
        assert!(reply.replayed);
        assert_eq!(reply.outcome, expected.outcome);
        assert_eq!(reply.recorded_at_ms, expected.recorded_at_ms);
        assert_eq!(
            fresh
                .mutation_outcome(&selected, &key(operation))
                .unwrap()
                .unwrap()
                .outcome,
            expected.outcome
        );
    }
    let after = backend.inspect_source(&selected).unwrap();
    assert_replay_unchanged(before.records, after.records.clone());
    assert!(!after.records.base.grants[&1].active);
    assert!(!after.records.executors[&1].active);
    assert!(!after.records.cleanup[&1].active);
    assert!(!after.records.artifact_stores[&1].active);
    assert!(after.records.base.repository.is_none());
    assert_eq!(after.records.base.mutations.len(), 9);
    assert_eq!(
        fresh
            .authenticate_existing_session(&selected, 1, |_, _| Ok(()))
            .err(),
        Some(EngineError::Unauthorized)
    );
    assert_eq!(
        fresh
            .authenticate_existing_source_executor(&selected, 1, |_, _| Ok(()))
            .err(),
        Some(EngineError::Unauthorized)
    );
    assert_eq!(
        fresh
            .authenticate_existing_source_cleanup(&selected, 1, |_, _| Ok(()))
            .err(),
        Some(EngineError::Unauthorized)
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn administrative_sql_faults_preserve_one_allocation_and_original_revocation_per_resource() {
    let selected = scope("admin-fault-recovery");
    authorize_source(&mut owner(), &selected);
    let backend = Arc::new(authority());
    let (_, admin) = Engine::with_authority_backend(backend.clone());
    let requests = allocations().into_iter().chain(revocations());
    for (operation, request) in requests {
        for stage in [AuthorityStage::StateLoaded, AuthorityStage::BeforeCommit] {
            let (fault, armed) = faulting(stage);
            armed.store(true, Ordering::SeqCst);
            let before = backend.inspect_source(&selected);
            assert_eq!(
                fault.mutate(&selected, key(operation), request.clone()),
                Err(EngineError::Authority(AuthorityFailure::Unavailable))
            );
            assert_eq!(backend.inspect_source(&selected), before);
        }
        let (fault, armed) = faulting(AuthorityStage::AfterCommit);
        armed.store(true, Ordering::SeqCst);
        assert_eq!(
            fault.mutate(&selected, key(operation), request.clone()),
            Err(EngineError::Authority(
                AuthorityFailure::CommitIndeterminate
            ))
        );
        let before = backend.inspect_source(&selected).unwrap();
        let reply = admin.mutate(&selected, key(operation), request).unwrap();
        assert!(reply.replayed);
        assert!(reply.outcome.is_ok(), "{reply:?}");
        assert_replay_unchanged(
            before.records,
            backend.inspect_source(&selected).unwrap().records,
        );
    }
    let records = backend.inspect_source(&selected).unwrap().records;
    assert_eq!(records.base.session_counter, 1);
    assert_eq!(records.executor_counter, 1);
    assert_eq!(records.cleanup_counter, 1);
    assert_eq!(records.artifact_counter, 1);
    assert_eq!(records.base.mutations.len(), 8);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn runtime_database_login_cannot_forge_an_administrative_journal_actor() {
    let selected = scope("admin-actor-row-boundary");
    authorize_source(&mut owner(), &selected);
    let (engine, admin) = Engine::with_authority_backend(Arc::new(authority()));
    let request = AdministrativeMutation::RevokeSession {
        grant: 999,
        principal: principal(),
    };
    assert_eq!(
        admin
            .mutate(&selected, key("original"), request)
            .unwrap()
            .outcome,
        Err(EngineError::Unauthorized)
    );
    // Copy every insertable column in catalog order; derived generated columns
    // must be recomputed by PostgreSQL. Only the key changes. This
    // specifically probes the authenticated role boundary, not payload parsing.
    let columns=owner().query("SELECT column_name FROM information_schema.columns WHERE table_schema='celetixo' AND table_name='engine_mutations' AND is_generated='NEVER' ORDER BY ordinal_position",&[]).unwrap().into_iter().map(|row|row.get::<_,String>(0)).collect::<Vec<_>>();
    let select = columns
        .iter()
        .map(|column| {
            if column == "idempotency_key" {
                "'forged'::text"
            } else {
                column.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut runtime = connection("runtime", None).connect().unwrap();
    let error=runtime.execute(&format!("INSERT INTO celetixo.engine_mutations ({}) SELECT {select} FROM celetixo.engine_mutations WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",columns.join(",")),&[&selected.organization.as_str(),&selected.workspace.as_str(),&selected.repository.as_str()]).unwrap_err();
    assert_eq!(error.code().unwrap().code(), "42501");
    let worker = admin
        .grant(principal(), selected.clone(), Permission::ALL.into())
        .unwrap();
    let mut migration = owner();
    let config = fixture();
    let runtime_name = &config.roles["runtime"].user;
    migration
        .batch_execute(&format!(
            "GRANT INSERT ON celetixo.artifact_store_registrations TO {runtime_name}"
        ))
        .unwrap();
    // Privileges are checked afresh on each operation, including changes made
    // after the backend and opaque session were created.
    let refused = engine.current(&worker, &selected);
    migration
        .batch_execute(&format!(
            "REVOKE INSERT ON celetixo.artifact_store_registrations FROM {runtime_name}"
        ))
        .unwrap();
    assert_eq!(
        refused,
        Err(EngineError::Authority(AuthorityFailure::ScopeDenied))
    );
    assert_eq!(
        engine.current(&worker, &selected),
        Err(EngineError::NotFound)
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn administrative_allocation_races_reconcile_one_original_id_per_resource() {
    let selected = scope("admin-allocation-races");
    authorize_source(&mut owner(), &selected);
    for (operation, request) in allocations() {
        let barrier = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let selected = selected.clone();
                let request = request.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let (_, admin) = Engine::with_authority_backend(Arc::new(authority()));
                    barrier.wait();
                    admin.mutate(&selected, key(operation), request).unwrap()
                })
            })
            .collect::<Vec<_>>();
        let replies = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_ne!(replies[0].replayed, replies[1].replayed);
        assert_eq!(replies[0].outcome, replies[1].outcome);
        assert_eq!(replies[0].recorded_at_ms, replies[1].recorded_at_ms);
        assert!(matches!(
            replies[0].outcome,
            Ok(MutationValue::Allocation { id: 1, .. })
        ));
    }
    let records = authority().inspect_source(&selected).unwrap().records;
    assert_eq!(records.base.session_counter, 1);
    assert_eq!(records.executor_counter, 1);
    assert_eq!(records.cleanup_counter, 1);
    assert_eq!(records.artifact_counter, 1);
    assert_eq!(records.base.mutations.len(), 4);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn grants_expiring_before_sql_publication_roll_back_allocation_and_key() {
    for cleanup in [false, true] {
        let selected = scope(if cleanup {
            "cleanup-final-expiry"
        } else {
            "executor-final-expiry"
        });
        authorize_source(&mut owner(), &selected);
        let (_, admin) = Engine::with_authority_backend(Arc::new(authority()));
        // Bootstrap without consuming the executor or cleanup allocator.
        admin
            .mutate(
                &selected,
                key("bootstrap"),
                AdministrativeMutation::RevokeSession {
                    grant: 999,
                    principal: principal(),
                },
            )
            .unwrap();
        let before = authority().inspect_source(&selected).unwrap();
        let expires_at_ms = now() + 5_000;
        let request = if cleanup {
            AdministrativeMutation::GrantCleanup {
                principal: principal(),
                backend: d("profile"),
                expires_at_ms,
            }
        } else {
            AdministrativeMutation::GrantExecutor {
                principal: principal(),
                backend: backend_id(),
                tiers: [ExecutionTier::T1].into(),
                resources: resources(),
                expires_at_ms,
            }
        };
        let backend = Arc::new(authority().with_checkpoint(Arc::new(move |stage| {
            if stage == AuthorityStage::BeforeCommit {
                std::thread::sleep(std::time::Duration::from_millis(
                    expires_at_ms.saturating_sub(now()) + 20,
                ));
            }
            Ok(())
        })));
        let (_, delayed) = Engine::with_authority_backend(backend);
        assert_eq!(
            delayed.mutate(&selected, key("allocate"), request.clone()),
            Err(EngineError::Authority(AuthorityFailure::Contention))
        );
        assert_eq!(authority().inspect_source(&selected).unwrap(), before);
        let retry = admin.mutate(&selected, key("allocate"), request).unwrap();
        assert!(!retry.replayed);
        assert_eq!(
            retry.outcome,
            Err(if cleanup {
                EngineError::InvalidTime
            } else {
                EngineError::SourceAttempt(SourceAttemptError::UntrustedExecutor)
            })
        );
        let records = authority().inspect_source(&selected).unwrap().records;
        assert_eq!(records.executor_counter, 0);
        assert_eq!(records.cleanup_counter, 0);
        assert!(records.executors.is_empty());
        assert!(records.cleanup.is_empty());
        assert_eq!(records.base.mutations.len(), 2);
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn administrative_sql_constraints_reject_partial_or_unbound_resource_results() {
    let selected = scope("admin-result-constraints");
    authorize_source(&mut owner(), &selected);
    let (_, admin) = Engine::with_authority_backend(Arc::new(authority()));
    for (operation, request) in allocations() {
        assert!(
            admin
                .mutate(&selected, key(operation), request)
                .unwrap()
                .outcome
                .is_ok()
        );
    }
    let before = authority().inspect_source(&selected).unwrap();
    let mut client = owner();
    let columns = client.query("SELECT column_name FROM information_schema.columns WHERE table_schema='celetixo' AND table_name='engine_mutations' AND is_generated='NEVER' ORDER BY ordinal_position", &[])
        .unwrap().into_iter().map(|row| row.get::<_, String>(0)).collect::<Vec<_>>();
    for (operation, replacements, code) in [
        (
            "session",
            vec![("actor_principal_id", "'forged-principal'::text")],
            "23514",
        ),
        ("session", vec![("admin_counter_before", "NULL")], "23514"),
        (
            "session",
            vec![("admin_target_active_before", "FALSE")],
            "23514",
        ),
        ("session", vec![("result_grant_kind", "NULL")], "23514"),
        ("session", vec![("result_resource_id", "NULL")], "23514"),
        ("session", vec![("result_grant_id", "NULL")], "23514"),
        (
            "store",
            vec![("result_store_registration_id", "NULL")],
            "23514",
        ),
        ("store", vec![("result_resource_kind", "NULL")], "23514"),
        (
            "store",
            vec![("result_grant_kind", "'session'::text")],
            "23514",
        ),
        (
            "session",
            vec![
                ("admin_counter_before", "98"),
                ("result_resource_id", "99"),
                ("result_grant_id", "99"),
            ],
            "23503",
        ),
        (
            "store",
            vec![
                ("admin_counter_before", "98"),
                ("result_resource_id", "99"),
                ("result_store_registration_id", "99"),
            ],
            "23503",
        ),
    ] {
        let select = columns
            .iter()
            .map(|column| {
                if column == "idempotency_key" {
                    "'constraint-probe'::text"
                } else {
                    replacements
                        .iter()
                        .find(|(name, _)| *name == column.as_str())
                        .map_or(column.as_str(), |(_, value)| *value)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "INSERT INTO celetixo.engine_mutations ({}) SELECT {select} FROM celetixo.engine_mutations WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND idempotency_key=$4::text",
            columns.join(",")
        );
        let error = client
            .execute(
                &sql,
                &[
                    &selected.organization.as_str(),
                    &selected.workspace.as_str(),
                    &selected.repository.as_str(),
                    &operation,
                ],
            )
            .unwrap_err();
        assert_eq!(
            error.code().unwrap().code(),
            code,
            "{replacements:?}: {error}"
        );
    }
    assert_eq!(authority().inspect_source(&selected).unwrap(), before);
}
