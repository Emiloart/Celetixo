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
| [Goal 04 foundation progress](implementation/goal-04-progress.md) | Implementation / validation | 04.1a schema, snapshot catalog and canonical protocol foundation; 04.1b runtime/state separation; full durable Engine and later milestones remain pending |
| [Goal 04 PostgreSQL schema](implementation/goal-04-postgres-schema.md) | Implementation | Scoped schema and required Rust authority integration |
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
