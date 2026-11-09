//! Two genuinely admitted copies contend for one backend's actual capacity.
//! Fault injection stops the controller after claim publication, before launch.
use crate::bench_support::{memory, micros};
use crate::fixture::{self, AdmittedFixture};
use celetixo_engine::engine::SourceAttemptState;
use celetixo_engine::ids::PrincipalId;
use celetixo_execution::backend::{ExecutionCheckpoint, ExecutionGuard, GuardDecision};
use celetixo_execution::coordinator::MonotonicEngineClock;
use celetixo_execution::materialize::{CleanupOutcome, PrivateWorkspace};
use celetixo_execution::oci::{OciBackend, OciError, execution_identity};
use serde_json::{Value, json};
use std::error::Error;
use std::time::Instant;

fn prepare(f: &AdmittedFixture) -> PrivateWorkspace {
    f.engine
        .claim_source_attempt(&f.executor, &f.admission)
        .unwrap();
    let workspace = fixture::workspace_registry()
        .materialize(
            &f.engine,
            &f.executor,
            &f.admission,
            &f.base,
            &f.patch,
            |_| Ok(()),
        )
        .unwrap();
    f.engine
        .mark_source_patch_applied(&f.executor, &f.admission)
        .unwrap();
    f.engine
        .begin_source_verification(&f.executor, &f.admission)
        .unwrap();
    workspace
}

struct Contender<'a> {
    backend: &'a OciBackend,
    other: &'a AdmittedFixture,
    workspace: &'a mut PrivateWorkspace,
    first_clock: MonotonicEngineClock,
    other_clock: MonotonicEngineClock,
    busy_us: Option<u64>,
}
impl ExecutionGuard for Contender<'_> {
    fn revalidate(&mut self) -> Result<GuardDecision, String> {
        self.first_clock.revalidate()
    }
    fn checkpoint(&mut self, checkpoint: ExecutionCheckpoint) -> Result<(), String> {
        if checkpoint != ExecutionCheckpoint::AfterClaimPublication {
            return Ok(());
        }
        let start = Instant::now();
        let other = self.backend.execute(
            &self.other.engine,
            &self.other.executor,
            &self.other.admission,
            self.workspace,
            &self.other.definition,
            &mut self.other_clock,
        );
        self.busy_us = Some(micros(start));
        if !matches!(other, Err(OciError::Busy)) {
            return Err(format!("expected bounded Busy rejection, got {other:?}"));
        }
        Err("benchmark controller interruption after durable claim".into())
    }
}

pub fn measure(backend: &OciBackend, iteration: usize) -> Result<Value, Box<dyn Error>> {
    let fixture = || {
        fixture::admitted_command(
            backend,
            b"package main\nfunc answer() int { return 42 }\n",
            "/toolchain/gofmt",
            &["-l", "/src/main.go"],
            fixture::limits(),
        )
    };
    let first = fixture();
    let other = fixture();
    let mut first_workspace = prepare(&first);
    let mut other_workspace = prepare(&other);
    let source_root = first_workspace
        .source_path()
        .parent()
        .unwrap()
        .to_path_buf();
    let execution = execution_identity(&first.admission, &first.definition);
    let start = Instant::now();
    let (result, busy_us) = {
        let mut guard = Contender {
            backend,
            other: &other,
            workspace: &mut other_workspace,
            first_clock: MonotonicEngineClock::new(first.admin.clone()),
            other_clock: MonotonicEngineClock::new(other.admin.clone()),
            busy_us: None,
        };
        let result = backend.execute(
            &first.engine,
            &first.executor,
            &first.admission,
            &mut first_workspace,
            &first.definition,
            &mut guard,
        );
        (result, guard.busy_us)
    };
    let prepared_failure_us = micros(start);
    assert!(
        matches!(result,Err(OciError::Guard(ref reason)) if reason=="benchmark controller interruption after durable claim")
    );
    assert!(source_root.join("execution-retention.json").is_file());
    assert!(matches!(
        first_workspace.quarantine("controlled benchmark controller loss"),
        CleanupOutcome::Quarantined { .. }
    ));
    let cleanup = first.admin.grant_source_cleanup(
        PrincipalId::new("benchmark-recovery")?,
        fixture::scope(),
        backend.identity().digest(),
        100_000,
    )?;
    let start = Instant::now();
    let recovered =
        backend.reconcile_execution(&first.engine, &cleanup, &fixture::scope(), execution)?;
    let orphan_recovery_us = micros(start);
    assert!(recovered.termination_confirmed);
    assert_eq!(recovered.cleanup, CleanupOutcome::Removed);
    assert!(!source_root.exists());
    let start = Instant::now();
    let again =
        backend.reconcile_execution(&first.engine, &cleanup, &fixture::scope(), execution)?;
    let repeated_recovery_us = micros(start);
    assert!(again.termination_confirmed);
    assert_eq!(again.cleanup, CleanupOutcome::Removed);
    assert!(
        backend
            .recover_execution_record(&first.engine, &cleanup, &fixture::scope(), execution)?
            .is_none()
    );
    assert_eq!(other_workspace.cleanup(), CleanupOutcome::Removed);
    for fixture in [&first, &other] {
        fixture.engine.finish_source_attempt(
            &fixture.executor,
            &fixture.admission,
            SourceAttemptState::ExecutionFailed,
            "controlled benchmark capacity/recovery probe; no check result".into(),
        )?;
    }
    Ok(
        json!({"iteration":iteration,"execution":execution.to_string(),"busy_rejection_us":busy_us.ok_or("no contender was tested")?,
        "preparation_and_controlled_failure_us":prepared_failure_us,"owned_orphan_recovery_us":orphan_recovery_us,
        "idempotent_repeated_recovery_us":repeated_recovery_us,"host_memory":memory(),
        "overlapping_admitted_attempts":2,"available_execution_slots":1,"queued_attempts":0,
        "case":"controller interruption after durable claim, before process launch; no check/evidence fabricated",
        "termination_confirmed":true,"owned_source_removed":true,"check_result_recovered":false}),
    )
}
