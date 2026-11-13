# Celetixo

> **An engineering-state engine for autonomous software development.**

Celetixo is not another coding chatbot.

It is a coordination and verification system for running multiple AI workers against the same codebase without reducing concurrency to file locks, prompt routing, or blind Git merges.

The central abstraction is a **semantic transaction**:

> **Intent → impact analysis → coordinated execution → verification → commit or rebase**

AI models generate the changes. Celetixo owns the engineering state around those changes.

**Documentation:** Start with the [documentation index](docs/README.md) for intended readers, document authority and implementation/validation status.

## Run the current Engine

Install Rust through rustup; this repository pins the toolchain and dependency lockfile. From the repository root:

```sh
cargo test --workspace --locked
cargo run --locked -p celetixo-engine --example concurrent_transactions
```

The example demonstrates compatible transactions committing after explicit revalidation and fresh evidence, and an invalidated read being rejected. Its mutation model remains an in-memory reference over declared integer-valued engineering elements. The repository-intelligence crate captures real Git/workspace inputs, builds versioned Tree-sitter indexes, supplements supported LSP relationships and derives deterministic impact with explicit uncertainty. Source integer patches are rejected; source publication and distributed durability remain separate contracts.

See the [Goal 01 implementation contract](docs/implementation/goal-01.md) for all build checks, the trust and atomicity boundaries, and the [validation record](docs/implementation/validation.md) for actual results.

Inspect a repository without executing its source:

```sh
cargo run --locked -p celetixo-repository --example inspect_repository -- /path/to/repository --changed src/example.ts
```

Git and a C compiler are required. Live capture reports its atomicity limitation; syntax parsing and individual LSP queries retain semantic coverage gaps. See [Goal 02 instructions and limitations](docs/implementation/goal-02.md), [validation](docs/implementation/goal-02-validation.md) and [measured benchmarks](docs/benchmarks/goal-02.md).

[Goal 03](docs/implementation/goal-03.md) implements exact-base source patches, scoped execution admission, policy-owned checks, a validated Linux T1 path and independent persisted-evidence admission. The [acceptance record](docs/implementation/goal-03-acceptance.md), [milestone history](docs/implementation/goal-03-progress.md) and [measured baseline](docs/benchmarks/goal-03.md) state the tested scope and limits. The validated execution harness checks Go syntax with pinned gofmt; incomplete real-source analysis still blocks execution. Source revision publication, production distributed services, stronger tiers and broader execution harnesses remain unsupported.

[Goal 04 progress](docs/implementation/goal-04-progress.md) records bounded PostgreSQL native commit authority and source lifecycle recovery through evidence receipts. [Original mutation outcomes](docs/implementation/goal-04-mutation-recovery.md) add typed transaction, repository and administrative recovery. Complete source commit, executor transition recovery and the remaining distributed service and operational gates are still in progress.

---

## Why Celetixo Exists

Today's coding agents are increasingly capable at individual tasks. The harder problem begins when several agents work on one repository.

A worker can change a function while another changes its caller.  
A third can modify a dependency while tests are running against the old dependency graph.  
A patch can apply cleanly while its interface contract is already invalid.

Git detects textual conflicts. It does not understand the engineering intent behind a change.

Celetixo is built around the missing layer:

```
              SOFTWARE REPOSITORY
                      │
              ┌───────▼────────┐
              │ Engineering    │
              │ State Graph    │
              └───────┬────────┘
                      │
              ┌───────▼────────┐
              │   Semantic     │
              │  Transactions  │
              └───────┬────────┘
                      │
        ┌─────────────┼─────────────┐
        ▼             ▼             ▼
     Worker A      Worker B      Worker C
        │             │             │
        └─────────────┼─────────────┘
                      ▼
              Tiered Verification
                      │
             ┌────────┴────────┐
             ▼                 ▼
          COMMIT              ABORT
                               │
                             REBASE
```

The objective is not to make more agents talk to each other.

The objective is to make **parallel software engineering safe, measurable, and economically useful**.

---

# The Core Primitive: Semantic Transactions

A worker does not receive a file and start editing.

The system first establishes what the worker intends to change.

For example:

```
Transaction
├── Intent
│   └── Rotate refresh-token implementation
│
├── Explicit writes
│   ├── AuthService.refreshToken
│   └── TokenPayload
│
├── Explicit reads
│   └── UserRepository.findById
│
├── Derived impact
│   ├── TokenPayload consumers
│   ├── refreshToken callers
│   └── affected tests
│
├── Invariants
│   ├── public authentication contract preserved
│   └── type safety preserved
│
└── Verification
    ├── targeted authentication tests
    ├── type checking
    └── downstream consumer tests
```

The worker may only know part of that footprint.

The **Engineering State Engine derives the rest** from the repository graph.

That distinction is fundamental.

---

# The Engineering State Engine

The Rust core is the authoritative semantic layer.

It maintains the relationship between:

- repositories
- revisions
- files
- symbols
- references
- calls
- types
- dependencies
- configuration
- transactions
- verification state

Its job is to answer questions such as:

- What does this change actually affect?
- Can these two transactions execute concurrently?
- Which assumptions became invalid after another transaction committed?
- What must be reverified?
- Is this patch still valid against the current repository revision?

The engine uses **optimistic concurrency control** with short-lived reservations.

Reservations coordinate concurrent work.  
**Verification establishes correctness.**

---

# Agent Hierarchy

Celetixo separates strategic reasoning from execution.

```
                         CHIEF
                           │
             global objective / system DAG
                           │
          ┌────────────────┼────────────────┐
          ▼                ▼                ▼
     SUPERVISOR        SUPERVISOR        SUPERVISOR
       backend           frontend          platform
          │                │                │
      ┌───┼───┐        ┌───┼───┐        ┌───┼───┐
      ▼   ▼   ▼        ▼   ▼   ▼        ▼   ▼   ▼
     W1  W2  W3       W4  W5  W6       W7  W8  W9
```

**Chief**

Owns the global objective, architecture, task decomposition, invariants, and escalation.

**Supervisors**

Own domain execution, dependency ordering, worker allocation, and local recovery.

**Workers**

Perform bounded implementation and verification tasks.

Supervisors and workers communicate through the event fabric. The Chief is not a message broker.

---

# Code Intelligence

Celetixo treats repository structure as first-class state.

### Structural layer

- Tree-sitter parsing
- symbol extraction
- scope and reference analysis
- call/dependency relationships
- incremental graph updates

### Semantic layer

- shared language servers
- versioned document state
- LSP query multiplexing
- SCIP-style static indexes where useful

### Engineering layer

- package manifests
- lockfiles
- build graphs
- CI configuration
- Dockerfiles
- YAML / JSON / TOML / HCL
- generated artifacts and their provenance

A language is not considered supported because an LLM can write its syntax.

It is supported when Celetixo can **index, coordinate, execute, verify, and record work in that ecosystem**.

---

# Initial Language Wedge

Celetixo starts with:

**TypeScript / JavaScript · Python · Go · Rust**

Each language receives an engineering harness covering:

1. structural parsing and indexing
2. semantic/LSP integration
3. deterministic formatting and linting
4. executable test and diagnostic adapters
5. dependency/lockfile handling
6. build-cache integration

Examples:

- **TypeScript / JavaScript:** TypeScript compiler, Biome, Vitest or repository-native tests
- **Python:** Pyright/MyPy, Ruff, pytest
- **Go:** gofmt, compiler diagnostics, go test
- **Rust:** rustfmt, cargo check/test, structured rustc diagnostics

The harness is the unit of language support.

---

# Verification Is Part of the Architecture

Celetixo does not ask a model whether its code is correct.

It progressively gathers evidence.

```
Tier 0  Parse / AST / patch validation
   ↓
Tier 1  Format / lint / type / static analysis
   ↓
Tier 2  Targeted tests
   ↓
Tier 3  Integration / subsystem verification
   ↓
Tier 4  Broad E2E when the impact requires it
```

A local variable rename should not trigger an entire production-scale test matrix.

A public interface change should.

The verification engine records **what ran, why it ran, and what evidence it produced**.

---

# Execution

The sandbox is selected according to risk and workload.

```
T0  Rust-local deterministic operations
    parsing • graph queries • patch checks

T1  isolated process / container / gVisor
    type checking • linting • unit tests

T2  Firecracker microVM
    untrusted execution • dependency installation
    integration tests • stronger isolation

T3  remote execution pools
    future large builds • GPU workloads • test matrices
```

Firecracker is an implementation backend, not a dependency of the transaction model.

Build caches and dependency caches are treated separately from VM snapshots because cache state, not only process warmness, determines verification economics.

---

# The Evidence Loop

Every meaningful run becomes an engineering record.

```
Repository revision
       ↓
Transaction intent
       ↓
Derived impact set
       ↓
Worker actions
       ↓
Patch
       ↓
Execution
       ↓
Verification
       ↓
Commit / Abort / Rebase
       ↓
Human outcome
```

Celetixo stores enough information to reconstruct that trajectory.

This produces the dataset required for later:

- procedural skill extraction
- learned routing
- worker post-training
- verifiable-reward training
- model distillation

**Training comes after evidence.**

The system does not assume that collecting model conversations is equivalent to collecting useful training data.

---

# System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ CONTROL PLANE                                               │
│ TypeScript / Next.js                                        │
│ Review • execution views • administration • evaluation UI  │
└──────────────────────────────┬──────────────────────────────┘
                               │
                         gRPC / WebSocket
                               │
┌──────────────────────────────▼──────────────────────────────┐
│ ORCHESTRATION                                               │
│ Go / Temporal                                               │
│ Durable workflows • DAGs • retries • compensation          │
└──────────────────────────────┬──────────────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────┐
│ EVENT FABRIC                                                │
│ NATS JetStream                                              │
│ Worker events • heartbeats • diagnostics • state signals   │
└──────────────────────────────┬──────────────────────────────┘
                               │
                         gRPC / Protobuf
                               │
┌──────────────────────────────▼──────────────────────────────┐
│ ENGINEERING STATE ENGINE                                    │
│ Rust                                                       │
│ Transactions • graph • LSP • OCC • sandbox scheduling     │
└──────────────┬───────────────────┬──────────────────────────┘
               │                   │
       ┌───────▼───────┐   ┌──────▼────────┐
       │ PostgreSQL    │   │ ClickHouse    │
       │ authoritative │   │ telemetry     │
       │ state         │   │ + evaluation  │
       └───────────────┘   └───────────────┘
               │
       ┌───────▼─────────────────────────┐
       │ S3-compatible object storage    │
       │ trajectories • patches • logs   │
       │ diagnostics • artifacts         │
       └─────────────────────────────────┘
```

The boundaries are intentional:

- **Temporal** owns durable workflow execution.
- **NATS** owns asynchronous event distribution.
- **Rust** owns semantic repository coordination.
- **PostgreSQL** owns authoritative current state.
- **ClickHouse** owns historical analytics.
- **Object storage** owns large immutable evidence.

No component is allowed to become an accidental replacement for another.

---

# Chief Agent Protocol

The Chief does not issue unconstrained instructions such as:

> "Go fix authentication."

It submits a typed intent.

The internal contract carries:

- repository and base revision
- objective
- explicit reads/writes
- invariants
- task dependencies
- impact policy
- verification requirements
- resource budget
- retry/rebase policy
- escalation policy
- model provenance

The Engineering State Engine decides the transaction mechanics.

The accepted target uses Protobuf v3; the ADR-0001 TaskIntent example is architectural, not an implemented wire schema. No canonical .proto source, generated bindings or deployed RPC service is currently present. See the [protocol reference](docs/reference/protocol.md).

---

# What Is Deliberately Not Being Built Yet

Celetixo is intentionally not starting with:

- training a proprietary foundation model
- RL/GRPO/PPO
- millions of synthetic tasks
- model distillation
- autonomous self-improvement
- a consumer IDE
- browser automation
- autonomous production deployment
- production incident remediation
- marketplace integrations
- enterprise multi-repository orchestration

These are downstream capabilities, not architectural exclusions. Their implementation may be sequenced behind foundational correctness, but the architecture must preserve clean extension points for them. Build time is not a reason to remove a capability from the target design.

First, Celetixo must establish the engineering substrate and prove its behavior with reproducible evidence.

---

# Evaluation: The Actual Test

The primary question is whether semantic transaction coordination improves verified engineering outcomes beyond both sequential execution and ordinary parallel worktrees.

The evaluation has three comparison arms:

1. **Sequential agent:** one strong coding agent completes the task sequentially.
2. **Parallel worktrees:** multiple agents work in isolated Git worktrees and integrate through ordinary Git merge/review, without Celetixo semantic transactions.
3. **Celetixo:** workers use semantic impact analysis, reservations, OCC validation, evidence-gated commit and semantic revalidation/rebase.

The second arm isolates the value of Celetixo's coordination model from the generic effects of parallel execution.

The task suite must be frozen before confirmatory evaluation. Arms receive equivalent repository snapshots, task constraints, tools and resource policies. Stochastic runs are repeated, and results report distributions and uncertainty rather than best-case anecdotes.

Measure verified task success, cost and time per verified outcome, regressions, human correction, throughput, impact precision/recall, false-negative impact, conflict precision/recall, false-positive conflicts, abort reasons, wasted execution cost, wasted-token ratio, verification cost and recovery correctness. Safety violations are reported separately and cannot be offset by speed or cost improvements.

High abort rate is not inherently bad: correct safety aborts must be distinguished from false-positive conflicts and avoidable wasted work.

The protocol, task-suite requirements, controls, metric definitions and decision rule are maintained in [EVALUATION.md](EVALUATION.md). Transaction-oracle and failure-injection requirements are specified in [ADR-0003](docs/adr/0003-transaction-consistency-and-validation-model.md) and [the failure model](docs/spec/failure-model.md).

There is no arbitrary pass threshold. Confirmatory decision rules must be published before the evaluation runs. This repository currently defines the protocol; it does not claim that Celetixo has passed it.

---

# Roadmap

```
v0  Semantic coordination + verification + evidence
 │
 ▼
v1  Persistent engineering graph + procedural skills
 │
 ▼
v2  Learned routing + specialized worker post-training
 │
 ▼
v3  Verifiable-reward training + synthetic environments
 │
 ▼
v4  Distilled engineering models + predictive coordination
 │
 ▼
v5  Autonomous software lifecycle management
```

The long-term system is not:

```
prompt → model → code
```

It is:

```
engineering state
      ↓
intent
      ↓
semantic coordination
      ↓
execution
      ↓
verification
      ↓
evidence
      ↓
learning
      ↺
```

---

## Architectural Rule

**Models are workers. Engineering state is the source of truth. Verification is the authority. Evidence is the learning substrate.**

See [ADR-0001](docs/adr/0001-celetixo-engineering-state-coordination.md), [ADR-0002](docs/adr/0002-platform-brain-client-and-correctness-extensions.md), [ADR-0003](docs/adr/0003-transaction-consistency-and-validation-model.md) and [ADR-0004](docs/adr/0004-internal-first-public-capable-architecture.md) for the accepted architectural decisions. [Deployment and product boundaries](docs/spec/deployment-and-product-boundaries.md) defines the internal-first operating model and future public-release requirements.

---

## Status

**Architecture:** ADR-0001 baseline plus ADR-0002, ADR-0003 and ADR-0004 accepted extensions  
**Implementation:** in-memory Rust reference Engine; repository capture, versioned structural/LSP indexes and impact analysis; exact-base source patches, scoped execution admission, local persisted evidence and a validated Linux x86_64 rootless-runc T1 profile. See [Goal 01](docs/implementation/goal-01.md), [Goal 02](docs/implementation/goal-02.md), [Goal 03 acceptance](docs/implementation/goal-03-acceptance.md) and the [capability matrix](docs/reference/capability-support-matrix.md). [Goal 04](docs/implementation/goal-04-progress.md) adds bounded PostgreSQL native commit and source lifecycle/mutation recovery. Source revision publication, complete distributed services, T2/T3 and production multi-tenant operation remain unsupported or target-only.
**Training:** deferred until trajectory quality, rights and governance are established  
**Evaluation:** Goal 02/03 measurements are published; the confirmatory three-arm agent evaluation remains future work

Celetixo will make performance claims only from its own reproducible engineering measurements.

---



## Platform and Brain Architecture

ADR-0002 extends the original architecture without replacing its transaction model.

**Brain runtime:** Python owns model/provider adapters, routing, context assembly, planning, Chief/Supervisor reasoning, worker allocation, evaluation and future learning pipelines. The Brain submits typed intent; it does not own repository truth, OCC, verification admissibility or commit authority.

**Native systems layer:** Rust remains authoritative for the Engineering State Engine and is used for deterministic, performance-sensitive operations. Python/Rust integration is justified at explicit boundaries. C++ is optional and requirement-driven, not a mandatory third language.

**Desktop workbench:** Eclipse Theia is the preferred candidate, subject to extension compatibility, licensing, packaging, security and operational validation. The desktop is a client, never an alternate transaction authority.

**Android:** Kotlin + Jetpack Compose is the native mobile direction. The mobile client provides secure monitoring, review, notifications and governed approvals; it does not replicate the Engine. Flutter is not part of the selected architecture.

## Internal-First Operating Boundary

Celetixo is initially operated internally by Emilo Labs under Emilo Labs' data-governance policies. **Internal-first is an operating and data-policy decision, not a reduced engineering target.**

The complete accepted architecture remains the target: transaction correctness, security, isolation, distributed execution, multiple repositories/workspaces, evidence integrity, provider neutrality, testing and evaluation are not downgraded or deferred because the first deployment is internal.

The core platform must not hardcode a single organization or create internal-only paths that bypass authentication, authorization, semantic analysis, verification, sandboxing or atomic commit. Public-facing onboarding, billing and customer administration may be staged; their foundational identity, scope, policy and isolation boundaries must be designed into the platform.

See [ADR-0004](docs/adr/0004-internal-first-public-capable-architecture.md) and the [deployment and product boundaries specification](docs/spec/deployment-and-product-boundaries.md). Internal deployment is the first operating configuration of the same platform, not a separate prototype or architecture.

## Snapshot and Evidence Correctness

A Git commit SHA alone does not identify a dirty workspace. Transactions must use immutable snapshot identity that accounts for included tracked/untracked content, staged and unstaged changes, editor buffers/overlays and relevant workspace inputs. Impact analysis and verification must identify the exact snapshot, index/analyzer version, patch digest, checks, toolchain and environment.

Unknown or incomplete impact is explicit; it is never silently interpreted as safe. A stale verification result cannot authorize a current commit. The commit gate must atomically recheck revision, snapshot, patch, evidence, transaction state, authorization and idempotency.

## Protocol and Security

Protobuf v3 is the accepted target for canonical internal typed contracts. ADR-0001 contains the TaskIntent architectural example, but the reviewed repository does not yet contain a canonical .proto source, generated bindings, or a deployed RPC service. See the [protocol reference](docs/reference/protocol.md); do not describe the wire protocol as implemented. The target architecture assigns gRPC to typed RPC, NATS JetStream to asynchronous events, and HTTPS/JSON to external interoperability.

An actor identifier is attribution, not authorization. Human approvals bind to the exact transaction, snapshot, diff and evidence. Force-rebase requests semantic reanalysis and revalidation; they never bypass commit checks.

## Long-term Architecture Rule

Implementation sequencing may reduce integration risk, but must not impose an arbitrary time ceiling or permanently exclude capabilities from the architecture. Additions must strengthen or preserve correctness, security, operability and scalability. A proposal is not accepted merely because it is fashionable; it must have a clear responsibility, contract, threat model, failure semantics and validation plan.

See [ADR-0002](docs/adr/0002-platform-brain-client-and-correctness-extensions.md) and [ADR-0003](docs/adr/0003-transaction-consistency-and-validation-model.md) for accepted extensions. ADR-0001 remains authoritative wherever later ADRs do not explicitly extend it.

Detailed contracts: [transaction semantics](docs/spec/transaction-semantics.md), [engineering identity](docs/spec/engineering-identity.md), [trajectory governance](docs/spec/trajectory-governance.md), [failure and recovery](docs/spec/failure-model.md), and [comparative evaluation](EVALUATION.md).

## Licensing and Distribution

No repository-wide software license has been selected or published. Celetixo is developed internal-first; no public redistribution license is granted by the repository. [CONTRIBUTING.md](CONTRIBUTING.md) already defines human-only Git attribution and its enforcement; it is not a software license. See the [licensing and distribution policy](docs/spec/licensing-and-distribution.md) before copying or distributing project material.

## Documentation

Start with the [documentation index](docs/README.md), which identifies intended readers, normative authority, implementation contracts, validation records, historical milestones and current support status. Key references: [threat model](docs/spec/threat-model.md), [local Linux T1 operations runbook](docs/operations/local-linux-t1-runbook.md), [capability/support matrix](docs/reference/capability-support-matrix.md), and [protocol reference](docs/reference/protocol.md).
