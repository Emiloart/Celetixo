# Transactional lifecycle events and JetStream delivery

**Status:** accepted for the reviewed local schema-v9 PostgreSQL/NATS profile.
The [acceptance audit](goal-04-events-acceptance.md) adds combined coverage of all
107 opt-in SQL parents and standalone publisher/consumer operation to the earlier
[fault checks](goal-04-events-validation/checks.json). The full Goal 04 remains
active. Receipts distinguish the initial failed full run from four corrected
exact-parent passes; no single green full-suite run is claimed.

## Authority and delivery

Native and captured-source publication write immutable lifecycle facts and their
canonical Protobuf frames in the same SQL transaction as authoritative state.
Existing commit event IDs, receipt digests and revision sequences retain their
meanings. Transaction changes, source attempt changes, original mutation outcomes
and authority generations have distinct typed aggregates. Sequences may have gaps;
Engine attempt numbers start at zero. Events carry bounded metadata and digests,
not source, patches, prompts, credentials or complete mutation outcomes.

The sole wire source is
[lifecycle_event.proto](../../proto/celetixo/platform/v1/lifecycle_event.proto).
Generated bindings and strict descriptor metadata reject unknown fields, enum
values, versions and ambiguous fact combinations. The compatibility checker now
supports closed enums while preserving all published TaskIntent semantics.
Services and Go/Python consumers remain outside this change.

[Migration 0009](../../crates/postgres/migrations/0009_lifecycle_delivery.sql)
adds immutable frame associations, an explicit upgrade origin, fenced delivery
claims, scoped projections/quarantine and administrator-owned role/OID bindings.
Migrations 0001–0008 remain byte-identical. Checked post-upgrade publication frames
old commit facts and current state without inventing pre-upgrade lifecycle history.
The frozen synthetic [v8 fixture](../../crates/postgres/tests/fixtures/published-v8-native.json)
was produced by the published v8 adapter, not the new v9 constructors.

The [SQL delivery adapter](../../crates/postgres/src/delivery.rs) uses separate
authenticated publisher and consumer logins. It checks scope, service identity,
role OID and inherited/table/column privileges on each operation. Event workloads
cannot own application objects, mutate Engine truth or administer role mappings.
Forced RLS restricts both repository and consumer identity. A shared advisory lock
stabilizes consumer metadata effects against Engine publication; it grants no
write permission on Engine tables. Checked reconstruction validates complete
outbox/frame/delivery/inbox/projection associations and quarantine body digests.

Publishers claim bounded batches with database-time expiry, increasing fences,
SKIP LOCKED, finite attempts and backoff. Unsupported immutable frames and exhausted
claims persist explicit quarantine status. The
[NATS adapter](../../crates/events/src/lib.rs) waits for a matching JetStream
PubAck before conditionally marking that exact live claim published. A lost PubAck
or database reply can cause duplicate delivery, never another Engine commit.

Consumers require an exact authoritative immutable frame and check current
transaction/attempt/revision bounds. Inbox insertion and a monotonic derived
metadata projection commit atomically before explicit broker acknowledgement.
Older or duplicate facts cannot replace a newer aggregate projection. This is
idempotent SQL metadata processing; it does not grant exactly-once messaging or
arbitrary external side effects. Unsupported/substituted inputs are durably
quarantined before terminal acknowledgement. Transient database failure causes a
delayed negative acknowledgement. Bounded reconciliation reads missing frames
from durable SQL independently of broker retention, cursor and deduplication window.

## Implemented operational boundary

The `celetixo-events --config /absolute/private/service.json` worker has private,
versioned operator configuration, bounded batches/reconnects/retries, fixed-key
numeric status output, periodic consumer reconciliation and signal-driven drain.
Its configuration reader rejects public files, symlinks, hardlinks, unknown fields
and oversized input. The [runbook](../operations/local-event-delivery-runbook.md)
describes credentials, grants, stream settings and replay. Separate standalone
publisher and consumer processes reached readiness, handled SIGTERM and exited
successfully with unchanged authoritative revision and generation.

The local fixture uses pinned NATS 2.15.3 and async-nats 0.52.0, client mutual TLS,
a separate route CA, distinct publisher/consumer/operator identities and an
isolated foreign account. Three bounded containers use file storage and
`sync_interval: always`, without published host ports. Brokers mount only their
own keys and public trust anchors. The test stream uses three replicas, 8 MiB,
10,000 messages, one-hour age, a 1 MiB message limit and a two-second deduplication
window. The latter is deliberately finite so tests exercise duplicates beyond it.
The explicit-ACK durable consumer has max delivery 8, pending ACKs 16 and batch 8.
These settings establish a test profile, not production capacity or zero loss.

## Actual selected evidence and limits

Six selected SQL parents cover fences/replay/reordering, role/consumer isolation,
poison/exhausted claims, a published v8 commit upgrade, source lifecycle
replay through committed state, and actual three-node JetStream delivery. All 556
ordinary workspace tests pass (125 environment-dependent cases ignored); strict
Clippy, formatting, canonical regeneration/compatibility and workflow checks pass. The
broker parent includes four independent publisher/consumer process-exit points,
duplicates beyond the deduplication window, replay/reordering, poison quarantine,
retention eviction and SQL reconciliation, scoped account/role rejection, leader
SIGKILL, quorum loss, a successful Engine commit while quorum is unavailable,
full broker stop/restart and exact original commit recovery.

Fixture verification separately rejects wrong CA, wrong hostname, missing client
certificate and client certificates at the cluster route boundary; a valid route
certificate is the positive control. Raw check output and source hashes accompany
the receipts. These are Linux/WSL local-service tests, not hosted CI results.

The [supplemental acceptance checks](goal-04-events-acceptance/checks.json) cover
the complete SQL parent set across the original 103 passes and four corrected
fixture reruns. Production Rust and migrations are unchanged by those fixes.

Production workload identity/transport, full-service
observability/alerts, partitions beyond tested broker quorum loss, database
failover, host power loss, backup/restore, load acceptance and malicious host
administrators are not established here. Actual T1 checks were not rerun for this
event change. Temporal (04.4), immutable S3/governance (04.5), analytics/shared
services (04.6) and the integrated operational programme (04.7) remain active under
the unchanged [Goal 04 contract](goal-04.md).
