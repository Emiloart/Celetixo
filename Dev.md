# Dev.md

## Purpose

This document is the practical build guide for Celetixo developers. It translates ADR-0001 through ADR-0004 into implementation discipline, sequencing, and acceptance criteria.

Start with the [documentation index](docs/README.md) for reader routing and document authority. The [capability/support matrix](docs/reference/capability-support-matrix.md) is the compact implementation-status view; the [threat model](docs/spec/threat-model.md) and [local Linux T1 runbook](docs/operations/local-linux-t1-runbook.md) govern security review and the currently validated local execution profile.

**The objective is to build the complete accepted architecture without weakening correctness, security, independence or scalability.** Implementation may proceed in validated stages, but build time is not an architectural constraint. Do not omit necessary capabilities or choose inferior permanent designs merely to shorten delivery.

Read these documents before changing core behavior:

1. `docs/adr/0001-celetixo-engineering-state-coordination.md` — engineering-state baseline.
2. `docs/adr/0002-platform-brain-client-and-correctness-extensions.md` — accepted platform and correctness extensions.
3. `docs/adr/0003-transaction-consistency-and-validation-model.md` — transaction consistency and commit validation.
4. `docs/adr/0004-internal-first-public-capable-architecture.md` — initial operating model and non-deferred engineering standard.
5. `docs/spec/transaction-semantics.md`, `docs/spec/engineering-identity.md` and `docs/spec/deployment-and-product-boundaries.md` — Engine and deployment contracts.
6. `docs/spec/failure-model.md` and `docs/spec/trajectory-governance.md` — reliability and data governance.
7. `EVALUATION.md` — controlled comparative evaluation.
8. `AGENTS.md` — repository-wide operating contract.
9. `README.md` — public system description.

If this file conflicts with the ADR, the ADR wins. If implementation exposes a flaw in a frozen decision, document the evidence and propose an ADR amendment; do not silently redefine the architecture.

---

## Internal-First Operating Rule

Internal-first defines the initial operator, deployment configuration and data-governance policies. It does not lower the engineering standard or reduce the accepted target architecture.

Build the complete platform to the highest technically justified standard. Stage work according to dependencies and evidence, not arbitrary time limits. Do not defer foundational correctness, security, isolation, scalability, reliability, observability, testing or evaluation because the first deployment is internal.

The initial Emilo Labs deployment must use the same Engine transaction, immutable snapshot, semantic analysis, evidence-gated verification, sandbox and atomic commit path intended for future public operation. No internal-only fast path or trusted-network exception may bypass these controls.

Keep organization, principal, workspace and repository scopes explicit from the beginning. Apply Emilo Labs' initial data policies through enforceable policy and deployment configuration. Public-facing onboarding, billing, customer administration and support surfaces may be staged, but their foundational identity, authorization, isolation, governance and integration boundaries must not be omitted.

Follow [ADR-0004](docs/adr/0004-internal-first-public-capable-architecture.md) and [deployment-and-product-boundaries.md](docs/spec/deployment-and-product-boundaries.md). A documented target is not an implemented or validated capability; report those states separately.

## 1. Build Principles

1. **Start with the state engine.** Celetixo's differentiator is correct coordination of repository state transitions, not its UI, agent hierarchy, or infrastructure count.
2. **Build an end-to-end vertical slice early.** A small real transaction that can be proposed, validated, committed or rejected is more valuable than many disconnected subsystems.
3. **Keep the core deterministic.** Transaction validity, revision comparison, conflict detection, and state transitions must not depend on an LLM's opinion.
4. **Use models as replaceable workers.** Provider-specific code belongs behind explicit interfaces; the Rust engine must remain model-agnostic.
5. **Make correctness observable.** Important decisions must return structured outcomes and preserve enough evidence to explain them.
6. **Prefer measured simplicity.** Avoid accidental complexity, but do not omit a service, database, queue, sandbox or abstraction required by the accepted target architecture. Validate ownership and failure boundaries before integration.
7. **Never fake completion.** No placeholder implementation presented as working, no success response without an operation, and no skipped verification hidden behind a green status.
8. **Keep scope narrow and cadence high.** Every change should compile, have focused tests, and leave the repository in a known state.

## 2. Goal 01 Baseline (Completed Historical Milestone)

The original first milestone was a runnable Rust Engineering State Engine reference implementation with deterministic concurrency tests. That milestone has since been implemented and its bounded behavior is documented in [Goal 01](docs/implementation/goal-01.md) and [Goal 01 validation](docs/implementation/validation.md).

The original acceptance scenario was to prove that Celetixo could:

1. Register a repository state identified by an immutable revision.
2. Accept a transaction intent with explicit reads, writes, and invariants.
3. Validate the intent and derive a minimal, explicit impact representation.
4. Compare transactions for compatibility.
5. Execute two transactions against a known base revision.
6. Reject a stale or incompatible commit using compare-and-swap semantics.
7. Accept a compatible transaction only when required validation evidence is tied to the state being committed.
8. Return a structured decision and a traceable reason for commit, abort, or revalidation.

Start with an in-memory implementation where it makes the semantics easier to test. Keep interfaces clean enough to replace the in-memory repository and transaction store with durable implementations later. Do not claim persistence, distributed coordination, semantic indexing, or sandbox isolation until those capabilities actually exist.

### Minimum proof scenario

Create a deterministic test where two transactions start from revision R:

- Transaction A writes a state element that Transaction B also depends on.
- A validates and commits, advancing the repository revision.
- B attempts to commit using its old assumptions.
- The engine rejects B's stale commit or requires explicit revalidation; it must never silently overwrite A.
- A second test proves that compatible transactions are not rejected merely because they started from the same revision.

The tests must assert final state and transaction outcomes, not only that a function returned without error.

## 3. Implementation Sequence

Deliver each phase as runnable code with tests. Reorder only when a concrete dependency requires it.

### Phase 0 — Establish the executable baseline

- Inspect the actual repository tree and toolchain before choosing paths or adding files.
- Record the current branch, working-tree state, installed compiler/tool versions, and existing test commands.
- Establish the smallest reproducible build and test command.
- Add a concise contributor quick-start once commands are verified.
- Do not replace existing structure or configuration without a demonstrated need.

**Exit gate:** a clean checkout can run the documented baseline checks.

### Phase 1 — Rust domain core

Implement typed domain models for repository revisions, transaction IDs, intents, read/write sets, impact sets, invariants, verification evidence, and transaction states.

Requirements:

- Explicit state machine; reject illegal transitions.
- Typed errors with actionable context.
- Deterministic behavior for equivalent inputs.
- No model calls, network dependencies, or database requirements in pure transaction logic.
- Unit tests for valid and invalid transitions.

**Exit gate:** core transaction semantics are testable without external services.

### Phase 2 — OCC and conflict semantics

Implement base-revision validation, read/write compatibility, stale transaction detection, compare-and-swap commit boundaries, and explicit revalidation outcomes.

Test at least:

- read/read compatibility;
- write/write conflict;
- read/write conflict;
- stale base revision;
- two concurrent commit attempts;
- incompatible repository advancement;
- idempotent retry behavior;
- illegal commit before verification;
- reservation expiry and recovery when reservations are introduced.

Do not use path-level locks as a substitute for semantic conflict analysis. Reservations are coordination aids, not correctness proofs.

**Exit gate:** deterministic concurrency tests establish that invalid state transitions cannot be committed.

### Phase 3 — Patch and verification evidence

Add versioned patch representation and a verification record tied to the exact transaction and repository revision.

- Reject malformed or wrong-base patches.
- Distinguish verification status from transaction status.
- Record what ran, why it ran, the environment/tool version, exit status, and relevant artifact references.
- Reject stale verification results.
- Preserve failed patches and diagnostics when configured to do so.
- Never treat model confidence or successful patch application as proof of correctness.

**Exit gate:** commit decisions require the correct evidence for the correct state.

### Phase 4 — Repository intelligence

Introduce Tree-sitter parsing, stable file/symbol identity, reference edges, and incremental updates in measured increments. Follow [engineering identity](docs/spec/engineering-identity.md); paths and names are attributes, and heuristic rename/move continuity must not silently authorize OCC decisions.

- Begin with one representative language harness, then expand to TypeScript/JavaScript, Python, Go, and Rust.
- Treat configuration, manifests, lockfiles, generated artifacts, tests, and CI files as engineering dependencies.
- Keep semantic indexing separate from source mutation.
- Use explicit versioned patches; do not adopt universal AST rendering as the canonical writer.
- Label unsupported or incomplete analysis honestly. Unknown impact must not be reported as proven safe.

**Exit gate:** a repository change can produce a reproducible structural impact result and tests cover index invalidation.

The implementation commands, capture policies, source-analysis authority contract, supported structural/LSP relationships and limitations are documented in [Goal 02](docs/implementation/goal-02.md). Reproduce the [recorded benchmarks](docs/benchmarks/goal-02.md) before changing performance targets. Tests must preserve known dynamic-reference misses and their incomplete-coverage result; passing a parser or a single language-server query cannot clear that result.

### Phase 5 — Goal 03: controlled patch application and verification (Completed)

[Goal 03](docs/implementation/goal-03.md) has an implementation and acceptance record. The validated scope includes exact-base source patches, scoped execution admission, policy-owned checks, a concrete Linux T1 path, persisted local evidence admission and owned-runtime recovery. See the [acceptance audit](docs/implementation/goal-03-acceptance.md), [milestone history](docs/implementation/goal-03-progress.md), [execution component](docs/implementation/goal-03-execution.md) and [measured baseline](docs/benchmarks/goal-03.md).

The completion claim is bounded: source revision publication, durable distributed authority, T2/T3, broad language execution harnesses and production operations remain unsupported or target-only.

Required sequence:
1. Canonical, exact-base patch contracts and deterministic application to a private snapshot-derived workspace.
2. Engine admission bound to transaction, scope, snapshot, impact generation, policy, reservation/lease and idempotency.
3. Deterministic verification plans and policy-owned check identities.
4. A backend-neutral executor with validated isolation; untrusted repository code must never fall back to host execution.
5. Immutable, provenance-complete evidence sealing and independent Engine admission.
6. End-to-end, adversarial and fault-injection validation plus reproducible measurements.

Keep source revision publication and production distributed commit explicitly outside this milestone. Passing checks does not itself authorize a source commit.

**Exit gate:** an exact-base patch is applied without mutating its source snapshot, required checks execute in a validated isolation backend against the resulting snapshot, and the Engine admits only complete, correctly bound evidence. All Goal 03 acceptance criteria must pass.

The [Goal 03 acceptance record](docs/implementation/goal-03-acceptance.md) maps that gate to implemented contracts, actual integration checks, immutable source/evidence versions, known limits and the [measured baseline](docs/benchmarks/goal-03.md). Its reference Engine and reviewed Linux T1 profile do not provide source publication or durable distributed authority.

### Phase 6 — Goal 04: durable orchestration and distributed commit coordination

[Goal 04](docs/implementation/goal-04.md) is the accepted implementation contract for durable transaction state, multi-instance Engine coordination, atomic distributed commit, persisted reservations/fencing, transactional outbox, NATS JetStream, Temporal workflows, immutable object storage, ClickHouse analytics, shared service lifecycle and operational recovery.

Preserve the authority boundaries:
1. Rust Engine alone evaluates semantic admissibility and makes transaction decisions.
2. PostgreSQL owns durable authoritative transaction/repository state and the atomic commit linearization point.
3. Temporal owns durable workflow progress, not transaction truth.
4. NATS distributes events; outbox/inbox processing is idempotent and replayable.
5. Object storage holds immutable evidence/artifacts; PostgreSQL controls metadata and admission.
6. ClickHouse is derived analytics and never authoritative.

Implement Goal 04 in its seven documented milestones. Use real PostgreSQL, NATS, Temporal and object-store integration tests for claims about durability and distributed behavior. Independent Engine processes must race against the same durable state. No process-local mutex, workflow completion, event arrival or worker response may substitute for atomic commit.

**Exit gate:** the complete Goal 04 definition of done passes, including multi-instance concurrency, crash ambiguity, idempotent recovery, durable evidence retrieval, isolation, backup/restore and operational proof.

### Phase 7 — Real worker integration and evaluation

Connect real coding workers through provider-neutral adapters. Run tasks against immutable repository snapshots and pass every patch through the same transaction and verification path.

Build the three-arm comparison defined in [EVALUATION.md](EVALUATION.md): (A) a strong sequential coding agent, (B) parallel Git worktrees with ordinary integration and no Celetixo semantic transactions, and (C) Celetixo semantic transactions. Freeze the task suite and decision rule before confirmatory runs. Measure verified outcomes, impact precision/recall, false-negative impact, false-positive conflicts, abort reasons, wasted execution/tokens, human correction, verification cost and recovery correctness.

**Exit gate:** the reference transaction oracle and failure-injection suite pass; comparative results can be reproduced from preserved evidence; performance claims are supported by the frozen task suite and controlled measurements.

---

## 4. Contract and validation specifications

The implementation must conform to the detailed contracts in:

- [Transaction semantics](docs/spec/transaction-semantics.md)
- [Engineering identity](docs/spec/engineering-identity.md)
- [Failure and recovery model](docs/spec/failure-model.md)
- [Trajectory governance](docs/spec/trajectory-governance.md)
- [Evaluation protocol](EVALUATION.md)

These specifications operationalize accepted ADR decisions. If implementation reveals a new architectural choice or contradiction, record it through an ADR rather than allowing a specification or code change to silently redefine authority.

## 5. Working Agreement

### Before implementation

- Read the relevant ADR and the existing code around the change.
- Identify the behavior, invariant, and acceptance test before editing.
- Inspect the current branch and working tree; do not overwrite unrelated user changes.
- Confirm the actual module/API shape rather than assuming a file or function exists.
- Split work into bounded tasks with explicit dependencies and file ownership.

### During implementation

- Make the smallest complete change that advances a defined milestone.
- Prefer domain types, explicit state transitions, pure functions, and narrow interfaces.
- Keep error handling explicit; never swallow errors to keep a workflow moving.
- Keep external side effects at clear boundaries.
- Use deterministic fixtures and controlled clocks for time-sensitive state-machine tests.
- Keep dependencies justified and pinned according to the repository's package manager.
- Update protocol definitions and generated bindings from their canonical source.
- Do not mix unrelated refactors with a correctness-critical change.

### Before marking a task complete

1. Run formatting and linting supported by the changed component.
2. Run focused tests, then relevant package or workspace tests.
3. Run type checking/build checks where applicable.
4. Inspect the complete diff, including generated files and lockfiles.
5. Confirm no test, assertion, authorization rule, or verification step was weakened to force a pass.
6. Document checks that could not run and the exact reason.
7. Record remaining limitations as limitations, not as implemented capabilities.

A task is complete only when its acceptance criteria and verification evidence are explicit.

## 6. Parallel Development

Parallelism should accelerate implementation without creating unreviewable integration work.

- Give each worker a bounded deliverable, clear acceptance criteria, and explicit ownership.
- Avoid concurrent edits to the same core contract or shared files unless a coordinator has defined the integration sequence.
- Assign protocol/schema ownership to one task at a time.
- Integrate small changes frequently; do not allow long-lived speculative branches to drift.
- A worker's claim that its code passes tests is not sufficient; the integrating path must verify the resulting repository state.
- Keep model, worker, transaction, and repository revision identities available in execution records.
- Never grant workers commit authority over the authoritative engineering state simply because they produced a patch.

The Chief owns global objectives, architecture, task decomposition, invariants, and escalation. Supervisors own bounded-domain execution and recovery. Workers implement and verify bounded tasks. The transaction engine remains authoritative for repository-state decisions.

## 7. Security and Isolation

- Treat generated code, patches, dependencies, and tool output as untrusted.
- Do not expose production credentials to workers or test processes.
- Default sandbox network access to denied; grant only explicit, task-scoped access.
- Apply time, CPU, memory, disk, process, and output limits to untrusted execution.
- Keep repository operations inside the intended worktree/snapshot.
- Record provenance for model calls, tool calls, patches, execution environments, and verification.
- Make retries idempotent or guard side effects with durable idempotency keys.
- Do not execute untrusted code on the host merely to simplify the first demo.

## 8. Performance Discipline

Optimize for **cost per successful, verified engineering outcome**, not raw token speed, worker count, or benchmark theater.

Measure separately:

- model inference;
- queue and event delivery;
- orchestration;
- impact analysis;
- sandbox startup;
- build and test execution;
- verification;
- commit/revalidation.

Start with a correct baseline. Profile real workloads before introducing caches or concurrency optimizations. Cache keys must account for all inputs that can change correctness, including relevant source state, dependency locks, toolchain, configuration, platform, and environment. A cache hit must never bypass a required verification step.

Do not establish latency targets until representative measurements exist.

## 9. Architecture Scope and Sequencing

Do not start v0/v1 by building:

- proprietary foundation-model training or RL pipelines;
- a polished IDE or consumer chat experience;
- browser automation or autonomous production deployment;
- enterprise multi-repository orchestration;
- a complex dashboard before there is real execution state to display;
- every infrastructure service as a prerequisite for local transaction tests.

Build the infrastructure required by the accepted architecture, with explicit dependency ordering and integration gates. Local deterministic tests do not substitute for the durable, distributed, isolated and observable production design.

## 10. Definition of Done

A change is ready to merge when:

- [ ] Its purpose and acceptance criteria are clear.
- [ ] It preserves the accepted architecture or includes an explicit ADR proposal.
- [ ] State transitions and failure cases are covered by tests where relevant.
- [ ] Verification is tied to the correct repository/transaction revision.
- [ ] Formatting, lint, type checks, and relevant tests pass, or blockers are documented precisely.
- [ ] No secrets, credentials, or unintended artifacts are committed.
- [ ] The diff is scoped and reviewed.
- [ ] Documentation matches actual behavior.
- [ ] Remaining gaps are stated plainly.

## 11. Engineering Standard

Every subsystem must have a clear responsibility, contract, threat model, failure semantics and validation plan. Build in coherent stages, but do not impose arbitrary time or capability ceilings. Preserve the complete accepted target architecture.

> **Models propose changes. The state engine decides whether they are admissible. Verification establishes evidence. The commit boundary preserves repository correctness.**
