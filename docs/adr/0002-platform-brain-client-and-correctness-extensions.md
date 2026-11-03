# ADR-0002: Celetixo Platform, Brain, Client and Correctness Extensions

- **Status:** Accepted architectural extension
- **Date:** 2026-11-03
- **Depends on:** ADR-0001
- **Decision scope:** Additive platform and subsystem architecture; ADR-0001 remains the transaction and coordination baseline.

## Context

ADR-0001 defines Celetixo's engineering-state coordination model and the authoritative boundaries for semantic transactions, OCC, verification, orchestration, events, state, analytics and evidence. It remains valid. This ADR extends the system around that core. It does not replace the Engineering State Engine, weaken transaction admission, collapse service boundaries, or redefine correctness as model confidence or successful patch application.

Celetixo is intended to become an independent, scalable engineering platform. Architecture selection is therefore based on long-term correctness, operability, extensibility, security, performance and ownership. Implementation duration is not an architectural constraint. Build sequencing may manage risk, but must not be used to justify a lower-quality target architecture or permanent capability ceiling.

External proposals are inputs for evaluation, not decisions. No proposed technology becomes mandatory merely because it appears in a proposal. Every addition must have a defined responsibility, interface, failure model, security model and measurable acceptance criteria.

## Decision

Adopt the following extensions while preserving every invariant in ADR-0001.

### 1. Preserve the authoritative engineering core

The Rust Engineering State Engine remains authoritative for:
- repository and snapshot identity;
- semantic state and impact analysis;
- transaction lifecycle and admissibility;
- read/write conflict analysis;
- reservations and leases;
- OCC validation and commit serialization;
- rebase/revalidation requirements;
- verification evidence admissibility;
- final commit decisions.

No Brain service, model, agent, editor, mobile client, orchestrator or external API may bypass this authority. The Engine must remain usable without a model provider and must make deterministic decisions for equivalent state and inputs.

### 2. Introduce a first-class Brain subsystem

Python is the primary Brain runtime for model-facing and learning-facing workloads. The Brain is a policy, reasoning and worker-management subsystem, not a second source of engineering truth.

Responsibilities:
- provider-neutral model gateway and adapter lifecycle;
- model selection, fallback and routing policy;
- context assembly and token-budget accounting;
- planning, task decomposition and agent-role policy;
- Chief and Supervisor reasoning;
- worker allocation and capability matching;
- tool policy and structured tool invocation;
- evaluation, trajectory processing and dataset curation;
- model serving integration, including vLLM where appropriate;
- procedural skill extraction and later training pipelines.

The Brain submits typed intents and consumes Engine decisions. It cannot create reservations, mark verification as passed, advance repository revisions or commit directly.

Provider adapters must expose normalized capabilities, structured output, tool-call semantics, cancellation, timeout, retry classification, rate-limit behavior, usage/cost accounting and provenance. Provider-specific features may be used behind capability negotiation; the internal system must not depend on one provider's proprietary message format.

Model routing must consider task class, quality requirements, context capacity, tool support, latency, cost, data handling policy, availability and evaluation history. Fallback must preserve task identity, provenance and policy. A fallback model cannot silently inherit authority or verification status from a prior model.

The Brain must support model-independent operation for deterministic tasks and graceful degradation when model providers are unavailable. Model calls are untrusted proposals.

### 3. Python and Rust boundary

Python owns orchestration-adjacent intelligence and model-facing workflows. Rust owns deterministic, performance-sensitive engineering-state operations.

Use Rust for CPU-bound or safety-critical native components where profiling or correctness requirements justify it, including Engine internals, parsing/indexing hot paths, graph operations, patch processing and high-throughput data transforms. Python may call native Rust libraries through a narrow, versioned interface where this improves throughput or memory behavior.

Do not duplicate Engine algorithms in Python. Do not move transaction authority into Python for implementation convenience. Do not introduce C++ as a mandatory general-purpose layer. C++ may be selected for a concrete requirement such as a native inference runtime, accelerator/kernel integration or a measured library interoperability need, with an ADR or documented subsystem decision defining ownership and ABI boundaries.

Language choice is responsibility-driven, not ideological. Avoid cross-language calls in hot paths unless measured benefits justify their operational and debugging cost.

### 4. Editor and desktop strategy

The desktop editor is a client of Celetixo services, not the owner of engineering state or transaction correctness.

Eclipse Theia is the preferred candidate for the extensible desktop workbench because it provides an extensible IDE platform and a path to a controlled product experience. This is a technology direction, not permission to assume compatibility. Before product adoption, validate:
- required VS Code extension compatibility and API gaps;
- extension distribution and marketplace policy;
- licensing and redistribution obligations;
- language tooling and LSP integration;
- workspace/snapshot isolation;
- remote execution and authentication;
- packaging, signing, updates and rollback;
- accessibility, performance and resource consumption.

The workbench should expose repository state, task/transaction status, impact explanations, worker activity, evidence, diffs, approvals and rebase/conflict workflows. It must not implement an alternate commit gate. Any editor buffer or unsaved document that participates in a task must be represented in an immutable, addressable workspace snapshot or overlay. A Git commit SHA alone is not a complete snapshot identity.

The editor platform remains replaceable behind stable client APIs. Do not couple Engine contracts to Theia-specific APIs.

### 5. Native mobile client

Android is a native application built with Kotlin and Jetpack Compose. Flutter is not part of the selected architecture.

The mobile client is a secure companion interface for:
- authenticated task and repository monitoring;
- notifications and escalation;
- impact and transaction inspection;
- diff and evidence review;
- explicit human approvals where policy requires them;
- worker/job controls within authorized permissions;
- secure account/session and device management.

It is not a second IDE, local Engine, or independent transaction authority. Mobile actions call authenticated APIs and are subject to server-side authorization and the same Engine admission rules. Sensitive approvals require clear transaction identity, repository/snapshot revision, diff/evidence summary, actor identity and an auditable decision. Offline queued actions must be re-authorized and revalidated when delivered.

iOS may be added as a native client when required, using a platform-appropriate implementation and the same API contracts. Cross-platform UI must not compromise platform security, accessibility or native integration.

### 6. Snapshot and workspace identity

A repository commit SHA is necessary but not sufficient to identify all working states. Introduce a canonical snapshot identity capable of representing:
- repository identity and base commit;
- tracked and untracked file content included in the task;
- staged and unstaged changes;
- editor buffers and overlays;
- generated or mounted inputs included in the workspace;
- submodule and dependency/workspace references as applicable;
- snapshot construction policy and content manifest;
- immutable content digests.

The exact storage encoding may evolve, but identity must be content-addressed and immutable. A transaction, impact analysis, patch, LSP result, verification result and commit attempt must refer to compatible snapshot identities. A dirty workspace must never be silently represented as its HEAD commit.

### 7. Versioned semantic analysis and impact provenance

Every impact result must identify the repository snapshot, semantic index generation/version, parser and analyzer versions, language coverage, configuration inputs, completeness status and provenance. An index built from an older or partial state cannot authorize a claim that impact is complete.

Unknown, unsupported, ambiguous or unresolved impact is represented explicitly. Policy decides whether it blocks, escalates or permits a bounded operation; it is never silently treated as no impact.

Semantic indexing, symbol resolution, dependency analysis, lexical search and embedding retrieval are distinct services/capabilities. SCIP is an index format/ecosystem, not a substitute for a query engine or vector database. Retrieval may help the Brain assemble context but does not establish transaction correctness.

### 8. Verification evidence contract

A verification result must bind to:
- transaction ID and immutable snapshot identity;
- patch/content digest;
- exact check identity, version and configuration;
- required checks and completed checks;
- toolchain, dependency and execution-environment identity;
- exit status and structured diagnostics;
- artifact/evidence references;
- start/end times and executor provenance;
- completeness and limitations.

A generic green boolean is insufficient. Evidence from a different snapshot, patch, toolchain, or check configuration is stale or inapplicable unless an explicit, deterministic equivalence rule proves reuse is safe. Cache reuse must be content-addressed and include every correctness-relevant input. A cache hit never waives a required check.

Verification establishes bounded evidence, not mathematical proof of arbitrary program correctness. The Engine's commit policy decides whether the available evidence satisfies the declared risk and invariant contract.

### 9. Protocol and transport discipline

Protocol Buffers v3 remains the canonical internal typed contract. Preserve and evolve the ADR-0001 `celetixo.orchestration.v1` TaskIntent semantics. Do not replace it with a reduced schema that drops task dependencies, explicit reads/writes, invariants, impact policy, verification requirements, resource budgets, retry/rollback/escalation policy or provenance.

Schema evolution must follow Protobuf compatibility rules, reserve removed field numbers and names, avoid semantic reuse of fields, and include compatibility tests. Wire compatibility does not establish transaction correctness.

Use gRPC for typed service RPC and streaming where suitable; NATS JetStream for asynchronous event distribution; HTTPS/JSON for external APIs; and an authenticated bidirectional channel such as WebSocket where interactive client updates require it. Transport choice does not alter ownership or authority. All API actions require authentication, authorization, tenant/repository scoping, validation, rate limiting and audit records as applicable.

An operator or user identifier is attribution, not authorization. Authorization is established by authenticated identity, scoped policy and server-side permission checks. Approval and force-rebase requests are commands to initiate governed workflows, never bypass flags.

### 10. Commit and authorization hardening

The commit boundary must be atomic with respect to authoritative revision advancement. Use database transactions, compare-and-swap, serializable coordination or an equivalent proven mechanism appropriate to the storage design. A process-local mutex is not sufficient for distributed correctness.

The commit gate rechecks snapshot identity, current revision, transaction state, reservation/lease validity where required, conflict assumptions, patch digest, evidence applicability, authorization and idempotency key. Duplicate, delayed or reordered requests must not produce duplicate commits or bypass checks.

A force-rebase operation means "request a fresh semantic analysis and revalidation." It never means unconditional overwrite. An approval records who approved what exact transaction, diff, snapshot and evidence at what time and under which policy.

### 11. Execution isolation and tiers

Preserve ADR-0001's four execution tiers:
- Tier 0: deterministic local operations that do not execute untrusted repository code;
- Tier 1: isolated process/container/gVisor for bounded checks;
- Tier 2: Firecracker microVM for stronger isolation and untrusted execution;
- Tier 3: remote execution pools for scale, specialized hardware and large matrices.

Tier selection is risk- and capability-driven. No tier implies blanket trust. Network egress, secrets, filesystem mounts, CPU, memory, disk, process count, time and output must be controlled. Remote pools require workload identity, isolation between tenants, artifact integrity, provenance and secure teardown. Sandbox isolation must be threat-modeled and tested; a container alone is not equivalent to a VM boundary.

### 12. Data, evidence and learning infrastructure

Preserve storage ownership:
- PostgreSQL: authoritative current operational and transaction state;
- ClickHouse: analytical events, metrics and historical evaluation;
- S3-compatible object storage: immutable large evidence, snapshots, patches and artifacts;
- Parquet: portable analytical/evidence interchange where useful.

A trajectory is a structured engineering record, not an unfiltered conversation dump. It should link intent, snapshot, index/impact provenance, model and prompt/config provenance, tool calls, worker actions, patch, execution environment, checks, evidence, retries, rebase/rollback and final human outcome.

Training eligibility is a separate governed decision. Filter for data rights, licenses, privacy, secrets, contamination, quality, correctness and representativeness. Preserve lineage and dataset versioning. Never assume all collected trajectories are safe or useful for training. Training, fine-tuning, RL and distillation remain future capabilities; the architecture must permit them without making them prerequisites for core correctness.

### 13. Observability, reliability and operations

Define end-to-end correlation across repository, snapshot, task, transaction, workflow, worker, model call, execution and evidence IDs. Use structured logs, metrics and distributed traces with explicit retention, redaction and access policy.

Specify service-level objectives only after representative workloads and failure modes are measured. Design for backpressure, bounded queues, cancellation, deadlines, retries with jitter, circuit breakers, idempotency, graceful shutdown, durable recovery and schema evolution. Retries must not duplicate side effects. Event delivery semantics must be explicit; consumers must tolerate duplicate and out-of-order delivery.

### 14. Performance and capacity engineering

Do not optimize for raw token speed or worker count. Optimize for successful verified engineering outcomes while also measuring tail latency, throughput, resource efficiency, reliability and human correction.

Benchmark representative repository sizes, languages, dependency graphs, task classes, concurrency levels and failure conditions. Measure indexing latency, incremental update cost, impact precision/recall, conflict false positives/negatives, reservation contention, commit serialization, cache correctness, sandbox startup, verification duration, model cost, queue delay and recovery time.

Use profiling and capacity models to guide parallelism, caching, sharding, memory layout, native acceleration and remote execution. Do not assert performance numbers without reproducible methodology and published workload conditions.

## Consequences

### Positive
- Adds a defined Brain without weakening Engine authority.
- Enables model/provider independence and future learning systems.
- Makes dirty workspaces and editor overlays first-class, versioned state.
- Strengthens evidence applicability, commit atomicity and authorization.
- Provides native Android access without introducing Flutter.
- Creates a desktop workbench direction while retaining client replaceability.
- Preserves independent scaling for operational state, analytics and immutable artifacts.

### Costs and obligations
- More contracts, services, security boundaries and operational responsibilities.
- Snapshot manifests and semantic index provenance add storage and lifecycle complexity.
- Provider abstraction and evaluation require continuous compatibility work.
- Desktop adoption requires extension, licensing and packaging validation.
- Native mobile requires a secure API and device/session lifecycle.
- Learning infrastructure requires data governance, not merely storage volume.

These costs are accepted as architectural investment. They are not grounds for removing correctness guarantees or imposing an arbitrary implementation-time ceiling.

## Explicitly rejected shortcuts

- Replacing the Engine with a model-driven coordinator.
- Making Python the authority for OCC or commit decisions.
- Treating Git HEAD as the complete identity of a dirty workspace.
- Treating a successful patch application, green boolean or model confidence as sufficient verification.
- Letting editor or mobile clients bypass server authorization or commit policy.
- Reducing the canonical TaskIntent contract to a smaller message without preserving semantics.
- Treating SCIP as a vector database or semantic query engine.
- Making C++ mandatory without a measured or concrete integration need.
- Treating containers as equivalent to microVM isolation.
- Sending every collected trajectory directly into training.
- Using force-commit, force-rebase or operator identifiers to bypass validation and authorization.

## Relationship to ADR-0001

ADR-0001 remains accepted and authoritative for the original engineering-state coordination architecture. ADR-0002 adds platform and subsystem requirements. Where ADR-0002 is more specific on Brain, snapshot identity, evidence contracts, clients, protocol evolution, security and operations, those extensions apply. Where it does not explicitly change a decision, ADR-0001 remains in force.

Any future change to Engine authority, OCC semantics, revision advancement, verification admissibility, protocol meaning or storage ownership requires a new ADR or explicit amendment with migration and compatibility analysis.

> **The Brain reasons. Workers execute. The Engineering State Engine governs admissibility. Verification supplies evidence. The commit boundary protects state.**
