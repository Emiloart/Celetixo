# Distributed Failure and Recovery Model

**Status:** Normative subsystem specification  
**Authority:** ADR-0001, ADR-0002 and ADR-0003  
**Purpose:** Define failure cases and recovery obligations across transactions, workers, orchestration, events, storage and execution.

## 1. Failure assumptions

Processes crash. Networks partition. Calls time out after a remote side effect succeeds. Events are duplicated, delayed and reordered. Workers may be malicious or buggy. Indexes may lag. Storage may be unavailable. A component may recover with incomplete local memory.

Correctness must depend on durable authoritative state and explicit identities, not process-local assumptions or event ordering.

## 2. Failure domains

Model failures independently across:
- client and API gateway;
- Python Brain and model providers;
- Go/Temporal workflows;
- NATS JetStream consumers and publishers;
- Rust Engine instances;
- PostgreSQL authoritative state;
- ClickHouse analytics;
- object storage;
- language servers/indexers;
- sandbox workers and remote execution pools;
- desktop/mobile clients.

A failure in analytics or UI must not mutate transaction truth. Event loss or delay must not reverse a durable commit.

## 3. Transaction and commit failures

Inject failures:
- before validation;
- during validation;
- immediately before the atomic commit operation;
- during the database transaction/CAS;
- after durable commit but before response;
- after commit but before event publication;
- during idempotency-record persistence;
- during recovery/reconciliation.

Required behavior:
- exactly one authoritative outcome per idempotency key;
- no partially advanced repository revision;
- no commit inferred from a worker's local success;
- lost responses recover the durable outcome;
- outbox/reconciliation republishes missing events without repeating the commit.

## 4. Worker and reservation failures

Test worker termination, heartbeat loss, lease expiry, renewal races, delayed messages from expired owners, reservation upgrades, partial acquisition and cancellation.

An expired or revoked worker must not commit. A replacement worker receives explicit authority for a new attempt and the correct immutable snapshot. Worker identity is attribution and routing context, not authorization.

## 5. Event and workflow failures

Test:
- duplicate delivery;
- out-of-order delivery;
- delayed delivery after terminal state;
- consumer restart and replay;
- publisher crash before/after persistence;
- NATS unavailable or backpressured;
- Temporal worker restart;
- workflow retry after external side effect;
- cancellation racing with completion;
- poison messages and schema incompatibility.

Consumers must be idempotent and validate event identity, transaction state and revision before acting. Event publication is not the commit linearization point.

## 6. Index and verification failures

Test:
- stale index generation;
- parser/analyzer upgrade during active transactions;
- partial language coverage;
- indexer crash during incremental update;
- LSP response for an obsolete document version;
- verification running against a snapshot different from the patch base;
- toolchain/configuration drift;
- artifact upload failure;
- cache poisoning or incorrect cache key;
- verification completion arriving after abort or supersession.

Stale or incomplete results cannot silently satisfy current commit requirements. Recompute, reject, or escalate according to policy.

## 7. Rebase and recovery failures

Test interrupted rebase, conflicting rebase, index update during rebase, worker replacement mid-rebase and evidence invalidation. Recovery must either resume from a durable checkpoint with validated assumptions or restart analysis. It must not preserve unverified assumptions for convenience.

## 8. Storage and analytics failures

PostgreSQL unavailability must prevent authoritative state advancement when durable commit cannot be established. ClickHouse unavailability may delay analytics without changing transaction outcome. Object-store failure must follow an explicit evidence durability policy; required evidence must not be declared complete before durable persistence is confirmed.

Define reconciliation for orphaned objects, missing references, duplicate uploads and delayed analytical ingestion.

## 9. Security failures

Test sandbox escape assumptions, unauthorized API actions, cross-tenant object access, credential exposure, malicious dependency lifecycle scripts, excessive resource use, network egress policy violations and compromised workers. Containers and microVMs have distinct threat models; isolation claims must be tested against the selected runtime and configuration.

## 10. Required test harness

The failure harness must support:
- deterministic clocks;
- controllable scheduling and message delivery;
- network/storage fault injection;
- process termination at named points;
- duplicate/reorder/drop event controls;
- reproducible seeds;
- state snapshot and invariant checks;
- minimized regression fixtures.

Every discovered failure becomes a regression test. Record expected durable state, observed state, recovery action and whether the system fails closed.
