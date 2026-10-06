# AGENTS.md

## Purpose

This file is the repository-level operating contract for AI coding agents working on Celetixo.

Celetixo is not being built as a conventional coding chatbot, IDE wrapper, or prompt-routing layer. It is an engineering-state coordination and verification system for autonomous software development.

The architectural thesis is:

> Models are workers. Engineering state is the source of truth. Verification is the authority. Evidence is the learning substrate.

Read this file before modifying the repository.

---

## 1. Source of Truth

The following documents define the current architecture:

1. `docs/adr/0001-celetixo-engineering-state-coordination.md`
2. `README.md`
3. This file, `AGENTS.md`

### Authority order

When documents disagree:

1. Accepted ADRs govern architectural decisions.
2. This file governs agent behavior and implementation discipline.
3. README explains the system publicly and should remain consistent with the ADR.
4. Source code is authoritative for behavior already implemented, but not a reason to silently violate an accepted architectural decision.

If implementation requires changing a frozen architectural invariant, stop and propose an ADR amendment instead of quietly changing the design.

---

## 2. What Celetixo Is

Celetixo is an engineering-state coordination and optimistic-concurrency engine in which AI agents act as execution workers.

The core operation is a semantic transaction:

```
Intent
  ↓
Impact analysis
  ↓
Coordination
  ↓
Execution
  ↓
Verification
  ↓
Commit / Abort / Rebase
```

The important abstraction is not "agent conversation."

It is the repository state transition.

A worker may generate a patch. Celetixo determines whether that patch is compatible with the current engineering state and whether sufficient evidence exists to accept it.

---

## 3. Non-Negotiable Architectural Rules

Do not introduce an implementation that violates these rules without an ADR.

### 3.1 Semantic coordination, not file locking

Do not implement concurrency around file locks as the primary correctness mechanism.

Files are useful physical units. They are not sufficient semantic units.

A change can affect:

- symbols
- callers
- references
- interfaces
- types
- dependencies
- generated artifacts
- configuration
- tests
- build behavior

The Engineering State Engine derives semantic impact from repository state.

### 3.2 Reservations are coordination, not correctness

Write reservations prevent conflicting work from executing concurrently.

They do not prove correctness.

Correctness requires validation against the relevant repository state and transaction invariants.

Reservations must support:

- read/write compatibility
- leases
- expiration
- heartbeat renewal
- deterministic acquisition ordering
- cancellation
- upgrade handling
- crash recovery

A crashed worker must never permanently block a repository.

### 3.3 Optimistic concurrency control

Transactions operate against an explicit repository revision.

Commit must verify:

1. declared invariants pass;
2. verification corresponds to the transaction snapshot;
3. the repository base is still valid, or the transaction has been semantically revalidated;
4. no incompatible committed transaction invalidated the transaction's assumptions;
5. the final patch applies cleanly to the validated state.

Repository revision advancement uses compare-and-swap semantics at the commit boundary.

### 3.4 Rebase is semantic

Never treat rebase as merely:

```
git rebase
```

A Celetixo rebase means:

1. identify the newer repository revision;
2. recompute relevant semantic state;
3. re-evaluate transaction assumptions;
4. adapt or regenerate the patch when necessary;
5. rerun the minimum verification required to re-establish invariants.

If the original intent is no longer compatible, abort or escalate.

### 3.5 Semantic state is separate from source mutation

Tree-sitter, symbol graphs, LSP, and static indexes provide semantic intelligence.

Workers mutate source through explicit, versioned patches.

Do not make universal AST rendering the canonical source-writing mechanism. Source mutation must preserve comments, trivia, formatting conventions, macros, configuration syntax, and language-specific constructs.

AST-aware edits are allowed where they are demonstrably safer or more precise.

### 3.6 Shared LSP

Do not create one LSP process per worker for the same repository.

Use shared, multiplexed, version-aware language services.

LSP results must be associated with an explicit document/repository version. Stale results must not silently influence commit decisions.

Tree-sitter remains the first structural read path for dirty or temporarily invalid source.

### 3.7 Tiered execution

Do not execute every operation inside Firecracker.

Use the cheapest environment that provides sufficient safety and evidence:

- T0: deterministic local Rust operations
- T1: isolated process/container/gVisor
- T2: Firecracker microVM
- T3: future remote execution pools

Firecracker is an execution backend, not a dependency of transaction semantics.

### 3.8 Separate workflow orchestration from events

Temporal owns durable workflows, retries, compensation, long-running execution, and DAG state.

NATS JetStream owns asynchronous event distribution, heartbeats, streamed diagnostics, and state signals.

Do not turn NATS into a workflow engine.

Do not turn Temporal into a high-frequency event bus.

### 3.9 Separate state from analytics and evidence

PostgreSQL is authoritative current engineering state.

ClickHouse is historical telemetry and evaluation analytics.

S3-compatible object storage plus Parquet is immutable evidence.

Do not use ClickHouse or object storage as an accidental transactional source of truth.

### 3.10 Evidence is first-class

A meaningful execution must be reconstructable.

Record, directly or by durable references:

- repository revision
- semantic graph/version
- transaction intent
- derived impact set
- reservations
- model identity/version
- prompt/configuration provenance
- tool calls
- patches
- execution environment
- verification results
- failures and retries
- rollback/rebase events
- final diff
- human review outcome

Do not optimize away evidence that is required to reproduce or evaluate a decision.

---

## 4. Agent Hierarchy

Celetixo uses a hierarchy of responsibilities.

### Chief

The Chief owns:

- global objective
- architecture
- task decomposition
- global DAG
- invariants
- escalation
- system-level tradeoffs

The Chief does not directly edit arbitrary source code as its primary role.

### Supervisors

Supervisors own bounded domains.

They:

- decompose domain work
- allocate workers
- manage local dependencies
- coordinate parallel execution
- recover from local failures
- communicate laterally through the event fabric

The Chief is not a message broker.

### Workers

Workers perform bounded implementation and verification tasks.

A worker should receive enough context to act, but should not be trusted as the final authority on:

- repository-wide impact
- concurrency correctness
- commit validity
- security boundaries
- verification sufficiency

The Engineering State Engine remains authoritative.

---

## 5. Chief / Supervisor / Worker Contract

Agent instructions should be represented as structured intent rather than unconstrained prose whenever they cross a system boundary.

The canonical internal protocol is Protobuf v3.

A task intent should carry, as applicable:

- task ID
- parent task ID
- repository ID
- base revision
- objective
- explicit reads
- explicit writes
- invariant contracts
- dependencies
- impact policy
- verification requirements
- resource budget
- retry policy
- rollback policy
- escalation policy
- model/agent provenance

The DSL requests a transaction.

It does not grant the requester reservation or commit authority.

---

## 6. Repository Revision Discipline

Every implementation task that mutates source must have a known repository base.

Never assume the working tree is unchanged merely because a file still looks familiar.

At minimum, a transaction must be able to identify:

```
repository_id
base_revision
transaction_id
```

When another transaction advances the repository:

- invalidate stale assumptions;
- recompute affected semantic state;
- decide whether the transaction remains compatible;
- revalidate or rebase before commit.

Never silently apply a patch generated against an obsolete state.

---

## 7. Engineering State Engine

The Rust Engineering State Engine is the semantic core.

It is responsible for capabilities such as:

- repository revision tracking
- semantic transactions
- explicit and derived impact sets
- reservation compatibility
- conflict analysis
- OCC validation
- symbol/reference graphs
- Tree-sitter integration
- incremental graph updates
- LSP multiplexing
- execution scheduling
- build/dependency cache coordination

Keep this layer independent of model-specific behavior.

The engine should be able to reason about a transaction even if the underlying model changes from one provider to another.

---

## 8. Code Intelligence

Repository intelligence has multiple layers.

### Structural

Use Tree-sitter for:

- parsing
- symbol discovery
- scopes
- references where available
- structural relationships
- incremental syntax state

### Semantic

Use language servers and static indexes for:

- type information
- definitions
- references
- diagnostics
- semantic queries

Use SCIP-style indexing where it provides durable repository-level value.

### Engineering

Track relationships involving:

- package manifests
- lockfiles
- build graphs
- generated files
- CI configuration
- Dockerfiles
- YAML
- JSON
- TOML
- HCL
- shell
- SQL
- scripts and tooling

A configuration change can be a semantic dependency even when it is not represented as a conventional programming-language symbol.

---

## 9. Initial Language Wedge

The initial supported languages are:

- TypeScript / JavaScript
- Python
- Go
- Rust

A language is supported only when its engineering harness provides the required capabilities.

### TypeScript / JavaScript

Expected integration points include:

- TypeScript compiler
- Biome or repository-native formatter/linter
- Vitest or repository-native test runner
- package manager and lockfile
- build cache

### Python

Expected integration points include:

- Pyright or Mypy
- Ruff
- pytest
- package/dependency manager
- build/test cache

### Go

Expected integration points include:

- gofmt
- compiler diagnostics
- go test
- go.mod / go.sum
- build cache

### Rust

Expected integration points include:

- rustfmt
- cargo check
- cargo test
- structured rustc diagnostics
- Cargo.lock
- Cargo build cache

Exact tools must remain repository-configurable.

Do not hard-code a tool as universally correct when a repository declares another valid toolchain.

---

## 10. Verification Policy

Verification is an architectural primitive.

Never treat:

- model confidence
- generated-code plausibility
- patch application
- successful process exit

as sufficient proof of correctness by themselves.

Use adaptive verification.

Typical progression:

```
T0  parse / AST / patch validation
 ↓
T1  format / lint / type / static analysis
 ↓
T2  targeted tests
 ↓
T3  subsystem / integration tests
 ↓
T4  broad E2E where impact requires it
```

The engine should prefer the smallest verification set that proves the required invariants.

However, a cheap verification result must not be used to avoid a required expensive verification.

Every verification decision should answer:

1. What changed?
2. What could that change affect?
3. Which invariant is being established?
4. What evidence establishes it?
5. Is the evidence tied to the correct repository/environment revision?

---

## 11. Patch Discipline

Workers should produce explicit patches against a known base revision.

Before acceptance:

1. patch must apply to the expected state;
2. syntax must be valid;
3. formatting policy must be satisfied;
4. relevant semantic state must be updated;
5. LSP/type diagnostics must correspond to the correct revision;
6. required verification must pass;
7. transaction invariants must hold.

Do not rewrite source merely to make a textual diff disappear.

Do not delete tests, weaken assertions, bypass type checks, or alter verification configuration to manufacture success.

Such behavior is a correctness failure.

---

## 12. Dependency and Build Cache Discipline

Dependency changes can invalidate more than a single package.

When a transaction modifies:

- dependency manifests
- lockfiles
- compiler configuration
- build configuration
- generated code
- package-manager configuration

recompute the appropriate dependency/build state.

Build caches should be keyed by enough environmental state to prevent false reuse.

At minimum consider:

- repository revision or relevant source digest
- dependency lock state
- toolchain version
- build configuration
- target platform/architecture
- relevant environment inputs

Never trade correctness for cache hit rate.

---

## 13. Sandbox and Security Rules

Generated code is untrusted until verified.

Workers must not receive implicit production credentials.

Sandbox execution must enforce bounded:

- CPU
- memory
- disk
- network
- process count
- execution time

Network access should be denied by default and explicitly granted only when the task requires it.

Do not introduce a path by which worker-generated code can silently reach production systems.

Execution artifacts should remain attributable to:

- task
- transaction
- worker
- model
- sandbox
- repository revision

---

## 14. Failure Semantics

Design for failure as the normal operating condition.

Expected failures include:

- worker crash
- model timeout
- malformed patch
- invalid syntax
- stale LSP result
- test failure
- dependency failure
- sandbox failure
- reservation expiry
- transaction conflict
- repository advancement
- partial execution
- orchestration retry

A retry must not accidentally duplicate a committed side effect.

A failed transaction must not become committed state.

A worker crash must not permanently hold a reservation.

A stale result must not satisfy a current verification requirement.

A rebase must not preserve assumptions that have become invalid.

---

## 15. Storage Boundaries

### PostgreSQL

Authoritative current state:

- repositories
- revisions
- tasks
- transactions
- reservations
- workers
- supervisors
- symbols/edges as appropriate
- verification runs
- architecture decisions
- provenance

### ClickHouse

Historical analytics:

- agent actions
- model calls
- token usage
- execution latency
- verification events
- transaction conflicts
- failures
- evaluation metrics

### Object storage + Parquet

Immutable large evidence:

- trajectories
- patches
- stdout/stderr
- compiler diagnostics
- test artifacts
- snapshots
- large execution outputs

Keep these ownership boundaries explicit.

---

## 16. Observability

Use structured telemetry.

Prefer OpenTelemetry-compatible traces, metrics, and logs where practical.

Important dimensions include:

- repository
- revision
- transaction
- task
- supervisor
- worker
- model
- execution tier
- verification stage
- outcome
- retry count
- conflict reason
- resource usage

Every important latency measurement should have enough context to distinguish:

- model latency
- orchestration latency
- queue/event latency
- graph analysis latency
- sandbox startup latency
- build/test latency
- verification latency
- commit/rebase latency

Do not collapse the entire system into one "agent latency" number.

---

## 17. Evaluation Rules

The primary question is not whether Celetixo can execute a multi-agent workflow.

The question is whether the architecture produces better verified engineering outcomes than a strong sequential coding-agent baseline on the same work.

Evaluation should measure at least:

- task success rate
- cost per successful PR
- latency per successful PR
- regression rate
- human intervention/fix rate
- semantic conflict rate
- verification compute
- agent actions

When adding an optimization, identify which metric it is expected to improve.

Do not claim performance improvements without controlled measurements.

Do not optimize for benchmark scores at the expense of actual repository correctness.

The evaluation harness must preserve enough evidence to reproduce a result.

---

## 18. Training and Learning

Do not train a proprietary model merely because the architecture contains agents.

The intended progression is:

```
v0  semantic coordination + verification + evidence
 ↓
v1  persistent engineering graph + procedural skills
 ↓
v2  learned routing + worker post-training
 ↓
v3  verifiable-reward training + synthetic environments
 ↓
v4  distilled engineering models + predictive coordination
 ↓
v5  autonomous software lifecycle management
```

The evidence loop comes before model training.

Useful learning data is not simply chat history. It includes:

- repository state
- intent
- impact analysis
- chosen worker
- model/version
- tools
- patches
- verification
- failures
- retries
- conflicts
- rebases
- final outcome
- human judgment

Reward hacking must be treated as a first-class threat.

Protected tests, hidden evaluation cases, and human acceptance signals should be considered where appropriate.

---

## 19. Code Quality Rules

Prefer:

- small composable modules
- explicit state machines
- typed contracts
- deterministic behavior where possible
- immutable event/evidence records
- idempotent operations
- explicit error types
- bounded retries
- structured logging
- dependency inversion at execution boundaries
- testable pure logic for state transitions

Avoid:

- hidden global state
- implicit cross-service coupling
- model-specific business logic inside the state engine
- unbounded retry loops
- magic timeouts
- silent fallbacks
- catch-all error suppression
- duplicated protocol definitions
- stringly typed transaction states
- architecture hidden inside prompt text

Prefer explicit state transitions over boolean soup.

Prefer domain types over primitive strings when a value has semantic meaning.

---

## 20. Type and Protocol Discipline

Protobuf is the canonical internal contract where the architecture specifies Protobuf.

Do not create competing definitions of the same protocol in:

- TypeScript
- Rust
- Go
- Python

Generated bindings should derive from the canonical schema.

External JSON APIs may use JSON Schema, but the internal contract must remain unambiguous.

When changing a protocol:

1. identify all consumers;
2. preserve compatibility where required;
3. update generated code;
4. update tests;
5. update documentation;
6. record architectural implications.

---

## 21. Testing Expectations

Tests should validate system semantics, not merely line coverage.

For transaction code, test:

- compatible reads
- incompatible writes
- read/write conflicts
- lease expiry
- heartbeat renewal
- cancellation
- deterministic acquisition ordering
- stale revisions
- successful CAS commit
- failed CAS commit
- semantic rebase
- abort
- retry idempotency
- crash recovery

For code intelligence, test:

- parsing
- incremental updates
- symbol identity
- reference updates
- dependency changes
- generated artifacts
- stale index rejection

For verification, test:

- correct tier selection
- stale result rejection
- environment identity
- cache invalidation
- evidence capture
- failure propagation

For orchestration, test:

- retry behavior
- compensation
- worker replacement
- supervisor recovery
- event ordering assumptions
- duplicate event handling

---

## 22. Determinism

Where deterministic behavior is possible, prefer it.

Examples:

- transaction IDs where externally supplied
- reservation acquisition ordering
- impact-set ordering
- graph traversal rules
- verification-plan generation
- event schemas
- state transition rules

Do not rely on LLM output for a decision that can be expressed deterministically from repository state.

Models propose.

The system decides.

---

## 23. Idempotency

Every operation that can be retried must define its idempotency behavior.

Examples:

- event consumers
- task creation
- transaction reservation
- verification execution
- artifact upload
- commit requests
- rebase requests
- workflow callbacks

Use stable operation/task/transaction IDs.

Do not create duplicate engineering state merely because an orchestrator retried a message.

---

## 24. Changes to Frozen Architecture

The following require an ADR or explicit ADR amendment:

- transaction semantics
- repository revision/commit semantics
- Engineering State Engine ownership
- Chief DSL contract
- storage ownership
- Temporal/NATS boundaries
- execution isolation model
- fundamental agent hierarchy
- semantic impact model

Implementation details can evolve without an ADR when the architectural invariants remain intact.

When uncertain, preserve the current boundary and document the proposed change before implementing it.

---

## 25. Repository Workflow for Coding Agents

Before changing code:

1. Read this file.
2. Read the relevant ADR.
3. Inspect the current implementation.
4. Identify the architectural boundary being changed.
5. Find existing tests and contracts.
6. Determine the smallest coherent implementation.

During implementation:

1. Keep changes scoped.
2. Preserve public contracts unless the task explicitly changes them.
3. Do not mix unrelated refactors with feature work.
4. Add or update tests with behavioral changes.
5. Keep error and state semantics explicit.
6. Record architectural decisions when required.

Before committing:

1. Run the narrowest relevant checks.
2. Run broader checks when the change crosses subsystem boundaries.
3. Inspect the final diff.
4. Confirm no tests or validation logic were weakened.
5. Confirm generated files and protocol bindings are synchronized.
6. Confirm documentation remains consistent.

Commit messages should describe the actual change, for example:

```
feat(state): add semantic transaction reservations
fix(lsp): reject stale diagnostics
test(state): cover CAS commit conflicts
docs(adr): record transaction rebase semantics
```

Do not use vague messages such as:

```
update stuff
fix things
changes
agent work
```

---

## 26. What Not to Build by Accident

Do not allow Celetixo to become:

- a chat UI with tools
- a collection of prompt templates
- a wrapper around existing coding agents
- a file-locking system with multiple workers
- a Git merge automation layer
- a benchmark-specific hack
- a model-training project without an engineering substrate
- an orchestration system with no semantic repository state
- an observability dashboard mistaken for the core product

If an implementation could be described as "send better prompts to more agents," it is probably below Celetixo's intended abstraction level.

---

## 27. Architectural Test

When deciding whether a proposed feature belongs in Celetixo, ask:

### Does it improve one of these?

1. representation of engineering state;
2. semantic impact analysis;
3. safe concurrent execution;
4. verification quality or efficiency;
5. durable orchestration;
6. evidence quality;
7. measurable engineering outcomes;
8. later learning from verified trajectories.

If not, it is probably not core infrastructure.

---

## 28. Final Rule

Celetixo is built around a strict separation:

```
MODEL
  proposes

ENGINEERING STATE
  describes what is true

TRANSACTION ENGINE
  determines what may change

SANDBOX
  executes untrusted work

VERIFICATION
  establishes evidence

COMMIT
  accepts a validated state transition

EVIDENCE
  records what happened
```

Do not reverse these responsibilities.

The model is not the source of truth.

A green test is not automatically sufficient evidence.

A clean Git merge is not semantic correctness.

A reservation is not correctness.

A successful agent run is not a successful engineering outcome.

Celetixo exists to make those distinctions executable.
