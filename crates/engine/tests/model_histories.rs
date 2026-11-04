//! Generated histories checked against a numeric, independently structured oracle.
//!
//! The oracle uses bit masks and a transitive-reachability matrix, rather than the
//! Engine's access-set intersections or dependency graph traversal. It establishes
//! the declared reference model, not completeness of a source dependency extractor.
use celetixo_engine::access::{AccessSet, Completeness, Condition, Invariant, Predicate};
use celetixo_engine::engine::{Engine, EngineError, InMemoryStore, Permission, Session};
use celetixo_engine::evidence::{
    Artifact, CheckResult, CheckSpec, EvidenceBindings, ExecutionProvenance, VerificationEvidence,
};
use celetixo_engine::ids::{
    CheckId, Digest, ElementId, IdempotencyKey, InvariantId, OrganizationId, PrincipalId,
    RepositoryId, Revision, Scope, TransactionId, WorkspaceId,
};
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::patch::{Change, Patch};
use celetixo_engine::snapshot::{InclusionPolicy, Snapshot, SnapshotManifest, State};
use celetixo_engine::transaction::{TransactionIntent, TransactionState};
use std::collections::{BTreeMap, BTreeSet};

const ELEMENTS: usize = 8;
const EXISTING: usize = 6;

#[derive(Debug, Clone)]
struct Spec {
    reads: u16,
    writes: u16,
    declared_scopes: u16,
    invariant: u16,
    predicates: Vec<String>,
    value: i64,
}

#[derive(Debug, Clone)]
struct History {
    /// For each consumer, a bit mask of its directly declared dependencies.
    graph: [u16; ELEMENTS],
    /// Execution order is generated independently of transaction construction order.
    transactions: Vec<Spec>,
}

#[derive(Clone, Copy)]
struct Footprint {
    reads: u16,
    writes: u16,
    scopes: u16,
}

fn bit(index: usize) -> u16 {
    1 << index
}

fn name(index: usize) -> String {
    format!("group:{}:{index}", index % 3)
}

fn element(index: usize) -> ElementId {
    ElementId::new(name(index)).unwrap()
}

fn members(mask: u16) -> impl Iterator<Item = usize> {
    (0..ELEMENTS).filter(move |index| mask & bit(*index) != 0)
}

fn ids(mask: u16) -> BTreeSet<ElementId> {
    members(mask).map(element).collect()
}

/// Floyd-Warshall is independent of the production analyzer's work-list traversal.
fn oracle_footprint(history: &History, spec: &Spec) -> Footprint {
    let mut reaches = [[false; ELEMENTS]; ELEMENTS];
    for (consumer, dependencies) in history.graph.iter().enumerate() {
        for dependency in members(*dependencies) {
            reaches[consumer][dependency] = true;
        }
    }
    for middle in 0..ELEMENTS {
        for consumer in 0..ELEMENTS {
            for dependency in 0..ELEMENTS {
                reaches[consumer][dependency] |=
                    reaches[consumer][middle] && reaches[middle][dependency];
            }
        }
    }

    let initial = spec.reads | spec.writes | spec.invariant | spec.declared_scopes;
    let mut graph_members = 0;
    for (consumer, dependencies) in history.graph.iter().enumerate() {
        if *dependencies != 0 {
            graph_members |= bit(consumer) | dependencies;
        }
    }
    let mut predicate_roots = 0;
    for index in members(graph_members) {
        if spec
            .predicates
            .iter()
            .any(|prefix| name(index).starts_with(prefix))
        {
            predicate_roots |= bit(index);
        }
    }
    let mut roots = initial | predicate_roots;
    let mut downstream = 0;
    for (consumer, dependencies) in reaches.iter().enumerate() {
        if members(spec.writes).any(|written| dependencies[written]) {
            roots |= bit(consumer);
            downstream |= bit(consumer);
        }
    }
    let mut reads = spec.reads | spec.invariant | downstream | predicate_roots;
    for root in members(roots) {
        for (dependency, reachable) in reaches[root].iter().enumerate() {
            if *reachable {
                reads |= bit(dependency);
            }
        }
    }
    Footprint {
        reads,
        writes: spec.writes,
        scopes: spec.declared_scopes | spec.invariant,
    }
}

/// Enumerate observations per element instead of copying access-set intersection code.
fn oracle_incompatible(
    left: &Spec,
    left_effective: Footprint,
    right: &Spec,
    right_effective: Footprint,
) -> bool {
    for index in 0..ELEMENTS {
        let token = bit(index);
        let left_write = left_effective.writes & token != 0;
        let right_write = right_effective.writes & token != 0;
        let left_observed = (left_effective.reads | left_effective.scopes) & token != 0;
        let right_observed = (right_effective.reads | right_effective.scopes) & token != 0;
        if (left_write && (right_write || right_observed)) || (right_write && left_observed) {
            return true;
        }
        if left_effective.scopes & token != 0 && right_effective.scopes & token != 0 {
            return true;
        }
        let label = name(index);
        if (left_write
            && right
                .predicates
                .iter()
                .any(|prefix| label.starts_with(prefix)))
            || (right_write
                && left
                    .predicates
                    .iter()
                    .any(|prefix| label.starts_with(prefix)))
        {
            return true;
        }
    }
    false
}

fn oracle_decisions(history: &History) -> Vec<bool> {
    let footprints: Vec<_> = history
        .transactions
        .iter()
        .map(|spec| oracle_footprint(history, spec))
        .collect();
    let mut accepted = Vec::new();
    let mut decisions = Vec::new();
    for (index, spec) in history.transactions.iter().enumerate() {
        let valid = accepted.iter().all(|previous: &usize| {
            !oracle_incompatible(
                spec,
                footprints[index],
                &history.transactions[*previous],
                footprints[*previous],
            )
        });
        decisions.push(valid);
        if valid {
            accepted.push(index);
        }
    }
    decisions
}

fn initial_values() -> [Option<i64>; ELEMENTS] {
    std::array::from_fn(|index| if index < EXISTING { Some(0) } else { None })
}

fn state(values: &[Option<i64>; ELEMENTS]) -> State {
    values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| value.map(|value| (element(index), value)))
        .collect()
}

fn with_writes(mut values: [Option<i64>; ELEMENTS], spec: &Spec) -> [Option<i64>; ELEMENTS] {
    for index in members(spec.writes) {
        values[index] = Some(spec.value);
    }
    values
}

fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("model-org").unwrap(),
        workspace: WorkspaceId::new("model-workspace").unwrap(),
        repository: RepositoryId::new("model-repository").unwrap(),
    }
}

fn digest(label: &str) -> Digest {
    Digest::of("model-history-tests/v1", label.as_bytes())
}

fn check_spec() -> CheckSpec {
    CheckSpec::new(
        CheckId::new("nonnegative-state/v1").unwrap(),
        vec!["reference-state-evaluator".into(), "nonnegative/v1".into()],
        "integer-reference/v1".into(),
        digest("environment"),
        digest("nonnegative/v1"),
    )
    .unwrap()
}

/// This test verifier really evaluates its bounded check and retains its output.
/// It executes no shell command and makes no claim about repository-code checks.
fn evidence(
    bindings: EvidenceBindings,
    verifier: &Session,
    values: &[Option<i64>; ELEMENTS],
) -> VerificationEvidence {
    let passed = values.iter().flatten().all(|value| *value >= 0);
    let result = if passed {
        CheckResult::Passed { exit_code: 0 }
    } else {
        CheckResult::Failed { exit_code: 1 }
    };
    let output = format!("nonnegative={passed}; state={values:?}");
    VerificationEvidence::new(
        bindings.clone(),
        check_spec(),
        result,
        ExecutionProvenance {
            verifier: verifier.principal().clone(),
            runner: "test-integer-evaluator/v1".into(),
            run_id: format!("{}:{}", bindings.transaction, bindings.attempt),
            started_at: 0,
            finished_at: 0,
        },
        vec![],
        vec![Artifact::new("numeric-check.txt", "text/plain", output.into_bytes()).unwrap()],
    )
    .unwrap()
}

#[derive(Default)]
struct Counts {
    accepted: usize,
    rejected: usize,
    revalidated: usize,
}

fn ensure(condition: bool, detail: impl FnOnce() -> String) -> Result<(), String> {
    if condition { Ok(()) } else { Err(detail()) }
}

fn result<T>(result: Result<T, EngineError>, operation: &str) -> Result<T, String> {
    result.map_err(|error| format!("{operation}: {error:?}"))
}

fn check_history(history: &History) -> Result<Counts, String> {
    let (store, administrator) = InMemoryStore::new();
    let engines = [Engine::new(store.clone()), Engine::new(store)];
    let scope = scope();
    let actor = administrator
        .grant(
            PrincipalId::new("model-actor").unwrap(),
            scope.clone(),
            Permission::ALL
                .into_iter()
                .filter(|permission| *permission != Permission::Verify)
                .collect(),
        )
        .unwrap();
    let verifier = administrator
        .grant(
            PrincipalId::new("model-verifier").unwrap(),
            scope.clone(),
            [Permission::Read, Permission::Verify].into(),
        )
        .unwrap();
    let original = Snapshot::new(
        scope.clone(),
        Revision(0),
        SnapshotManifest {
            base_commit: "model-base".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "generated-history-capturer/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        state(&initial_values()),
    )
    .unwrap();
    let model = EngineeringModel {
        generation: 1,
        analyzer: "declared-integer-model".into(),
        version: "1".into(),
        configuration: digest("model-config"),
        dependencies: history
            .graph
            .iter()
            .enumerate()
            .filter(|(_, dependencies)| **dependencies != 0)
            .map(|(index, dependencies)| (element(index), ids(*dependencies)))
            .collect(),
        completeness: Completeness::Complete,
    };
    result(
        engines[0].register(&actor, original.clone(), model),
        "register",
    )?;

    // Register in reverse execution order. Every transaction sees one immutable base;
    // no prior accepted transaction can leak into its initial analysis.
    for (index, spec) in history.transactions.iter().enumerate().rev() {
        let id = TransactionId::new(format!("generated:{index}")).unwrap();
        let invariants = if spec.invariant == 0 {
            vec![]
        } else {
            vec![Invariant {
                id: InvariantId::new("bounded-sum").unwrap(),
                condition: Condition::SumAtMost {
                    elements: ids(spec.invariant),
                    maximum: 10_000,
                },
            }]
        };
        let intent = TransactionIntent {
            objective: "Evaluate a generated semantic transaction history".into(),
            accesses: AccessSet {
                reads: ids(spec.reads),
                writes: ids(spec.writes),
                predicates: spec
                    .predicates
                    .iter()
                    .cloned()
                    .map(Predicate::Prefix)
                    .collect(),
                invariant_scopes: ids(spec.declared_scopes),
            },
            invariants,
            required_checks: vec![check_spec()],
            depends_on: BTreeSet::new(),
        };
        let engine = &engines[index % engines.len()];
        result(
            engine.propose(&actor, &scope, id.clone(), original.id(), intent),
            "propose",
        )?;
        result(engine.analyze(&actor, &scope, &id), "analyze")?;
    }

    let decisions = oracle_decisions(history);
    let mut expected_values = initial_values();
    let mut expected_revision = 0;
    let mut counts = Counts::default();
    for (index, spec) in history.transactions.iter().enumerate() {
        let engine = &engines[index % engines.len()];
        let id = TransactionId::new(format!("generated:{index}")).unwrap();
        result(engine.reserve(&actor, &scope, &id, 100_000), "reserve")?;
        result(engine.execute(&actor, &scope, &id), "execute")?;
        let changes = members(spec.writes)
            .map(|index| {
                (
                    element(index),
                    Change {
                        expected: initial_values()[index],
                        value: Some(spec.value),
                    },
                )
            })
            .collect();
        let patch = Patch::new(original.id(), changes).unwrap();
        let initial_binding = result(
            engine.submit_patch(&actor, &scope, &id, patch),
            "submit patch",
        )?;
        result(
            engine.record_evidence(
                &verifier,
                &scope,
                &id,
                evidence(
                    initial_binding,
                    &verifier,
                    &with_writes(initial_values(), spec),
                ),
            ),
            "record initial evidence",
        )?;

        let before = result(engine.current(&actor, &scope), "read before commit")?;
        let initial_key = IdempotencyKey::new(format!("commit:{index}:initial")).unwrap();
        let mut committed = engine.commit(&actor, &scope, &id, initial_key.clone());
        let key;
        if expected_revision != 0 {
            ensure(
                matches!(committed, Err(EngineError::RevalidationRequired { .. })),
                || {
                    format!(
                        "transaction {index}: stale commit did not require revalidation: {committed:?}"
                    )
                },
            )?;
            ensure(
                result(engine.current(&actor, &scope), "read after stale rejection")? == before,
                || format!("transaction {index}: stale rejection mutated repository"),
            )?;
            let revalidated = engine.revalidate(&actor, &scope, &id);
            if !decisions[index] {
                ensure(
                    matches!(&revalidated, Err(EngineError::Conflict(conflicts)) if !conflicts.is_empty()),
                    || {
                        format!(
                            "transaction {index}: expected incompatible history rejection, got {revalidated:?}"
                        )
                    },
                )?;
                ensure(
                    result(engine.current(&actor, &scope), "read after conflict")? == before,
                    || format!("transaction {index}: rejected revalidation mutated repository"),
                )?;
                result(
                    engine.abort(&actor, &scope, &id, "oracle conflict".into()),
                    "abort rejected attempt",
                )?;
                counts.rejected += 1;
                continue;
            }
            let bindings = result(revalidated, "revalidate compatible history")?;
            ensure(bindings.attempt == 1, || {
                format!("transaction {index}: revalidation did not version attempt")
            })?;
            ensure(bindings.base_snapshot == before.id(), || {
                format!("transaction {index}: revalidation kept stale base")
            })?;
            let fresh_key = IdempotencyKey::new(format!("commit:{index}:revalidated")).unwrap();
            let missing = engine.commit(
                &actor,
                &scope,
                &id,
                IdempotencyKey::new(format!("missing:{index}")).unwrap(),
            );
            ensure(
                matches!(missing, Err(EngineError::MissingEvidence(_))),
                || format!("transaction {index}: fresh attempt accepted old evidence: {missing:?}"),
            )?;
            result(
                engine.record_evidence(
                    &verifier,
                    &scope,
                    &id,
                    evidence(bindings, &verifier, &with_writes(expected_values, spec)),
                ),
                "record fresh evidence",
            )?;
            committed = engine.commit(&actor, &scope, &id, fresh_key.clone());
            key = fresh_key;
            counts.revalidated += 1;
        } else {
            key = initial_key;
        }
        ensure(decisions[index], || {
            format!("transaction {index}: oracle unexpectedly rejected first commit")
        })?;
        let receipt = result(committed, "commit expected compatible transaction")?;
        expected_revision += 1;
        expected_values = with_writes(expected_values, spec);
        let after = result(engine.current(&actor, &scope), "read after commit")?;
        ensure(after.revision() == Revision(expected_revision), || {
            format!("transaction {index}: revision did not advance exactly once")
        })?;
        ensure(after.state() == &state(&expected_values), || {
            format!("transaction {index}: committed state differs from independent oracle")
        })?;
        ensure(receipt.snapshot == after.id(), || {
            format!("transaction {index}: receipt does not identify current snapshot")
        })?;
        ensure(
            result(
                engine.transaction(&actor, &scope, &id),
                "read committed transaction",
            )?
            .state
                == TransactionState::Committed,
            || format!("transaction {index}: published transaction is not terminal committed"),
        )?;
        ensure(
            result(engine.commit(&actor, &scope, &id, key), "idempotent replay")? == receipt,
            || format!("transaction {index}: replay changed recorded outcome"),
        )?;
        ensure(
            result(engine.current(&actor, &scope), "read after replay")? == after,
            || format!("transaction {index}: replay changed state or revision"),
        )?;
        counts.accepted += 1;
    }
    Ok(counts)
}

/// Fixed xorshift64 schedule. A failure reports the seed plus the minimized history.
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit as u64) as usize
    }
    fn one_in(&mut self, count: usize) -> bool {
        self.below(count) == 0
    }
}

fn generated(seed: u64) -> History {
    let mut random = Random(seed ^ 0x9e37_79b9_7f4a_7c15);
    let mut graph = [0; ELEMENTS];
    if !seed.is_multiple_of(3) {
        for (consumer, dependencies) in graph.iter_mut().enumerate() {
            for dependency in 0..ELEMENTS {
                if consumer != dependency && random.one_in(17) {
                    *dependencies |= bit(dependency);
                }
            }
        }
    }
    let count = 2 + random.below(8);
    let mut transactions = Vec::new();
    for _ in 0..count {
        let mut spec = Spec {
            reads: 0,
            writes: bit(random.below(ELEMENTS)),
            declared_scopes: 0,
            invariant: 0,
            predicates: vec![],
            value: 1 + random.below(32) as i64,
        };
        if random.one_in(3) {
            spec.writes |= bit(random.below(ELEMENTS));
        }
        for index in 0..ELEMENTS {
            if random.one_in(7) {
                spec.reads |= bit(index);
            }
        }
        if random.one_in(4) {
            spec.declared_scopes = bit(random.below(ELEMENTS));
        }
        if random.one_in(4) {
            spec.invariant = bit(random.below(EXISTING)) | bit(random.below(EXISTING));
        }
        if random.one_in(4) {
            spec.predicates.push(if random.one_in(6) {
                String::new()
            } else {
                format!("group:{}:", random.below(3))
            });
        }
        transactions.push(spec);
    }
    for index in (1..transactions.len()).rev() {
        let other = random.below(index + 1);
        transactions.swap(index, other);
    }
    History {
        graph,
        transactions,
    }
}

/// Greedy structural shrinker. Every accepted reduction is replayed against the
/// mismatch predicate. It removes transactions, individual accesses, predicates,
/// graph edges and unnecessarily large write values until no reduction remains.
fn minimize(mut history: History, mismatch: impl Fn(&History) -> bool) -> History {
    assert!(mismatch(&history), "shrinker requires an actual mismatch");
    loop {
        let mut variants = Vec::new();
        for index in 0..history.transactions.len() {
            let mut smaller = history.clone();
            smaller.transactions.remove(index);
            variants.push(smaller);
        }
        for index in 0..history.transactions.len() {
            let spec = &history.transactions[index];
            for access in 0..4 {
                let mask = match access {
                    0 => spec.reads,
                    1 => spec.writes,
                    2 => spec.declared_scopes,
                    _ => spec.invariant,
                };
                if access == 1 && mask.count_ones() == 1 {
                    continue;
                }
                for member in members(mask) {
                    let mut smaller = history.clone();
                    let spec = &mut smaller.transactions[index];
                    let target = match access {
                        0 => &mut spec.reads,
                        1 => &mut spec.writes,
                        2 => &mut spec.declared_scopes,
                        _ => &mut spec.invariant,
                    };
                    *target &= !bit(member);
                    variants.push(smaller);
                }
            }
            for predicate in 0..spec.predicates.len() {
                let mut smaller = history.clone();
                smaller.transactions[index].predicates.remove(predicate);
                variants.push(smaller);
            }
            if spec.value > 1 {
                let mut smaller = history.clone();
                smaller.transactions[index].value = 1;
                variants.push(smaller);
            }
        }
        for consumer in 0..ELEMENTS {
            for dependency in members(history.graph[consumer]) {
                let mut smaller = history.clone();
                smaller.graph[consumer] &= !bit(dependency);
                variants.push(smaller);
            }
        }
        if let Some(smaller) = variants.into_iter().find(|smaller| mismatch(smaller)) {
            history = smaller;
        } else {
            return history;
        }
    }
}

#[test]
fn deterministic_generated_histories_match_independent_oracle() {
    let mut totals = Counts::default();
    for seed in 0..192 {
        let history = generated(seed);
        match check_history(&history) {
            Ok(counts) => {
                totals.accepted += counts.accepted;
                totals.rejected += counts.rejected;
                totals.revalidated += counts.revalidated;
            }
            Err(mismatch) => {
                let minimized =
                    minimize(history.clone(), |smaller| check_history(smaller).is_err());
                panic!("seed={seed}; {mismatch}\nminimized={minimized:#?}\noriginal={history:#?}");
            }
        }
    }
    assert!(
        totals.accepted > 192,
        "generated suite must include compatible concurrent work"
    );
    assert!(
        totals.rejected > 0,
        "generated suite must exercise conflict rejection"
    );
    assert!(
        totals.revalidated > 0,
        "generated suite must exercise fresh revalidation"
    );
}

#[test]
fn shrinking_preserves_and_minimizes_an_injected_faulty_oracle_mismatch() {
    let mut history = History {
        graph: [0; ELEMENTS],
        transactions: vec![
            Spec {
                reads: bit(3),
                writes: bit(0),
                declared_scopes: bit(5),
                invariant: 0,
                predicates: vec!["unused:".into()],
                value: 7,
            },
            Spec {
                reads: 0,
                writes: bit(2),
                declared_scopes: 0,
                invariant: 0,
                predicates: vec![],
                value: 9,
            },
            Spec {
                reads: bit(4),
                writes: bit(0),
                declared_scopes: 0,
                invariant: 0,
                predicates: vec![],
                value: 5,
            },
        ],
    };
    history.graph[4] = bit(6);
    // Inject a safety defect: the faulty oracle accepts every transaction, ignoring
    // conflicts. The shrinker must retain the conflicting pair and drop decoration.
    let faulty_mismatch = |candidate: &History| {
        oracle_decisions(candidate) != vec![true; candidate.transactions.len()]
    };
    let smaller = minimize(history.clone(), faulty_mismatch);
    assert_eq!(smaller.transactions.len(), 2);
    assert!(faulty_mismatch(&smaller));
    assert!(smaller.graph.iter().all(|dependencies| *dependencies == 0));
    assert!(smaller.transactions.iter().all(|spec| {
        spec.reads == 0
            && spec.writes.count_ones() == 1
            && spec.declared_scopes == 0
            && spec.invariant == 0
            && spec.predicates.is_empty()
            && spec.value == 1
    }));
    for index in 0..smaller.transactions.len() {
        let mut without_one = smaller.clone();
        without_one.transactions.remove(index);
        assert!(!faulty_mismatch(&without_one));
    }
    assert_eq!(oracle_decisions(&smaller), [true, false]);
    check_history(&smaller).unwrap();
}

#[test]
fn phantom_insertions_and_shared_read_dependencies_have_distinct_outcomes() {
    let writer = |index| Spec {
        reads: bit(5),
        writes: bit(index),
        declared_scopes: 0,
        invariant: 0,
        predicates: vec![],
        value: 1,
    };
    let compatible = History {
        graph: [0; ELEMENTS],
        transactions: vec![writer(0), writer(1)],
    };
    assert_eq!(oracle_decisions(&compatible), [true, true]);
    let counts = check_history(&compatible).unwrap();
    assert_eq!((counts.accepted, counts.revalidated), (2, 1));

    let mut phantom = compatible;
    phantom.transactions[0].predicates = vec!["group:0:".into()];
    phantom.transactions[1].writes = bit(6); // Absent at the shared base snapshot.
    assert_eq!(oracle_decisions(&phantom), [true, false]);
    assert_eq!(check_history(&phantom).unwrap().rejected, 1);
}

#[test]
fn declared_invariant_scopes_depend_on_their_graph_inputs() {
    let mut history = History {
        graph: [0; ELEMENTS],
        transactions: vec![
            Spec {
                reads: 0,
                writes: bit(0),
                declared_scopes: bit(1),
                invariant: 0,
                predicates: vec![],
                value: 1,
            },
            Spec {
                reads: 0,
                writes: bit(2),
                declared_scopes: 0,
                invariant: 0,
                predicates: vec![],
                value: 2,
            },
        ],
    };
    // The declared scope at element 1 consumes element 2. Neither transaction
    // directly reads element 2 or writes the invariant scope itself.
    history.graph[1] = bit(2);
    assert_eq!(oracle_decisions(&history), [true, false]);
    let counts = check_history(&history).unwrap();
    assert_eq!((counts.accepted, counts.rejected), (1, 1));
    history.graph[1] = 0;
    assert_eq!(oracle_decisions(&history), [true, true]);
    assert_eq!(check_history(&history).unwrap().accepted, 2);
}
