# Milestone 04.2 atomic commit acceptance

**Status:** accepted for the reviewed PostgreSQL native and captured-source
adapters at schema v8. The full Goal 04 objective remains active; 04.3–04.7 are
incomplete. This audit does not establish a production network service or
universal program correctness.

[Final checks](goal-04-source-history-validation/checks.json),
[output](goal-04-source-history-validation/checks.log) and the
[frozen source manifest](goal-04-source-history-validation/source-manifest.json)
record 552 workspace passes (118 environment-dependent cases ignored), 48 generated
Engine source histories, a 16-history real PostgreSQL process corpus and three
related SQL recovery/race parents. Strict Clippy and formatting pass. SQL tests
use optimization with debug assertions and overflow checks enabled; the workspace
and Clippy receipts retain their separately stated profile on the same sources.

The original [04.2 criterion](goal-04.md#042--atomic-distributed-commit) requires a
multi-instance commit protocol, conditional revision advancement, durable receipts
and idempotency, validated by independent Engine processes with real PostgreSQL
contention and faults. Its linearization point is the PostgreSQL transaction that
atomically publishes the Engine-approved outcome and successor revision.

| Criterion | Evidence and boundary |
|---|---|
| Single atomic boundary | The [source SQL boundary](../../crates/postgres/src/authority/source.rs) takes the scoped repository database lock, loads and checks normalized state, evaluates Rust Engine semantics, [conditionally advances head/state generations](../../crates/postgres/src/authority/native/publish.rs), independently reads back the published rows, rechecks final database time/current authority and confirms SQL commit before success. History, terminal outcome, lease release, original result and outbox fact share that transaction. No process mutex supplies authority. |
| Same-base compatible/conflicting histories | The [native process race and 16-seed SQL oracle](../../crates/postgres/tests/foundation/native_authority.rs) cover both outcomes. The [source interpreter](../../crates/engine/tests/support/source_history_model.rs) runs 48 Engine histories and [16 actual PostgreSQL histories](../../crates/postgres/tests/foundation/source_authority/publication_histories.rs) with two distinct source transactions; each SQL history starts independent commit processes and compares final bytes with the independently computed serial interpretation for the observed winning order. |
| Fresh source epochs | Every stale source attempt is rejected before publication. The loser reanalyzes the immutable intent against the committed head, gets a fresh fence, creates an explicit current-base patch from newly observed values and supplies fresh evidence. Old admissions/evidence cannot authorize the new attempt. Even originally compatible source work follows this conservative fresh-attempt policy. |
| Declared dependency inputs in the finite model | Generated source cases vary explicit reads, both dependency directions/cycles, prefix membership, invariant scopes and execution order. A separate numeric reachability matrix checks effective reads/writes/scopes; it does not call Engine analysis/conflict functions. Native reference histories retain the broader generated graph/invariant programme. Unknown/incomplete source facts remain fail-closed. These are finite model tests, not a proof of language extraction completeness. |
| Idempotent original outcomes | Native and source duplicate-key races converge on one original receipt. Distinct source keys on one transaction retain one acceptance and one typed rejection. Changed bodies under one key reject. Generated histories recover old stale rejections and fresh successes after cold reopen without new revisions or renewed authority. |
| Crash ambiguity | Independent source process exits before/after SQL commit respectively roll back the complete checked scope or recover one exact durable result. Native process-death, lost-response and connection-loss records remain separately scoped. A database error is never translated into an invented commit. |
| Final preconditions | Source request, snapshot/base/revision, patch/application, impact/model/index/policy, evidence/storage registration, both current sessions, executor, lease/fence/expiry, cancellation and exact idempotency binding remain checked by the [Engine metadata gate](../../crates/engine/src/engine/source_publication/gate.rs) and final SQL gate. Current authorization is rechecked after unlocked artifact reads. Existing negative/tamper tests remain unchanged. |
| Durable recovery | [04.1 acceptance](goal-04-authority-acceptance.md) establishes normalized native/source reopen and actual local PostgreSQL restart. Multiple source epochs, historical definitions, exact outcomes, reservations, evidence associations and committed history are restored through checked constructors. |

## Source-history interpretation

The independent test model uses two numeric vertices and a Boolean transitive
reachability matrix. It calculates each transaction's read footprint and an
observable numeric source update. The final interpreter applies the accepted
first attempt and the newly executed second attempt in their actual commit order.
The test compares both file contents, revision 2, one committed attempt per
transaction, the loser's separate fresh attempt, exact original replies and two
history/terminal/outbox facts. It rejects delivering the old evidence into the new
attempt. No clean patch application or green synthetic check replaces those checks.

Both memory and SQL runners minimize a failing history by replaying progressively
smaller flag sets. A separate regression injects a stale-read interpreter error
and checks that the minimized counterexample still fails and no one-step reduction
does. The earlier native 192-seed reference suite retains its independent larger
graph generator and structural shrinker.

## Scope and remaining Goal 04 gates

The SQL source cases use real PostgreSQL, independent operating-system processes
and independently verified retained local objects. Their source check records
and finite analyzer facts are synthetic. Actual Linux T1 execution was validated
separately in the preceding source-reader milestone, whose production code is
unchanged here. No actual-T1 source race or new language harness is inferred.

Production workload identity/transport and cross-service protocol integration,
transactional NATS delivery, Temporal workflows, immutable S3 storage/governance,
ClickHouse/shared services, database failover/partitions, host-loss recovery,
backup/restore and load/observability acceptance remain under the original full
Goal 04 contract. A tested local atomic adapter is not a deployed service. Events
and workflows cannot create or replace commit truth.

The accepted [Goal 04 contract](goal-04.md) remains unchanged at SHA-256
`339c328bf93bb512b8f90a8006d3dc07b8ed031399a93c3a3c151f45940b8743`.
