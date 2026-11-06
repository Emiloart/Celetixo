# Goal 03 — Controlled patch application and verification execution

- **Status:** Accepted implementation objective
- **Depends on:** ADR-0001 through ADR-0004; Goals 01 and 02
- **Authority:** Rust Engineering State Engine
- **Primary implementation boundary:** versioned source mutation and isolated verification
- **Explicit non-goal:** production distributed commit publication or agent/provider integration

## 1. Objective

Goal 03 closes the gap between repository intelligence and executable engineering changes.

Goal 01 established deterministic transaction semantics and evidence-gated reference commits. Goal 02 established immutable repository capture, versioned structural indexes, snapshot-bound impact analysis and an Engine-owned source-analysis contract. Goal 02 deliberately does not mutate source. Goal 03 introduces a controlled path that takes an authorized transaction and an explicit patch, applies it to a workspace derived from the exact captured snapshot, executes policy-selected verification against the resulting state, and returns immutable evidence to the Engine.

The output is not "a patch that ran." It is a provenance-complete execution attempt whose inputs, transformations, environment, checks, results and resulting snapshot can be independently validated.

The Engine remains the sole authority for transaction state, authorization, reservations, conflict decisions, evidence admissibility, revalidation and commit decisions. Patch applicators and executors are subordinate components. A worker, model, sandbox, CLI, language server or orchestration process cannot mark a transaction verified or committed.

**Core invariant:** no patch is applied to an ambiguous base, no untrusted repository code is executed outside an approved isolation boundary, no verification result is detached from the exact state it tested, and no execution result directly advances authoritative repository revision.

## 2. Scope and deliverables

Goal 03 delivers these integrated capabilities:

1. A typed, versioned patch contract bound to a transaction, scope and exact base snapshot.
2. Deterministic patch validation and application to a private snapshot-derived workspace.
3. A resulting immutable snapshot and patch-application report.
4. Pre-execution admission that checks authorization, transaction state, snapshot, impact coverage, policy and resource requirements.
5. A policy-driven verification planner that resolves justified checks to repository-configured commands and exact toolchain identities.
6. A tiered execution interface with a local deterministic tier and isolated execution backends; untrusted commands must not run directly on the host.
7. Structured execution records, logs, diagnostics, exit outcomes and immutable artifact references.
8. Verification evidence cryptographically/content-addressably bound to the exact transaction, base snapshot, applied patch, resulting snapshot, checks, environment and execution attempt.
9. Engine-side evidence validation and explicit rejection of stale, incomplete, mismatched, unauthorized or insufficient results.
10. Failure injection, security tests, reproducible fixtures and measured performance baselines.

The implementation may be split into commits and internal sub-milestones, but the final acceptance path must be end-to-end. A disconnected patch parser, fake executor, or evidence object that is never checked by the Engine does not complete Goal 03.

## 3. Authority and trust boundaries

### 3.1 Rust Engine

The Engine is authoritative for:
- transaction identity, scope and legal lifecycle transitions;
- authorization and capability checks;
- base snapshot and accepted repository revision;
- installed repository-analysis generation and impact completeness;
- required invariants and verification policy;
- reservations, leases and cancellation state;
- whether a patch attempt may start or continue;
- whether returned evidence is admissible;
- whether a transaction must abort, revalidate or may proceed to a later commit protocol.

The Engine issues or validates a bounded execution admission containing the exact transaction, snapshot, patch digest, policy generation, permitted checks, execution tier, resource ceiling, credential/network grants, expiry and idempotency identity. The executor cannot widen this admission.

The Engine does not trust executor-supplied booleans such as `authorized`, `impact_complete`, `safe`, `verified` or `commit_allowed`. It validates typed fields, provenance, bindings and policy itself.

### 3.2 Patch application component

The patch component:
- validates patch syntax and declared base;
- applies operations to a private materialization;
- detects path and content conflicts;
- emits a deterministic application report;
- produces the exact resulting content manifest and snapshot identity.

It does not choose transaction scope, waive impact gaps, grant access, execute repository code or publish a repository revision.

### 3.3 Verification planner

The planner maps Engine-authorized verification requirements and repository-owned configuration to concrete check definitions. It may resolve tool paths, versions and dependency identities, but cannot invent permission to run arbitrary commands or silently omit required checks.

A model may suggest checks. Only trusted policy and Engine admission determine the required set.

### 3.4 Executor and sandbox

The executor receives a narrow, expiring capability for one admitted attempt. It treats repository files, patches, package scripts, build systems, test code, dependencies, compiler plugins, language servers and tool output as untrusted.

The executor cannot:
- access the host outside its declared mounts;
- read ambient developer credentials or production secrets;
- access unrelated repositories, workspaces or tenant data;
- change Engine state or repository revision;
- modify the authoritative evidence record;
- widen network, filesystem, process or resource permissions;
- reuse another transaction's writable workspace.

### 3.5 Evidence store

Evidence artifacts are immutable and content-addressed. A storage adapter may initially be local for deterministic development, but its interface must preserve the object-storage target and integrity semantics. A successful upload is not assumed until the storage adapter confirms it. Required evidence persistence failure makes the attempt incomplete and blocks evidence admission.

## 4. Patch contract

### 4.1 Canonical patch envelope

Define a versioned, language-neutral patch envelope. At minimum it binds:

- schema and patch-format version;
- organization, principal, workspace and repository scope;
- transaction and attempt IDs;
- exact base snapshot ID and authoritative base revision;
- source-analysis artifact and index generation used for impact analysis;
- canonical ordered operation list;
- declared affected paths and operation kinds;
- expected preimage digests for modified/deleted inputs;
- resulting content digests for added/modified inputs;
- patch digest over canonical serialization;
- producer identity and provenance (worker/model/tool, if applicable);
- creation time and expiry where applicable.

The patch digest is computed by the trusted implementation over canonical bytes. A producer-supplied digest is checked, never trusted.

### 4.2 Operations

The initial operation set must support:
- create file;
- replace file;
- delete file;
- rename/move with explicit source and destination;
- mode changes where supported;
- symlink operations only under an explicit policy.

Binary content is represented as bounded, digest-addressed payloads, not lossy text hunks. Directory creation is derived from file operations unless a separately justified operation is needed.

Patch ordering is canonical and deterministic. Duplicate or contradictory operations, path traversal, absolute paths, platform-ambiguous paths, reserved control paths, case-fold collisions where relevant, invalid encodings under text operations, and symlink escapes are rejected.

### 4.3 Base and preimage checks

Every operation is evaluated against the exact base snapshot. Replace/delete/rename operations require expected preimage identity. A patch against a different snapshot is rejected, even if its textual hunks appear to apply.

No fuzzy application, three-way merge, conflict-marker insertion, or implicit offset adjustment is allowed in the initial deterministic application contract. If a patch cannot be applied exactly, it returns a structured conflict. Rebase is a separate Engine-governed process requiring fresh capture, indexing, impact analysis, patch generation/adaptation and verification.

### 4.4 Patch semantics and source preservation

The canonical mutation representation is an explicit versioned patch, not universal AST serialization. Preserve comments, formatting, source trivia, macros, generated constructs and language-specific syntax. AST-aware edits may be used by a producer, but the final patch remains explicit and reproducible.

The patch applier must not execute Git hooks, filters, clean/smudge filters, package scripts, build scripts or arbitrary repository programs. Git object/tree operations, if used, must be configured to avoid repository-controlled execution and network fetching.

## 5. Snapshot-derived workspace materialization

The executor must run against a private workspace materialized from the exact Goal 02 snapshot plus the accepted patch. It must not run against the developer's live working directory.

The materializer must:
- validate the source capture and its uncertainty;
- reproduce the declared input manifest, file bytes, modes and supported symlink semantics;
- preserve staged/unstaged/editor-overlay distinctions as explicit provenance even if the execution view is a normalized merged tree;
- include declared generated, mounted, dependency and configuration inputs;
- record any excluded or unavailable input and retain the associated uncertainty;
- apply the patch without mutating the source snapshot;
- compute and validate the resulting snapshot manifest and identity;
- prevent path escape and cross-workspace writes;
- clean up ephemeral materializations safely without deleting source or unrelated data.

A live Goal 02 capture carrying `LiveSourceNotAtomic` cannot be upgraded to an atomic snapshot by materialization. Policy may reject it for execution or require recapture from an immutable source. The executor must not erase the uncertainty.

An immutable Git-tree capture can be used only for the exact declared tree and overlays. Missing submodules, generated inputs or other required dependencies remain explicit. A materializer is not a complete hermetic execution environment merely because it copied source files.

## 6. Admission protocol

Before applying a patch or launching a command, the Engine/admission service must establish:

1. The caller is authenticated and authorized for the exact scope and operation.
2. The transaction exists and is in a legal state for patch application or verification.
3. The transaction's base snapshot equals the patch base snapshot.
4. The repository-analysis artifact and index generation are the expected ones for that snapshot.
5. The impact result is bound to the exact request, snapshot and generation.
6. Required impact coverage satisfies policy. Incomplete or unknown coverage blocks by default; any exception must be an explicit, narrow, auditable policy decision and cannot be represented as complete analysis.
7. The patch digest, operations, path scope and preimage requirements validate.
8. Reservations/leases are valid and sufficient for the intended operation.
9. Required checks are derived from declared invariants, impacted elements, repository configuration and policy.
10. Execution tier, resource limits, network policy, credential references, retention policy and evidence requirements are resolved.
11. The attempt is assigned an idempotency key and bounded expiry.

A failed admission creates no runnable capability. The executor must revalidate admission immediately before side effects and stop when its lease, cancellation token or policy grant expires.

Goal 02's current incomplete structural impact results do not become execution-safe merely because a patch is small. This is a hard boundary. During early development, tests may use explicitly constructed complete reference artifacts, but production-like admission tests must prove incomplete analysis is rejected.

## 7. Execution and verification model

### 7.1 Separate application from execution

Patch application and command execution are separate stages with separate records:

`PROPOSED → ADMITTED → PATCHING → PATCH_APPLIED → VERIFYING → EVIDENCE_SEALED`

Failures transition to explicit terminal or recovery states such as `REJECTED`, `PATCH_CONFLICT`, `EXECUTION_FAILED`, `TIMED_OUT`, `CANCELLED`, `EVIDENCE_INCOMPLETE` or `REVALIDATION_REQUIRED`.

These attempt states do not replace the authoritative transaction lifecycle. The Engine maps attempt outcomes to transaction decisions.

Patch application success does not imply syntax validity, build validity or verification success. Verification success does not itself commit source.

### 7.2 Verification plan

The verification plan is a deterministic, versioned artifact derived from:
- Engine-authorized invariants and required checks;
- Goal 02 impact output and its coverage gaps;
- repository-owned, policy-approved check configuration;
- language and project harness configuration;
- declared build/test dependencies;
- security policy and permitted execution tier.

A plan records required checks, optional checks, rationale, dependency ordering, cache eligibility, expected artifacts, timeout and resource budgets, and failure semantics.

A worker or model may propose additional checks. It cannot remove mandatory checks or mark a skipped check successful. Missing, ambiguous or unsupported required checks make the plan incomplete.

### 7.3 Check identity

A check identity binds at least:
- stable check ID and configuration digest;
- exact command/argument vector or typed runner operation;
- working directory and declared input/output paths;
- toolchain and executable identity/version;
- dependency and lockfile digests;
- environment allowlist and relevant variable values or protected references;
- network policy and execution tier;
- timeout, resource and output limits;
- expected result/artifact contract.

Shell-string concatenation is not the canonical command interface. Prefer typed executable plus argument vector. If a shell is unavoidable for a repository-defined task, the shell and script bytes are explicit, hashed inputs and execute only inside the sandbox.

### 7.4 Check execution

Each check runs against the same immutable resulting source snapshot, with a separately declared writable scratch/output area. A check must not mutate the source view used by another check. If checks need generated outputs or shared build state, the dependency and artifact handoff are explicit and content-addressed.

Parallel checks are allowed only when their declared input/output sets and resource budgets are compatible. Otherwise execute in deterministic dependency order. A check that modifies source invalidates the source snapshot and requires a new patch/application attempt.

Exit status is necessary but not sufficient. The runner also records timeout, signal, cancellation, infrastructure failure, output truncation, diagnostics, produced artifacts and whether the process tree was fully terminated.

### 7.5 Initial language harnesses

Support must be claimed per validated harness, not inferred from parser support. The initial target matrix is:

| Ecosystem | Candidate checks (repository-configurable) |
|---|---|
| TypeScript / JavaScript | TypeScript compiler, Biome or configured lint/format, Vitest or repository-native tests |
| Python | Pyright/Mypy, Ruff, pytest or repository-native tests |
| Go | gofmt, go vet, go test and compiler diagnostics |
| Rust | rustfmt, cargo check/test/clippy and compiler diagnostics |

Do not install or execute a tool merely because a manifest names it. Dependency acquisition is a separate, policy-controlled operation with provenance, integrity verification, network policy and cache identity. A language harness is validated only when its exact toolchain, commands, isolation and evidence contract have integration tests.

## 8. Tiered isolation and resource policy

Preserve the accepted execution tiers:

| Tier | Permitted work | Security boundary |
|---|---|---|
| T0 | Pure deterministic patch validation, hashing, snapshot/manifest checks, static operations that do not execute repository code | Trusted Engine-controlled process; no untrusted command execution |
| T1 | Bounded checks with a validated isolated process/container/gVisor profile | Restricted filesystem, process, CPU, memory, disk, time and network |
| T2 | Untrusted build scripts, dependency hooks, plugins, integration tests or workloads requiring stronger isolation | Firecracker microVM or equivalently justified isolation with explicit validation |
| T3 | Remote execution pools | Authenticated, attested, policy-equivalent workers with immutable inputs and evidence return |

A container is not automatically equivalent to a microVM. The implementation must document and test the threat boundary of each backend. Do not label a host subprocess as sandboxed.

All execution defaults to:
- no network egress;
- no ambient host environment;
- no host home-directory or credential mounts;
- read-only source inputs plus explicit scratch/output mounts;
- least-privilege user and syscall/device access;
- bounded CPU, memory, disk, process count, file descriptors, wall time and output;
- process-tree termination on timeout/cancellation;
- per-attempt isolation and cleanup;
- explicit, scoped credential references only when policy authorizes them.

Network access is an explicit task capability with destination/port policy, time limit, audit record and secret-handling rules. Package installation is not implicitly permitted by a test command. No production credentials are available to workers or repository code.

If the required isolation backend is unavailable, the attempt fails closed or is routed to an approved stronger backend. It must not silently downgrade to host execution.

## 9. Evidence contract

Verification evidence must be immutable, content-addressed and independently validated by the Engine.

The sealed record binds:
- evidence schema/version and evidence ID;
- organization/principal/workspace/repository scope;
- transaction ID and attempt ID;
- exact base snapshot and authoritative revision;
- patch envelope digest and canonical operation digest;
- exact resulting snapshot ID and manifest digest;
- source-analysis artifact, index generation and impact request/result digest;
- admission/policy generation and authorization provenance;
- verification-plan digest;
- required and executed check identities/configuration digests;
- skipped checks and explicit reasons;
- toolchain, runtime, sandbox image/backend/version and relevant environment digest;
- network/credential grants actually used;
- start/end times, timeout/resource observations and execution outcome;
- stdout/stderr/diagnostics digests and immutable artifact references;
- exit codes/signals and process termination status;
- generated artifact digests and provenance;
- completeness gaps, infrastructure faults and attestation level;
- executor identity and authenticated result provenance.

The evidence digest is computed over canonical serialization. Every referenced artifact must be retrievable and digest-verified before evidence is admitted. Truncated output is marked truncated. Missing logs, uncertain process termination, unavailable artifacts or unexecuted mandatory checks cannot be represented as complete success.

A green aggregate status is derived from individual required checks and policy. It is not an input supplied by a worker. Optional check failure is recorded and handled according to policy; it cannot silently change mandatory check outcomes.

The Engine rejects:
- wrong transaction, scope, base or resulting snapshot;
- patch or plan digest mismatch;
- stale policy or index generation;
- missing required check or substituted command/toolchain;
- untrusted or unverifiable executor provenance;
- incomplete required artifacts;
- expired/revoked admission or lease;
- duplicate evidence with conflicting content;
- evidence produced after cancellation or attempt expiry;
- any result whose completeness cannot be established.

## 10. Caching

Caching is an optimization, never an authority source or verification bypass.

A cache key must include every correctness-relevant input: source/result snapshot, patch digest where applicable, check definition, command and arguments, toolchain/runtime, dependency/lockfile state, configuration, platform/architecture, relevant environment, sandbox policy/image, network/credential mode and declared external inputs.

Cache entries bind the original complete evidence and artifact digests. A cache hit is admissible only if the Engine's current policy explicitly permits reuse and all inputs match. Expired, incomplete, unverifiable or cross-scope cache entries are misses. No cache may convert an incomplete analysis or missing check into success.

Do not implement a broad remote build cache before the key and evidence contracts are tested against adversarial input changes.

## 11. Failure, retry and recovery semantics

The attempt state machine must explicitly handle:
- patch schema/digest failure;
- wrong base snapshot or preimage mismatch;
- path traversal, symlink escape and case collision;
- source capture incompleteness or mutation;
- materialization failure or disk exhaustion;
- sandbox startup failure;
- policy/admission expiry or revocation;
- worker/process crash;
- timeout and process-tree kill failure;
- cancellation before and during execution;
- network denial or permitted-network failure;
- dependency acquisition failure;
- partial check completion;
- output truncation or artifact-store outage;
- duplicate, delayed or reordered result delivery;
- lost response after a completed side effect;
- host/VM/remote worker loss;
- stale evidence after repository advancement;
- cleanup failure and orphaned workspaces.

Retries use stable idempotency identity and attempt-specific records. A retry does not overwrite prior evidence. A timed-out process must be confirmed terminated before its writable workspace is reused. If termination cannot be established, quarantine the workspace/worker and escalate.

A lost response is reconciled by querying the durable attempt identity and artifact digests, not by blindly executing the command again. In-memory development implementations must model this contract and label their durability limits.

No automatic retry may change patch, snapshot, command, environment, policy or execution tier under the same attempt identity. Any changed input creates a new attempt and new evidence lineage.

## 12. Security and adversarial test matrix

At minimum, tests must include:

### Patch and filesystem safety
- wrong snapshot/base/preimage;
- malformed and non-canonical patch;
- duplicate/conflicting operations;
- absolute paths, traversal, reserved paths and platform path ambiguity;
- symlink escape and symlink replacement races;
- case-fold and Unicode normalization collisions where applicable;
- binary payload digest/size mismatch;
- mode and rename semantics;
- source snapshot remains unchanged after success and failure;
- concurrent source mutation and materialization consistency.

### Admission and authority
- unauthorized principal and cross-scope attempt;
- revoked/expired capability, reservation and lease;
- stale repository-analysis artifact/index generation;
- incomplete impact and unknown coverage;
- attempt outside legal transaction state;
- altered patch/check/policy digest;
- duplicate idempotency key with same and different inputs;
- executor cannot widen resources, network, credentials or path scope;
- workers cannot forge evidence or advance repository revision.

### Execution isolation
- read access outside mounted workspace;
- writes outside scratch/output mounts;
- ambient credential/environment access;
- process, fork, memory, CPU, disk and output exhaustion;
- network egress denied by default and explicit allowlist enforcement;
- child process survives parent timeout;
- sandbox escape probes appropriate to selected backend;
- backend unavailable cannot downgrade to host execution;
- cleanup failure quarantines resources.

### Verification and evidence
- wrong resulting snapshot or patch digest;
- omitted or substituted mandatory check;
- toolchain/config/dependency/environment mismatch;
- stale LSP or analyzer result;
- false green aggregate status;
- nonzero exit, signal, timeout, cancellation, infrastructure error;
- missing/truncated/unretrievable artifacts;
- evidence digest tampering;
- duplicate and out-of-order result delivery;
- cache key omission mutation tests;
- failed attempt evidence remains immutable and distinguishable from retry.

### Concurrency and recovery
- competing patch attempts on same base;
- lease expiry during patching and verification;
- repository advancement during verification;
- cancellation racing with completion;
- worker crash before and after side effects;
- lost result response after evidence sealing;
- stale attempt cannot satisfy a newer transaction;
- cleanup and restart reconciliation.

Use deterministic fake clocks, fault injection and controlled executor fixtures for state-machine tests. Use real sandbox integration tests for the security properties that cannot be established by mocks.

## 13. Performance and benchmark protocol

Establish a baseline before optimization. Measure separately:
- snapshot materialization throughput and bytes copied/reused;
- patch validation/application latency by operation and file size;
- resulting manifest/hash cost;
- sandbox startup and teardown by tier;
- per-check wall time and resource consumption;
- evidence sealing, artifact upload and retrieval;
- cache lookup and validation overhead;
- concurrency, queueing and backpressure;
- cleanup latency and orphan recovery.

Use representative repositories and reproducible toolchains. Preserve raw measurements, hardware, OS/kernel, compiler/runtime, sandbox backend/image, fixture digests, source commit and exact commands. Separate generated fixtures from actual repositories. Do not extrapolate production capacity from small fixtures or a single developer machine. Targets are set from observed data, not invented in advance.

## 14. Implementation sequence

The following sequence is dependency-driven; each milestone must leave a compiling, tested and documented repository state.

### 03.1 — Contracts and deterministic patch engine
- Define canonical patch envelope, operation types, digest and validation.
- Implement exact-base/preimage application to an isolated in-memory or temporary materialization.
- Produce deterministic resulting manifests and application reports.
- Add path/symlink/binary/mode safety tests.
- Prove the original snapshot is never mutated.

### 03.2 — Engine admission and source-attempt state
- Extend Engine contracts for source patch attempts without weakening Goal 01/02.
- Bind admission to transaction, snapshot, index/impact, patch, policy, lease and idempotency.
- Add legal attempt transitions, cancellation and expiry semantics.
- Reject incomplete impact by default.
- Keep final source revision publication explicitly unsupported until a later commit-coordination milestone.

### 03.3 — Verification plan and check identity
- Define policy-owned check registry and repository harness configuration.
- Resolve mandatory checks from invariants and impact without model authority.
- Canonicalize command, args, working directory, inputs, outputs, toolchain and resource policy.
- Test missing, ambiguous, substituted and unauthorized checks.

### 03.4 — Isolated executor
- Define backend-neutral executor contract.
- Implement deterministic T0 operations and one validated isolated execution backend.
- Enforce filesystem, network, credential, process and resource boundaries.
- Add real integration probes; mocks alone do not validate isolation.
- Add tier routing that fails closed when a required backend is unavailable.

### 03.5 — Evidence sealing and Engine admission
- Record complete per-check outcomes and artifact references.
- Seal canonical immutable evidence.
- Validate all bindings and completeness in the Engine.
- Prove stale, forged, mismatched, partial and unpersisted evidence is rejected.
- Preserve retries as separate immutable attempts.

### 03.6 — End-to-end proof and measured baseline
- Execute a real patch against a disposable snapshot-derived repository.
- Run configured checks in the approved isolation backend.
- Return and admit evidence through the Engine.
- Demonstrate failure, timeout, cancellation, retry, stale-base rejection and source immutability.
- Publish reproducible benchmark and validation records.

## 15. Definition of done

Goal 03 is complete only when all conditions hold:

1. Canonical, versioned patches are bound to exact scope, transaction and base snapshot.
2. Patch application is deterministic, exact-base and non-mutating with respect to source inputs.
3. Path traversal, symlink escape, conflicting operations and malformed payloads are rejected.
4. A resulting immutable snapshot and complete application provenance are produced.
5. The Engine admits execution only under valid authorization, transaction state, impact, policy and lease.
6. Incomplete/unknown impact cannot silently authorize execution.
7. Verification plans and check identities are deterministic and policy-owned.
8. Untrusted repository code executes only in a validated isolation backend; no silent host fallback exists.
9. Every required check runs against the exact resulting snapshot and declared environment.
10. Evidence binds all inputs, checks, toolchain, execution, outcomes and artifacts.
11. The Engine independently validates evidence and rejects stale, forged, partial or mismatched records.
12. Cancellation, timeouts, retries, crashes, duplicate delivery and artifact failures have tested recovery behavior.
13. End-to-end tests prove patch → isolated verification → sealed evidence → Engine admission.
14. Benchmarks and raw evidence are reproducible and make no unsupported production claims.
15. Documentation states actual platform/language/backend coverage and remaining limitations.
16. All work is committed with the repository's human-only author/committer policy.

## 16. Explicit non-goals and deferred boundaries

Goal 03 does not claim or deliver:
- production distributed atomic commit across multiple Engine instances;
- durable PostgreSQL transaction/reservation storage;
- Temporal workflow orchestration or NATS event distribution;
- full Firecracker fleet or remote T3 execution pool unless explicitly selected as the first validated backend;
- real AI provider/worker integration or Brain planning;
- complete language-server coverage or language-wide semantic correctness;
- public multi-tenant production deployment;
- automatic dependency installation or unrestricted network access;
- source revision publication merely because verification passed.

These remain governed by accepted ADRs and later implementation milestones. Do not use these exclusions to weaken the interfaces or security model required now.

## 17. Required completion report

The final Goal 03 report must state:
- exact source commit and clean-checkout status;
- implemented patch formats and operation semantics;
- supported snapshot/materialization modes and known uncertainty;
- actual isolation backend, threat boundary and tested properties;
- supported language harnesses and exact toolchain versions;
- complete test/CI commands and outcomes;
- adversarial/fault-injection coverage;
- raw benchmark references and measured environment;
- Engine evidence-admission behavior;
- unsupported capabilities and remaining risks.

A planned backend, mocked integration, parser fixture or successful local command must never be reported as a validated production capability.
