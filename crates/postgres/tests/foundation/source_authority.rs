//! Actual source-profile PostgreSQL checks. Synthetic definitions never execute
//! repository commands and do not establish object-store durability.
use super::*;
use celetixo_engine::engine::authority::*;
use celetixo_engine::engine::*;
use celetixo_engine::ids::*;
use celetixo_engine::source_evidence::{SourceArtifactError, SourceArtifactReader};
use celetixo_engine::source_execution::*;
#[path = "source_authority/guards.rs"]
#[cfg(target_os = "linux")]
mod guards;
#[path = "source_authority/isolation.rs"]
#[cfg(target_os = "linux")]
mod isolation;
#[path = "source_authority/lifecycle.rs"]
#[cfg(target_os = "linux")]
mod lifecycle;
#[path = "source_authority/migration.rs"]
mod migration;
#[path = "source_authority/recovery.rs"]
#[cfg(target_os = "linux")]
mod recovery;

#[cfg(target_os = "linux")]
pub(super) fn prepare_database_restart()
-> (Scope, celetixo_postgres::SourceRegistrationView, PathBuf) {
    let h = lifecycle::Harness::new("source-database-restart");
    let (patch, admission, application) = h.ready("source-attempt");
    let (evidence, envelope) = h.synthetic_evidence(&patch, &admission, &application, 0);
    h.engine
        .record_source_evidence(&h.executor, &admission, &h.storage, evidence, envelope)
        .unwrap();
    let reply = h
        .engine
        .mutate(
            &h.owner,
            h.scope(),
            IdempotencyKey::new("restart-renew").unwrap(),
            MutationRequest::Renew {
                transaction: admission.binding().transaction.clone(),
                ttl_ms: 300_000,
            },
        )
        .unwrap();
    assert!(reply.outcome.is_ok());
    (
        h.scope().clone(),
        h.authority.inspect_source(h.scope()).unwrap(),
        h.artifact_path.clone(),
    )
}

#[cfg(target_os = "linux")]
pub(super) fn assert_database_restart(
    scope: &Scope,
    before: &celetixo_postgres::SourceRegistrationView,
    path: &std::path::Path,
) {
    use celetixo_execution::artifacts::{ArtifactStoreLimits, LocalSourceArtifactStore};
    let authority = Arc::new(authority());
    assert_eq!(&authority.inspect_source(scope).unwrap(), before);
    let (engine, admin) = Engine::with_authority_backend(authority);
    let owner = admin
        .authenticate_existing_session(scope, 1, |_, _| Ok(()))
        .unwrap();
    assert_eq!(before.records.base.mutations.len(), 1);
    for (key, record) in &before.records.base.mutations {
        let actual = engine
            .mutate(&owner, scope, key.key.clone(), record.request.clone())
            .unwrap();
        assert!(actual.replayed);
        assert_eq!(actual.outcome, record.outcome);
        assert_eq!(actual.recorded_at_ms, record.recorded_at_ms);
    }
    let executor = admin
        .authenticate_existing_source_executor(scope, 1, |_, _| Ok(()))
        .unwrap();
    let reader = Arc::new(
        LocalSourceArtifactStore::open(
            path,
            scope.clone(),
            d("retention"),
            ArtifactStoreLimits::default(),
        )
        .unwrap(),
    );
    let configuration = Digest::of(
        "source-sql-test-artifact-configuration/v1",
        path.as_os_str().as_encoded_bytes(),
    );
    let storage = admin
        .resolve_existing_source_artifact_store(scope, 1, |s, record| {
            assert_eq!(s, scope);
            assert_eq!(
                record.adapter.as_ref().unwrap().configuration,
                configuration
            );
            Ok(reader.clone())
        })
        .unwrap();
    let attempt = before.records.attempts.values().next().unwrap();
    let metadata = &before.records.evidence[&attempt.evidence.unwrap()];
    let (evidence, receipt) = engine
        .source_evidence_record(&owner, scope, attempt.evidence.unwrap())
        .unwrap();
    assert_eq!(evidence, metadata.evidence);
    assert_eq!(receipt, metadata.receipt);
    assert_eq!(
        reader
            .read(scope, receipt.envelope.content, receipt.envelope.bytes)
            .unwrap(),
        evidence.canonical_bytes()
    );
    assert_eq!(
        engine
            .record_source_evidence(
                &executor,
                &attempt.admission,
                &storage,
                evidence,
                receipt.envelope.clone()
            )
            .unwrap(),
        receipt
    );
    assert_eq!(
        engine.current(&owner, scope).unwrap().revision(),
        Revision(0)
    );
}

fn d(text: &str) -> Digest {
    Digest::of("source-sql-fixture/v1", text.as_bytes())
}
fn resources() -> ResourceLimits {
    ResourceLimits {
        wall_time_ms: 1000,
        cpu_time_ms: 1000,
        memory_bytes: 1024 * 1024,
        disk_bytes: 1024 * 1024,
        process_count: 8,
        file_descriptors: 32,
        stdout_bytes: 4096,
        stderr_bytes: 4096,
        artifact_bytes: 1024 * 1024,
    }
}
fn backend() -> BackendIdentity {
    BackendIdentity {
        name: "synthetic-reference".into(),
        version: "v1".into(),
        image: d("image"),
        profile: d("backend"),
    }
}
fn now() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}
struct UnavailableReader;
impl SourceArtifactReader for UnavailableReader {
    fn read(&self, _: &Scope, _: Digest, _: u64) -> Result<Vec<u8>, SourceArtifactError> {
        Err(SourceArtifactError::Unavailable(
            "test has no objects".into(),
        ))
    }
    fn retention_policy(&self) -> Digest {
        d("retention")
    }
}
fn authorize_source(owner: &mut Client, selected: &Scope) {
    authorize_registration(owner, selected);
    let tables = "source_analyses,source_policy_catalog,source_check_definitions,source_policy_dependencies,source_attempt_contexts,source_attempt_policies,source_admission_contexts,source_operation_contexts,source_rejection_contexts,source_rejection_events,source_unprepared_commits,source_executor_definitions,source_executor_tiers,source_cleanup_definitions,artifact_contents,artifact_locations,artifact_references,source_evidence_objects";
    let tables = tables
        .split(',')
        .map(|t| format!("celetixo.{t}"))
        .collect::<Vec<_>>()
        .join(",");
    let config = fixture();
    for role in ["runtime", "runtime_other"] {
        let name = &config.roles[role].user;
        owner.batch_execute(&format!("GRANT SELECT ON {tables} TO {name}; GRANT INSERT ON celetixo.source_analyses,celetixo.source_policy_catalog,celetixo.source_check_definitions,celetixo.source_policy_dependencies,celetixo.source_attempt_contexts,celetixo.source_attempt_policies,celetixo.source_admission_contexts,celetixo.source_operation_contexts,celetixo.source_rejection_contexts,celetixo.source_rejection_events,celetixo.source_unprepared_commits,celetixo.artifact_contents,celetixo.artifact_locations,celetixo.artifact_references,celetixo.source_evidence_objects,celetixo.source_attempt_events,celetixo.source_attempt_outcomes,celetixo.source_rejections,celetixo.evidence_receipts,celetixo.policies TO {name}; GRANT INSERT,UPDATE ON celetixo.policy_heads,celetixo.source_attempts TO {name};")).unwrap();
        if role == "runtime_other" {
            owner.batch_execute(&format!("GRANT INSERT ON celetixo.source_executor_definitions,celetixo.source_executor_tiers,celetixo.source_cleanup_definitions,celetixo.artifact_store_registrations TO {name}; GRANT UPDATE(active,generation) ON celetixo.artifact_store_registrations TO {name};")).unwrap();
        }
    }
}
fn authority() -> PostgresAuthority {
    PostgresAuthority::connect_source(
        connection("runtime", None),
        connection("runtime_other", None),
    )
    .unwrap()
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_profile_pre_registration_grants_and_configured_store_reopen_without_runtime_resources() {
    let selected = scope("source-grants");
    authorize_source(&mut owner(), &selected);
    let authority_backend = Arc::new(authority());
    let (engine, admin) = Engine::with_authority_backend(authority_backend.clone());
    let session = admin
        .grant(
            PrincipalId::new("human").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    let executor = admin
        .grant_source_executor(
            PrincipalId::new("executor").unwrap(),
            selected.clone(),
            backend(),
            [ExecutionTier::T1].into(),
            resources(),
            now() + 120_000,
        )
        .unwrap();
    let cleanup = admin
        .grant_source_cleanup(
            PrincipalId::new("cleanup").unwrap(),
            selected.clone(),
            d("backend"),
            now() + 120_000,
        )
        .unwrap();
    let adapter = SourceArtifactAdapter {
        identity: SourceArtifactAdapterId::new("configured-test-reader").unwrap(),
        configuration: d("configured-reader-v1"),
    };
    let store = admin
        .register_configured_source_artifact_store(
            selected.clone(),
            adapter.clone(),
            Arc::new(UnavailableReader),
        )
        .unwrap();
    let before = authority_backend.inspect_source(&selected).unwrap();
    assert!(before.records.base.repository.is_none());
    assert_eq!(before.records.executors.len(), 1);
    assert_eq!(before.records.cleanup.len(), 1);
    assert_eq!(before.records.artifact_stores.len(), 1);
    assert!(matches!(
        sql_authority().inspect_native(&selected),
        Err(AuthorityFailure::UnsupportedProfile)
    ));
    let fresh_backend = Arc::new(authority());
    let (fresh, fresh_admin) = Engine::with_authority_backend(fresh_backend.clone());
    assert_eq!(fresh_backend.inspect_source(&selected).unwrap(), before);
    assert!(matches!(
        fresh.current(&session, &selected),
        Err(EngineError::Unauthorized)
    ));
    assert!(fresh_admin.revoke_source_executor(&executor).is_err());
    assert!(fresh_admin.revoke_source_cleanup(&cleanup).is_err());
    assert!(fresh_admin.revoke_source_artifact_store(&store).is_err());
    let fresh_session = fresh_admin
        .authenticate_existing_session(&selected, 1, |principal, actual_scope| {
            assert_eq!(principal, session.principal());
            assert_eq!(actual_scope, &selected);
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        fresh.current(&fresh_session, &selected),
        Err(EngineError::NotFound)
    ));
    let fresh_executor = fresh_admin
        .authenticate_existing_source_executor(&selected, 1, |actual, grant| {
            assert_eq!(actual, &selected);
            assert_eq!(grant.backend, backend());
            Ok(())
        })
        .unwrap();
    let fresh_cleanup = fresh_admin
        .authenticate_existing_source_cleanup(&selected, 1, |actual, grant| {
            assert_eq!(actual, &selected);
            assert_eq!(grant.backend, d("backend"));
            Ok(())
        })
        .unwrap();
    let fresh_store = fresh_admin
        .resolve_existing_source_artifact_store(&selected, 1, |actual, record| {
            assert_eq!(actual, &selected);
            assert_eq!(record.adapter.as_ref(), Some(&adapter));
            Ok(Arc::new(UnavailableReader))
        })
        .unwrap();
    fresh_admin.revoke_source_executor(&fresh_executor).unwrap();
    fresh_admin.revoke_source_cleanup(&fresh_cleanup).unwrap();
    fresh_admin
        .revoke_source_artifact_store(&fresh_store)
        .unwrap();
    let after = authority().inspect_source(&selected).unwrap();
    assert!(!after.records.executors[&1].active);
    assert!(!after.records.cleanup[&1].active);
    assert!(!after.records.artifact_stores[&1].active);
    assert_eq!(
        after.records.artifact_stores[&1].adapter.as_ref(),
        Some(&adapter)
    );
    drop((engine, admin));
}
