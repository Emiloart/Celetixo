# Celetixo Evaluation Protocol

**Status:** Evaluation contract  
**Purpose:** Define a reproducible method for testing whether Celetixo improves verified engineering outcomes. This document does not claim that Celetixo has passed the evaluation.

## 1. Primary research question

Does semantic transaction coordination produce better verified engineering outcomes than strong sequential and ordinary parallel coding-agent workflows on equivalent repository tasks and resource conditions?

The evaluation must test the value of the coordination system, not merely whether multiple agents can be launched.

## 2. Comparison arms

Use three arms:

| Arm | Execution model | Coordination |
|---|---|---|
| A — Sequential | One strong coding agent performs the task sequentially | Ordinary agent workflow |
| B — Parallel worktrees | Multiple agents work in isolated Git worktrees | Ordinary Git integration/merge; no Celetixo semantic transactions |
| C — Celetixo | Multiple workers operate through Celetixo transactions | Semantic impact, reservations, OCC validation, evidence-gated commit/rebase |

Arm B is essential. Without it, the evaluation cannot distinguish the benefit of semantic coordination from the generic benefit or cost of parallel execution.

All arms receive equivalent task statements, repository snapshots, allowed tools, model access where practical, resource ceilings and acceptance criteria. Record any unavoidable differences. Do not give one arm hidden context or privileged tests.

## 3. Task suite

Freeze a versioned task suite before confirmatory evaluation. The suite should include:
- independent changes that should safely run concurrently;
- overlapping edits that should conflict;
- caller/callee and interface changes;
- shared type/schema changes;
- dependency and lockfile changes;
- configuration/build/CI changes;
- generated-code and code-generation changes;
- predicate or membership dependencies;
- rename/move/refactor tasks;
- incomplete or unsupported language regions;
- tasks with misleadingly clean textual merges;
- tasks where semantic revalidation should permit safe concurrency;
- tasks where revalidation must abort or escalate.

Use multiple repositories, sizes, language harnesses and task difficulties. Document repository licenses, task provenance and benchmark contamination controls. Keep hidden acceptance tests and reference outcomes isolated from agents.

## 4. Experimental controls

- Pin repository snapshot identities and task-suite versions.
- Pin or record model/provider versions, prompts, tool configurations and sampling parameters.
- Match compute, concurrency, token/tool budgets and wall-clock policies across arms.
- Randomize task/arm order where practical.
- Repeat stochastic runs; report distributions and confidence intervals, not only a best run.
- Separate exploratory runs from confirmatory evaluation.
- Pre-register primary outcomes, exclusion rules and analysis methods before confirmatory runs.
- Preserve complete run provenance and artifacts.
- Do not tune Celetixo on the hidden confirmatory set.

## 5. Primary outcomes

Report at minimum:
- **Verified task success rate:** proportion of tasks meeting all predeclared acceptance checks.
- **Cost per verified success:** total measured model, compute, execution and infrastructure cost divided by verified successful outcomes.
- **Time to verified success:** wall-clock time from task admission to accepted outcome, including retries and rework.
- **Regression rate:** proportion of runs introducing failures outside the intended task acceptance scope.
- **Human correction rate:** proportion of outcomes requiring human code correction beyond review/approval.
- **Throughput:** verified successful tasks per unit of resource/time under declared concurrency.
- **Commit safety violations:** any accepted state transition that violates the declared transaction model or protected invariant. Report separately; do not average away.

## 6. Coordination and diagnostic metrics

Define and report:
- **Impact precision:** true positive affected elements divided by all elements predicted affected.
- **Impact recall:** true positive affected elements divided by all affected elements in the adjudicated reference set.
- **Impact false-negative rate:** missed affected elements divided by all affected elements. This is safety-critical.
- **Conflict precision/recall:** evaluated against adjudicated transaction compatibility.
- **False-positive conflict rate:** compatible pairs/histories rejected as conflicting.
- **Abort rate:** aborted transactions divided by admitted transactions, broken down by correct safety abort, conflict, stale base, worker failure, policy rejection and false-positive abort.
- **Wasted execution cost:** compute/model/tool cost spent on work that is discarded, invalidated or repeated.
- **Wasted-token ratio:** tokens consumed by invalidated/discarded attempts divided by total tokens consumed, with numerator categories disclosed.
- **Verification cost:** time and compute spent on checks, cache validation and reruns.
- **Human intervention:** review time, manual conflict resolution and correction effort.
- **Recovery correctness:** proportion of injected failures that recover to the specified durable state without invariant violation.

High abort rate is not inherently a defect. A correct safety abort may be preferable to an unsafe commit. Report false-positive aborts and avoidable wasted work separately.

## 7. Reference oracles and correctness tests

Before interpreting comparative results, validate the Engine against:
- an independent reference transaction model;
- generated histories with deterministic seeds and shrinking;
- invariant checks over accepted histories;
- multi-instance commit races;
- crash injection around commit linearization;
- stale/incomplete impact and stale-evidence cases.

The oracle validates the declared consistency model, not completeness of real-world semantic dependency extraction. Impact quality requires separate adjudicated ground truth.

## 8. Statistical reporting

Report sample counts, run distributions, uncertainty intervals, missing runs, exclusions and all predeclared metrics. Include per-task and per-language breakdowns where sample size permits. Do not report only aggregate averages if they conceal safety regressions or subgroup failures.

Separate:
- engineering correctness gates;
- primary comparative outcomes;
- diagnostic metrics;
- exploratory findings.

No performance improvement claim is valid without reproducible artifacts, pinned task suite and transparent methodology.

## 9. Decision rule

Before confirmatory evaluation, publish:
- frozen task suite and version;
- primary metric(s);
- safety invariants and zero-tolerance violation definitions;
- statistical analysis plan;
- minimum sample rationale;
- exclusion and rerun rules;
- model/resource parity policy.

A performance win cannot compensate for a commit safety violation. If safety criteria fail, the result is a correctness failure regardless of throughput or cost. If safety criteria pass but outcome improvements are inconclusive, report inconclusive rather than claiming superiority.

## 10. Evidence and governance

Every run should preserve task, repository/snapshot, arm, model, prompt/configuration, tool, resource, patch, test, verification, cost, timing, human outcome and failure-injection provenance. Apply [trajectory governance](docs/spec/trajectory-governance.md) to collection, retention, evaluation and any future training use.

## 11. Current status

The protocol is a plan. No comparative result is implied by the existence of this document. Evaluation claims must link to a dated report, frozen suite version, raw/derived evidence location and reproducible analysis.
