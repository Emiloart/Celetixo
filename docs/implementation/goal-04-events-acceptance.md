# Milestone 04.3 lifecycle event acceptance

**Status:** accepted for the reviewed local PostgreSQL 17.9 / three-node NATS
2.15.3 profile at schema v9. Full Goal 04 remains active. This audit establishes
the original [04.3 criterion](goal-04.md#043--transactional-outbox-and-nats-jetstream)
within that profile; it does not establish production deployment or whole-platform
operational acceptance.

| Criterion | Implemented boundary and actual evidence |
|---|---|
| Durable lifecycle outbox | Native and captured-source authority publish typed immutable facts and canonical frames in the same SQL transaction as Engine state. Schema-v8 upgrade preserves original commits and explicitly bounds historical coverage. [Earlier receipts](goal-04-events-validation/checks.json) include real upgrade and source lifecycle checks. |
| Recoverable publication | Scoped nonadministrative publisher roles, bounded expiring fenced claims and matching PubAck checks. Independent processes exit before/after broker acknowledgement and SQL publication marking. Engine commits succeed during tested broker quorum loss; backlog recovers after restart. |
| Idempotent consumers | Authenticated scope/consumer bindings, exact authoritative frame validation and SQL inbox/projection effects before ACK. Duplicate and reordered replay cannot replace newer projections or repeat Engine commits. Consumer exits before/after SQL commit exercise recovery. |
| Replay, retention and incompatibility | Finite broker deduplication, actual retention eviction, replay and SQL reconciliation are tested. Unsupported/substituted frames and exhausted claims are quarantined. Full broker restart, leader SIGKILL, quorum loss and role/transport rejection are recorded. |
| Regression and worker operation | [Supplemental receipts](goal-04-events-acceptance/checks.json) retain the full initial SQL run, corrected exact parents, strict Clippy and formatting. Separate publisher/consumer processes reach readiness and stop successfully on SIGTERM without changing authoritative revision or generation. |

The complete opt-in foundation run at production commit
`8923d9164b99a8e7387bba5b5e133a08af83bf00` finished with **103 passed and four
failed** in 4,792 seconds. Three test fixtures copied stored generated columns,
causing PostgreSQL to reject their INSERTs before the intended constraint or role
check. The fourth privileged fixture omitted lifecycle facts required for a
coherent schema-v9 future-clock publication. These fixture constructors are now
corrected; all four exact parents pass with their original SQLSTATE, integrity and
clock-regression assertions preserved. Production Rust, canonical protocol and
migrations are unchanged.

This supplies **107 distinct passing SQL parents across the two runs**. The full
80-minute command was not repeated after fixture-only changes. Its original
exit-101 receipt and raw output remain published; this is not a claim of one green
full-suite invocation. The seven filtered cases are process helpers or ordinary
tests excluded by the opt-in command. Generated native and captured-source
histories completed in the full run, including the independent source-process
corpus. All debug assertions and overflow checks remained enabled.

For reproduction, use the [local event runbook](../operations/local-event-delivery-runbook.md)
and fixture preparation, then run the exact SQL/check commands in the receipts.
Use the parent commit for the original full run and the supplemental
[source manifest](goal-04-events-acceptance/source-manifest.json) for corrected
parents. Standalone smoke checks invoke `celetixo-events --config` with separate
private publisher/consumer configurations, wait for JSON `ready`, send SIGTERM,
require exit zero and final `stopped`, and compare scoped authoritative revision
and generation before/after. Private configurations and credentials are excluded.
These worker smokes check process operation; independent crash/replay integration
checks establish the stronger delivery behavior.

The earlier record separately supplies 556 ordinary workspace passes, six selected
real PostgreSQL parents, strict build/protocol checks and real three-node fault
and security tests. Its frozen evidence remains unchanged. Hosted CI and actual T1
execution were not rerun here. Production workload identity/transport, database
failover, host power loss, full-service observability/alerts, backup/restore and
load acceptance remain open. No exactly-once messaging, zero-loss guarantee or
production capacity claim is made. Continue 04.4 Temporal, 04.5 immutable object
storage/governance, 04.6 analytics/shared services and 04.7 integrated operations
under the unchanged [full Goal 04 contract](goal-04.md).
