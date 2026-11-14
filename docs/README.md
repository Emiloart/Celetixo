# Documentation index

This page is the navigation and status map for the Celetixo documentation corpus. Read it before relying on an individual document.

## How to read the documentation

Celetixo documentation uses four distinct evidence classes:

| Class | Meaning | Authority |
|---|---|---|
| **Normative / target** | Accepted architecture, required contracts, constraints or future release gates | ADRs are authoritative for architectural decisions; linked specifications define their detailed requirements |
| **Implementation** | What a named code milestone is intended to implement or how a subsystem is designed | Implementation contracts explain scope; they do not prove completion |
| **Validation** | What was actually built, tested, measured or rejected, with source revisions and environment | Validation records and reproducible artifacts are evidence for their stated scope only |
| **Historical** | Earlier milestone sequence, superseded state, development chronology or retained raw evidence | Useful for provenance; never overrides a later accepted decision or current acceptance record |

A document may contain more than one class. Its status header and opening scope statement determine how it may be used. When documents disagree, apply this order:

1. Accepted ADRs govern architectural decisions.
2. Normative specifications operationalize those decisions without silently changing them.
3. Implementation contracts describe intended subsystem behavior.
4. Current acceptance/validation records establish what has actually been demonstrated.
5. Historical milestone records preserve chronology and must not be mistaken for current status.

Implementation evidence can reveal that an accepted decision is flawed, but it does not silently amend that decision. Propose an ADR amendment and preserve the evidence.

## Intended readers

| Reader | Start here |
|---|---|
| New contributor or reviewer | This index, [README](../README.md), [CONTRIBUTING](../CONTRIBUTING.md), then [AGENTS](../AGENTS.md) |
| System/solution architect | ADR-0001 through ADR-0004, then the normative specifications |
| Rust Engine developer | [Dev guide](../Dev.md), ADR-0001/0003, transaction and identity specifications, Goal 01–04 implementation records |
| Repository-intelligence developer | [Goal 02 contract](implementation/goal-02.md), [Goal 02 validation](implementation/goal-02-validation.md), [Engine contract](implementation/goal-02-engine-contract.md) |
| Execution/security engineer | [Threat model](spec/threat-model.md), [failure model](spec/failure-model.md), [Linux T1 runbook](operations/local-linux-t1-runbook.md), Goal 03 execution and acceptance |
| Operator of the validated local profile | [Local Linux T1 runbook](operations/local-linux-t1-runbook.md) |
| Evaluator/researcher | [Evaluation protocol](../EVALUATION.md), Goal 02/03 benchmark protocols and measured baselines |
| Protocol/API implementer | [Protocol reference](reference/protocol.md), then ADR-0001 and ADR-0002 |
| Distribution or licensing reviewer | [Licensing and distribution policy](spec/licensing-and-distribution.md), [CONTRIBUTING](../CONTRIBUTING.md) |

## Architecture decisions

| Document | Class | Authority / scope |
|---|---|---|
| [ADR-0001: Engineering-state coordination](adr/0001-celetixo-engineering-state-coordination.md) | Normative / target | Frozen v1.0 architectural baseline |
| [ADR-0002: Platform, Brain, clients and correctness extensions](adr/0002-platform-brain-client-and-correctness-extensions.md) | Normative / target | Accepted additive platform and correctness extensions |
| [ADR-0003: Transaction consistency and validation](adr/0003-transaction-consistency-and-validation-model.md) | Normative / target | Accepted transaction, serializability and validation requirements |
| [ADR-0004: Internal-first, public-capable architecture](adr/0004-internal-first-public-capable-architecture.md) | Normative / target | Accepted deployment and product-boundary decision |

## Normative specifications and operational policy

| Document | Class | What it governs |
|---|---|---|
| [Transaction semantics](spec/transaction-semantics.md) | Normative / target | Transaction state, access semantics, conflicts, commit and revalidation |
| [Engineering identity](spec/engineering-identity.md) | Normative / target | Scope, snapshot, file, semantic element, occurrence and evidence identity |
| [Failure and recovery model](spec/failure-model.md) | Normative / target | Distributed failure assumptions, crash points, recovery and fault-injection obligations |
| [Trajectory governance](spec/trajectory-governance.md) | Normative / target | Rights, purpose, privacy, secrets, retention, evaluation separation and training eligibility |
| [Deployment and product boundaries](spec/deployment-and-product-boundaries.md) | Normative / target | Organization/workspace scope, policy enforcement, isolation and public-release gates |
| [Threat model](spec/threat-model.md) | Normative security model | Assets, trust boundaries, attackers, abuse cases, controls, residual risks and evidence status |
| [Licensing and distribution](spec/licensing-and-distribution.md) | Normative repository policy | Current no-license state, contribution terms boundary and release restrictions |

## Implementation contracts and current acceptance evidence

| Document | Class | How to interpret it |
|---|---|---|
| [Goal 01: Engineering State Engine](implementation/goal-01.md) | Implementation | Bounded transaction Engine contract and its explicit reference-model limits |
| [Goal 01 validation](implementation/validation.md) | Validation | Actual Goal 01 test and implementation evidence |
| [Goal 02: Repository intelligence](implementation/goal-02.md) | Implementation | Capture, structural indexing, LSP boundary, impact analysis and known gaps |
| [Goal 02 validation](implementation/goal-02-validation.md) | Validation | Actual implementation checks and source-bound evidence |
| [Goal 02 Engine contract](implementation/goal-02-engine-contract.md) | Implementation / validation | Engine admission and repository-analysis integration boundary |
| [Goal 03: Controlled patch and verification](implementation/goal-03.md) | Implementation | Accepted source-mutation and execution objective |
| [Goal 03 execution component](implementation/goal-03-execution.md) | Implementation / validation | Actual private materialization, Linux T1 runner, evidence and recovery behavior |
| [Goal 03 acceptance](implementation/goal-03-acceptance.md) | Validation | Current acceptance audit, measured source revisions, integration evidence and limitations |
| [Goal 03 detailed contract comparison](implementation/goal-03-contract-review.md) | Validation | Full-objective cross-check, conditional capability boundaries and fresh focused/live evidence |
| [Goals 01–03 regression audit](implementation/goals-01-03-audit.md) | Validation | Cross-goal requirement review, fresh checks, evidence integrity and reporting/history corrections |
| [Goal 03 progress](implementation/goal-03-progress.md) | Historical / implementation | Chronological milestones; consult acceptance record for current status |
| [Goal 04 foundation progress](implementation/goal-04-progress.md) | Implementation / validation | 04.1/04.2 accepted for the reviewed PostgreSQL adapters; later service/operational milestones remain pending |
| [Milestone 04.1 acceptance](implementation/goal-04-authority-acceptance.md) | Validation | Original durable-state criteria audited at schema v8: normalized native/source state, exact outcomes, reservations/fences, every supported prior upgrade and actual checked restart; full Goal 04 remains active |
| [Milestone 04.2 atomic commit acceptance](implementation/goal-04-commit-acceptance.md) | Validation | Reviewed native/source PostgreSQL atomic commit, independent-process histories and exact crash/outcome recovery; 48 generated Engine source histories and 16 real SQL histories; full Goal 04 remains active |
| [Goal 04 PostgreSQL schema](implementation/goal-04-postgres-schema.md) | Implementation | Scoped schema and required Rust authority integration |
| [Goal 04 semantic persistence](implementation/goal-04-semantic-state.md) | Implementation | Checked immutable model/analysis records and scoped PostgreSQL candidate catalog; current-head installation and transaction authority remain pending |
| [Goal 04 state evaluation/publication](implementation/goal-04-state-access.md) | Implementation | Explicit reference-state update boundary and retained domain-error outcomes; no durable transaction backend or distributed commit claim |
| [Goal 04 lifecycle/request contracts](implementation/goal-04-lifecycle-records.md) | Implementation / validation | Checked native lifecycle payloads, exact reference commit requests/fences and supported v1-to-v2 schema upgrade; SQL lifecycle authority remains pending |
| [Goal 04 authority reconstruction/backend boundary](implementation/goal-04-authority-boundary.md) | Implementation / validation | 04.1f checked native state, fresh session resolution, typed outcomes and schema v3 upgrade; no SQL backend or distributed authority |
| [Goal 04 PostgreSQL registration authority](implementation/goal-04-registration-authority.md) | Implementation / validation | 04.1g actual SQL backend for native grants/repository/model registration, checked reopen, role/schema/clock fencing and bounded failures; complete lifecycle and distributed commit remain pending |
| [Goal 04 native SQL lifecycle authority](implementation/goal-04-native-sql-authority.md) | Implementation / validation | 04.1h normalized native lifecycle, exact requests/results, leased atomic commit and outbox facts; complete source recovery and full Goal 04 remain pending |
| [Goal 04 checked source metadata codecs](implementation/goal-04-source-codecs.md) | Implementation / validation | 04.1i pure checked policy/plan/admission/evidence recovery prerequisites; no SQL source enablement or restored runtime authority |
| [Source authority reconstruction and fresh runtime recovery](implementation/goal-04-source-recovery.md) | Implementation / validation | Consolidated source-state graph, original requests/results, scoped callbacks and fresh capability/reader resolution; original evidence uses a memory adapter, with subsequent SQL integration below |
| [Captured-source PostgreSQL lifecycle authority](implementation/goal-04-source-sql-authority.md) | Implementation / validation | Normalized source lifecycle through evidence receipts, fresh runtime resolution and additive schema v4; that record covers receipts, with subsequent source publication/revalidation below; full Goal 04 remains pending |
| [Durable original mutation outcomes](implementation/goal-04-mutation-recovery.md) | Implementation / validation | Typed original transaction, repository and administrative outcomes, additive schema v5 and atomic recovery; the subsequent executor extension is recorded below |
| [Original source-executor outcomes](implementation/goal-04-executor-recovery.md) | Implementation / validation | Keyed claim/patch/verification/finish recovery, additive schema v6 and coordinator launch recovery; subsequent source commit/revalidation are recorded below; full Goal 04 remains pending |
| [Source publication and prepared request bindings](implementation/goal-04-source-publication.md) | Implementation / validation | Atomic source head/history/model/outcome/outbox, exact original recovery and checked SQL history with schema v7; fresh revalidation is recorded below; full Goal 04 remains pending |
| [Fresh source revalidation](implementation/goal-04-source-revalidation.md) | Implementation / validation | Atomic fresh source attempts, recomputed impact, fenced reservations and original recovery with schema v8; 548 workspace tests, 14 selected SQL parents and 10 actual T1 cases; full Goal 04 remains pending |
| [Independent source commit processes](implementation/goal-04-source-processes.md) | Implementation / validation | Scoped read-only local artifact access, independent source commit races and process-exit recovery; 550 workspace passes, two selected SQL parents and 10 actual T1 regressions; complete 04.2 remains open |
| [Goal 04 session handoff](implementation/goal-04-session-handoff.md) | Implementation / operations | Original pause after 04.1f, resumed on 2026-11-12; remaining objective, workspace preservation and SQL integration work |
| [Goal 04: Durable orchestration and distributed commit](implementation/goal-04.md) | Implementation | Accepted contract for durable authority, multi-instance commit, workflow/event coordination, evidence storage and recovery; implementation in progress |

## Security, operations and support references

| Document | Class | Purpose |
|---|---|---|
| [Threat model](spec/threat-model.md) | Normative model with implementation-status mapping | Security review and control traceability |
| [Local Linux T1 operations runbook](operations/local-linux-t1-runbook.md) | Operational procedure with explicit validation scope | Reproduce and operate only the reviewed local Linux profile |
| [Capability and support matrix](reference/capability-support-matrix.md) | Dated status reference | Separate parsing, semantic analysis, execution and environment validation |
| [Protocol reference](reference/protocol.md) | Protocol status/reference | Identify accepted wire-contract semantics and the canonical TaskIntent schema, strict Rust intake and remaining service boundary |

## Evaluation and benchmarks

| Document | Class | Purpose |
|---|---|---|
| [Evaluation protocol](../EVALUATION.md) | Normative evaluation protocol | Comparative study design, controls, metrics and decision rules |
| [Goal 02 benchmark report](benchmarks/goal-02.md) | Validation / measured baseline | Repository-intelligence measurements and quality limitations |
| [Goal 03 benchmark protocol](benchmarks/goal-03-protocol.md) | Normative measurement procedure | Reproducible T0/T1 measurement method |
| [Goal 03 measured baseline](benchmarks/goal-03.md) | Validation / measured baseline | Actual repetitions, raw evidence, provisional tracking targets and limits |
| Raw benchmark JSON, manifests and validation fixtures | Historical / validation artifacts | Immutable inputs and detailed reproduction evidence linked from the reports |

## Repository-level contributor and agent instructions

- [AGENTS.md](../AGENTS.md) is the repository-wide instruction contract for automated and human-assisted engineering workflows. It does not override ADRs.
- [Dev.md](../Dev.md) is the practical engineering/build sequence. It translates accepted decisions into implementation order and working discipline; it is not a substitute for subsystem specifications.
- [CONTRIBUTING.md](../CONTRIBUTING.md) defines human Git attribution and local/CI enforcement. It is not a software license and does not grant redistribution rights.
- [README.md](../README.md) is the concise system overview and entry point, not the complete normative specification.

## Status discipline

- **Accepted target** does not mean implemented.
- **Implemented** does not mean validated in every environment.
- **Validated** applies only to the exact source, toolchain, platform, fixture and checks recorded.
- **Benchmark measured** does not mean production capacity or a universal SLO.
- **Unsupported** means the implementation must reject or fail closed; it must not silently fall back.
- **Historical** evidence remains useful for provenance but cannot be promoted into a current capability claim.

The [capability/support matrix](reference/capability-support-matrix.md) is the compact status view. Its date and linked evidence must be refreshed whenever implementation or validation changes.
