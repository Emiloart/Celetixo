use super::*;
use lifecycle::Harness;
use std::process::Stdio;
use std::time::{Duration, Instant};

const CHILD: &str = "source_authority::recovery::source_process_child";

fn directory(h: &Harness, label: &str) -> PathBuf {
    let path = h
        .artifact_path
        .with_file_name(format!("{}-{label}", h.scope().organization.as_str()));
    std::fs::create_dir(&path).unwrap();
    path
}
fn spawn(h: &Harness, directory: &std::path::Path, mode: &str, name: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env("CELETIXO_PG_SOURCE_CHILD", mode)
        .env(
            "CELETIXO_PG_SOURCE_ORGANIZATION",
            h.scope().organization.as_str(),
        )
        .env("CELETIXO_PG_SOURCE_DIRECTORY", directory)
        .env("CELETIXO_PG_SOURCE_NAME", name)
        .stdout(Stdio::from(
            std::fs::File::create(directory.join(format!("{name}.stdout"))).unwrap(),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(directory.join(format!("{name}.stderr"))).unwrap(),
        ))
        .spawn()
        .unwrap()
}

#[test]
#[ignore = "subprocess helper; invoked by source PostgreSQL recovery tests"]
fn source_process_child() {
    let Ok(mode) = std::env::var("CELETIXO_PG_SOURCE_CHILD") else {
        return;
    };
    let scope = Scope {
        organization: OrganizationId::new(
            std::env::var("CELETIXO_PG_SOURCE_ORGANIZATION").unwrap(),
        )
        .unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    };
    let directory = PathBuf::from(std::env::var_os("CELETIXO_PG_SOURCE_DIRECTORY").unwrap());
    assert!(directory.is_absolute());
    let name = std::env::var("CELETIXO_PG_SOURCE_NAME").unwrap();
    let authority = authority();
    let admission = authority.inspect_source(&scope).unwrap().records.attempts
        [&SourceAttemptId::new("source-attempt").unwrap()]
        .admission
        .clone();
    let target = match mode.as_str() {
        "before" => Some(AuthorityStage::BeforeCommit),
        "after" => Some(AuthorityStage::AfterCommit),
        "race" => None,
        _ => panic!("unknown controlled source child mode"),
    };
    let authority = authority.with_checkpoint(Arc::new(move |stage| {
        if Some(stage) == target {
            std::process::exit(75);
        }
        Ok(())
    }));
    let (engine, admin) = Engine::with_authority_backend(Arc::new(authority));
    let executor = admin
        .authenticate_existing_source_executor(&scope, 1, |_, _| Ok(()))
        .unwrap();
    if mode == "race" {
        std::fs::write(
            directory.join(format!("{name}.ready")),
            b"fresh process authenticated",
        )
        .unwrap();
        let start = Instant::now();
        while !directory.join("go").exists() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "parent did not release race"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let result = match engine.claim_source_attempt(&executor, &admission) {
        Ok(view) => {
            assert_eq!(view.state, SourceAttemptState::Patching);
            "claimed"
        }
        Err(EngineError::SourceAttempt(SourceAttemptError::AlreadyClaimed)) => "already-claimed",
        other => panic!("unexpected independent source claim result: {other:?}"),
    };
    assert_eq!(mode, "race", "crash checkpoint was not reached");
    std::fs::write(directory.join(format!("{name}.result")), result).unwrap();
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and independent Engine processes"]
fn source_independent_process_claim_race_has_one_durable_winner() {
    let h = Harness::new("source-process-race");
    h.prepare();
    let admission = h
        .engine
        .admit_source_attempt(
            &h.owner,
            h.scope(),
            h.patch("source-attempt"),
            IdempotencyKey::new("original-request").unwrap(),
        )
        .unwrap();
    let directory = directory(&h, "race");
    let mut children = [
        spawn(&h, &directory, "race", "a"),
        spawn(&h, &directory, "race", "b"),
    ];
    let start = Instant::now();
    while !["a", "b"]
        .iter()
        .all(|name| directory.join(format!("{name}.ready")).exists())
    {
        for child in &mut children {
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before race; retained logs: {directory:?}"
            );
        }
        if start.elapsed() > Duration::from_secs(10) {
            for child in &mut children {
                child.kill().unwrap();
                child.wait().unwrap();
            }
            panic!("fresh processes did not reach barrier; retained logs: {directory:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::write(directory.join("go"), b"release independent claims").unwrap();
    for child in &mut children {
        assert!(
            child.wait().unwrap().success(),
            "child failed; retained logs: {directory:?}"
        );
    }
    let mut results = ["a", "b"]
        .map(|name| std::fs::read_to_string(directory.join(format!("{name}.result"))).unwrap());
    results.sort();
    assert_eq!(results, ["already-claimed", "claimed"]);
    let after = h.authority.inspect_source(h.scope()).unwrap();
    let attempt = &after.records.attempts[&admission.binding().source_attempt];
    assert_eq!(attempt.state, SourceAttemptState::Patching);
    assert_eq!(attempt.executor_grant, Some(1));
    assert_eq!(attempt.events.len(), 2);
    assert_eq!(after.records.operations.len(), 1);
    assert!(after.records.evidence.is_empty());
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and independent Engine processes"]
fn source_process_death_before_and_after_commit_reconciles_original_admission() {
    for mode in ["before", "after"] {
        let h = Harness::new(&format!("source-process-death-{mode}"));
        h.prepare();
        let patch = h.patch("source-attempt");
        let key = IdempotencyKey::new("original-request").unwrap();
        let admission = h
            .engine
            .admit_source_attempt(&h.owner, h.scope(), patch.clone(), key.clone())
            .unwrap();
        let before = h.authority.inspect_source(h.scope()).unwrap();
        let directory = directory(&h, mode);
        assert_eq!(
            spawn(&h, &directory, mode, "crash").wait().unwrap().code(),
            Some(75),
            "retained logs: {directory:?}"
        );
        let after = authority().inspect_source(h.scope()).unwrap();
        if mode == "before" {
            assert_eq!(after, before);
        } else {
            assert_eq!(
                after.records.attempts[&admission.binding().source_attempt].state,
                SourceAttemptState::Patching
            );
            assert_eq!(
                after.records.attempts[&admission.binding().source_attempt]
                    .events
                    .len(),
                2
            );
        }
        let (engine, admin) = Engine::with_authority_backend(Arc::new(authority()));
        let owner = admin
            .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
            .unwrap();
        let executor = admin
            .authenticate_existing_source_executor(h.scope(), 1, |_, _| Ok(()))
            .unwrap();
        assert_eq!(
            engine.admit_source_attempt(&owner, h.scope(), patch, key),
            Ok(admission.clone())
        );
        if mode == "before" {
            engine.claim_source_attempt(&executor, &admission).unwrap();
        } else {
            assert_eq!(
                engine.claim_source_attempt(&executor, &admission),
                Err(EngineError::SourceAttempt(
                    SourceAttemptError::AlreadyClaimed
                ))
            );
        }
        let recovered = authority().inspect_source(h.scope()).unwrap();
        assert_eq!(recovered.records.operations, before.records.operations);
        assert_eq!(recovered.records.attempts.len(), 1);
        assert_eq!(
            recovered.records.attempts[&admission.binding().source_attempt]
                .events
                .len(),
            2
        );
        assert_eq!(engine.current(&owner, h.scope()).unwrap(), h.base);
    }
}
