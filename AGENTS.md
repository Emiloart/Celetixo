# AGENTS.md

## Purpose

This is the repository-level operating contract for AI coding agents working on Celetixo. Read it before modifying the repository.

**Models are workers. Engineering state is the source of truth. Verification is the authority. Evidence is the learning substrate.**

## 1. Authority and required reading

Start with the [documentation index](docs/README.md) to identify normative authority, implementation contracts and validation evidence. Read the relevant documents before changing the affected subsystem:

- [Threat model](docs/spec/threat-model.md) for security boundaries and control status.
- [Capability/support matrix](docs/reference/capability-support-matrix.md) before claiming a supported language, platform or execution tier.
- [Local Linux T1 runbook](docs/operations/local-linux-t1-runbook.md) before changing or operating the validated execution profile.
- [Protocol reference](docs/reference/protocol.md) before changing cross-service contracts.

1. [ADR-0001: Engineering-state coordination](docs/adr/0001-celetixo-engineering-state-coordination.md)
2. [ADR-0002: Platform, Brain, clients and correctness extensions](docs/adr/0002-platform-brain-client-and-correctness-extensions.md)
3. [ADR-0003: Transaction consistency and validation](docs/adr/0003-transaction-consistency-and-validation-model.md)
4. [ADR-0004: Internal-first, public-capable architecture](docs/adr/0004-internal-first-public-capable-architecture.md)
5. [Deployment and product boundaries](docs/spec/deployment-and-product-boundaries.md)
6. [Dev.md](Dev.md) for implementation discipline and build sequence
7. The applicable specifications under [docs/spec](docs/spec)

When documents disagree, accepted ADRs govern architecture; this file governs agent behavior; Dev.md governs implementation workflow; README.md describes the public system. Source code describes implemented behavior but does not silently supersede an accepted decision.

If implementation evidence exposes a flaw in an accepted decision, stop and propose an ADR amendment. Do not quietly weaken or reinterpret the contract.

## 2. System boundary

Celetixo coordinates intent-driven, semantically analyzed and verifiably executed repository-state transitions.

The Rust Engineering State Engine is authoritative for:
- immutable repository/workspace snapshot identity;
- semantic state and impact analysis;
- transaction admission and lifecycle;
- access/conflict analysis;
- reservations and leases;
- OCC validation and commit serialization;
- rebase/revalidation requirements;
- verification-evidence admissibility;
- final commit decisions.

The Python Brain owns model/provider adapters, routing, context assembly, planning, Chief/Supervisor reasoning, worker allocation, evaluation and future learning workflows. It submits typed intent and consumes Engine decisions. It must not duplicate or assume Engine authority.

Go/Temporal owns durable workflow execution. NATS JetStream distributes asynchronous events. PostgreSQL owns authoritative current operational state. ClickHouse owns analytics. S3-compatible object storage owns immutable large evidence and artifacts. Keep these responsibilities distinct.

Desktop and mobile applications are clients, not transaction authorities. Theia is the preferred desktop candidate subject to validation. Android is native Kotlin + Jetpack Compose. Mobile and desktop actions use authenticated APIs and server-side policy.

## Internal-first operating boundary

Internal-first means that the initial deployment is operated by Emilo Labs under Emilo Labs' data-governance policies. It does not authorize reduced engineering quality, deferred foundational security, weaker correctness, a smaller target architecture or temporary bypasses.

Agents must preserve the complete accepted architecture and build to the highest technically justified standard. Staging implementation by dependency and validation evidence is acceptable; omitting necessary capabilities or choosing inferior permanent designs to save time is not.

The initial deployment and any future public product use the same authoritative Engine transaction, verification, evidence and commit path. Do not introduce internal-only bypasses around authentication, authorization, immutable snapshots, semantic impact analysis, sandboxing, evidence admissibility, OCC or atomic commit.

Keep organization, principal, workspace and repository scopes explicit. Do not hardcode Emilo Labs as a global owner in core contracts. Apply the initial Emilo Labs data policies through explicit, enforceable configuration and policy boundaries. Public-facing onboarding, billing and customer administration may be staged, but foundational identity, authorization, isolation, governance and integration contracts must not be deferred.

Follow [ADR-0004](docs/adr/0004-internal-first-public-capable-architecture.md) and [deployment-and-product-boundaries.md](docs/spec/deployment-and-product-boundaries.md). Never claim public readiness, multi-tenant isolation or a security property without implementation and validation evidence.

## 3. Non-negotiable correctness rules

### Semantic coordination
Do not use file locks as the primary correctness model. Account for symbols, references, calls, types, dependencies, configuration, generated artifacts, predicates, tests and invariant scopes.

### Transaction consistency
Follow ADR-0003: accepted histories must be conflict-serializable with respect to the declared and derived dependency model, with invariant preservation and atomic revision advancement. Do not claim strict serializability or universal program correctness.

Explicit and derived reads/writes must retain provenance. Include predicate/range and invariant dependencies where they affect behavior. Unknown, unsupported, ambiguous or incomplete impact is not an empty impact set and must never silently be treated as safe.

### Reservations
Reservations coordinate work; they do not prove correctness. Implement leases, expiry, renewal, cancellation, deterministic acquisition and crash recovery. An expired or revoked owner cannot commit.

### Snapshot identity
Git HEAD alone does not identify a dirty workspace. Transactions must bind to immutable snapshots that account for all included tracked/untracked content, staged/unstaged changes, editor overlays and relevant generated, mounted, submodule and workspace inputs. Follow [engineering identity](docs/spec/engineering-identity.md).

### Verification
Evidence binds to transaction, snapshot, patch digest, exact check identity/configuration, toolchain/environment, result, diagnostics, artifacts and provenance. Stale or mismatched evidence cannot satisfy current requirements. A green boolean, successful patch application or model confidence is insufficient.

### Commit and rebase
The commit gate must atomically validate admissibility and advance authoritative revision across distributed Engine instances. Recheck snapshot, revision, transaction state, access/conflict assumptions, invariants, patch digest, evidence, authorization, lease policy and idempotency. A process-local mutex is insufficient.

Rebase means fresh semantic analysis and revalidation, not unconditional overwrite. A lost commit response must be recoverable through durable idempotency; it must not create a second commit.

### Source mutation
Use explicit versioned patches. Tree-sitter, LSP and indexes provide analysis; they are not universal canonical source writers. Preserve comments, formatting, macros, language-specific syntax and source trivia. AST-aware edits are appropriate only where demonstrably safer.

### LSP and indexes
Use shared, multiplexed, version-aware language services. Bind results to exact document/snapshot versions. Stale LSP results, partial indexes and unsupported language regions must be explicit and cannot authorize a complete-impact claim.

## 4. Agent authority

**Chief:** global objective, architecture, task decomposition, DAG, invariants, escalation and system tradeoffs.

**Supervisors:** bounded-domain decomposition, worker allocation, dependency ordering, local recovery and coordination.

**Workers:** bounded implementation and verification tasks; produce patches and evidence.

No model, worker, supervisor, client or orchestration component may grant itself reservation, authorization, verification-pass or commit authority. Actor IDs are attribution, not authorization.

Cross-service instructions use canonical typed Protobuf contracts. Preserve the full TaskIntent semantics: dependencies, explicit reads/writes, invariants, impact policy, verification requirements, budgets, retry/rollback/escalation and provenance. Do not create competing protocol definitions.

## 5. Security and execution

Generated code, patches, dependencies and tool output are untrusted.

Preserve the four execution tiers:
- **T0:** deterministic local operations that do not execute untrusted repository code.
- **T1:** isolated process/container/gVisor for bounded checks.
- **T2:** Firecracker microVM for stronger isolation and untrusted execution.
- **T3:** remote execution pools for scale and specialized workloads.

A container is not equivalent to a microVM. Enforce least privilege and bounded CPU, memory, disk, network, process count, time and output. Network egress is denied by default and granted only for an explicit task requirement. Workers receive no implicit production credentials.

Authenticate and authorize every sensitive API operation. Scope access by tenant/repository. Human approval binds to the exact transaction, snapshot, diff and evidence. Offline client actions must be re-authorized and revalidated when received.

## 6. Data and trajectory governance

A trajectory is a structured engineering record, not an unfiltered chat dump. Preserve lineage across intent, snapshot, impact, model/configuration, tool calls, patches, execution, checks, evidence, retries, rebase/rollback and human outcome.

Storage does not imply permission to train. Follow [trajectory governance](docs/spec/trajectory-governance.md): rights, repository authorization, purpose, privacy, secrets, quality, contamination, retention, deletion and dataset lineage must be evaluated separately. Keep evaluation data isolated from training and retrieval.

## 7. Reliability and failure behavior

Assume process crashes, partitions, timeouts after successful side effects, duplicate/delayed/reordered events, stale indexes, worker compromise and storage unavailability.

Follow [the failure model](docs/spec/failure-model.md). Design for idempotency, durable recovery, backpressure, cancellation, deadlines, bounded retries, circuit breakers, graceful shutdown and reconciliation. Event publication is not commit. Analytics/UI failure must not mutate transaction truth. Required evidence is not complete until durable persistence is confirmed.

## 8. Code and protocol discipline

Prefer typed domain models, explicit state machines, immutable evidence, deterministic pure logic, narrow interfaces, explicit errors, idempotent side effects and structured telemetry.

Avoid hidden global state, duplicated protocol definitions, stringly typed transaction states, unbounded retries, silent fallbacks, catch-all error suppression and model-specific business logic inside the Engine.

Protobuf v3 is canonical for internal typed contracts. Evolve schemas compatibly, reserve removed field numbers/names, never reuse field meaning, and test compatibility. Generated bindings derive from canonical schemas.

Initial language wedge: TypeScript/JavaScript, Python, Go and Rust. A language is supported only when its harness can index, coordinate, execute, verify and record work in that ecosystem. Tool choices remain repository-configurable.

## 9. Testing requirements

Test semantics and failure behavior, not only line coverage.

Transaction tests include:
- read/read, write/write, read/write and predicate conflicts;
- invariant-scope overlap;
- compatible concurrent transactions;
- stale revisions and atomic CAS;
- idempotency and lost responses;
- leases, expiry, renewal, cancellation and recovery;
- rebase and evidence invalidation;
- illegal state transitions.

Use an independently structured reference model and generated transaction histories with deterministic seeds and shrinking. Test commits across multiple Engine instances. The oracle validates the declared consistency model; it does not prove that semantic extraction discovers every program dependency.

Identity/index tests include rename/move continuity, ambiguous matches, path reuse, staged/unstaged/untracked content, editor overlays, canonical manifests, stale occurrences and analyzer-version changes.

Verification tests include snapshot/patch/environment binding, stale result rejection, cache invalidation, required-check enforcement and durable artifact capture.

Orchestration/failure tests include worker crashes, duplicate/out-of-order events, retries, cancellation races, service interruption, interrupted rebase and crash injection around the commit linearization point.

## 10. Evaluation and completion claims

Follow [EVALUATION.md](EVALUATION.md). Compare:
1. sequential coding agent;
2. parallel Git worktrees with ordinary integration and no Celetixo transactions;
3. Celetixo semantic transactions and evidence-gated commit.

Measure verified outcomes, cost/time, regressions, human correction, impact precision/recall, false-negative impact, conflict errors, abort reasons, wasted execution/tokens, verification cost and recovery correctness. A safety violation cannot be offset by performance.

Distinguish:
- **architectural target** — accepted design;
- **implemented** — code exists and works under stated conditions;
- **validated** — tests/experiments establish stated behavior.

Do not claim a capability, test, benchmark, security property, persistence guarantee or deployment exists unless verified in the repository and execution evidence. State unrun checks and limitations explicitly.

## 11. Working procedure

Before editing:
1. inspect branch, working tree and repository structure;
2. read relevant ADR/spec and existing implementation;
3. identify invariants, dependencies and acceptance tests;
4. avoid overwriting unrelated work;
5. define bounded file ownership for parallel tasks.

During editing:
1. make a complete, scoped change;
2. keep external side effects at explicit boundaries;
3. use deterministic fixtures and controlled clocks;
4. update canonical schemas before generated bindings;
5. avoid mixing unrelated refactors into correctness-critical changes.
6. follow [CONTRIBUTING.md](CONTRIBUTING.md): use the human contributor's established Git identity for authorship and committer metadata, and never add an AI tool or bot as an author or co-author.

Before completion:
1. format and lint changed components;
2. run focused and relevant workspace tests;
3. run type/build checks where applicable;
4. inspect the full diff and generated files;
5. confirm no assertions, tests, authorization or verification requirements were weakened;
6. report exact checks run, checks not run and why;
7. update documentation to match actual behavior.
8. commit and push each verified milestone separately, preserving remote contributions and checking attribution before publication.
9. after every successful push, tell the user which milestone was pushed and include its commit identity.

> **Models propose changes. The Engine decides admissibility. Verification supplies evidence. The atomic commit boundary protects repository state.**
