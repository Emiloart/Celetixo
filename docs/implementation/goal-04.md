# Goal 04 — Durable Orchestration and Distributed Commit Coordination

**Status:** Accepted implementation objective; not yet implemented or validated as a complete goal  
**Depends on:** Goals 01–03; ADR-0001 through ADR-0004; transaction, identity, failure, threat and deployment specifications  
**Primary authority:** Rust Engineering State Engine, backed by durable transactional state

## 1. Objective

Deliver durable platform services and a distributed commit protocol without weakening the semantic transaction contract established by the reference Engine.

Multiple Engine instances must admit, reserve, validate and commit transactions against the same repository scope without shared process memory or a process-local mutex. A commit must have one durable linearization point. A crash or lost response must resolve to one unambiguous outcome. Workflow and event infrastructure must coordinate work without becoming an alternative source of transaction truth.

The goal establishes durable storage boundaries for evidence and artifacts, operational recovery, and the service integration required to run beyond one local process.

**Core invariant:** PostgreSQL records authoritative transaction and repository state; the Rust Engine owns transaction semantics and makes the authoritative decision; the database transaction is the atomic distributed linearization point. Temporal coordinates durable workflows. NATS distributes events. Object storage holds immutable content. ClickHouse holds derived analytics. None may impersonate another.

This is a platform-integration goal, not permission to replace the Engine with SQL business logic or let workflow/event systems decide commits.

## 2. Required architecture and authority boundaries

### 2.1 Rust Engineering State Engine

- Remains the only component that evaluates transaction admissibility, semantic conflicts, invariant preservation, evidence applicability and revalidation.
- Exposes explicit repository/transaction operations through versioned typed interfaces.
- Is stateless between requests where practical; cached state is reconstructible and never authoritative.
- Loads authoritative state and performs final validation within a database transaction or a rigorously specified optimistic protocol that guarantees equivalent atomicity.
- Must not acknowledge commit before durable commit is confirmed.
- Reconciles ambiguous database responses by idempotency and transaction identity, never by blindly repeating a side effect.

### 2.2 PostgreSQL — authoritative state

PostgreSQL is the durable authority for:
- organizations, principals, workspaces, repositories and authorization/policy versions;
- repository revision heads and immutable snapshot references;
- transaction lifecycle, attempts and terminal outcomes;
- reservations, leases, fencing generations and ownership;
- idempotency keys and canonical request digests;
- commit records and revision advancement;
- verification/evidence admission receipts and immutable artifact metadata;
- workflow correlation and transactional outbox/inbox state;
- schema/migration version and operational recovery checkpoints.

Requirements:
- Use database-enforced uniqueness, foreign keys, check constraints and transactional invariants where they strengthen correctness.
- Commit validation and repository revision advancement share one atomic transaction boundary. A compare-and-swap update is conditional on the expected revision and relevant state; zero affected rows is a conflict/revalidation outcome, not success.
- Enforce one terminal outcome per transaction, attempt and idempotency identity.
- Use explicit isolation/locking semantics, documented lock ordering, bounded transactions and retry classification. Serializable isolation alone does not make semantic validation correct; the Engine still evaluates the declared dependency model.
- Multi-instance races are tested against a real PostgreSQL instance with independent Engine processes/connections.
- Migrations are versioned, reproducible, restart-safe and tested from empty and prior supported schemas. Destructive migrations require an explicit data migration/rollback strategy.
- No in-memory fallback for authoritative state when PostgreSQL is unavailable.

### 2.3 Distributed commit protocol

The protocol must define:
1. Request authentication and scope resolution.
2. Idempotency key and canonical request digest validation.
3. Durable transaction/attempt lookup and state validation.
4. Current revision, snapshot, policy, index and evidence reference loading.
5. Engine semantic validation against current authoritative state.
6. Atomic recheck of mutable preconditions and revision advancement.
7. Durable commit record and outbox event insertion in the same database transaction.
8. Response only after commit confirmation.
9. Reconciliation when the response is lost or the database outcome is initially unknown.

The commit linearization point is the PostgreSQL transaction that atomically records the accepted outcome and advances the repository revision. No worker response, Temporal completion, NATS event, local file write or API response is the linearization point.

The transaction rechecks at minimum: expected repository revision, transaction state, immutable base snapshot, effective read/write and invariant assumptions, patch digest, admitted evidence, authorization/policy version, reservation/fencing token where required, expiry/cancellation state and idempotency digest.

The implementation must prove that independent Engine instances racing on compatible and conflicting histories cannot produce duplicate advancement, lost updates or an outcome inconsistent with the reference model.

### 2.4 Reservations and leases

- Persist reservation ownership, scope, expiry, renewal, release and fencing generation.
- Use monotonically increasing fencing tokens or an equivalent database-verifiable generation so an expired owner cannot act after a replacement acquires authority.
- Lease timestamps use an explicit authoritative clock policy. Clock skew is not treated as harmless.
- Reservation acquisition/upgrade is atomic for the requested scope and has deterministic ordering/deadlock handling.
- Reservations reduce wasted work; they do not replace semantic commit validation.
- Expired, revoked or superseded workers cannot commit even if they retain a delayed message or local process.
- Recovery reconciles abandoned reservations without silently extending authority.

### 2.5 Temporal — durable workflow orchestration

Temporal owns durable workflow progress, timers, retries, cancellation, compensation and long-running DAG execution.

- Workflow state is coordination state, not repository/transaction truth.
- Activities are idempotent or carry durable idempotency identities for every external side effect.
- Workflow retries query/reconcile authoritative Engine/PostgreSQL state before repeating a side effect.
- Workflow code is deterministic; nondeterministic I/O belongs in activities.
- Define retryable versus terminal errors, bounded retry/backoff, deadlines, cancellation semantics, heartbeat/worker-loss behavior and deployed workflow versioning.
- Workflow completion cannot imply transaction commit. A workflow reads the authoritative commit receipt.
- Test worker restart, replay, activity timeout after side effect, cancellation races, deployment/version changes and recovery from durable checkpoints.

### 2.6 NATS JetStream — event distribution

NATS JetStream distributes transaction lifecycle events, worker heartbeats, execution status, diagnostics and internal notifications.

- Publish commit events through a transactional outbox written in the same PostgreSQL transaction as the commit.
- Outbox publishers may publish more than once. Consumers are idempotent using stable event IDs and aggregate/revision sequence.
- Consumers validate scope, schema version, transaction state and revision before acting.
- Use durable consumers, explicit acknowledgement, bounded redelivery, poison-message quarantine and replay procedures.
- Events may be delayed, duplicated or reordered. Arrival order is never authority.
- Event loss/delay cannot reverse or alter a durable commit.
- Define stream retention, replication, storage mode, max age/bytes, consumer recovery, backpressure and operational alerts. Do not claim zero loss without a tested failure model and configured replication.
- NATS unavailability does not cause a committed transaction to be repeated or rolled back; outbox backlog is reconciled later.

### 2.7 Immutable object storage — evidence and artifacts

- Add a backend-neutral object-store interface and a production-capable S3-compatible implementation.
- Store immutable content-addressed objects for patches, snapshots/manifests, execution logs, diagnostics, verification artifacts and governed trajectories.
- Verify digest and length on write and read; use conditional create or equivalent immutability controls.
- Separate object bytes from PostgreSQL metadata, authorization and retention records.
- Evidence admission succeeds only after every required object is durably persisted and independently retrievable by digest.
- Coordinate metadata and object writes using an explicit staged protocol: upload immutable objects, verify them, then atomically admit references; reconcile orphaned objects and missing references.
- Define encryption, key rotation, access scope, retention, legal hold, deletion/tombstone semantics, backup and restore. Never claim physical erasure where storage/versioning/backup semantics cannot prove it.
- A missing or unverifiable object makes dependent evidence incomplete; no local cache or stale copy silently satisfies the durable evidence contract.

### 2.8 ClickHouse — derived analytics

- Receives events and measurements through replayable ingestion.
- Is never authoritative for transaction, authorization, reservation or repository state.
- Ingestion is idempotent or deduplicated by stable event identity.
- Analytics lag or outage does not block or mutate commit truth. Any synchronous safety control belongs in the Engine/authoritative state path, not ClickHouse.
- Define retention, schema evolution, late-arriving data and rebuild-from-source procedures.

### 2.9 Shared language intelligence and execution adapters

Integrate shared/version-aware LSP services and execution adapters only behind existing snapshot, provenance, admission and evidence contracts.

- LSP sessions and responses remain bound to immutable snapshot and document/index generations. Stale responses cannot update current analysis.
- Shared services require tenant/workspace isolation, quotas, lifecycle management and cache keys including all semantic inputs.
- Execution workers remain subordinate to Engine admission and evidence validation.
- T1/T2/T3 adapters report actual backend identity and policy. Unsupported tiers fail closed; no host fallback.
- T2 Firecracker and T3 remote pools are not automatically required to be production-complete in this goal unless selected as the concrete implementation path. Their interfaces and authority boundaries remain explicit; only validated backends may be advertised as supported.
- Do not broaden language execution support based on parser support alone.

## 3. Security and isolation requirements

- Authenticate service-to-service calls with workload identity and mutually authenticated transport where appropriate.
- Authorize every operation against organization, principal, workspace, repository and transaction scope. Actor IDs are attribution, not authority.
- Enforce least privilege for database roles, object-store credentials, NATS accounts/subjects, Temporal namespaces/task queues and worker identities.
- Separate control-plane, data-plane and execution-plane trust boundaries.
- Protect secrets through a dedicated secret-management boundary; do not place ambient host credentials in workers or workflow payloads.
- Bind approvals to exact transaction, patch, snapshot, policy and evidence digests; recheck revocation and expiry at commit.
- Prevent cross-tenant reads/writes through database constraints/policies and service authorization, not only UI filtering.
- Audit security-sensitive reads, policy changes, approvals, reservation changes, evidence access and administrative recovery.
- Apply TLS, key rotation, resource quotas, bounded payloads, schema validation and safe deserialization.
- Threat-model PostgreSQL, NATS, Temporal, object storage, service identities and administrative recovery. A trusted network is not an authorization boundary.

## 4. Recovery, consistency and failure semantics

Inject failures at:
- before and during database commit;
- after database commit but before API response;
- after commit but before outbox publication;
- after event publication but before outbox acknowledgement;
- duplicate and out-of-order event delivery;
- Temporal activity timeout after an external side effect;
- worker crash, lease expiry and delayed stale-owner message;
- object upload before metadata admission, and metadata admission after verified upload;
- object-store outage during evidence sealing or retrieval;
- PostgreSQL failover/restart and connection loss during commit;
- NATS partition, redelivery and consumer restart;
- analytics ingestion lag/outage;
- schema migration interruption and application-version skew;
- two or more independent Engine instances racing on the same repository.

Required outcomes:
- one authoritative terminal outcome per idempotency identity;
- exactly-once state effect, not an assumption of exactly-once message delivery;
- no partial repository revision advancement;
- lost responses recover the original durable result;
- no stale lease/fencing token can commit;
- outbox/inbox replay cannot duplicate semantic effects;
- required evidence is not admitted before durable object verification;
- analytics failure does not change transaction truth;
- recovery resumes from a verified durable checkpoint or safely restarts;
- ambiguous outcomes are reconciled, never guessed.

Every fault case records expected durable state, observed state, recovery action and regression test identity.

## 5. Observability and operations

Deliver operationally useful observability from the start:
- correlated trace IDs across API, Brain, Temporal, Engine, PostgreSQL, NATS, object storage and workers;
- structured logs with stable transaction, attempt, repository, workflow, event and artifact identities;
- metrics for commit latency/conflicts, idempotent replays, reservation age/expiry, outbox lag, consumer lag/redelivery, workflow retries, worker availability, object integrity failures, evidence admission failures, storage saturation and recovery backlog;
- health/readiness checks distinguishing process liveness from dependency readiness;
- actionable alerts, runbooks and dashboards;
- bounded queues, backpressure, admission control, deadlines, cancellation and graceful shutdown;
- backup/restore and disaster-recovery procedures with measured recovery point/time objectives rather than assumed guarantees;
- capacity and load tests at realistic concurrency and repository/workspace counts.

Do not introduce arbitrary SLOs or performance claims. Establish baselines from reproducible tests and select service objectives from measured needs.

## 6. Implementation milestones

### 04.1 — Durable state model and PostgreSQL authority
Define schema, constraints, migrations, transaction persistence, revision heads, attempts, idempotency, reservations/fencing and repository-scoped state. Preserve Engine domain semantics and prove restart/reopen behavior.

### 04.2 — Atomic distributed commit
Implement the multi-instance commit protocol, conditional revision advancement, durable receipts and commit idempotency. Validate with independent Engine processes and real PostgreSQL contention/fault tests.

### 04.3 — Transactional outbox and NATS JetStream
Publish durable lifecycle events from the outbox. Implement idempotent consumers, replay, redelivery, schema evolution and reconciliation.

### 04.4 — Temporal durable orchestration
Implement durable workflows and activities around Engine operations. Prove replay, retry, cancellation and side-effect reconciliation without moving authority into workflow state.

### 04.5 — Durable object storage and evidence lifecycle
Integrate immutable object storage, digest verification, metadata admission, retention/access controls, orphan reconciliation, backup and restore. Preserve Goal 03 evidence semantics.

### 04.6 — Analytics and shared service integration
Add replayable ClickHouse ingestion, shared/version-aware LSP lifecycle and policy-governed execution adapter integration where required by the end-to-end path.

### 04.7 — Multi-instance end-to-end and operational proof
Run the complete path across independently running service instances. Execute concurrency, crash, partition, failover, replay, recovery, security, load and disaster-recovery tests. Publish reproducible measurements and an acceptance audit.

Milestones may be reordered only for a demonstrated dependency. Each milestone requires a bounded diff, tests, evidence and a commit before proceeding.

## 7. Required test programme

**Database and commit correctness**
- Concurrent same-base commits from independent Engine processes.
- Compatible and conflicting transactions against a reference oracle.
- Revision compare-and-swap races and stale base rejection.
- Duplicate idempotency key with identical and different request digests.
- Crash before commit, during commit, after commit and before response.
- Database failover/connection loss with outcome reconciliation.
- Reservation acquire/renew/release/expiry and fencing-token races.
- Migration from empty and every supported prior schema.

**Temporal and NATS**
- Workflow replay and worker restart.
- Activity timeout after successful external side effect.
- Cancellation racing with commit.
- Duplicate, delayed, reordered and poison events.
- Publisher crash around outbox acknowledgement.
- Consumer crash before/after side effect and acknowledgement.
- Stream retention/replay and schema incompatibility.
- NATS unavailable while commits continue and outbox backlog recovers.

**Object storage and evidence**
- Corrupt, truncated, missing, substituted and cross-scope objects.
- Upload success followed by metadata failure; metadata admission only after digest verification.
- Duplicate upload, orphan cleanup, delayed deletion and restore.
- Evidence retrieval after process restart and from another service instance.
- Retention, authorization, key rotation and backup/restore behavior.

**Security, operations and performance**
- Cross-tenant access probes across every service boundary.
- Forged service identity, revoked authorization, stale approval and expired fencing token.
- Credential leakage and least-privilege checks.
- Backpressure, queue saturation, slow dependencies and graceful shutdown.
- Restore drills and recovery-point verification.
- Load tests with multiple repositories, workspaces, Engine instances and active workflows.
- Trace/log correlation and alert correctness under injected faults.

Mocks are useful for unit tests but do not establish database isolation, distributed atomicity, message durability, object-store behavior or service recovery. Those claims require integration tests against the actual selected components.

## 8. Definition of done

Goal 04 is complete only when all of the following are demonstrated:

1. Authoritative transaction/repository state survives process and host-service restart under the documented durability model.
2. Independent Engine instances safely coordinate against the same durable repository state.
3. The atomic database commit operation is the single linearization point and is tested under races and crash ambiguity.
4. Idempotency returns the original durable outcome and rejects request-digest substitution.
5. Durable reservations and fencing prevent expired owners from committing.
6. Temporal replay/retry/cancellation cannot duplicate or invent transaction effects.
7. NATS outbox publication and consumer replay are recoverable and idempotent.
8. Required evidence is durably stored, digest-verified and retrievable before Engine admission.
9. ClickHouse remains derived and rebuildable; its failure cannot change commit truth.
10. Authorization, tenant/workspace isolation, audit and secret boundaries are tested across services.
11. Backup/restore and recovery procedures are exercised with measured results.
12. Multi-instance end-to-end, fault-injection, security and load tests pass against real dependencies.
13. Documentation, capability matrix, operations runbooks, threat model and measured baselines reflect the exact validated scope.
14. All commits preserve the repository human-only author/committer attribution policy.

A deployed component is not sufficient. A successful happy-path integration is not sufficient. Goal 04 requires proof of distributed correctness and recovery.

## 9. Explicit non-goals and boundaries

- Replacing Rust Engine semantic authority with Temporal workflows, SQL triggers or event consumers.
- Treating NATS delivery as exactly once or as a commit decision.
- Claiming strict serializability or universal program correctness beyond ADR-0003.
- Automatically making T2/T3 execution, every language harness, native Windows/macOS or public production deployment supported without separate validation.
- Training models or using stored trajectories for training without the separate governance and eligibility process.
- Public signup, billing and customer-facing administration surfaces unless separately scheduled; identity, policy and isolation foundations remain mandatory.
- Source publication or deployment solely because a workflow completed or verification passed.
- Arbitrary SLOs, capacity claims or zero-loss guarantees without measurements and a stated failure model.

**Goal 04 outcome:** Celetixo moves from a locally validated transaction/execution core to a durable, recoverable, multi-instance engineering platform while preserving Engine semantic authority and the invariant that models propose, policies authorize, workflows execute, and audit records prove what happened.
