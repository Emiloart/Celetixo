# Celetixo

> **An engineering-state engine for autonomous software development.**

Celetixo is not another coding chatbot.

It is a coordination and verification system for running multiple AI workers against the same codebase without reducing concurrency to file locks, prompt routing, or blind Git merges.

The central abstraction is a **semantic transaction**:

> **Intent → impact analysis → coordinated execution → verification → commit or rebase**

AI models generate the changes. Celetixo owns the engineering state around those changes.

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

The protocol is defined in **Protobuf v3** and documented in [ADR-0001](docs/adr/0001-celetixo-engineering-state-coordination.md).

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

Those are downstream capabilities.

First, Celetixo must prove that its engineering substrate creates measurable advantage.

---

# Day 90: The Actual Test

The first milestone is not "the multi-agent system works."

The question is:

> **Does Celetixo produce better verified engineering outcomes than a strong sequential coding agent on the same work?**

The task suite is frozen before evaluation.

Both systems receive the same repository state and task constraints.

Measure:

- success rate
- cost per successful PR
- latency per successful PR
- regression rate
- human fix rate
- semantic conflict rate
- verification compute
- agent actions

There is no arbitrary 50% threshold.

If the hierarchy does not earn its complexity, the architecture changes.

That is a feature of the design, not a failure condition.

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

See [ADR-0001](docs/adr/0001-celetixo-engineering-state-coordination.md) for the frozen v1.0 architectural decision.

---

## Status

**Architecture:** v1.0 frozen  
**Implementation:** v0 foundation  
**Training:** deferred  
**Evaluation:** controlled benchmark under construction

Celetixo will make performance claims only from its own reproducible engineering measurements.

---

## License

License and contribution policy will be defined as implementation begins.
