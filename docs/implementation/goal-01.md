# Goal 01: Engineering State Engine

This document describes the bounded reference environment used to establish the Engine's transaction semantics. Accepted [ADR-0001](../adr/0001-celetixo-engineering-state-coordination.md), [ADR-0002](../adr/0002-platform-brain-client-and-correctness-extensions.md), [ADR-0003](../adr/0003-transaction-consistency-and-validation-model.md) and [ADR-0004](../adr/0004-internal-first-public-capable-architecture.md) remain the architectural authority. Actual command results and implementation milestones belong in [validation.md](validation.md).

## Reproducible commands

The repository pins Rust 1.94.1 in `rust-toolchain.toml` and dependency resolution in `Cargo.lock`. From the repository root, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
cargo run --locked -p celetixo-engine --example concurrent_transactions
```

The same commands are configured in `.github/workflows/ci.yml`. A configured workflow is not evidence that a remote CI run has passed. The example and tests execute the reference model's numeric engineering state; they do not execute untrusted repository code.

## Proof boundary

The initial Engine operates on typed semantic element identities and integer values. An element may represent a symbol, dependency, configuration input, generated input, test scope or other modeled engineering dependency. The Engine receives explicit accesses and derives effective dependencies through its versioned, Engine-owned graph closure within this reference model. It preserves separate provenance for explicit and derived accesses, predicate assumptions, invariant scopes and impact completeness.

This representation makes OCC histories executable without coupling correctness to a source parser, UI, model provider or orchestration service. It proves only the consistency properties of the represented dependency model. It does not establish that a real source extractor discovers every dependency, support for a programming language, correctness of arbitrary programs or strict serializability.

Unknown, stale, ambiguous or incomplete analysis must reject admission or validation. Empty impact is permissible only when analysis establishes an empty impact for the supported model. An externally supplied actor ID or an analysis assertion does not grant commit authority.

## Identity and snapshots

Organization, principal, workspace and repository scope remain explicit. Transactions, patches, snapshots and evidence have typed identities. A snapshot binds a canonical manifest to the engineering state and authoritative revision it represents. Snapshot content is immutable after registration.

The manifest distinguishes included tracked content, index/staged content, unstaged content, untracked content, editor overlays, generated inputs, mounted inputs, submodule references and other declared workspace/environment inputs. Construction policy, canonicalization version, modes, symlink representation, content digests and provenance are part of its identity where represented. Changing any relevant included input must change snapshot identity even when Git HEAD is unchanged. Canonical ordering must not depend on insertion order.

The caller of this reference environment supplies the captured inputs. There is no filesystem scanner or source-control workspace capturer. The library cannot establish that a caller omitted no relevant real-world input. A future trusted capturer must implement and validate complete capture, policy, filesystem-race handling and repository-specific inclusion rules before source transactions rely on it.

## Transaction and revalidation contract

The lifecycle is typed:

```text
PROPOSED → ANALYZED → RESERVED → EXECUTING → VALIDATING → COMMITTED
     └────────────── any permitted nonterminal failure ─────→ ABORTED
```

The Engine validates every transition. A reservation is an expiring coordination lease, not proof of compatibility. Expiry, renewal, cancellation and reclaiming abandoned work must use the authoritative clock and must never revive authority for an expired owner.

An advance of the repository revision returns `RevalidationRequired` before mutation. Explicit `revalidate()` checks intervening commits against the original effective accesses and recomputes graph closure and its analysis stamp. A compatible transaction retains its transaction identity and starts a versioned attempt, reanchors the patch to the current snapshot after checking before-images, clears previous evidence and produces a new exact candidate snapshot. Prior attempts remain part of history. Fresh verification is required for the new attempt, and evidence bindings include the attempt number. A conflicting transaction must be rejected rather than overwrite prior work. This implementation has no evidence-reuse equivalence rule.

`COMMITTED` and `ABORTED` are terminal. Retry of an already completed operation uses the same operation identity and returns its recorded result. A retry or rebase representing new work uses a new transaction identity or an explicitly versioned attempt, fresh analysis, a current snapshot and renewed evidence. It cannot revive an aborted transaction.

### Proposed additive clarification to ADR-0001

ADR-0001's original lifecycle diagram draws `ABORTED → REBASE`. Read literally, that arrow conflicts with the terminal state rule in the transaction specification. The proposed amendment is:

> `ABORTED` is terminal. The diagram's outgoing rebase arrow denotes a new transaction or explicitly versioned attempt derived from the aborted intent. It does not mutate or revive the aborted transaction. Rebase requires fresh semantic analysis and verification against the current immutable snapshot. Already committed operation retries recover the original outcome through idempotency.

This is an explicit proposal, not a change to ADR status. Implementation follows the transaction specification's terminal history rule and preserves ADR-0003's fresh revalidation requirements. Accepting a weaker commit or resurrection rule would require architectural review.

## Commit gate

The embedding application is trusted to manage administrator and verifier capabilities. Actor-facing operations require an unforgeable in-process session capability scoped to the authorized resources. Administrative grant/revoke operations serialize with commit so revoked authority cannot pass a later gate. This is an explicit library boundary; no network service or network authentication protocol is exposed or claimed.

The reference commit boundary must check all of the following against one authoritative state before advancing a revision:

1. Authenticated capability and current authorization for the scoped operation.
2. Operation identity, recorded idempotent result and legal transaction state.
3. Exact immutable snapshot and validated authoritative revision.
4. Current lease ownership, scope, expiry and cancellation status.
5. Effective explicit and derived reads/writes, intervening conflicts, predicate assumptions and invariant dependencies.
6. Supported impact completeness, analyzer/configuration provenance and current analysis inputs.
7. Versioned patch identity/digest, applicability and resulting state invariants.
8. Required checks, their exact configurations and execution environment, and complete matching evidence.

Successful publication changes state, revision, transaction outcome and idempotency outcome together. Rejection changes none of the repository content or authoritative revision. Read/read compatibility permits useful concurrency; write conflicts, invalidated reads, phantom changes and unsafe invariant overlap must not silently produce an accepted history.

## Evidence contract

Evidence binds transaction, snapshot, patch digest, exact required check identity/configuration, toolchain and execution environment, result, diagnostics, artifact digests and provenance. Required evidence must exist in the authoritative evidence store before it can satisfy the gate. The Engine must reject missing, failed, stale, mismatched or incomplete evidence. Evidence for one transaction cannot authorize another, and a caller-supplied successful boolean is insufficient.

This reference environment stores evidence and artifacts in memory. Their capture is authoritative only within the lifetime and trust boundary of that store. It does not claim object-store durability, isolated execution, trustworthy external worker attestation or secure capture of artifacts from arbitrary processes. The reference harness's trusted verification boundary must remain separate from transaction actors.

## Atomicity and failure recovery

Independent Engine handles share a single in-memory authoritative store. A process-local critical section serializes validation and publication in that environment. It is a reference implementation of one atomic state transition, not production distributed atomicity.

The linearization point is publication of the fully validated commit record, new revision, new state and idempotency result. Failures injected before publication leave the previous state intact. Failures after publication may lose the response, but replay using the same idempotency key must recover the original outcome and must not advance revision again. Cancellation and lease changes compete with commit at that same authoritative boundary.

These recovery guarantees require the shared store to remain alive. Process death loses repository state, transaction history, evidence, leases and idempotency records. The reference does not recover a durable commit after a process restart, simulate network partitions or publish durable events. It must never report its process-local synchronization as a distributed durability guarantee.

The accepted target remains PostgreSQL-backed durable atomic validation/revision advancement across Engine instances, immutable evidence/artifact persistence, durable idempotency and event publication through an outbox or equivalent mechanism. A durable adapter must preserve the same gate, transaction history and failure semantics, and must validate process/storage failures before claiming those guarantees. Analytics, UI and event delivery cannot become commit authority.

## Acceptance evidence

The executable suite must cover compatible transactions from one base, read/read, write/write and read/write histories, predicates, invariant scope, invalidated reads, stale revisions, stale or incomplete analysis, illegal states, repeated operations, lease expiry/renewal/cancellation/recovery, authorization revocation, exact evidence binding, interrupted revalidation and commit boundary failures.

An independently structured reference oracle must compare generated transaction histories, preserve deterministic seeds and shrink mismatches into smaller failing histories. Tests must inspect final state and accepted outcomes, including racing commits through independent Engine handles sharing the authoritative store. This validates the declared model within the stated environment; it does not validate distributed services, real source indexing, external sandboxing or end-to-end tenant isolation.

Milestones must be committed separately after their documented checks pass. Report missing commands or failed checks explicitly in the validation record; planned coverage is not evidence of completion.
