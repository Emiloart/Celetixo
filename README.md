# Celetixo

> Autonomous software engineering infrastructure built around code intelligence, semantic coordination, verification, and learning.

Celetixo is an experimental engineering system for coordinating multiple coding agents against the same software system without treating the repository as a collection of unrelated text files.

The v0 objective is deliberately narrow:

**Prove that a coordinated multi-agent system can outperform a strong sequential coding agent on the same engineering tasks, measured by verified outcomes rather than model benchmarks alone.**

---

## v0: The Core Loop

Celetixo v0 contains four tightly connected systems:

```
                    ┌──────────────────────┐
                    │   Engineering Task   │
                    └──────────┬───────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │   Code Intelligence  │
                    │ AST + symbols + deps │
                    └──────────┬───────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │ Semantic Transaction │
                    │ intent + dependencies│
                    └──────────┬───────────┘
                               │
                    ┌──────────▼───────────┐
                    │ Coordinated Workers  │
                    │ isolated execution   │
                    └──────────┬───────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │ Tiered Verification  │
                    │ syntax → tests → E2E │
                    └──────────┬───────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │ Immutable Trajectory │
                    │ state → actions →    │
                    │ outcome → review     │
                    └──────────────────────┘
```

The resulting trajectories become the foundation for later skills, routing improvements, post-training, and reinforcement learning.

---

# v0 Scope

## In

### 1. Code Intelligence

Build a deterministic representation of the repository:

- Tree-sitter parsing
- symbol extraction
- dependency relationships
- references and call relationships
- language-server-backed type information where available
- incremental graph updates after changes

The LLM should query structural facts instead of repeatedly rediscovering them from raw files.

### 2. Semantic Transactions

Workers must declare their intended change before execution.

A transaction contains:

- objective
- files and symbols expected to change
- read dependencies
- write dependencies
- affected contracts
- verification requirements

The coordination engine determines whether transactions can execute concurrently.

The first version may use AST-level locks, but the abstraction must be transaction-oriented rather than file-lock-oriented.

### 3. Coordinated Agent Runtime

Implement the minimum hierarchy required to test coordination:

```
Chief
  └── Supervisor
        ├── Worker
        ├── Worker
        └── Worker
```

The Chief owns the global task DAG and system-level objective.

The Supervisor owns domain/task coordination.

Workers perform bounded implementation and verification tasks.

Workers and supervisors must be able to publish relevant state changes without forcing every event through the Chief.

### 4. Tiered Verification

Verification must escalate according to the change:

1. syntax / parse validity
2. formatting / lint
3. type checking / static analysis
4. targeted unit or component tests
5. subsystem/integration tests
6. broader E2E verification when required

A change should not trigger expensive verification when deterministic lower-level evidence is sufficient.

The system must record exactly which verification levels were executed and why.

### 5. Isolated Execution

Provide isolated execution environments for workers.

v0 should support a practical sandbox abstraction and must not couple the coordination layer directly to one runtime implementation.

Firecracker is a candidate execution backend, not the architectural contract.

### 6. Immutable Trajectory Logging

Every meaningful agent execution must capture:

- repository state
- code graph state relevant to the task
- transaction declarations
- locks acquired/released
- model and model configuration
- prompts/instructions
- tool calls
- file/symbol mutations
- execution output
- verification results
- final diff
- rollback/recovery events
- human review outcome

The trajectory is an engineering record, not merely a chat transcript.

### 7. Baseline Models

Use existing models through a swappable model interface.

**No Celetixo-specific weight training is required for v0.**

The architecture must allow different models for Chief, Supervisor, and Worker roles without changing the coordination runtime.

### 8. Comparative Evaluation

Build a fixed task suite before Day 90.

Every task must be executable by:

- a strong sequential baseline agent
- Celetixo

Both systems receive the same repository state, task description, and relevant constraints.

Primary measurements:

- task success rate
- cost per successful PR
- latency per successful PR
- regression rate
- human intervention/fix rate
- number of agent actions
- verification compute
- semantic conflict rate

---

# Explicitly Out

The following are **not v0 deliverables**:

- training a proprietary foundation model
- reinforcement learning
- GRPO/PPO infrastructure
- large-scale synthetic task generation
- millions of trajectories
- model distillation
- autonomous model improvement
- a public marketplace
- a polished consumer IDE
- visual website generation as a primary capability
- messaging-platform integrations
- autonomous production deployment
- production incident remediation
- mobile-device farms
- multi-repository enterprise orchestration
- arbitrary browser automation
- replacing every existing coding-agent feature
- benchmark optimization without corresponding engineering outcomes

These may become later capabilities. They do not belong in the first proof.

---

# Day-90 Exit Criteria

Day 90 is a **comparative engineering experiment**, not a feature-count milestone.

Celetixo exits v0 only if all of the following are true:

### A. The core loop works end-to-end

A real repository can move through:

```
task
→ graph analysis
→ transaction planning
→ coordinated execution
→ verification
→ trajectory
→ final result
```

without manual orchestration between every stage.

### B. Semantic coordination is demonstrable

At least two workers can operate concurrently on related parts of a repository while the transaction system detects incompatible changes and prevents or resolves unsafe execution.

The system must report conflicts as semantic relationships, not only Git line conflicts.

### C. Verification is adaptive

The system can select targeted verification for low-risk changes and escalate verification for higher-risk changes.

Every escalation must be observable.

### D. Trajectories are complete

A replayable record exists for every evaluation run, including graph state, transaction state, actions, execution results, verification, and final diff.

### E. The hierarchy earns its complexity

On the fixed Day-90 task suite, Celetixo must be compared directly against a strong sequential agent.

The result must report at minimum:

```
success rate
cost / successful PR
latency / successful PR
regression rate
human fix rate
semantic conflict rate
```

There is **no arbitrary required percentage** before measurement.

The hierarchy remains in the architecture only if the measured evidence shows that coordination provides a meaningful engineering advantage.

### F. No hidden success criteria

The task suite, baseline configuration, measurement definitions, and acceptance rules must be frozen before the final comparison.

---

# Architecture Direction After v0

If v0 demonstrates that coordinated execution creates measurable value, development can expand in this order:

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

The model layer is downstream of the engineering environment.

The long-term objective is not to build a larger coding chatbot. It is to build an engineering system in which:

**models + code intelligence + coordination + execution + verification + memory + learning form a closed loop.**

---

## Design Principles

### Engineering state over raw context

The system should know what a repository means, not merely retrieve more of it.

### Semantic coordination over file locking

Conflicts are about dependencies and contracts, not only overlapping lines.

### Verification over confidence

An agent's confidence is not evidence that a change works.

### Evidence over benchmark theater

Performance is measured by verified engineering outcomes.

### Data before training

Do not train models until the environment produces high-quality verified trajectories.

### Complexity must earn itself

Every additional agent, model tier, service, and orchestration layer must demonstrate measurable value.

---

## Repository Status

**Current stage:** v0 specification / initial implementation

**Primary objective:** establish the coordination, verification, and trajectory loop.

**Training:** deferred.

**Public benchmark claims:** intentionally avoided until Celetixo has its own controlled measurements.

---

## License

License and contribution policy will be defined as implementation begins.
