# Semantic Transaction Contract

**Status:** Normative subsystem specification  
**Authority:** ADR-0001, ADR-0002 and ADR-0003  
**Purpose:** Define the transaction lifecycle and validation interfaces without creating an independent architectural authority.

## 1. Contract boundary

A semantic transaction is a proposed repository-state transition evaluated against an immutable workspace snapshot. The Rust Engineering State Engine is authoritative for transaction admission, impact/conflict analysis, reservation policy, validation, rebase requirements and commit decisions.

The Python Brain, model providers, workers, desktop client, Android client, Go/Temporal orchestration and event consumers may request or observe transitions. None may independently declare a transaction committable.

## 2. Required transaction record

A transaction record must identify:

| Field | Meaning |
|---|---|
| transaction_id | Globally unique, stable transaction identity |
| repository_id | Repository/tenant scope |
| base_snapshot_id | Immutable content-addressed workspace snapshot |
| base_revision | Authoritative repository revision represented by the snapshot |
| intent | Typed objective and acceptance contract |
| explicit_reads / explicit_writes | Requester's declared access set |
| derived_reads / derived_writes | Engine-derived dependencies with provenance |
| predicates | Query/range/membership assumptions |
| invariants | Required properties and dependency scope |
| impact_result_id | Versioned impact analysis and completeness |
| patch_digest | Digest of the candidate mutation |
| verification_set | Required checks and evidence references |
| authorization_context | Authenticated principal and policy decision references |
| idempotency_key | Stable key for retry-safe side effects |

Missing optional analysis is represented explicitly; it is not represented as an empty successful result.

## 3. Lifecycle

The lifecycle is:

`PROPOSED → ANALYZED → RESERVED → EXECUTING → VALIDATING → COMMITTED | ABORTED`

A transaction may enter a revalidation/rebase subflow when its base becomes stale. The exact internal state names may evolve, but transitions must be typed, durable where required, auditable and validated against an allowed transition table.

- **PROPOSED:** intent and base snapshot are registered.
- **ANALYZED:** explicit and derived accesses, impact provenance, completeness and policy outcome are recorded.
- **RESERVED:** required coordination leases have been acquired.
- **EXECUTING:** workers operate only within the assigned snapshot/workspace boundary.
- **VALIDATING:** patch, invariants, conflict assumptions and verification evidence are checked against current authoritative state.
- **COMMITTED:** atomic commit boundary advanced the authoritative revision exactly once.
- **ABORTED:** transaction is terminal and cannot mutate authoritative state.

A terminal state is immutable. A retry that represents new work receives a new transaction identity or an explicitly versioned attempt identity; it must not rewrite history.

## 4. Access and dependency semantics

The effective access set combines caller-declared and Engine-derived dependencies. The model must distinguish:
- source content reads and writes;
- symbol/interface/type reads and writes;
- reference/call/dependency edges;
- package, lockfile and build graph dependencies;
- configuration and environment inputs;
- generated artifact inputs and outputs;
- tests and invariant scopes;
- predicates, ranges and membership conditions.

The system must represent access identity independently of display names and paths. See [engineering identity](engineering-identity.md).

A dependency extractor must report language/tool coverage, index generation, parser/analyzer versions, configuration, unresolved references and completeness. An incomplete graph cannot be treated as a complete graph with no additional edges.

## 5. Conflict and consistency

The consistency target is conflict serializability with respect to the declared and derived dependency model, as specified in ADR-0003. This is not strict serializability and does not prove arbitrary program behavior.

At minimum, validation detects write/write, read/write, write/read, predicate/phantom and invariant-scope conflicts. It also considers dependencies that can alter build, runtime, generated output or verification assumptions.

A conflict result includes:
- both transaction identities;
- conflicting access identities;
- conflict class;
- relevant committed revision;
- provenance for each access;
- analysis completeness;
- recommended action: wait, revalidate, rebase, abort or escalate.

A false-positive conflict is an efficiency defect; a false-negative conflict may be a correctness defect. Evaluation must distinguish them.

## 6. Reservations and leases

Reservations are advisory coordination controls over semantic access scopes. They reduce wasted work but do not replace commit validation.

Lease records need owner, transaction, scope, acquisition order, expiry, renewal history and terminal reason. Expiry must be based on a controlled clock abstraction in tests. Recovery must reclaim abandoned leases without allowing an expired owner to commit using stale authority.

Reservation upgrades and partial acquisition must not deadlock indefinitely. Acquisition ordering and cancellation semantics must be deterministic and observable.

## 7. Commit gate

The commit gate must perform a single durable atomic decision that:
1. authenticates and authorizes the actor/action;
2. checks idempotency and transaction state;
3. checks current revision and base snapshot;
4. checks reservations/leases where required;
5. revalidates access sets, predicates and invariant scopes;
6. verifies patch digest and patch applicability;
7. checks that required evidence is complete and bound to the relevant snapshot, patch, checks and environment;
8. advances authoritative revision once and records the commit outcome.

The commit linearization point is the durable atomic operation that both validates admissibility and advances the authoritative revision. Event publication may follow through an outbox or equivalent durable mechanism; an event is not the commit itself.

If the commit response is lost, retry with the same idempotency key must return the original outcome. It must not create a second revision.

## 8. Revalidation and rebase

A stale base triggers semantic revalidation. The Engine must not merely replay a Git rebase and assume the result is safe.

Revalidation identifies intervening commits, updates compatible semantic state, checks affected assumptions and invariants, determines evidence reuse eligibility, and requests patch adaptation or renewed checks when necessary. Any equivalence rule for reusing evidence must be deterministic, versioned and include all correctness-relevant inputs.

## 9. Crash and recovery states

The implementation must define recovery for crashes:
- before commit validation;
- during the atomic commit operation;
- after durable commit but before response;
- after commit but before event publication;
- during lease renewal or expiry;
- during rebase/revalidation;
- while verification is running.

The durable record must make the outcome recoverable without trusting worker memory or event delivery order.

## 10. Required tests

- State-machine transition table and illegal-transition tests.
- Compatibility matrix for read/read, write/write and read/write.
- Predicate/phantom and invariant-scope conflict tests.
- Generated transaction histories compared with an independent reference oracle.
- Concurrent commit tests across multiple Engine instances.
- Idempotency tests with lost responses and duplicate requests.
- Lease expiry, renewal, cancellation, upgrade and crash-recovery tests.
- Stale evidence, stale index and incomplete-impact rejection tests.
- Rebase tests where assumptions remain valid, become invalid or cannot be resolved.

All generated-history tests record seeds and preserve minimized counterexamples.
