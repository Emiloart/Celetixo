# Celetixo

> Autonomous software engineering infrastructure built around code intelligence, semantic coordination, verification, and learning.

Celetixo is an engineering-state coordination system in which AI agents act as execution workers against shared software repositories.

The v0 objective is deliberately narrow:

**Prove that a coordinated multi-agent system can outperform a strong sequential coding agent on the same engineering tasks, measured by verified outcomes rather than model benchmarks alone.**

The **v1.0 architecture is frozen in [ADR-0001](docs/adr/0001-celetixo-engineering-state-coordination.md)**.

---

## Core Loop

```
Engineering Task
      ↓
Code Intelligence
      ↓
Intent / Invariants
      ↓
Semantic Transaction
      ↓
Impact Analysis
      ↓
Coordinated Workers
      ↓
Tiered Verification
      ↓
Commit / Abort / Rebase
      ↓
Immutable Trajectory
```

The key abstraction is not a file edit. It is an **intent-driven, semantically analyzed, verifiably executed repository state transition**.

---

# v0 Scope

## In

### 1. Code Intelligence

Build a deterministic representation of the repository:

- Tree-sitter parsing
- symbol extraction
- dependency relationships
- references and call relationships
- language-server-backed semantic information
- incremental graph updates
- dependency and configuration impact

The model should query structural facts instead of repeatedly rediscovering them from raw files.

### 2. Semantic Transactions

Celetixo uses intent-driven optimistic concurrency control.

The Chief/Supervisor declares:

- objective
- explicit reads
- explicit writes
- invariants
- task dependencies
- verification requirements
- resource constraints

The Engineering State Engine derives the transaction footprint and impact set, coordinates short-lived write reservations, validates against repository revisions, and controls commit, abort, or rebase.

Workers do not directly manipulate semantic locks.

### 3. Coordinated Agent Runtime

```
Chief
  └── Supervisor
        ├── Worker
        ├── Worker
        └── Worker
```

The Chief owns global objectives and task decomposition.

Supervisors own domain/task coordination.

Workers perform bounded implementation and verification work.

Workers and supervisors publish relevant state changes through the event fabric without routing every event through the Chief.

### 4. Tiered Verification

Verification escalates according to the change:

1. syntax / parse
2. formatting / lint
3. type checking / static analysis
4. targeted tests
5. subsystem/integration tests
6. broader E2E verification when required

The system records which levels ran and why.

### 5. Tiered Isolated Execution

Execution uses the cheapest environment that provides sufficient safety and evidence:

- Tier 0: deterministic local analysis
- Tier 1: isolated process/container/gVisor
- Tier 2: Firecracker microVM
- Tier 3: future remote execution pool

Firecracker is an execution backend, not an architectural coupling point.

### 6. Immutable Trajectory Logging

Meaningful execution records include:

- repository revision
- relevant code graph state
- transaction intent
- derived impact set
- reservations
- model/configuration provenance
- prompts/instructions
- tool calls
- patches
- execution output
- verification results
- rollback/rebase events
- final diff
- human review outcome

The trajectory is an engineering evidence record, not merely a chat transcript.

### 7. Baseline Models

Use existing models through a swappable model interface.

**No Celetixo-specific weight training is required for v0.**

Different models can fill Chief, Supervisor, and Worker roles without changing the coordination runtime.

### 8. Comparative Evaluation

Build a fixed task suite before Day 90.

Every task must be executable by:

- a strong sequential baseline agent
- Celetixo

Both receive the same repository state, task description, and relevant constraints.

Primary measurements:

- task success rate
- cost per successful PR
- latency per successful PR
- regression rate
- human intervention/fix rate
- agent actions
- verification compute
- semantic conflict rate

---

# Explicitly Out

The following are not v0 deliverables:

- proprietary foundation model training
- reinforcement learning
- GRPO/PPO infrastructure
- large-scale synthetic task generation
- millions of trajectories
- model distillation
- autonomous model improvement
- public marketplace
- polished consumer IDE
- visual website generation as the primary capability
- messaging-platform integrations
- autonomous production deployment
- production incident remediation
- mobile-device farms
- multi-repository enterprise orchestration
- arbitrary browser automation
- replacing every existing coding-agent feature
- benchmark optimization without corresponding engineering outcomes

---

# Day-90 Exit Criteria

Day 90 is a comparative engineering experiment, not a feature-count milestone.

### A. End-to-end core loop

A real repository can move through:

```
task
→ graph analysis
→ transaction planning
→ coordinated execution
→ verification
→ commit/abort/rebase
→ trajectory
→ final result
```

without manual orchestration between every stage.

### B. Semantic coordination

At least two workers can operate concurrently on related repository areas while the transaction system detects incompatible changes and prevents or resolves unsafe execution.

Conflicts must be reported as semantic relationships, not only Git line conflicts.

### C. Adaptive verification

Low-risk changes receive targeted verification.

Higher-risk changes escalate.

Every escalation is observable.

### D. Complete trajectories

Every evaluation run has a replayable record containing graph state, transaction state, actions, execution results, verification, and final diff.

### E. Complexity earns itself

Celetixo is compared directly with a strong sequential agent on the frozen task suite.

Report at minimum:

```
success rate
cost / successful PR
latency / successful PR
regression rate
human fix rate
semantic conflict rate
```

There is no arbitrary required percentage before measurement.

### F. Frozen evaluation

The task suite, baseline configuration, measurement definitions, and acceptance rules are frozen before the final comparison.

---

# Architecture Direction After v0

If v0 demonstrates measurable value from coordinated execution:

```
v0
Core coordination + verification + trajectories
        ↓
v1
Persistent engineering state graph + procedural skills
        ↓
v2
Learned routing + specialized worker post-training
        ↓
v3
Verifiable-reward training + synthetic task generation
        ↓
v4
Model distillation + predictive engineering
        ↓
v5
Autonomous software lifecycle management
```

The model layer remains downstream of the engineering environment.

The long-term objective is not a larger coding chatbot. It is an engineering system in which:

**models + code intelligence + coordination + execution + verification + memory + learning form a closed loop.**

---

# Design Principles

### Engineering state over raw context

The system should understand repository state, not merely retrieve more text.

### Semantic coordination over file locking

Conflicts are about dependencies, contracts, and affected state, not only overlapping lines.

### Verification over confidence

Agent confidence is not evidence that a change works.

### Evidence over benchmark theater

Performance is measured by verified engineering outcomes.

### Data before training

Do not train models until the environment produces high-quality verified trajectories.

### Complexity must earn itself

Every additional agent, model tier, service, and orchestration layer must demonstrate measurable value.

---

## Architecture

**Frozen baseline:** [ADR-0001: Celetixo Engineering-State Coordination Architecture](docs/adr/0001-celetixo-engineering-state-coordination.md)

The ADR defines:

- semantic transactions and OCC
- impact-set derivation
- repository revision and commit semantics
- Tree-sitter/LSP/SCIP code intelligence
- language engineering harnesses
- tiered execution
- PostgreSQL / ClickHouse / object-storage boundaries
- trajectory evidence
- Chief Agent Protobuf contract
- Temporal / NATS responsibilities
- security and failure semantics

---

## Repository Status

**Current stage:** v1.0 architecture frozen / v0 implementation

**Primary objective:** establish the coordination, verification, and trajectory loop.

**Training:** deferred.

**Public benchmark claims:** intentionally avoided until Celetixo has its own controlled measurements.

---

## License

License and contribution policy will be defined as implementation begins.
