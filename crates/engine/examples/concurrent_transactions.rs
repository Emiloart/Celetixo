//! Run with `cargo run --locked -p celetixo-engine --example concurrent_transactions`.
//! This evaluates declared integer-state invariants in process; it executes no repository code.
use celetixo_engine::access::{AccessSet, Completeness, Condition, Invariant};
use celetixo_engine::engine::{Engine, EngineError, InMemoryStore, Permission, Session};
use celetixo_engine::evidence::{
    Artifact, CheckResult, CheckSpec, ExecutionProvenance, VerificationEvidence,
};
use celetixo_engine::ids::*;
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::patch::{Change, Patch};
use celetixo_engine::snapshot::{InclusionPolicy, Snapshot, SnapshotManifest, State};
use celetixo_engine::transaction::{TransactionIntent, TransactionState};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::Write as _;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn element(value: &str) -> ElementId {
    ElementId::new(value).expect("constant element identity")
}
fn transaction(value: &str) -> TransactionId {
    TransactionId::new(value).expect("constant transaction identity")
}
fn key(value: &str) -> IdempotencyKey {
    IdempotencyKey::new(value).expect("constant idempotency key")
}

struct Scenario {
    first: Engine,
    second: Engine,
    worker_a: Session,
    worker_b: Session,
    verifier: Session,
    scope: Scope,
    initial: Snapshot,
    check: CheckSpec,
}
impl Scenario {
    fn new(name: &str) -> Result<Self> {
        let scope = Scope {
            organization: OrganizationId::new("example-organization")?,
            workspace: WorkspaceId::new("example-workspace")?,
            repository: RepositoryId::new(name)?,
        };
        let (store, administrator) = InMemoryStore::new();
        // Separate Engine handles share only the reference authoritative store.
        let first = Engine::new(store.clone());
        let second = Engine::new(store);
        let setup = administrator.grant(
            PrincipalId::new("setup")?,
            scope.clone(),
            [Permission::Register, Permission::Read, Permission::Analyze].into(),
        )?;
        // Analyze requests deterministic Engine-owned analysis; it cannot supply a report.
        let worker_permissions: BTreeSet<Permission> = [
            Permission::Read,
            Permission::Propose,
            Permission::Analyze,
            Permission::Reserve,
            Permission::Execute,
            Permission::Commit,
            Permission::Cancel,
        ]
        .into();
        let worker_a = administrator.grant(
            PrincipalId::new("worker-a")?,
            scope.clone(),
            worker_permissions.clone(),
        )?;
        let worker_b = administrator.grant(
            PrincipalId::new("worker-b")?,
            scope.clone(),
            worker_permissions,
        )?;
        let verifier = administrator.grant(
            PrincipalId::new("trusted-reference-verifier")?,
            scope.clone(),
            [Permission::Read, Permission::Verify].into(),
        )?;
        let initial = Snapshot::new(
            scope.clone(),
            Revision(0),
            SnapshotManifest {
                base_commit: "reference-fixture/v1".into(),
                canonicalization_version: 1,
                inclusion_policy: InclusionPolicy::complete(),
                provenance: "complete empty workspace census; state fixture/v1".into(),
                inputs: vec![],
                environment: BTreeMap::new(),
            },
            [(element("symbol:x"), 1), (element("symbol:y"), 2)].into(),
        )?;
        first.register(
            &setup,
            initial.clone(),
            EngineeringModel {
                generation: 1,
                analyzer: "declared-reference-model".into(),
                version: "1".into(),
                configuration: Digest::of("reference/model-config/v1", b"no undeclared edges"),
                dependencies: BTreeMap::new(),
                completeness: Completeness::Complete,
            },
        )?;
        let check = CheckSpec::new(
            CheckId::new("reference-invariant-evaluation")?,
            vec![
                "reference-evaluator/v1".into(),
                "declared-invariants".into(),
            ],
            format!("celetixo-engine/{}", env!("CARGO_PKG_VERSION")),
            Digest::of(
                "reference/environment/v1",
                b"T0 deterministic integer state",
            ),
            Digest::of(
                "reference/check-configuration/v1",
                b"all declared invariants",
            ),
        )?;
        Ok(Self {
            first,
            second,
            worker_a,
            worker_b,
            verifier,
            scope,
            initial,
            check,
        })
    }

    fn prepare(
        &self,
        engine: &Engine,
        worker: &Session,
        id: &TransactionId,
        write: &str,
        value: i64,
        reads: &[&str],
    ) -> Result<Vec<Invariant>> {
        let write = element(write);
        let invariants = vec![Invariant {
            id: InvariantId::new(format!("{id}:assigned-value"))?,
            condition: Condition::Equals {
                element: write.clone(),
                value,
            },
        }];
        engine.propose(
            worker,
            &self.scope,
            id.clone(),
            self.initial.id(),
            TransactionIntent {
                objective: format!("Set {write} to {value}"),
                accesses: AccessSet {
                    reads: reads.iter().map(|read| element(read)).collect(),
                    writes: [write.clone()].into(),
                    ..Default::default()
                },
                invariants: invariants.clone(),
                required_checks: vec![self.check.clone()],
                depends_on: Default::default(),
            },
        )?;
        engine.analyze(worker, &self.scope, id)?;
        engine.reserve(worker, &self.scope, id, 100)?;
        engine.execute(worker, &self.scope, id)?;
        let patch = Patch::new(
            self.initial.id(),
            [(
                write.clone(),
                Change {
                    expected: self.initial.state().get(&write).copied(),
                    value: Some(value),
                },
            )]
            .into(),
        )?;
        engine.submit_patch(worker, &self.scope, id, patch)?;
        Ok(invariants)
    }

    /// Trusted deterministic evaluator. Status derives from the actual candidate state;
    /// every evaluation and result is retained in the immutable transcript artifact.
    fn verify(
        &self,
        engine: &Engine,
        id: &TransactionId,
        invariants: &[Invariant],
    ) -> Result<VerificationEvidence> {
        let bindings = engine.bindings(&self.verifier, &self.scope, id)?;
        let candidate = engine.candidate(&self.verifier, &self.scope, id)?;
        assert_eq!(bindings.candidate_snapshot, candidate.id());
        let mut transcript = format!(
            "reference-evaluator/v1\ntransaction={id}\nattempt={}\ncandidate={}\n",
            bindings.attempt,
            candidate.id()
        );
        for (element, value) in candidate.state() {
            writeln!(transcript, "state {element}={value}")?;
        }
        let mut diagnostics = vec![];
        for invariant in invariants {
            match invariant.evaluate(candidate.state()) {
                Ok(()) => writeln!(transcript, "PASS {}", invariant.id)?,
                Err(error) => {
                    writeln!(transcript, "FAIL {}: {error}", invariant.id)?;
                    diagnostics.push(error.to_string());
                }
            }
        }
        let result = if diagnostics.is_empty() {
            CheckResult::Passed { exit_code: 0 }
        } else {
            CheckResult::Failed { exit_code: 1 }
        };
        writeln!(transcript, "result={result:?}")?;
        let evidence = VerificationEvidence::new(
            bindings.clone(),
            self.check.clone(),
            result,
            ExecutionProvenance {
                verifier: self.verifier.principal().clone(),
                runner: "reference-evaluator/v1".into(),
                run_id: format!("{id}:attempt:{}", bindings.attempt),
                started_at: 0,
                finished_at: 0,
            },
            diagnostics,
            vec![Artifact::new(
                "evaluation-transcript",
                "text/plain; charset=utf-8",
                transcript.into_bytes(),
            )?],
        )?;
        engine.record_evidence(&self.verifier, &self.scope, id, evidence.clone())?;
        Ok(evidence)
    }
}

fn compatible_transactions() -> Result<()> {
    let scenario = Scenario::new("compatible")?;
    let a = transaction("compatible:a");
    let b = transaction("compatible:b");
    let a_invariants =
        scenario.prepare(&scenario.first, &scenario.worker_a, &a, "symbol:x", 10, &[])?;
    let b_invariants = scenario.prepare(
        &scenario.second,
        &scenario.worker_b,
        &b,
        "symbol:y",
        20,
        &[],
    )?;
    scenario.verify(&scenario.first, &a, &a_invariants)?;
    let old_b_evidence = scenario.verify(&scenario.second, &b, &b_invariants)?;
    assert!(matches!(
        scenario.second.record_evidence(
            &scenario.worker_b,
            &scenario.scope,
            &b,
            old_b_evidence.clone()
        ),
        Err(EngineError::Unauthorized)
    ));
    let receipt = scenario.first.commit(
        &scenario.worker_a,
        &scenario.scope,
        &a,
        key("compatible:a:commit"),
    )?;
    println!("Compatible: A committed revision {}.", receipt.revision.0);
    assert!(matches!(
        scenario.second.commit(
            &scenario.worker_b,
            &scenario.scope,
            &b,
            key("compatible:b:stale")
        ),
        Err(EngineError::RevalidationRequired { .. })
    ));
    println!("Compatible: B requires revalidation after A's commit.");
    scenario
        .second
        .revalidate(&scenario.worker_b, &scenario.scope, &b)?;
    assert!(matches!(
        scenario
            .second
            .record_evidence(&scenario.verifier, &scenario.scope, &b, old_b_evidence),
        Err(EngineError::EvidenceMismatch)
    ));
    scenario.verify(&scenario.second, &b, &b_invariants)?;
    let receipt = scenario.second.commit(
        &scenario.worker_b,
        &scenario.scope,
        &b,
        key("compatible:b:revalidated"),
    )?;
    let final_snapshot = scenario
        .first
        .current(&scenario.worker_a, &scenario.scope)?;
    let expected: State = [(element("symbol:x"), 10), (element("symbol:y"), 20)].into();
    assert_eq!(final_snapshot.state(), &expected);
    assert_eq!(final_snapshot.revision(), Revision(2));
    println!(
        "Compatible: B committed revision {} with fresh evidence; final x=10, y=20.",
        receipt.revision.0
    );
    Ok(())
}

fn incompatible_transactions() -> Result<()> {
    let scenario = Scenario::new("incompatible")?;
    let a = transaction("incompatible:a");
    let b = transaction("incompatible:b");
    let a_invariants =
        scenario.prepare(&scenario.first, &scenario.worker_a, &a, "symbol:x", 10, &[])?;
    let b_invariants = scenario.prepare(
        &scenario.second,
        &scenario.worker_b,
        &b,
        "symbol:y",
        20,
        &["symbol:x"],
    )?;
    scenario.verify(&scenario.first, &a, &a_invariants)?;
    scenario.verify(&scenario.second, &b, &b_invariants)?;
    scenario.first.commit(
        &scenario.worker_a,
        &scenario.scope,
        &a,
        key("incompatible:a:commit"),
    )?;
    assert!(matches!(
        scenario.second.commit(
            &scenario.worker_b,
            &scenario.scope,
            &b,
            key("incompatible:b:stale")
        ),
        Err(EngineError::RevalidationRequired { .. })
    ));
    let conflict = scenario
        .second
        .revalidate(&scenario.worker_b, &scenario.scope, &b);
    assert!(matches!(&conflict, Err(EngineError::Conflict(details)) if !details.is_empty()));
    if let Err(EngineError::Conflict(reports)) = conflict {
        println!(
            "Incompatible: B's read of x conflicts with {} at revision {}; revalidation rejected.",
            reports[0].committed, reports[0].committed_revision.0
        );
    }
    scenario.second.abort(
        &scenario.worker_b,
        &scenario.scope,
        &b,
        "read assumption invalidated by A".into(),
    )?;
    assert_eq!(
        scenario
            .second
            .transaction(&scenario.worker_b, &scenario.scope, &b)?
            .state,
        TransactionState::Aborted
    );
    let final_snapshot = scenario
        .first
        .current(&scenario.worker_a, &scenario.scope)?;
    let expected: State = [(element("symbol:x"), 10), (element("symbol:y"), 2)].into();
    assert_eq!(final_snapshot.state(), &expected);
    assert_eq!(final_snapshot.revision(), Revision(1));
    println!("Incompatible: B aborted; final x=10, y=2.");
    Ok(())
}

fn main() -> Result<()> {
    compatible_transactions()?;
    incompatible_transactions()?;
    println!("All reference-engine assertions passed. Storage is in memory.");
    Ok(())
}
