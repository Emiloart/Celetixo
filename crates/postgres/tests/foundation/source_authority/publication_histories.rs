//! Generated distinct-transaction process races and fresh execution, checked by
//! an independent numeric serial interpreter. Finite analysis/check fixtures.
use super::*;
use celetixo_engine::access::Predicate;
use celetixo_engine::snapshot::effective_workspace;
use std::sync::atomic::{AtomicUsize, Ordering};
#[path = "../../../../engine/tests/support/source_history_model.rs"]
mod history_model;
use history_model::History;

fn vertex(index: usize) -> ElementId {
    if index == 0 {
        element()
    } else {
        ElementId::new("symbol:z").unwrap()
    }
}
fn path(index: usize) -> &'static str {
    if index == 0 {
        "src/main.rs"
    } else {
        "src/z.rs"
    }
}
fn transaction(index: usize) -> TransactionId {
    TransactionId::new(if index == 0 { "a" } else { "b" }).unwrap()
}
fn accesses(history: History, index: usize) -> AccessSet {
    let other = vertex(1 - index);
    AccessSet {
        writes: [vertex(index)].into(),
        reads: if history.explicit_read(index) {
            [other.clone()].into()
        } else {
            BTreeSet::new()
        },
        invariant_scopes: if history.scope(index) {
            [other.clone()].into()
        } else {
            BTreeSet::new()
        },
        predicates: if history.predicate(index) {
            [Predicate::Prefix(other.as_str().into())].into()
        } else {
            BTreeSet::new()
        },
    }
}
fn footprint(history: History, index: usize, actual: &AccessSet) {
    assert_eq!(actual.writes, [vertex(index)].into());
    assert_eq!(
        actual.reads,
        history
            .reads(index)
            .into_iter()
            .enumerate()
            .filter(|(_, read)| *read)
            .map(|(i, _)| vertex(i))
            .collect()
    );
    assert_eq!(
        actual.invariant_scopes,
        accesses(history, index).invariant_scopes
    );
}
fn fresh_policy(h: &mut Harness) {
    let r = h.authority.inspect_source(h.scope()).unwrap().records;
    let facts = &r.analyses[&r.analysis_head.unwrap()];
    let mut input = h.policy.input().clone();
    input.generation += 1;
    input.snapshot = h.base.id();
    input.analysis_artifact = facts.digest();
    input.index_generation = facts.index_generation();
    input.capture.snapshot = h.base.id();
    input.capture.manifest = h.base.manifest().digest();
    if h.base.revision().0 > 0 {
        input.capture.consistency = SourceCaptureConsistency::EnginePublication;
        input.capture.capture_policy = r.history.last().unwrap().receipt.digest();
    }
    let files = effective_workspace(&h.base).unwrap();
    for (path, rule) in &mut input.paths {
        rule.expected_base = Some(Preimage::of(&files[path]).unwrap());
    }
    let policy = SourceExecutionPolicy::new(input).unwrap();
    h.engine
        .install_source_policy(&h.owner, h.scope(), Some(h.policy.digest()), policy.clone())
        .unwrap();
    h.policy = policy;
}
fn run(
    h: &Harness,
    index: usize,
    name: &str,
    value: u64,
) -> (SourceAdmission, SourceEvidence, SourceArtifactReference) {
    let context = h
        .engine
        .source_patch_context(&h.owner, h.scope(), &transaction(index))
        .unwrap();
    let files = effective_workspace(&h.base).unwrap();
    let patch = SourcePatch::new(
        context
            .bind(
                SourceAttemptId::new(name).unwrap(),
                "generated-sql-source-history".into(),
                context.maximum_expiry_ms.min(now() + 180_000),
            )
            .unwrap(),
        h.policy.input().patch_policy.clone(),
        vec![PatchOperation::Replace {
            path: path(index).into(),
            expected: Preimage::of(&files[path(index)]).unwrap(),
            entry: PatchEntry::file(value.to_string().into_bytes(), FileMode::Regular).unwrap(),
        }],
    )
    .unwrap();
    let admission = h
        .engine
        .admit_source_attempt(
            &h.owner,
            h.scope(),
            patch.clone(),
            IdempotencyKey::new(name).unwrap(),
        )
        .unwrap();
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
        .record_source_evidence(
            &h.executor,
            &admission,
            &h.storage,
            evidence.clone(),
            envelope.clone(),
        )
        .unwrap();
    (admission, evidence, envelope)
}
fn commit(h: &Harness, admission: &SourceAdmission, key: &str) -> SourceCommitRequest {
    let candidate = h
        .engine
        .source_publication_candidate(&h.owner, h.scope(), admission.id())
        .unwrap();
    let r = h.authority.inspect_source(h.scope()).unwrap().records;
    let old = &r.analyses[&r.analysis_head.unwrap()];
    let analysis = RepositoryAnalysis::new(
        candidate.snapshot(),
        RepositoryAnalysisInput {
            index_generation: d(key),
            analyzer_version: "finite-process-history/v1".into(),
            configuration: d("process-history-config"),
            elements: old.elements().clone(),
            dependencies: old.dependencies().clone(),
            checks: old.checks().clone(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    h.engine
        .prepare_source_commit_request(
            &h.owner,
            &h.owner,
            h.scope(),
            SourceCommitPreparation {
                admission: admission.id(),
                analysis,
                key: IdempotencyKey::new(key).unwrap(),
            },
        )
        .unwrap()
}
fn check_history(history: History) {
    static RUN: AtomicUsize = AtomicUsize::new(0);
    let ordinal = RUN.fetch_add(1, Ordering::Relaxed);
    let mut h = Harness::configured(&format!("source-history-{}-{ordinal}", history.flags), true);
    let r = h.authority.inspect_source(h.scope()).unwrap().records;
    let original = &r.analyses[&r.analysis_head.unwrap()];
    let analysis = RepositoryAnalysis::new(
        &h.base,
        RepositoryAnalysisInput {
            index_generation: d(&format!("history-{}", history.flags)),
            analyzer_version: "finite-process-history/v1".into(),
            configuration: d("process-history-config"),
            elements: original.elements().clone(),
            dependencies: (0..2)
                .filter(|consumer| history.edge(*consumer, 1 - consumer))
                .map(|consumer| (vertex(consumer), [vertex(1 - consumer)].into()))
                .collect(),
            checks: original.checks().clone(),
            gaps: BTreeSet::new(),
        },
    )
    .unwrap();
    h.engine
        .install_repository_analysis(&h.owner, h.scope(), original.digest(), analysis)
        .unwrap();
    fresh_policy(&mut h);
    let mut attempts = Vec::new();
    let mut requests = Vec::new();
    for index in 0..2 {
        let id = transaction(index);
        h.engine
            .propose(
                &h.owner,
                h.scope(),
                id.clone(),
                h.base.id(),
                TransactionIntent {
                    objective: "generated independent source history".into(),
                    accesses: accesses(history, index),
                    invariants: vec![],
                    required_checks: vec![h.policy.checks()[&check_id()].input().check.clone()],
                    depends_on: BTreeSet::new(),
                },
            )
            .unwrap();
        h.engine.analyze(&h.owner, h.scope(), &id).unwrap();
        h.engine.reserve(&h.owner, h.scope(), &id, 300_000).unwrap();
        h.engine.execute(&h.owner, h.scope(), &id).unwrap();
        let a = run(
            &h,
            index,
            &format!("old-source-{index}"),
            history.output(index, [1, 2]),
        );
        requests.push(commit(&h, &a.0, &format!("commit-{index}")));
        attempts.push(a);
        let r = h.authority.inspect_source(h.scope()).unwrap().records;
        footprint(
            history,
            index,
            r.base.transactions[&id].attempts[0]
                .effective
                .as_ref()
                .unwrap(),
        );
    }
    let directory = directory(&h);
    for (index, file) in ["a.request", "b.request"].into_iter().enumerate() {
        save_request(&h, &directory, &requests[index]);
        std::fs::rename(directory.join("request"), directory.join(file)).unwrap();
    }
    let mut children = [
        child_request(&h, &directory, "a", "commit-0", "race", "a.request"),
        child_request(&h, &directory, "b", "commit-1", "race", "b.request"),
    ];
    let deadline = Instant::now() + Duration::from_secs(60);
    while !["a.ready", "b.ready"]
        .iter()
        .all(|name| directory.join(name).exists())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(25));
    }
    let ready = ["a.ready", "b.ready"]
        .iter()
        .all(|name| directory.join(name).exists());
    std::fs::write(directory.join("go"), b"go").unwrap();
    let statuses: Vec<_> = children.iter_mut().map(finish).collect();
    assert!(
        ready && statuses.iter().all(ExitStatus::success),
        "history child failure: {}",
        directory.display()
    );
    let results =
        ["a.result", "b.result"].map(|name| std::fs::read_to_string(directory.join(name)).unwrap());
    assert_eq!(
        results
            .iter()
            .filter(|r| r.starts_with("accepted:"))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_str() == "rejected:RevalidationRequired")
            .count(),
        1
    );
    let first = usize::from(results[1].starts_with("accepted:"));
    let second = 1 - first;
    let stale =
        h.engine
            .commit_source_prepared(&h.owner, &h.owner, h.scope(), requests[second].clone());
    assert!(matches!(
        stale,
        Err(EngineError::RevalidationRequired { .. })
    ));
    let before = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(before.records.history.len(), 1);
    assert_eq!(
        before.records.history[0].receipt.transaction,
        transaction(first)
    );
    h.base = h.engine.current(&h.owner, h.scope()).unwrap();
    fresh_policy(&mut h);
    let revalidation = h
        .engine
        .prepare_source_revalidation_request(
            &h.owner,
            h.scope(),
            &transaction(second),
            180_000,
            IdempotencyKey::new("fresh-history").unwrap(),
        )
        .unwrap();
    let epoch = h
        .engine
        .revalidate_source_prepared(&h.owner, h.scope(), revalidation.clone())
        .unwrap();
    footprint(history, second, &epoch.accesses);
    assert_eq!(epoch.attempt, 1);
    assert!(epoch.fence > revalidation.fence);
    let mut values = [1, 2];
    values[first] = history.output(first, values);
    let fresh = run(&h, second, "fresh-source", history.output(second, values));
    assert!(
        h.engine
            .record_source_evidence(
                &h.executor,
                &fresh.0,
                &h.storage,
                attempts[second].1.clone(),
                attempts[second].2.clone()
            )
            .is_err()
    );
    let fresh_request = commit(&h, &fresh.0, "fresh-commit");
    let second_receipt = h
        .engine
        .commit_source_prepared(&h.owner, &h.owner, h.scope(), fresh_request.clone())
        .unwrap();
    h.reopen();
    let current = h.engine.current(&h.owner, h.scope()).unwrap();
    assert_eq!(current.revision(), Revision(2));
    let files = effective_workspace(&current).unwrap();
    let ordered = History {
        flags: (history.flags & !1) | first as u16,
    };
    for (index, value) in ordered.serial().into_iter().enumerate() {
        assert_eq!(
            files[path(index)].bytes.as_ref().unwrap(),
            value.to_string().as_bytes()
        );
    }
    let r = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(r.history.len(), 2);
    assert_eq!(r.base.transactions[&transaction(first)].attempts.len(), 1);
    assert_eq!(r.base.transactions[&transaction(second)].attempts.len(), 2);
    assert_eq!(
        h.engine
            .commit_source_prepared(&h.owner, &h.owner, h.scope(), requests[second].clone()),
        stale
    );
    assert_eq!(
        h.engine
            .commit_source_prepared(&h.owner, &h.owner, h.scope(), fresh_request)
            .unwrap(),
        second_receipt
    );
    assert_eq!(
        h.engine
            .revalidate_source_prepared(&h.owner, h.scope(), revalidation)
            .unwrap(),
        epoch
    );
    let s = h.scope();
    let parameters = [
        s.organization.as_str(),
        s.workspace.as_str(),
        s.repository.as_str(),
    ];
    let mut sql = owner();
    for table in [
        "source_committed_changes",
        "transaction_outcomes",
        "outbox",
        "outbox_delivery",
    ] {
        assert_eq!(sql.query_one(&format!("SELECT count(*) FROM celetixo.{table} WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"),&[&parameters[0],&parameters[1],&parameters[2]]).unwrap().get::<_,i64>(0),2,"{table}");
    }
}

#[test]
#[ignore = "requires the reviewed PostgreSQL fixture and retained Linux artifacts"]
fn generated_distinct_source_process_histories_match_independent_serial_interpreter() {
    let mut compatible = 0;
    let mut conflicting = 0;
    for seed in 0..16 {
        let history = History::generated(seed);
        println!(
            "source history seed={seed}, flags={} starting",
            history.flags
        );
        if history.original_conflict() {
            conflicting += 1
        } else {
            compatible += 1
        }
        if std::panic::catch_unwind(|| check_history(history)).is_err() {
            let minimized = history_model::minimize(history, |h| {
                std::panic::catch_unwind(|| check_history(h)).is_err()
            });
            panic!(
                "source SQL history mismatch seed={seed}; history={history:?}; minimized={minimized:?}"
            );
        }
        println!("source history seed={seed}, flags={} passed", history.flags);
    }
    assert!(compatible > 0 && conflicting > 0);
}
