//! Real PostgreSQL atomic epoch replacement and independently authenticated
//! process recovery. Check execution remains a synthetic fixture here.
use super::*;
use celetixo_engine::engine::authority::AuthorityState;
use celetixo_engine::persistence::restore_source_revalidation_request;
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;

fn key(name: &str) -> IdempotencyKey {
    IdempotencyKey::new(name).unwrap()
}

// Lock acquisition may legitimately time out while another Engine publishes.
// Retry only that transient storage outcome, under a finite deadline and count.
// Every operation retry retains the exact original request and idempotency key.
fn retry_contention<T>(
    mut operation: impl FnMut() -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    use std::time::{Duration, Instant};
    let started = Instant::now();
    let mut retries = 0;
    loop {
        match operation() {
            Err(EngineError::Authority(AuthorityFailure::Contention))
                if retries < 4 && started.elapsed() < Duration::from_secs(30) =>
            {
                retries += 1;
                std::thread::sleep(Duration::from_millis(25 << retries));
            }
            result => return result,
        }
    }
}
fn fresh_policy(h: &mut Harness) {
    let records = h.authority.inspect_source(h.scope()).unwrap().records;
    let facts = &records.analyses[&records.analysis_head.unwrap()];
    let mut policy = h.policy.input().clone();
    policy.generation += 1;
    policy.snapshot = h.base.id();
    policy.analysis_artifact = facts.digest();
    policy.index_generation = facts.index_generation();
    policy.capture.snapshot = h.base.id();
    policy.capture.manifest = h.base.manifest().digest();
    if h.base.revision().0 > 0 {
        policy.capture.consistency = SourceCaptureConsistency::EnginePublication;
        policy.capture.capture_policy = records.history.last().unwrap().receipt.digest();
    }
    let files = celetixo_engine::snapshot::effective_workspace(&h.base).unwrap();
    for (path, rule) in &mut policy.paths {
        rule.expected_base = Some(Preimage::of(&files[path]).unwrap());
    }
    let next = SourceExecutionPolicy::new(policy).unwrap();
    h.engine
        .install_source_policy(&h.owner, h.scope(), Some(h.policy.digest()), next.clone())
        .unwrap();
    h.policy = next;
}
fn ready(label: &str, ttl: u64) -> (Harness, SourceAdmission, SourceRevalidationRequest) {
    let mut h = Harness::new(label);
    let (_, old, _) = h.ready_with_ttl("epoch-source-0", ttl);
    fresh_policy(&mut h);
    let request = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            h.scope(),
            &transaction_id(),
            180_000,
            key("fresh-epoch"),
        )
        .unwrap();
    (h, old, request)
}
fn request_file(h: &Harness, request: &SourceRevalidationRequest) -> PathBuf {
    let path = h.artifact_path.with_extension("revalidation-request");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(&request.canonical_bytes(h.scope(), h.owner.principal()))
        .unwrap();
    file.sync_all().unwrap();
    path
}
const CHILD: &str = "source_authority::lifecycle::revalidation::source_revalidation_process_child";
fn child(h: &Harness, path: &std::path::Path, fault: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            CHILD,
            "--nocapture",
            "--test-threads=1",
        ])
        .env("CELETIXO_REVAL_REQUEST", path)
        .env("CELETIXO_REVAL_ORG", h.scope().organization.as_str())
        .env("CELETIXO_REVAL_WS", h.scope().workspace.as_str())
        .env("CELETIXO_REVAL_REPO", h.scope().repository.as_str())
        .env("CELETIXO_REVAL_FAULT", fault);
    command
}
#[test]
#[ignore = "only invoked by the retained PostgreSQL parent process test"]
fn source_revalidation_process_child() {
    let Ok(path) = std::env::var("CELETIXO_REVAL_REQUEST") else {
        return;
    };
    let selected = Scope {
        organization: OrganizationId::new(std::env::var("CELETIXO_REVAL_ORG").unwrap()).unwrap(),
        workspace: WorkspaceId::new(std::env::var("CELETIXO_REVAL_WS").unwrap()).unwrap(),
        repository: RepositoryId::new(std::env::var("CELETIXO_REVAL_REPO").unwrap()).unwrap(),
    };
    let bytes = std::fs::read(path).unwrap();
    let principal = PrincipalId::new("human").unwrap();
    let request = restore_source_revalidation_request(
        &selected,
        &principal,
        key("fresh-epoch"),
        Digest::of(SourceRevalidationRequest::FORMAT, &bytes),
        &bytes,
        Default::default(),
    )
    .unwrap();
    let fault = std::env::var("CELETIXO_REVAL_FAULT").unwrap();
    let authority = super::super::authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit && fault == "before" {
            std::process::exit(93)
        }
        if stage == AuthorityStage::AfterCommit && fault == "after" {
            std::process::exit(94)
        }
        Ok(())
    }));
    let (engine, admin) = Engine::with_authority_backend(Arc::new(authority));
    let owner = retry_contention(|| {
        admin.authenticate_existing_session(&selected, request.authorization_grant, |p, s| {
            assert_eq!(p, &principal);
            assert_eq!(s, &selected);
            Ok(())
        })
    })
    .unwrap();
    let receipt =
        retry_contention(|| engine.revalidate_source_prepared(&owner, &selected, request.clone()))
            .unwrap();
    println!("revalidation-receipt:{}", receipt.digest());
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and retained local artifacts"]
fn source_revalidation_reopens_fresh_epoch_rejects_late_evidence_and_publishes_fresh_checks() {
    let (mut h, old, request) = ready("source-revalidation-reopen", 300_000);
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let receipt = h
        .engine
        .revalidate_source_prepared(&h.owner, h.scope(), request.clone())
        .unwrap();
    assert_eq!(receipt.attempt, 1);
    assert_eq!(
        receipt.invalidated_source_attempts,
        [old.binding().source_attempt.clone()].into()
    );
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(
        after.records.base.reservation_fence,
        before.records.base.reservation_fence + 1
    );
    assert_eq!(
        after.records.base.transactions[&transaction_id()]
            .attempts
            .len(),
        2
    );
    AuthorityState::from_source_records(after.records.clone(), Default::default()).unwrap();
    h.reopen();
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, h.scope(), request.clone())
            .unwrap(),
        receipt
    );
    assert!(
        h.engine
            .mark_source_patch_applied(&h.executor, &old)
            .is_err()
    );
    let patch = h.patch("epoch-source-1");
    let admission = h
        .engine
        .admit_source_attempt(&h.owner, h.scope(), patch.clone(), key("epoch-source-1"))
        .unwrap();
    assert_eq!(admission.binding().transaction_attempt, 1);
    h.engine
        .claim_source_attempt(&h.executor, &admission)
        .unwrap();
    let application = h
        .engine
        .source_application(&h.executor, &admission)
        .unwrap();
    h.engine
        .mark_source_patch_applied(&h.executor, &admission)
        .unwrap();
    h.engine
        .begin_source_verification(&h.executor, &admission)
        .unwrap();
    let (evidence, envelope) = h.synthetic_evidence(&patch, &admission, &application, 0);
    h.engine
        .record_source_evidence(&h.executor, &admission, &h.storage, evidence, envelope)
        .unwrap();
    let candidate = h
        .engine
        .source_publication_candidate(&h.owner, h.scope(), admission.id())
        .unwrap();
    let facts = RepositoryAnalysis::new(
        candidate.snapshot(),
        RepositoryAnalysisInput {
            index_generation: d("fresh-published-index"),
            analyzer_version: "finite-fresh-sql/v1".into(),
            configuration: d("fresh-published-config"),
            elements: [element()].into(),
            dependencies: BTreeMap::new(),
            checks: [(element(), [check_id()].into())].into(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    let commit = h
        .engine
        .prepare_source_commit_request(
            &h.owner,
            &h.owner,
            h.scope(),
            SourceCommitPreparation {
                admission: admission.id(),
                analysis: facts,
                key: key("fresh-commit"),
            },
        )
        .unwrap();
    h.engine
        .commit_source_prepared(&h.owner, &h.owner, h.scope(), commit)
        .unwrap();
    h.base = h.engine.current(&h.owner, h.scope()).unwrap();
    fresh_policy(&mut h);
    h.reopen();
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, h.scope(), request)
            .unwrap(),
        receipt
    );
    assert_eq!(h.base.revision(), Revision(1));
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and independently launched processes"]
fn source_revalidation_process_crashes_roll_back_or_recover_one_original_epoch() {
    for fault in ["before", "after"] {
        let (mut h, _, request) = ready(&format!("source-revalidation-crash-{fault}"), 300_000);
        let path = request_file(&h, &request);
        let before = h.authority.inspect_source(h.scope()).unwrap();
        let output = child(&h, &path, fault).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(if fault == "before" { 93 } else { 94 }),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let after = h.authority.inspect_source(h.scope()).unwrap();
        if fault == "before" {
            assert_eq!(before, after)
        } else {
            assert_eq!(
                after.records.base.transactions[&transaction_id()]
                    .attempts
                    .len(),
                2
            );
        }
        h.reopen();
        let receipt = h
            .engine
            .revalidate_source_prepared(&h.owner, h.scope(), request.clone())
            .unwrap();
        assert_eq!(receipt.attempt, 1);
        let once = h.authority.inspect_source(h.scope()).unwrap();
        assert_eq!(
            h.engine
                .revalidate_source_prepared(&h.owner, h.scope(), request)
                .unwrap(),
            receipt
        );
        assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), once);
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and independently launched processes"]
fn source_revalidation_processes_share_exactly_one_original_result_and_reservation() {
    let (h, _, request) = ready("source-revalidation-process-race", 300_000);
    let path = request_file(&h, &request);
    let mut children = Vec::new();
    for _ in 0..2 {
        children.push(
            child(&h, &path, "")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    let receipt = retry_contention(|| {
        h.engine
            .revalidate_source_prepared(&h.owner, h.scope(), request.clone())
    })
    .unwrap();
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("revalidation-receipt:{}", receipt.digest()))
        );
    }
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(
        after.records.base.transactions[&transaction_id()]
            .attempts
            .len(),
        2
    );
    assert_eq!(
        after.records.base.transactions[&transaction_id()]
            .lease_history
            .iter()
            .filter(|e| matches!(e.kind, LeaseEventKind::Acquired { .. }))
            .count(),
        2
    );
    assert_eq!(
        after
            .records
            .base
            .mutations
            .values()
            .filter(|record| matches!(record.outcome, Ok(MutationValue::SourceRevalidated(_))))
            .count(),
        1
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and a bounded database-time wait"]
fn final_database_time_rejects_revalidation_after_the_original_lease_expires() {
    use std::time::Duration;
    let (h, _, request) = ready("source-revalidation-final-time", 18_000);
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let expiry = before.records.base.transactions[&transaction_id()]
        .lease
        .as_ref()
        .unwrap()
        .expires_at;
    let injected = super::super::authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            assert!(now() < expiry);
            let remaining = expiry.saturating_add(2).saturating_sub(now());
            assert!(remaining <= 9_002);
            std::thread::sleep(Duration::from_millis(remaining));
        }
        Ok(())
    }));
    let (engine, admin) = Engine::with_authority_backend(Arc::new(injected));
    let owner = admin
        .authenticate_existing_session(h.scope(), request.authorization_grant, |_, _| Ok(()))
        .unwrap();
    let outside = expiry.saturating_sub(now().saturating_add(9_000));
    std::thread::sleep(Duration::from_millis(outside));
    assert_eq!(
        engine.revalidate_source_prepared(&owner, h.scope(), request),
        Err(EngineError::Authority(AuthorityFailure::Contention))
    );
    assert_eq!(
        h.authority.inspect_source(h.scope()).unwrap(),
        before,
        "fresh attempt, invalidation, new fence, journal and scope publication all roll back"
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_revalidation_contexts_are_immutable_forced_rls_and_required_for_checked_recovery() {
    let (h, _, request) = ready("source-revalidation-context-guards", 300_000);
    h.engine
        .revalidate_source_prepared(&h.owner, h.scope(), request)
        .unwrap();
    let mut sql = owner();
    let a = [
        h.scope().organization.as_str(),
        h.scope().workspace.as_str(),
        h.scope().repository.as_str(),
    ];
    for table in [
        "source_revalidation_mutations",
        "source_revalidation_leases",
        "source_revalidation_sources",
    ] {
        assert!(sql.query_one(&format!("SELECT count(*) FROM celetixo.{table} WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"),&[&a[0],&a[1],&a[2]]).unwrap().get::<_,i64>(0)>0);
        let flags=sql.query_one("SELECT relrowsecurity,relforcerowsecurity FROM pg_class WHERE oid=$1::text::regclass",&[&format!("celetixo.{table}")]).unwrap();
        assert!(flags.get::<_, bool>(0));
        assert!(flags.get::<_, bool>(1));
        for action in [
            format!("UPDATE celetixo.{table} SET actor_digest=actor_digest"),
            format!("DELETE FROM celetixo.{table}"),
        ] {
            let mut tx = sql.transaction().unwrap();
            assert!(tx.execute(&format!("{action} WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"),&[&a[0],&a[1],&a[2]]).is_err());
            tx.rollback().unwrap();
        }
    }
    let mut tampered = h.authority.inspect_source(h.scope()).unwrap().records;
    tampered
        .base
        .mutations
        .retain(|_, record| !matches!(record.request, MutationRequest::SourceRevalidate(_)));
    assert!(AuthorityState::from_source_records(tampered, Default::default()).is_err());
    let foreign = scope("unmapped-revalidation");
    let mut runtime = connection("runtime", None).connect().unwrap();
    runtime
        .execute(
            "SELECT set_config('celetixo.organization',$1,false)",
            &[&foreign.organization.as_str()],
        )
        .unwrap();
    for table in [
        "source_revalidation_mutations",
        "source_revalidation_leases",
        "source_revalidation_sources",
    ] {
        assert_eq!(
            runtime
                .query_one(
                    &format!(
                        "SELECT count(*) FROM celetixo.{table} WHERE organization_id=$1::text"
                    ),
                    &[&foreign.organization.as_str()]
                )
                .unwrap()
                .get::<_, i64>(0),
            0
        );
    }
}
