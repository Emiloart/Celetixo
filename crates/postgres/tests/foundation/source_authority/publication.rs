//! Real SQL publication, retained filesystem objects and independently reopened
//! Engine embeddings. Synthetic checks do not establish sandbox execution.
use super::*;

#[path = "publication_checks.rs"]
mod checks;

fn ready(label: &str) -> (Harness, Session, SourceCommitRequest) {
    ready_with_ttl(label, 300_000)
}
fn ready_with_ttl(label: &str, ttl: u64) -> (Harness, Session, SourceCommitRequest) {
    let h = Harness::new(label);
    let (patch, admission, application) = h.ready_with_ttl("source-publication", ttl);
    let (evidence, envelope) = h.synthetic_evidence(&patch, &admission, &application, 0);
    h.engine
        .record_source_evidence(&h.executor, &admission, &h.storage, evidence, envelope)
        .unwrap();
    let analyst = h
        .admin
        .grant(
            PrincipalId::new("independent-publication-approver").unwrap(),
            h.scope().clone(),
            [Permission::ManageModel].into(),
        )
        .unwrap();
    let candidate = h
        .engine
        .source_publication_candidate(&h.owner, h.scope(), admission.id())
        .unwrap();
    let analysis = RepositoryAnalysis::new(
        candidate.snapshot(),
        RepositoryAnalysisInput {
            index_generation: d("publication-index"),
            analyzer_version: "complete-finite-publication/v1".into(),
            configuration: d("publication-config"),
            elements: [element()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element(), [check_id()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    let request = h
        .engine
        .prepare_source_commit_request(
            &h.owner,
            &analyst,
            h.scope(),
            SourceCommitPreparation {
                admission: admission.id(),
                analysis,
                key: IdempotencyKey::new("source-commit").unwrap(),
            },
        )
        .unwrap();
    (h, analyst, request)
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_publication_atomically_reopens_history_result_head_and_outbox() {
    let (h, analyst, request) = ready("source-published-reopen");
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let receipt = h
        .engine
        .commit_source_prepared(&h.owner, &analyst, h.scope(), request.clone())
        .unwrap();
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(after.records.history.len(), 1);
    assert_eq!(after.records.history[0].receipt, receipt);
    assert_eq!(
        after.records.base.repository.as_ref().unwrap().current,
        request.publication_snapshot
    );
    assert_eq!(after.records.analysis_head, Some(request.analysis.digest()));
    assert_eq!(
        after.records.base.mutations.len(),
        before.records.base.mutations.len() + 1
    );
    assert_eq!(
        after.records.base.transactions[&transaction_id()].state,
        TransactionState::Committed
    );
    assert!(
        after.records.base.transactions[&transaction_id()]
            .lease
            .is_none()
    );
    assert_eq!(after.records.policy_head, before.records.policy_head);
    let (reopened, admin) = Engine::with_authority_backend(Arc::new(super::super::authority()));
    let recovered_owner = admin
        .authenticate_existing_session(h.scope(), request.authorization_grant, |_, _| Ok(()))
        .unwrap();
    let analyst = admin
        .authenticate_existing_session(h.scope(), request.analysis_grant, |_, _| Ok(()))
        .unwrap();
    // No artifact reader is attached to the new embedding. Original success
    // recovery must return before any new artifact or lease operation.
    assert_eq!(
        reopened
            .commit_source_prepared(&recovered_owner, &analyst, h.scope(), request.clone())
            .unwrap(),
        receipt
    );
    assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), after);
    let a = [
        h.scope().organization.as_str(),
        h.scope().workspace.as_str(),
        h.scope().repository.as_str(),
    ];
    let mut sql = owner();
    for table in [
        "source_committed_changes",
        "source_commit_mutations",
        "transaction_outcomes",
        "outbox",
        "outbox_delivery",
    ] {
        assert_eq!(sql.query_one(&format!("SELECT count(*) FROM celetixo.{table} WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"),&[&a[0],&a[1],&a[2]]).unwrap().get::<_,i64>(0),1,"{table}");
    }
    for table in ["source_commit_mutations", "source_committed_changes"] {
        let flags=sql.query_one("SELECT relrowsecurity,relforcerowsecurity FROM pg_class WHERE oid=$1::text::regclass",&[&format!("celetixo.{table}")]).unwrap();
        assert!(flags.get::<_, bool>(0));
        assert!(flags.get::<_, bool>(1));
        for operation in [
            format!("UPDATE celetixo.{table} SET receipt_digest=receipt_digest"),
            format!("DELETE FROM celetixo.{table}"),
        ] {
            let mut tx = sql.transaction().unwrap();
            assert!(tx.execute(&format!("{operation} WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"),&[&a[0],&a[1],&a[2]]).is_err());
            tx.rollback().unwrap();
        }
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_publication_rolls_back_precommit_failure_and_recovers_lost_database_response() {
    for stage in [AuthorityStage::BeforeCommit, AuthorityStage::AfterCommit] {
        let (h, _, request) = ready(if stage == AuthorityStage::BeforeCommit {
            "source-commit-rollback"
        } else {
            "source-commit-lost-response"
        });
        let injected = Arc::new(super::super::authority().with_checkpoint(Arc::new(
            move |actual| {
                if actual == stage {
                    Err(AuthorityFailure::Unavailable)
                } else {
                    Ok(())
                }
            },
        )));
        let (engine, admin) = Engine::with_authority_backend(injected);
        let session = admin
            .authenticate_existing_session(h.scope(), request.authorization_grant, |_, _| Ok(()))
            .unwrap();
        let analyst = admin
            .authenticate_existing_session(h.scope(), request.analysis_grant, |_, _| Ok(()))
            .unwrap();
        // Resolve the original configured reader through the trusted embedding.
        // The test's original filesystem reader remains alive, so share that
        // exact installed adapter instead of opening a competing writer.
        admin
            .resolve_existing_source_artifact_store(
                h.scope(),
                request.store_registration,
                |_, _| Ok(h.artifacts.as_ref().unwrap().clone()),
            )
            .unwrap();
        let before = h.authority.inspect_source(h.scope()).unwrap();
        let result = engine.commit_source_prepared(&session, &analyst, h.scope(), request.clone());
        if stage == AuthorityStage::BeforeCommit {
            assert_eq!(
                result,
                Err(EngineError::Authority(AuthorityFailure::Unavailable))
            );
            assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), before);
        } else {
            assert_eq!(
                result,
                Err(EngineError::Authority(
                    AuthorityFailure::CommitIndeterminate
                ))
            );
            let after = h.authority.inspect_source(h.scope()).unwrap();
            assert_eq!(after.records.history.len(), 1);
            let receipt = h
                .engine
                .commit_source_prepared(
                    &h.owner,
                    &h.admin
                        .authenticate_existing_session(h.scope(), request.analysis_grant, |_, _| {
                            Ok(())
                        })
                        .unwrap(),
                    h.scope(),
                    request,
                )
                .unwrap();
            assert_eq!(receipt, after.records.history[0].receipt);
            assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), after);
        }
    }
}
