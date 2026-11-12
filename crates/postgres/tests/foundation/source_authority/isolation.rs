use super::*;
use lifecycle::Harness;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_same_embedding_reused_grant_and_store_numbers_remain_scoped() {
    let a = Harness::new("source-scope-a");
    let b = Harness::new("source-scope-b");
    let (_, admission_a, _) = a.ready("same-attempt-id");
    let (patch_b, admission_b, application_b) = b.ready("same-attempt-id");
    let (evidence_b, envelope_b) = b.synthetic_evidence(&patch_b, &admission_b, &application_b, 0);
    for h in [&a, &b] {
        h.admin
            .grant_source_cleanup(
                PrincipalId::new("same-cleanup-principal").unwrap(),
                h.scope().clone(),
                d("backend"),
                now() + 120_000,
            )
            .unwrap();
    }
    // One StoreIdentity deliberately holds both scopes, all with token 1.
    // A rejection cannot be attributed merely to an old embedding identity.
    let (engine, admin) = Engine::with_authority_backend(Arc::new(authority()));
    let executor_a = admin
        .authenticate_existing_source_executor(a.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let executor_b = admin
        .authenticate_existing_source_executor(b.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let cleanup_a = admin
        .authenticate_existing_source_cleanup(a.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let cleanup_b = admin
        .authenticate_existing_source_cleanup(b.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let store_a = admin
        .resolve_existing_source_artifact_store(a.scope(), 1, |_, _| {
            Ok(a.artifacts.as_ref().unwrap().clone())
        })
        .unwrap();
    let store_b = admin
        .resolve_existing_source_artifact_store(b.scope(), 1, |_, _| {
            Ok(b.artifacts.as_ref().unwrap().clone())
        })
        .unwrap();
    assert_eq!(
        engine.validate_source_attempt(&executor_a, &admission_b),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::UntrustedExecutor
        ))
    );
    assert_eq!(
        engine.validate_source_cleanup(&cleanup_a, b.scope(), d("backend")),
        Err(EngineError::Unauthorized)
    );
    assert_eq!(
        engine.record_source_evidence(
            &executor_b,
            &admission_b,
            &store_a,
            evidence_b.clone(),
            envelope_b.clone(),
        ),
        Err(EngineError::Unauthorized)
    );
    admin.revoke_source_executor(&executor_a).unwrap();
    admin.revoke_source_cleanup(&cleanup_a).unwrap();
    admin.revoke_source_artifact_store(&store_a).unwrap();
    assert_eq!(
        engine.validate_source_attempt(&executor_a, &admission_a),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::UntrustedExecutor
        ))
    );
    engine
        .validate_source_cleanup(&cleanup_b, b.scope(), d("backend"))
        .unwrap();
    engine
        .record_source_evidence(&executor_b, &admission_b, &store_b, evidence_b, envelope_b)
        .unwrap();
    let after_a = a.authority.inspect_source(a.scope()).unwrap();
    let after_b = b.authority.inspect_source(b.scope()).unwrap();
    assert!(!after_a.records.executors[&1].active);
    assert!(!after_a.records.cleanup[&1].active);
    assert!(!after_a.records.artifact_stores[&1].active);
    assert!(after_a.records.evidence.is_empty());
    assert!(after_b.records.executors[&1].active);
    assert!(after_b.records.cleanup[&1].active);
    assert!(after_b.records.artifact_stores[&1].active);
    assert_eq!(after_b.records.evidence.len(), 1);
    // Database-enforced scope also holds for object locations: a B object
    // cannot be referenced through A's numerically identical registration.
    let mut owner = owner();
    let a_scope = [
        a.scope().organization.as_str(),
        a.scope().workspace.as_str(),
        a.scope().repository.as_str(),
    ];
    let b_scope = [
        b.scope().organization.as_str(),
        b.scope().workspace.as_str(),
        b.scope().repository.as_str(),
    ];
    assert_eq!(owner.query_one("SELECT count(*) FROM celetixo.artifact_contents WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a_scope[0],&a_scope[1],&a_scope[2]]).unwrap().get::<_,i64>(0),0);
    let object: Vec<u8> = owner.query_one("SELECT object_digest::bytea FROM celetixo.artifact_contents WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text ORDER BY object_digest LIMIT 1", &[&b_scope[0],&b_scope[1],&b_scope[2]]).unwrap().get(0);
    let mut transaction = owner.transaction().unwrap();
    let error = transaction.execute("INSERT INTO celetixo.artifact_locations (organization_id,workspace_id,repository_id,registration_id,object_digest,locator_kind) VALUES ($1::text,$2::text,$3::text,1,$4::bytea,'ContentAddressed')", &[&a_scope[0],&a_scope[1],&a_scope[2],&object]).unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23503");
    transaction.rollback().unwrap();
}

struct HookReader {
    inner: Arc<dyn SourceArtifactReader>,
    fired: AtomicBool,
    on_first: Box<dyn Fn() + Send + Sync>,
}
impl SourceArtifactReader for HookReader {
    fn read(
        &self,
        scope: &Scope,
        content: Digest,
        bytes: u64,
    ) -> Result<Vec<u8>, SourceArtifactError> {
        if !self.fired.swap(true, Ordering::SeqCst) {
            (self.on_first)();
        }
        self.inner.read(scope, content, bytes)
    }
    fn retention_policy(&self) -> Digest {
        self.inner.retention_policy()
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_revocation_during_artifact_io_prevents_receipt_publication() {
    let h = Harness::new("source-io-revocation");
    let (patch, admission, application) = h.ready("source-attempt");
    let (evidence, envelope) = h.synthetic_evidence(&patch, &admission, &application, 0);
    let (_, revoker) = Engine::with_authority_backend(Arc::new(authority()));
    let revocation_capability = revoker
        .resolve_existing_source_artifact_store(h.scope(), 1, |_, _| {
            Ok(h.artifacts.as_ref().unwrap().clone())
        })
        .unwrap();
    let reader = Arc::new(HookReader {
        inner: h.artifacts.as_ref().unwrap().clone(),
        fired: AtomicBool::new(false),
        on_first: Box::new(move || {
            revoker
                .revoke_source_artifact_store(&revocation_capability)
                .unwrap();
        }),
    });
    let (engine, admin) = Engine::with_authority_backend(Arc::new(authority()));
    let executor = admin
        .authenticate_existing_source_executor(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    let storage = admin
        .resolve_existing_source_artifact_store(h.scope(), 1, |_, _| Ok(reader.clone()))
        .unwrap();
    assert_eq!(
        engine.record_source_evidence(&executor, &admission, &storage, evidence, envelope),
        Err(EngineError::Unauthorized)
    );
    assert!(reader.fired.load(Ordering::SeqCst));
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert!(after.records.evidence.is_empty());
    assert!(!after.records.artifact_stores[&1].active);
    assert_eq!(
        after.records.attempts[&admission.binding().source_attempt].state,
        SourceAttemptState::Verifying
    );
    assert_eq!(
        engine.current(&h.owner, h.scope()),
        Err(EngineError::Unauthorized)
    );
}
