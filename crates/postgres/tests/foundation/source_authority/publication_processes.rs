//! Independent processes read real immutable objects and race the SQL boundary.
//! Check evidence remains synthetic; no sandbox-execution claim is made here.
use super::*;
use celetixo_engine::persistence::restore_source_commit_request;
use celetixo_execution::artifacts::LocalSourceArtifactReader;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const CHILD: &str =
    "source_authority::lifecycle::publication::processes::source_commit_process_child";

fn retry_contention<T>(
    mut action: impl FnMut() -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let start = Instant::now();
    for retry in 0..=4 {
        match action() {
            Err(EngineError::Authority(AuthorityFailure::Contention))
                if retry < 4 && start.elapsed() < Duration::from_secs(30) =>
            {
                std::thread::sleep(Duration::from_millis(50 << retry));
            }
            result => return result,
        }
    }
    unreachable!()
}

fn directory(h: &Harness) -> PathBuf {
    let directory = h.artifact_path.with_extension("commit-processes");
    std::fs::create_dir(&directory).unwrap();
    directory
}

fn save_request(h: &Harness, directory: &std::path::Path, request: &SourceCommitRequest) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("request"))
        .unwrap();
    file.write_all(&request.canonical_bytes(h.scope(), h.owner.principal()))
        .unwrap();
    file.sync_all().unwrap();
}

fn child(h: &Harness, directory: &std::path::Path, tag: &str, key: &str, mode: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            CHILD,
            "--nocapture",
            "--test-threads=1",
        ])
        .env("CELETIXO_COMMIT_DIRECTORY", directory)
        .env("CELETIXO_COMMIT_OBJECTS", &h.artifact_path)
        .env("CELETIXO_COMMIT_ORG", h.scope().organization.as_str())
        .env("CELETIXO_COMMIT_TAG", tag)
        .env("CELETIXO_COMMIT_KEY", key)
        .env("CELETIXO_COMMIT_MODE", mode)
        .stdout(Stdio::from(
            File::create(directory.join(format!("{tag}.stdout"))).unwrap(),
        ))
        .stderr(Stdio::from(
            File::create(directory.join(format!("{tag}.stderr"))).unwrap(),
        ))
        .spawn()
        .unwrap()
}

fn finish(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            return child.wait().unwrap();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "only invoked by the retained source commit process parents"]
fn source_commit_process_child() {
    let Some(directory) = std::env::var_os("CELETIXO_COMMIT_DIRECTORY") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let path = PathBuf::from(std::env::var_os("CELETIXO_COMMIT_OBJECTS").unwrap());
    assert!(directory.is_absolute() && path.is_absolute());
    let tag = std::env::var("CELETIXO_COMMIT_TAG").unwrap();
    let mode = std::env::var("CELETIXO_COMMIT_MODE").unwrap();
    let scope = Scope {
        organization: OrganizationId::new(std::env::var("CELETIXO_COMMIT_ORG").unwrap()).unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    };
    let principal = PrincipalId::new("human").unwrap();
    let bytes = std::fs::read(directory.join("request")).unwrap();
    let request = restore_source_commit_request(
        &scope,
        &principal,
        IdempotencyKey::new(std::env::var("CELETIXO_COMMIT_KEY").unwrap()).unwrap(),
        Digest::of(SourceCommitRequest::FORMAT, &bytes),
        &bytes,
        Default::default(),
    )
    .unwrap();
    let target = match mode.as_str() {
        "race" => None,
        "before" => Some(AuthorityStage::BeforeCommit),
        "after" => Some(AuthorityStage::AfterCommit),
        _ => panic!("unknown controlled source commit mode"),
    };
    let backend = super::super::super::authority().with_checkpoint(Arc::new(move |stage| {
        if Some(stage) == target {
            std::process::exit(if stage == AuthorityStage::BeforeCommit {
                95
            } else {
                96
            });
        }
        Ok(())
    }));
    let (engine, admin) = Engine::with_authority_backend(Arc::new(backend));
    let owner = retry_contention(|| {
        admin.authenticate_existing_session(&scope, request.authorization_grant, |p, s| {
            assert_eq!(p, &principal);
            assert_eq!(s, &scope);
            Ok(())
        })
    })
    .unwrap();
    let analyst = retry_contention(|| {
        admin.authenticate_existing_session(&scope, request.analysis_grant, |p, s| {
            assert_eq!(p, &request.analysis_principal);
            assert_eq!(s, &scope);
            Ok(())
        })
    })
    .unwrap();
    admin
        .resolve_existing_source_artifact_store(&scope, request.store_registration, |s, record| {
            assert_eq!(s, &scope);
            assert_eq!(
                record.adapter.as_ref().unwrap().configuration,
                Digest::of(
                    "source-sql-test-artifact-configuration/v1",
                    path.as_os_str().as_encoded_bytes()
                )
            );
            Ok(Arc::new(
                LocalSourceArtifactReader::open(
                    &path,
                    scope.clone(),
                    record.retention,
                    ArtifactStoreLimits::default(),
                )
                .map_err(|_| AuthorityFailure::Unavailable)?,
            ))
        })
        .unwrap();
    std::fs::write(directory.join(format!("{tag}.ready")), b"ready").unwrap();
    if mode == "race" {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !directory.join("go").exists() {
            assert!(
                Instant::now() < deadline,
                "parent did not release source commit race"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    let result = retry_contention(|| {
        engine.commit_source_prepared(&owner, &analyst, &scope, request.clone())
    });
    assert_eq!(
        retry_contention(|| engine.commit_source_prepared(
            &owner,
            &analyst,
            &scope,
            request.clone()
        )),
        result
    );
    let text = match result {
        Ok(receipt) => format!("accepted:{}", receipt.digest()),
        Err(EngineError::SourceAttempt(SourceAttemptError::InvalidState)) => {
            "rejected:InvalidState".into()
        }
        other => panic!("unexpected source process outcome: {other:?}"),
    };
    std::fs::write(directory.join(format!("{tag}.result")), text).unwrap();
}

#[test]
#[ignore = "requires the reviewed PostgreSQL fixture and retained Linux artifacts"]
fn independent_source_commit_processes_recover_one_revision_for_same_or_distinct_keys() {
    for same_key in [true, false] {
        let (h, _, request) = ready(if same_key {
            "source-process-same"
        } else {
            "source-process-distinct"
        });
        let directory = directory(&h);
        save_request(&h, &directory, &request);
        let tags = ["first", "second"];
        let keys = if same_key {
            ["source-commit", "source-commit"]
        } else {
            ["source-commit", "other-source-commit"]
        };
        let mut children: Vec<_> = tags
            .into_iter()
            .zip(keys)
            .map(|(tag, key)| child(&h, &directory, tag, key, "race"))
            .collect();
        let deadline = Instant::now() + Duration::from_secs(60);
        while !tags
            .iter()
            .all(|tag| directory.join(format!("{tag}.ready")).exists())
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(25));
        }
        let all_ready = tags
            .iter()
            .all(|tag| directory.join(format!("{tag}.ready")).exists());
        std::fs::write(directory.join("go"), b"go").unwrap();
        let statuses: Vec<_> = children.iter_mut().map(finish).collect();
        assert!(
            all_ready,
            "source children did not independently resolve objects; logs at {}",
            directory.display()
        );
        assert!(
            statuses.iter().all(ExitStatus::success),
            "source child failure; logs at {}",
            directory.display()
        );
        let outcomes = tags
            .map(|tag| std::fs::read_to_string(directory.join(format!("{tag}.result"))).unwrap());
        if same_key {
            assert_eq!(outcomes[0], outcomes[1]);
            assert!(outcomes[0].starts_with("accepted:"));
        } else {
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|r| r.starts_with("accepted:"))
                    .count(),
                1
            );
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|r| r.as_str() == "rejected:InvalidState")
                    .count(),
                1
            );
        }
        let records = h.authority.inspect_source(h.scope()).unwrap().records;
        assert_eq!(records.history.len(), 1);
        assert_eq!(
            records.base.repository.as_ref().unwrap().current,
            request.publication_snapshot
        );
        assert_eq!(
            records.base.transactions[&transaction_id()].state,
            TransactionState::Committed
        );
        assert_eq!(
            records
                .base
                .mutations
                .values()
                .filter(|r| matches!(r.outcome, Ok(MutationValue::SourceCommitted(_))))
                .count(),
            1
        );
        assert_eq!(
            records
                .base
                .mutations
                .values()
                .filter(|r| matches!(r.request, MutationRequest::SourceCommit(_)))
                .count(),
            if same_key { 1 } else { 2 }
        );
        assert_one_commit_rows(&h);
    }
}

#[test]
#[ignore = "requires the reviewed PostgreSQL fixture and retained Linux artifacts"]
fn source_commit_process_exit_before_or_after_publication_recovers_exact_original_effect() {
    for mode in ["before", "after"] {
        let (h, analyst, request) = ready(if mode == "before" {
            "source-process-before"
        } else {
            "source-process-after"
        });
        let before = h.authority.inspect_source(h.scope()).unwrap();
        let directory = directory(&h);
        save_request(&h, &directory, &request);
        let status = finish(&mut child(&h, &directory, "crash", "source-commit", mode));
        assert_eq!(
            status.code(),
            Some(if mode == "before" { 95 } else { 96 }),
            "source child logs at {}",
            directory.display()
        );
        let observed = h.authority.inspect_source(h.scope()).unwrap();
        if mode == "before" {
            assert_eq!(observed, before);
        } else {
            assert_eq!(observed.records.history.len(), 1);
        }
        let receipt = h
            .engine
            .commit_source_prepared(&h.owner, &analyst, h.scope(), request.clone())
            .unwrap();
        let after = h.authority.inspect_source(h.scope()).unwrap();
        assert_eq!(after.records.history.len(), 1);
        assert_eq!(after.records.history[0].receipt, receipt);
        assert_eq!(
            h.engine
                .commit_source_prepared(&h.owner, &analyst, h.scope(), request)
                .unwrap(),
            receipt
        );
        assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), after);
        assert_one_commit_rows(&h);
    }
}

fn assert_one_commit_rows(h: &Harness) {
    let s = h.scope();
    let parameters = [
        s.organization.as_str(),
        s.workspace.as_str(),
        s.repository.as_str(),
    ];
    let mut database = owner();
    for table in [
        "source_committed_changes",
        "transaction_outcomes",
        "outbox",
        "outbox_delivery",
    ] {
        assert_eq!(database.query_one(&format!("SELECT count(*) FROM celetixo.{table} WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"), &[&parameters[0], &parameters[1], &parameters[2]]).unwrap().get::<_, i64>(0), 1, "{table}");
    }
}
