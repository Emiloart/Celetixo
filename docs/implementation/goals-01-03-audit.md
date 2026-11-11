# Goals 01–03 regression audit

Recorded 2026-11-11. This audit compares the three implementation contracts,
accepted ADRs, relevant specifications, current source, executable tests and
retained measurements. It found two reporting/provenance defects, corrected
below. No unmet acceptance item was found within the goals' documented bounded
implementation scopes. This is not a production-readiness or complete-language
claim.

## Source and preservation boundary

Fresh workspace checks started in an isolated checkout at
`949cda584649026b31551824374a860ea992b8ae`. The owner's subsequent Goal 04 documents
were integrated by fast-forward to
`32dd36da6dade8c3fc1e9b184f13ebec75969a49`. Those intervening commits change only
documentation; Rust source, fixtures, manifests, lockfile and toolchain are
identical. The audit changes reporting, regression automation and documentation,
with no Rust runtime or transaction-test changes. Build caches were reused.

The original checkout's README edits and untracked environment file were left
untouched. Neither was captured, executed or included in this publication.

## Findings and corrections

1. **Goal 03 summary validation could disappear under Python optimization.**
   The helper used `assert` for sample counts, input consistency, successful
   commands, disabled caching and confirmed recovery. Explicit errors now retain
   those checks under `python -O`, also requiring recorded clean source before
   collection and a nonempty command list. The regression script checks both
   Python modes: accepted raw records reproduce the summary byte for byte, and
   13 altered-input cases each reject without replacing an existing report.
   These are 28 subprocess cases in two unittest groups. CI runs the regression.
   This defect concerned benchmark reporting, not Engine evidence admission.
2. **Goal 01 referenced three commits outside the corrected public history.**
   Exact Git-tree comparison maps the original foundation, transaction and oracle
   milestones to `da623358728937f7b43b8106f8385009d9d1aca7`,
   `7331071fad9810b86e616cf9a3382c558ced0f71` and
   `dac9ea59df74c00e6f988e91be24fe470455a6b9`. The historical record now names
   those reachable human-attributed commits and identifies its then-current
   limitations as historical. Its original 62-test result is preserved; no new
   execution of the historical checkout is claimed.

## Requirement review

| Goal / requirement | Current implementation and evidence | Boundary retained |
|---|---|---|
| 01: typed state, scoped authority, immutable identity | Engine domain/snapshot/transaction modules; authorization and snapshot regressions | In-process embedding authenticates and administers capabilities |
| 01: compatible concurrency and conflict rejection | OCC gate; explicit/derived graph accesses, predicates/invariants; independent bit-mask/matrix oracle over 192 seeds; racing Engine handles | Declared dependency model, not arbitrary program correctness |
| 01: exact evidence, leases, revalidation and recovery | Required-check binding, verifier revocation, fresh-attempt evidence, lease/cancellation and five commit-fault schedules | One shared in-memory authority store; no distributed durability |
| 02: real input capture and race handling | Capture regressions cover clean/dirty, staging, deletions/renames, untracked/ignored files, overlays, Git metadata, submodules, modes/links, generated/mounted inputs and budgets | Every live capture retains `LiveSourceNotAtomic`; immutable-tree completeness names only its selected inputs |
| 02: versioned language indexing | Pinned Tree-sitter adapters for TS/JS, Python, Go and Rust; scopes/definitions/references/imports/types/packages/configuration; version, identity and incremental/rebuild tests | Structural coverage gaps remain; historical lineage is conservative rather than a durable identity registry |
| 02: deeper semantic queries | Exact snapshot/document/session-bound LSP, sealed response provenance, bounded transport and actual rust-analyzer probe | TS/JS, Python and Go server processes remain unvalidated; query-only results cannot clear coverage gaps |
| 02: defensible impact and Engine integration | Deterministic direct/transitive/cyclic impact, read/write/predicate/invariant reconsideration, justified checks; scoped artifact installation/CAS and stale-result rejection | Unresolved/unsupported information never becomes proof of isolation |
| 02: accuracy and measurements | Independent five-scenario reference, raw timing/RSS/query records, scan-lookup oracle and exact incremental/rebuild equivalence | Known Python dynamic miss retained: 15 TP / 0 FP / 1 FN, 93.75% pooled file recall; no actual-repository quality oracle |
| 03: canonical patch and immutable candidate | Exact-base/preimage operations, binary/mode/rename/link/path rules, provenance replay and no-follow private materialization | No fuzzy patching; unsupported path/metadata forms reject |
| 03: authoritative execution and checks | Scoped source attempts, invariant closure, installed policy, exact environment/check identities, lease/executor/cleanup grants and configured-harness denial tests | Real incomplete structural analysis rejects; positive integration uses a separately enumerated finite reference model |
| 03: actual isolation and recovery | Ten live Linux T1 integration groups: physical boundaries/resources, cancellation/deadline, SIGKILL/controller loss, publication faults, source retention/cleanup and fresh retry | Reviewed rootless runc profile; host-kernel/operator trust; no T2/T3 or host fallback |
| 03: persisted evidence and independent admission | Canonical evidence v2, executor authentication, durable local object publication/readback, independent Engine hashing/binding and post-I/O authorization | Engine state remains in memory; local persistence is not a distributed object store |
| 03: final outcome and benchmarks | Actual patch → isolated Go syntax check → persisted evidence → Engine receipt; raw measured corpus/tool/source bindings and summary reproduction | Receipt moves to Validating; source publication still returns `SourceMutationUnsupported`; no general Go build/test harness |

See the original [Goal 01](goal-01.md), [Goal 02](goal-02.md) and
[Goal 03](goal-03.md) contracts and their validation records for the detailed
acceptance criteria. The [capability matrix](../reference/capability-support-matrix.md)
and [threat model](../spec/threat-model.md) correctly retain these boundaries.

## Fresh checks and retained-evidence integrity

The [workspace/runtime log](goals-01-03-audit/checks.log) and
[command manifest](goals-01-03-audit/checks.json) record exact commands, results,
compiler workaround and timings. All ten commands passed:

- Workspace formatting, strict all-target Clippy and locked/offline tests:
  **339 passed**, zero failed. The 12 normally ignored groups are not counted
  as passing in this command.
- Explicit LSP run: **28 passed**, including actual rust-analyzer. Its 27 ordinary
  groups overlap the workspace count.
- Workspace/example build and concurrency example: passed, retaining both
  compatible writes and rejecting the invalidated-read case.
- Attribution regressions: **25 passed**; all reachable commits at the tested
  baseline passed raw author/committer/co-author policy checks.
- Explicit OCI/source-flow run: **10 passed**, zero ignored. The internal
  subprocess crash helper is exercised through its parent test rather than
  invoked as a standalone successful test.
- Whitespace check: passed.

The [follow-up log](goals-01-03-audit/followup.log) records the new Python
regressions, optimized-Python Goal 02 comparison/summary checks, release inspector
build, exact-tree self-inspection and benchmark smoke. These are functional
checks, not replacement performance samples.

All six [follow-up commands](goals-01-03-audit/followup.json) passed. The
[self-inspection summary](goals-01-03-audit/inspection-summary.json) binds the exact
`32dd36d` tree: 181 files, 11,947 elements and 35,379 edges. Capture was complete
for that immutable input set; 22,634 coverage gaps kept both impact layers
incomplete and source mutation unsupported. The full 4.8 MB deterministic report
is retained locally, with its digest and reproduction command published. The
[single-iteration benchmark smoke](goals-01-03-audit/benchmark-smoke.json) again
passed incremental/rebuild equivalence and retained incomplete impact.

[Integrity checks](goals-01-03-audit/integrity.json) independently verify all
111 benchmark-recorded source blobs at `fa888ed`, all four summary input hashes,
the evidence-v2 validation-log hash and the three Goal 01 Git-tree mappings.
Current runtime/runner sources are unchanged from the measured revision. The
later LSP test timing correction remains present. Retained benchmark results,
targets and Engine admissibility requirements were not relaxed.

## Limits and later work

The audit does not rerun every three-repetition performance workload or rebuild
the old Goal 01 checkout. Exact source/tree integrity, retained raw records,
byte-identical summary reproduction and current regression checks establish the
claims stated here; no new benchmark speedup is claimed.

Actual TS/JS/Python/Go language-server processes, general language execution
harnesses, native Windows/macOS capture or isolation, source revision publication,
durable PostgreSQL authority, distributed recovery, T2/T3, public multi-tenant
isolation, Brain/orchestration and the three-arm coding-agent evaluation remain
unvalidated or unimplemented. The owner's [Goal 04](goal-04.md) is a separate
contract for the next platform stage. This audit does not mark it implemented.
