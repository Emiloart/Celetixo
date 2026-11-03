# ADR-0003: Transaction Consistency and Validation Model

- **Status:** Accepted
- **Date:** 2026-11-03
- **Depends on:** ADR-0001, ADR-0002
- **Decision scope:** Define the minimum consistency contract for semantic transactions, conflict validation and commit linearization. This ADR does not claim strict serializability or universal program correctness.

## Context

ADR-0001 establishes optimistic concurrency control (OCC), semantic read/write analysis, reservations and compare-and-swap revision advancement. ADR-0002 strengthens snapshot identity, impact provenance, evidence applicability and atomic commit requirements. Those decisions require an explicit consistency contract: what histories the Engine promises to prevent, what constitutes a conflict, and what a commit linearization point means.

A patch applying cleanly is not proof that concurrent transactions are compatible. File-level conflict detection is insufficient. Conversely, rejecting every pair of transactions that share a repository revision would eliminate useful concurrency without establishing a meaningful correctness property.

## Decision

### 1. Consistency target

The Engine must admit only transaction histories that are **conflict-serializable with respect to the Engine's declared and derived dependency model**, while preserving declared invariants and the repository revision's atomic advancement.

This is a conservative engineering-state consistency guarantee, not a claim that arbitrary program behavior is serializable. It is not strict serializability: this ADR does not impose real-time ordering between non-overlapping transactions. Any future requirement for strict serializability, linearizability of additional operations, or stronger cross-repository guarantees requires an explicit ADR and a proof/validation plan.

The Engine must state the boundary of the model. Undeclared, undiscovered, unsupported or dynamically created dependencies cannot be assumed absent. Incomplete analysis is represented as incomplete and handled by policy.

### 2. Transaction access model

Each transaction carries:
- an immutable base snapshot identity;
- explicit read and write declarations;
- derived read and write dependencies from the versioned semantic model;
- invariant contracts and their dependency scope;
- predicate/range dependencies where queries or membership conditions affect behavior;
- impact-analysis provenance and completeness;
- a patch digest and verification evidence bound to the relevant snapshot and patch.

The effective access set is the union of explicit and derived accesses. The Engine must preserve provenance so a conflict can be explained and audited.

### 3. Conflict classes

At minimum, validation considers:
- write/write overlap;
- read/write and write/read overlap;
- changes to values or relationships read by a transaction;
- predicate/range or membership changes that can create phantom effects;
- invariant-scope overlap;
- dependency, configuration, generated-artifact and build-graph changes;
- unresolved or incomplete impact.

A conflict is not limited to identical paths or symbol names. The semantic model must define how each supported language and non-code dependency maps into access identities.

Unknown impact is not equivalent to an empty access set. Policy may block, escalate, or permit a specifically bounded operation, but must not silently classify unknown as safe.

### 4. Commit validation and linearization

A commit attempt must validate against the latest authoritative repository revision and the transaction's immutable base snapshot. The final decision and revision advancement must be atomic across all Engine instances.

The commit linearization point is the durable, atomic operation that both:
1. confirms the validated transaction is admissible against the current authoritative state; and
2. advances the authoritative revision exactly once.

The operation must recheck transaction state, snapshot identity, access/conflict assumptions, invariant status, patch digest, evidence applicability, authorization, reservation/lease requirements and idempotency. A process-local mutex is not sufficient.

A failed or interrupted commit must recover to one unambiguous state: committed exactly once, or not committed. Recovery must not infer success from a worker response or an event that may have been duplicated.

### 5. Revalidation and rebase

When the authoritative revision advances after a transaction's base:
- the Engine identifies intervening committed changes;
- recomputes affected semantic state using compatible index generations;
- validates the transaction's read/write and invariant assumptions against the new state;
- determines which verification evidence remains applicable;
- requires patch adaptation and renewed verification when needed.

Rebase is a new validation path, not a bypass. If assumptions cannot be re-established, the transaction aborts or escalates.

### 6. Reservations

Reservations reduce avoidable conflicting execution. They do not establish serializability and do not replace commit validation. Lease expiration, renewal, cancellation, acquisition ordering and crash recovery must be explicit. Reservation state must be durable where required by the distributed execution model.

### 7. Required validation

The Engine must be tested with:
- a simple independent reference model implementing the declared consistency rules;
- generated transaction histories with randomized read/write/predicate sets;
- deterministic seeds and reproducible failing cases;
- shrinking/minimization of failing histories;
- concurrent commit attempts across independent Engine instances;
- crash and retry injection around the commit linearization point;
- invariant preservation checks over every accepted history.

The reference model must be simpler and independently structured enough to reduce common-mode implementation errors. The test oracle validates the declared model; it does not prove that the semantic dependency extractor discovers every real program dependency.

## Consequences

- Conflict detection must account for semantic dependencies and predicates, not only files.
- The Engine must expose its analysis boundary and completeness.
- Commit validation and revision advancement form one atomic correctness boundary.
- Property-based/model-based testing becomes a core validation technique.
- The system does not claim strict serializability or universal program correctness.
- Stronger consistency guarantees require a separate decision and evidence.

## Relationship to prior ADRs

ADR-0001 remains the architecture baseline. ADR-0002's snapshot, evidence and atomicity extensions remain in force. This ADR specifies the transaction consistency target and validation contract. If a future implementation cannot meet this contract, it must stop and propose an ADR amendment rather than silently weaken it.

> **Reservations coordinate execution. Semantic validation governs admissibility. Atomic commit linearizes accepted state transitions.**
