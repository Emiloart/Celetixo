# Goal 03 detailed contract comparison

**Reviewed:** 2026-11-11

**Tested source:** `9538e107a688e21aa0730a23a6cf00b3719ce73d`

**Authority:** the accepted [Goal 03 objective](goal-03.md), ADRs and specifications;
this review does not amend their requirements.

The Goal 03 exit gate is complete for the documented reference Engine and
reviewed local Linux T1 profile. This comparison found no additional blocker
within that scope. It checks the detailed contract, not only the sixteen-item
definition of done or a successful test count.

The conclusion is narrower than a general repository agent: actual incomplete
source analysis still blocks execution, positive execution uses an explicitly
enumerated reference fixture, and successful verification cannot publish source.
Those boundaries appear in the accepted objective itself (§6, §14.2 and §16).

## Requirement-by-requirement comparison

| Objective section | Implemented behavior and evidence | Interpretation / limit |
|---|---|---|
| §1–3: integrated deliverables and authority | Patch, admission, planner, backend, persisted artifacts and Engine assessment form one path. Separate scoped owner, executor and cleanup capabilities are enforced. | A producer ID or green process exit is not authority. Actual integration is proved by `tests/source_flow.rs`, not a mock alone. |
| §4: canonical patch contract | `source_patch.rs` and its tests bind exact scope/transaction/base/analysis, canonical operations and preimages; support binary create/replace/delete, simultaneous rename and modes; reject malformed/unsafe paths and link graphs. | Portable ASCII paths only; symlinks require an explicit policy. Unsupported forms reject rather than lose source bytes. |
| §5: snapshot-derived materialization | `materialize.rs` independently replays the patch, retains layered provenance, copies exact candidate bytes with no-follow handles and verifies the census. `workspaces.rs` publishes scoped ownership before copying. | No developer live-directory execution. Capture uncertainty survives; missing inputs and unsupported metadata cannot be silently omitted. |
| §6: admission | `source_attempts.rs` checks installed analysis/policy, exact impact, transaction state, grants, path/preimage scope, required checks, resources, lease, expiry and idempotency. The actual configured-harness test rejects incomplete structural analysis and direct bypass. | Manually established complete finite fixtures are permitted by §6. Their completeness is never substituted for the actual structural index. |
| §7.1–4: attempt states, plan and check execution | Typed source states remain separate from transaction states. Plans bind argv, cwd, exact inputs/environment/toolchain, required/optional checks, prerequisites, resources, artifacts and failure policy. The coordinator uses deterministic order and explicit skipped-check reasons. | Source is read-only; scratch is separate. A check result cannot change the candidate used by another check. Unsupported output handoff rejects. |
| §7.5: language harnesses | Actual T1 integration runs pinned Go 1.26.2 `gofmt -e`; measurements use `-l`. Check identity, resulting snapshot and environment are bound into evidence. | This proves syntax processing only. The objective calls the wider language table a candidate target matrix; it does not establish TS/JS/Python/Rust execution or Go compile/test support. |
| §8: isolation | Actual rootless-runc probes verify the pinned Linux T1 profile, effective controls before release, resource limits, denied host paths/environment/network, termination and cleanup. Unsupported tiers/grants have no host fallback. | One validated isolated backend is the §14.4 milestone. T1 shares the host kernel; host administrator/kernel compromise is outside its stated boundary. |
| §9: immutable evidence and Engine admission | Evidence v2 binds admission, source/result, patch, plan, checks, outcomes, artifacts and explicit executor-attestation strength. The Engine retrieves/hashes canonical bytes and rechecks authority after I/O before retaining its assessment. | Local scoped-executor authentication is not hardware/remote attestation. Missing, stale, partial, forged, truncated or unpersisted required evidence cannot pass. |
| §10: caching | Full plan/check/context key identity has omission-mutation tests. Reuse is disabled. | There is no reusable-hit or remote-cache claim; a cache cannot bypass current evidence policy. |
| §11: retry and recovery | Durable owned launch claims prevent blind reruns. Cleanup-only recovery cannot restore lost Engine authority. Failure/retry lineage, cancellation/expiry, duplicate delivery, lost receipts, storage faults and quarantine have tests. | Engine transaction authority remains in memory, as §11 permits for a labelled development implementation. Runtime journals/local artifacts have separately tested persistence. |
| §12: adversarial matrix | Patch/path/link/preimage, authority/lease/model/policy, check/evidence substitution, resource/host/network denial, actual controller loss, source integrity, cleanup and retry scenarios have executable coverage. | Positive tests for unavailable network/handoff/cache/publishing capabilities are not claimed; see the conditional cases below. Numeric OCC tests are not represented as a source-publishing race. |
| §13: measurements | Two actual-repository T0 corpora, three generated T1 cases, raw resource/timing samples, key construction, Busy backpressure and owned cleanup/recovery are recorded with source/tool/environment bindings. | Warm caches, filesystem differences, sampled memory and three repetitions are explicit. No queued-latency, reusable-cache-hit or production SLO claim. |
| §14–15: milestones and definition of done | The [acceptance record](goal-03-acceptance.md) maps all sixteen items and lists the separate human-attributed implementation milestones. The live source-flow proves patch → isolated check → persisted evidence → Engine admission. | Passing evidence moves to Validating, leaves the authoritative snapshot/revision unchanged and still receives `SourceMutationUnsupported` at commit. |
| §16–17: non-goals and completion report | Acceptance, execution instructions, tool provenance, benchmarks and validation records identify exact sources, commands, platform/harness, threat boundary, failures and limits. | Distributed authority, source publication, orchestration, broad language support and public production readiness are not inferred. |

## Conditional capabilities are not positive validation

These distinctions reconcile detailed requirements with the selected backend;
they do not waive the requirements when those capabilities are introduced:

- **Network allowlists and credentials:** the current runner supports DenyAll
  without credential delivery. Non-deny grants and protected references reject.
  There is no successful allowed-destination test or implemented network
  allowlist/credential-delivery feature. Supporting one later requires the
  destination/port/expiry/audit and secret boundaries required by §8 and §12.
- **Generated-output handoff:** captured immutable generated inputs participate
  in source/check identity. Sharing newly produced writable outputs between
  checks is unsupported; requested custom output/handoff rejects. A future
  implementation must provide the explicit content-addressed dependency handoff
  required by §7.4 and §9.
- **Source-head advancement:** source publication is deliberately unavailable
  under §14.2 and §16. Current tests prove stale/forged bindings, policy/index
  invalidation and that verification cannot advance source. They do not execute
  a concurrent source-publishing transaction. An adapter that adds publication
  must validate that actual race before permitting it.
- **Stronger tiers, cache reuse and other language checks:** unsupported requests
  must reject. Their positive isolation, reuse or harness tests remain required
  before support can be claimed; current mocks/parsers do not establish them.

## Documentation consistency

`Dev.md` marks the bounded Goal 03 phase complete and keeps durable distributed
authority in Goal 04. The [support matrix](../reference/capability-support-matrix.md),
[execution component](goal-03-execution.md), [threat model](../spec/threat-model.md)
and [Linux runbook](../operations/local-linux-t1-runbook.md) agree on Linux T1,
the actual Go check, local evidence and the same unsupported features.

The [progress record](goal-03-progress.md) is chronological. Earlier statements
that the exit gate was open are historical; the final 03.6b entry records its
closure. Older evidence-v1 records retain their original identity and do not
satisfy the current v2 gate. The measured runtime remains `fa888ed`; subsequent
LSP-test, reporting and documentation changes do not rewrite those measurements.

## Fresh focused validation

A clean isolated checkout at the tested source above passed:

- **78 Engine tests** across source patches, admission/attempts, verification
  plans and independent evidence admission.
- **1 actual structural-harness rejection test**, including attempted admission
  after incomplete analysis.
- **1 live OCI source-flow test** covering wrong-base rejection, interrupted
  first attempt, owned cleanup, fresh retry, real configured check, persisted
  evidence/duplicate receipt, immutable failed history and no source publication.

All 80 tests passed with zero failures or ignored tests in these commands.
[Exact commands and source/tool/log bindings](goal-03-contract-review/checks.json)
and [raw output](goal-03-contract-review/checks.log) are retained. The checkout was
clean before and after these checks; build/dependency caches were reused.

The broader [cross-goal audit](goals-01-03-audit.md) separately passed 339 workspace
tests, 28 LSP groups and ten actual OCI integration groups. Those counts overlap
this focused run. The tested commit's [hosted CI](https://github.com/Emiloart/Celetixo/actions/runs/43091567436)
passed; hosted CI does not run the opt-in physical-isolation groups. This review
does not rerun every OCI probe or the three-repetition performance collection.
No Rust implementation, test, authorization or evidence requirement was changed
to obtain this result.
