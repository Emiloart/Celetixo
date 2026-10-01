# ADR-0001: Celetixo Engineering-State Coordination Architecture

- **Status:** Accepted
- **Date:** 2026-10-01
- **Decision scope:** Celetixo v1.0 frozen architecture baseline
- **Supersedes:** v0 conceptual coordination model

## Context

Celetixo coordinates multiple AI execution workers against shared software repositories. File locking and prompt routing are insufficient because software changes propagate through symbols, references, interfaces, dependencies, build artifacts, configuration, and tests.

Celetixo therefore needs an explicit representation of engineering state and a concurrency model that reasons about proposed changes before and after code generation.

The initial language wedge is TypeScript/JavaScript, Python, Go, and Rust. A language is supported only when its engineering harness provides structural indexing, semantic queries, executable verification, deterministic formatting/linting, dependency handling, and build/test caching appropriate to that ecosystem.

## Decision

Celetixo is an **engineering-state coordination and optimistic concurrency engine in which AI agents act as execution workers**.

```
CONTROL PLANE
TypeScript / Next.js
Human review • telemetry • dashboards
              |
       REST / WebSocket
              |
ORCHESTRATION
Go / Temporal
Durable workflows • DAGs • retries • supervisor state
              |
       gRPC / Protobuf
              |
EVENT FABRIC
NATS JetStream
Heartbeats • state signals • streamed diagnostics
              |
       Events + gRPC
              |
ENGINEERING STATE ENGINE
Rust
Semantic transactions • code intelligence • OCC
LSP multiplexing • sandbox scheduling • build cache
              |
       +------+------+ 
       |      |      |
 PostgreSQL ClickHouse Object Storage
 system     analytics  immutable evidence
 state                 / Parquet
```

### Component ownership

| Subsystem | Technology | Responsibility |
|---|---|---|
| Control plane | TypeScript / Next.js | Human review, administration, live execution views |
| Orchestration | Go / Temporal | Durable workflows, task DAGs, retries, compensation, supervisor state |
| Event fabric | NATS JetStream | High-frequency asynchronous events and durable streams |
| Engineering State Engine | Rust | Semantic transactions, code graph, conflict analysis, execution scheduling |
| Code intelligence | Tree-sitter + LSP + SCIP-style index | Structural and semantic repository state |
| Execution | gVisor / Firecracker | Tiered isolated execution |
| Primary state | PostgreSQL | Authoritative transactional engineering state |
| Analytics | ClickHouse | Historical telemetry and evaluation analytics |
| Evidence | S3-compatible object storage + Parquet | Immutable trajectories and execution artifacts |
| ML/evaluation | Python + vLLM | Dataset processing, evaluation, post-training and model serving |

Temporal owns durable workflow state. NATS owns asynchronous event distribution. Neither replaces the other.

## 1. Semantic Transaction Model

The Chief and Supervisors express **intent and invariants**, not low-level locks.

```
Intent + explicit reads/writes + invariants + dependencies
                    |
                    v
             Semantic analysis
                    |
                    v
             Derived impact set
                    |
                    v
          Conflict / compatibility
                    |
                    v
                 Execute
                    |
                    v
                Validate
                    |
                    v
             Commit / Abort / Rebase
```

The Engineering State Engine derives the impact set from repository state. Workers are not trusted to enumerate every downstream consumer.

A transaction distinguishes:

- **Explicit set:** requested reads and writes.
- **Derived impact set:** symbols, references, contracts, dependencies, and artifacts that may be affected.
- **Write reservation set:** impact areas requiring exclusive coordination.
- **Verification set:** checks required to establish the declared invariants.

### Transaction lifecycle

```
PROPOSED
   |
ANALYZED
   |
RESERVED
   |
EXECUTING
   |
VALIDATING
  /      \
PASS     FAIL
 |         |
COMMITTED ABORTED
             |
           REBASE
```

### OCC semantics

Celetixo uses optimistic concurrency control with short-lived **write reservations**. A reservation coordinates concurrent workers; it is not the source of correctness.

Commit requires:

1. Transaction invariants pass.
2. Validation results correspond to the transaction snapshot.
3. The repository base remains valid, or the transaction has been revalidated against the newer revision.
4. No incompatible committed transaction invalidates its read/write assumptions.
5. The final patch applies cleanly to the validated repository state.

Repository revision advancement uses compare-and-swap semantics at the commit boundary.

Reservations provide read/write compatibility, lease expiration, heartbeat renewal, deterministic acquisition ordering, upgrade handling, cancellation, and crash recovery.

### Rebase semantics

A rebase is not a blind textual Git rebase.

When the repository advances, the engine:

1. identifies the new revision;
2. recomputes affected semantic state;
3. checks the transaction's assumptions against the new state;
4. regenerates or adapts the patch if required;
5. reruns the minimum verification required to re-establish invariants.

If intent is no longer compatible, the transaction is aborted or escalated.

## 2. Semantic State vs. Source Mutation

Celetixo separates semantic intelligence from source mutation.

```
Repository
   |
   +--> Tree-sitter / symbol graph --> semantic read path
   |
   +--> Worker unified patch -------> source mutation
                         |
                         v
                 syntax validation
                         |
                         v
                    formatter
                         |
                         v
                 versioned LSP
                         |
                         v
                    verification
```

Tree-sitter and derived indexes provide parsing, symbol identification, scope analysis, reference discovery, dependency relationships, impact analysis, transaction compatibility, and incremental graph updates.

ASTs are **not** the universal source-writing mechanism.

Workers produce explicit, versioned patches against a known repository revision. This preserves comments, source trivia, formatting conventions, macros, configuration syntax, and language-specific source constructs. AST-aware transformations may be used where advantageous, but AST rendering is not the canonical mutation mechanism.

## 3. Language Engineering Harness

A language is not supported merely because a model can generate valid-looking syntax.

Each supported language requires:

### Structural intelligence
- Tree-sitter parser
- symbol extraction
- reference/index generation
- incremental graph updates

### Semantic intelligence
- shared language-server instance
- query multiplexing
- versioned document state
- SCIP-style static index where useful

Workers do not independently spawn an LSP instance for the same repository.

### Execution and verification

Initial adapters include:

- TypeScript/JavaScript: tsc, Biome, Vitest or repository-native runner
- Python: Pyright/Mypy, Ruff, pytest
- Go: gofmt, go test, native compiler diagnostics
- Rust: rustfmt, cargo check/test, rustc JSON diagnostics

Exact tools remain repository-configurable.

### Dependency and build intelligence

The harness understands dependency manifests and lockfiles, including package.json/pnpm-lock.yaml, pyproject.toml/uv.lock, go.mod/go.sum, and Cargo.toml/Cargo.lock.

Dependency changes invalidate the appropriate graph and cache state. Build performance relies on cache-aware execution as well as process or VM snapshots.

### Non-code engineering artifacts

Early repository intelligence also covers YAML, JSON, TOML, HCL, Dockerfiles, shell, SQL, and CI configuration. These participate in impact analysis even when they are not conventional programming languages.

## 4. LSP Resilience

Workers may temporarily produce invalid source.

Therefore:

- Tree-sitter is the first structural read path for dirty buffers.
- Shared LSP services operate against explicitly versioned repository/document snapshots.
- LSP results are rejected when their document version does not match transaction state.
- Formatting is normalization, not the universal syntax gate.
- Compiler/type-check results are tied to a repository revision and execution environment.

```
Worker edit
   |
Tree-sitter parse
   |
   +-- invalid --> worker correction
   |
   +-- valid --> formatter --> versioned LSP --> compiler/type checker --> tests
```

## 5. Tiered Execution

Execution uses the cheapest environment that provides sufficient safety and evidence.

| Tier | Environment | Typical workload |
|---|---|---|
| Tier 0 | Rust/local deterministic execution | parsing, AST analysis, patch validation, formatting |
| Tier 1 | isolated process/container/gVisor | type checking, linting, unit tests |
| Tier 2 | Firecracker microVM | untrusted execution, dependency installation, integration tests |
| Tier 3 | remote execution pool | future large builds, GPU workloads, large test matrices |

Latency targets are not frozen until measured on representative repositories.

Firecracker is an execution backend, not a coupling point for the transaction engine.

## 6. Storage and Evidence

### PostgreSQL: authoritative current state

Stores repositories, revisions, tasks, transactions, reservations, workers, supervisors, symbols, dependency edges, verification runs, architecture decisions, and provenance.

### ClickHouse: historical analytics

Stores agent actions, model calls, token usage, execution latency, verification events, transaction conflicts, worker failures, and evaluation metrics.

ClickHouse is not authoritative engineering state.

### S3-compatible object storage + Parquet: immutable evidence

Stores complete trajectories, patches, stdout/stderr, compiler diagnostics, test artifacts, snapshots, and large execution outputs.

Object storage is not used as a transactional database.

## 7. Trajectory Requirements

Every meaningful execution preserves enough information to reconstruct what happened:

- repository revision
- relevant semantic graph state
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

This evidence is the foundation for later skill extraction, routing, evaluation, worker post-training, verifiable-reward training, and distillation. No model training is required by this ADR.

## 8. Chief Agent DSL

The Chief communicates through a typed protocol and does not directly manipulate repository locks or low-level worker state.

Protocol Buffers v3 is the canonical internal contract. JSON/JSON Schema may be exposed at external boundaries.

The intent contract expresses task identity, parentage, repository/base revision, objective, explicit reads/writes, invariants, task dependencies, impact policy, verification requirements, resource budget, retry/rollback/escalation policy, and provenance.

The engine derives transaction mechanics from the intent.

### Canonical contract

```proto
syntax = "proto3";

package celetixo.orchestration.v1;

message TaskIntent {
  string task_id = 1;
  string parent_task_id = 2;
  string repository_id = 3;
  string base_revision = 4;
  string objective_description = 5;

  repeated string explicit_reads = 6;
  repeated string explicit_writes = 7;
  repeated string invariant_contracts = 8;
  repeated string depends_on_task_ids = 9;

  ImpactPolicy impact_policy = 10;
  VerificationRequirements verification = 11;
  ResourceBudget budget = 12;
  RetryPolicy retry_policy = 13;
  RollbackPolicy rollback_policy = 14;
  EscalationPolicy escalation_policy = 15;
  Provenance provenance = 16;
}

message ImpactPolicy {
  uint32 max_reference_depth = 1;
  repeated string allowed_domains = 2;
  repeated string forbidden_domains = 3;
}

message VerificationRequirements {
  uint32 minimum_sandbox_tier = 1;
  repeated string required_test_targets = 2;
  bool require_strict_typecheck = 3;
  bool allow_adaptive_escalation = 4;
}

message ResourceBudget {
  uint64 max_token_limit = 1;
  uint64 timeout_seconds = 2;
  uint64 max_cost_microusd = 3;
}

message RetryPolicy {
  uint32 max_attempts = 1;
  bool allow_rebase = 2;
}

message RollbackPolicy {
  bool preserve_failed_artifacts = 1;
  bool preserve_failed_patch = 2;
}

message EscalationPolicy {
  string fallback_target = 1;
  uint32 max_escalations = 2;
}

message Provenance {
  string model_name = 1;
  string model_version = 2;
  string prompt_template_hash = 3;
  string agent_version = 4;
}
```

The transaction state is authoritative in the Engineering State Engine. The DSL requests a transaction; it does not grant itself reservation or commit authority.

## 9. Communication Boundaries

Internal communication uses:

- Protobuf as canonical schema
- gRPC for RPC
- Unix Domain Sockets for co-located low-latency services where appropriate
- gRPC over HTTP/2 between nodes
- NATS JetStream for asynchronous event distribution

HTTP/JSON is primarily an external API/interoperability surface.

## 10. Security and Failure Semantics

Workers can fail and generated code is untrusted until verified.

Required properties include:

- expiring reservations
- heartbeat-based lease renewal
- deterministic conflict handling
- sandbox isolation
- bounded resource budgets
- immutable execution evidence
- provenance tracking
- explicit rollback
- repository revision checks
- no implicit production credentials inside worker sandboxes

A worker crash must not permanently block a repository. A failed transaction must not silently become committed state.

## Consequences

### Positive

- Parallel agent work becomes semantic coordination rather than file locking.
- The Chief focuses on objectives and invariants.
- Workers remain replaceable and model-agnostic.
- Verification becomes a system primitive.
- Trajectories provide the data foundation for later learning.
- Current state, analytics, and immutable evidence scale independently.
- Multiple languages can share one coordination model.

### Costs

- The Engineering State Engine is considerably more complex than a conventional coding-agent runtime.
- Maintaining semantic indexes across changing repositories is difficult.
- LSP lifecycle management requires resource and version controls.
- Impact analysis can be computationally expensive.
- Temporal, NATS, Rust, Go, PostgreSQL, ClickHouse, and object storage create meaningful operational surface area.
- Correctness depends on validation semantics, not merely successful patch application.

These costs are accepted because the Day-90 evaluation tests whether coordinated execution earns the complexity.

## Rejected Alternatives

### File-level locking
Rejected because it does not model callers, interfaces, dependency changes, or semantic impact.

### Symbol-only locking
Rejected because a definition does not represent all affected references and contracts.

### Universal AST rewriting
Rejected as the canonical mutation mechanism because source trivia, macros, configuration constructs, and language-specific semantics are not uniformly preserved by AST rendering.

### One LSP instance per worker
Rejected because memory and warm-up costs scale poorly. Shared, versioned LSP services are preferred.

### Firecracker for every operation
Rejected because deterministic analysis and low-risk checks do not require VM isolation.

### Temporal as the event bus
Rejected because durable workflow orchestration and high-frequency asynchronous events are different concerns.

### NATS as the workflow engine
Rejected because event delivery does not replace durable workflow state, retries, compensation, and long-running DAG semantics.

### Model-first development
Rejected for v0/v1. The engineering environment, verification loop, and trajectory quality must exist before Celetixo trains its own models.

## Implementation Order

1. Repository revision model and immutable state boundaries
2. Tree-sitter parsing and symbol/reference graph
3. Semantic transaction model and OCC validation
4. Patch validation and deterministic verification
5. Tiered sandbox abstraction
6. Shared/versioned LSP service
7. NATS event contracts
8. Temporal orchestration
9. PostgreSQL authoritative state
10. ClickHouse analytical pipeline
11. Immutable trajectory storage
12. Chief/Supervisor/Worker protocol
13. Comparative evaluation harness

## Frozen Baseline

This ADR is the v1.0 architectural baseline.

Changes to transaction semantics, repository revision/commit semantics, Engineering State Engine responsibilities, Chief DSL contract, storage ownership boundaries, orchestration/event-fabric boundaries, or execution isolation require a new ADR or explicit amendment.

Implementation details may evolve when these architectural invariants are preserved.

> **Celetixo coordinates intent-driven, semantically analyzed, verifiably executed repository state transitions. AI models are execution components inside that system, not the system's primary source of truth.**
