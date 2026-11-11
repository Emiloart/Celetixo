# Goal 03 acceptance and completion record

This record audits the [accepted Goal 03](goal-03.md), retrieved with `Dev.md`
from `2c42f67d3e9492c2dcd2cc49ab6a4870f608dec8`. It describes the implemented
reference Engine and its validated local execution profile. It does not expand
those results into production, distributed-authority or language-wide claims.

The [2026-11-11 contract comparison](goal-03-contract-review.md) rechecks the
full objective against `Dev.md`, the implementation, support matrix, runbook and
measurements. Its clean-checkout validation at `9538e10` passed 78 focused Engine
tests, actual incomplete-analysis rejection and the live patch/recovery/retry
flow. The [cross-goal audit](goals-01-03-audit.md) records the broader regression
run and benchmark-reporting correction. Neither review changes the measured
runtime source below or promotes unsupported capabilities.

## Exact source and clean-checkout evidence

The measured runtime and runner source is
[`fa888ed49cda2f151f64491986aa8ea9d7afbb36`](https://github.com/Emiloart/Celetixo/commit/fa888ed49cda2f151f64491986aa8ea9d7afbb36),
tree `95394b1cb232e17d537490e05e6c265dc56ff5aa`. Its exact archived source was
built and checked from a new directory, with source hashes compared before and
after testing. Compiler build caches were reused; a cold build is not claimed.
The same source passed 339 workspace tests and ten explicit actual integration
groups. [Source-bound validation](goal-03-evidence-v2-validation/checks.json) and
[raw output](goal-03-evidence-v2-validation/checks.log) preserve those checks.

Full archived-tree checks also passed real rust-analyzer, example builds,
transaction assertions, repository capture/index/Engine self-inspection and the
repository benchmark smoke. An interrupted inspection process was confirmed
absent before resuming only the remaining checks. Debug and release inspectors
subsequently produced identical complete JSON results for that selected tree.

The hosted CI run for that commit passed formatting, linting and all workspace
tests, then exposed a startup retry-window defect in the real language-server
test. The isolated validation-harness correction is
[`8e64ee43a46b1969d32b40970fe67b13c3d5b831`](https://github.com/Emiloart/Celetixo/commit/8e64ee43a46b1969d32b40970fe67b13c3d5b831).
It uses elapsed monotonic loading windows and retains all definition, reference,
edge and incomplete-coverage assertions. All 28 local LSP groups passed. It does
not change the runtime or measurement runners. Benchmark publication verifies
their source blobs remain identical to the measured commit.
The corrected commit's [full hosted CI run](https://github.com/Emiloart/Celetixo/actions/runs/42608513914)
passed, including the actual language server, full self-inspection and whole-tree
T0 benchmark smoke.

The repository's [CI workflow](../../.github/workflows/ci.yml) checks fresh
checkouts, all workspace tests, real rust-analyzer, examples, transaction
assertions, full repository inspection and a whole-tree T0 measurement smoke.
Actual OCI probes remain explicit opt-in checks requiring reviewed tooling and
delegation; a hosted CI build is not evidence of physical containment.

Retrieve the exact documentation/data revision with `git rev-parse HEAD`; the
measured executable revision and every selected input remain pinned separately
in the [run manifest](../benchmarks/goal-03/run.json).

## Implemented mutation and snapshot contract

`celetixo/source-patch/v1` is a bounded canonical binary in-process domain format,
not a competing RPC schema. It binds exact scope, principal attribution,
transaction/attempt, snapshot/revision, analysis/index/impact, producer and
validity interval. Authorization is checked separately by the Engine.

Create, exact-preimage replacement/deletion, simultaneous rename and regular/
executable mode changes preserve exact bytes. Binary payload size/digest,
conflicting/ineffective operations, occupied destinations, path traversal,
case-fold aliases, inferred-directory collisions and reserved control paths
reject. The portable profile rejects non-ASCII paths explicitly. Symlinks are
denied by default; the permitted internal-relative profile validates the final
graph. Opaque Gitlinks and unsupported captured Unix-directory metadata reject
execution materialization rather than being omitted.

Application replays independently against the immutable original, creates an
immutable candidate at the same authoritative revision and retains the full
layered capture, exclusions, environment and application provenance. Recognized
Git administration is non-operational provenance, not an execution Git folder.
Pure application supports the layered snapshot contract; execution admits only
capture/analysis completeness established by its installed policy. Live capture
uncertainty, missing inputs and structural semantic gaps do not become safe
through a small patch or a configured check.

Private copying uses directory-relative no-follow operations and exact content,
mode, link and directory census. Actual execution requires durable scoped
registry ownership before copying, explicit quotas, live-owner protection and an
independent runtime-retention barrier. Cleanup rechecks exact identity and current
scoped authority. Malformed or damaged published ownership quarantines for
operator review; a path or an empty owner lock does not authorize arbitrary
deletion. Source snapshots remain unchanged after success and failure.

## Engine authority and evidence

Rust owns policy installation, exact impact/admission, reservations/leases,
source-attempt states, cancellation/expiry, separate executor/cleanup grants,
verification plans, evidence admissibility and transaction decisions. Workers
cannot remove mandatory checks, expand grants, create admissible green flags or
promote incomplete analysis. Explicit/derived accesses and invariant closure
retain provenance. Source publication remains `SourceMutationUnsupported`,
including after a passing evidence receipt.

Plans bind command/argv, working directory, candidate/dependency/configuration
inputs, executable/runtime/toolchain, environment, tier/backend, network and
credential policy, resources, artifact requirements, prerequisite order and
failure policy. Extra optional checks remain policy-owned and identity-bound.
Cache keys include the complete context; reuse is disabled.

Evidence envelope v2 explicitly binds `EngineScopedExecutor` attestation strength
alongside exact admission, check records and immutable artifact references. This
means current Engine-scoped executor authentication, not hardware, remote-worker
or microVM attestation. Metadata constructors cannot authenticate a caller.
Unattested claims and legacy envelope media reject. Historical v1 bytes remain
immutable artifacts and cannot be silently upgraded. Check/journal/artifact
formats retain their independent versions.

The Engine independently reads and hashes the same bytes it canonically compares,
checks every planned result/provenance/artifact, revalidates authority after I/O
and atomically retains its assessment. Missing/truncated/stale/forged/unretrievable
objects, unknown termination, incomplete cleanup or skipped required checks
cannot authorize success. Exact duplicate delivery acknowledges the historical
receipt; changed input or retry has separate immutable lineage. Passing source
evidence moves the transaction to Validating and advances no source revision.

Local artifact storage synchronizes data and directory publication before
returning references. This is confirmed local-filesystem persistence, not
distributed object storage or a retention daemon. The Engine authority store and
clock remain explicitly in-process reference implementations. Process loss does
not restore transaction authority; separately granted cleanup may reconcile owned
resources without executing or inventing evidence.

## Actual backend, language and threat boundary

Validated T1 is Linux x86_64 rootless runc 1.5.2 on the recorded WSL2 kernel,
delegated cgroup v2 and systemd user manager, with individually pinned formatter,
entrypoint, loader/libc and control-plane reader. The
[execution instructions](goal-03-execution.md) and
[tool provenance](goal-03-execution-validation/toolchain.json) define the exact
closure and reproduction prerequisites. Required properties fail closed; there
is no host-command fallback. T2/T3 are unavailable.

Actual probes verify private namespaces, read-only root/source, explicit bounded
scratch, empty capabilities/no-new-privileges, syscall/device restrictions,
denied network and host paths/environment/credentials, CPU/memory/swap/process/
FD/disk/output/wall limits, whole-process termination, independent controller-loss
deadline and owned cleanup. A shared host kernel, host administrator or adversary
already holding the embedding process's UID is outside this boundary. This is
not a microVM or a public multi-tenant security proof.

The validated language check is Go 1.26.2 gofmt parsing/rendering controlled source
with exact argv. `-e` is used by the integration fixture; `-l` by the benchmark.
Exit zero is syntax evidence, not format conformance, Go compilation/type checks,
`go vet`/tests or complete Go semantics. TypeScript/JavaScript, Python and Rust
execution harnesses are not claimed from Goal 02 parsing/LSP support. Custom
output handoff, credential grants, permitted-network execution and stronger tiers
fail closed until implemented and validated.

The actual structural indexes in integration fixtures remain incomplete. Positive
tests use a separate, manually established finite reference dependency model
for their exact controlled source/check inputs, as permitted by the objective.
Real configured structural-analysis admission tests prove the incomplete model
is rejected, including attempts to bypass failed analysis.

## Definition-of-done audit

| Requirement | Authoritative implementation and verification |
|---|---|
| 1. Canonical scope/transaction/base patches | `source_patch`; canonical decode/order/binding tests in [source_patches](../../crates/engine/tests/source_patches.rs). |
| 2. Deterministic non-mutating exact application | Exact preimages, binary/rename cycles, replay validation; [materialization](../../crates/execution/tests/materialize.rs); actual source-flow original-state equality. |
| 3. Unsafe/malformed/conflicting input rejection | Path/link/final-graph/bounded-decode cases; source-root substitution and no-follow materialization tests. |
| 4. Immutable result and full provenance | Application report/replay, layered-capture preservation and canonical-provenance substitution tests. |
| 5. Authoritative admission | [source_attempts](../../crates/engine/tests/source_attempts.rs): scope, owner/executor, state, policy, impact, reservation/lease, expiry and idempotency. |
| 6. Unknown impact cannot authorize | [Actual configured-harness tests](../../crates/repository/tests/harness.rs) reject incomplete policy/model admission and direct bypass attempts. |
| 7. Policy-owned deterministic checks | [source_execution_plans](../../crates/engine/tests/source_execution_plans.rs), harness configuration, mandatory/optional selection and input-digest tests. |
| 8. Actual isolation; no fallback | [Nine actual OCI groups](../../crates/execution/tests/oci.rs), pinned runtime/profile controls and unsupported-tier/grant rejection. |
| 9. Exact candidate and environment | Plan input validation, read-only source/census, exact toolchain/environment bindings and actual isolation artifacts. |
| 10. Complete immutable evidence | [Evidence tests](../../crates/engine/tests/source_evidence.rs), v2 attestation binding, registered-reader and persisted-object checks. |
| 11. Independent Engine rejection | Altered envelope/check/provenance, byte replacement, stale time/authority, missing results/artifacts and unknown-termination matrices. |
| 12. Failure/retry/recovery behavior | Store initialization/publication faults, SIGKILL ownership/copy/claim/controller tests, timeout/cancellation, source quarantine and duplicate delivery. |
| 13. Actual end-to-end path | [source_flow](../../crates/execution/tests/source_flow.rs): mismatched-base denial, interrupted first attempt, owned recovery, fresh successful retry, immutable failed history and no source publication. |
| 14. Reproducible raw measurements/targets | [Baseline](../benchmarks/goal-03.md), pinned raw run manifest, all samples, byte-identical summary reproduction and observed per-case targets. |
| 15. Actual coverage and limits | This report, execution contract/provenance, benchmark protocol and implementation history distinguish implemented/validated/unsupported behavior. |
| 16. Human-only published milestones | [CONTRIBUTING](../../CONTRIBUTING.md), local/CI attribution checks and the separate milestone commits below. |

The adversarial matrix includes wrong base/preimage/scope/generation, altered
patch/check/policy/toolchain/environment, authority changes during I/O, invariant
closure, source mutation, skipped prerequisites, partial/truncated artifacts,
lost responses, stale attempts, controller/process loss, deadline cleanup and
fresh retry. Source repository advancement/commit is deliberately unavailable;
Goal 01's numeric reference OCC/serialization tests remain intact.

## Commands, results and measured scope

Run the documented pinned toolchain and environment-specific C compiler setup:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --locked -p celetixo-repository --test lsp -- --include-ignored
cargo build --workspace --examples --locked
cargo run --locked -p celetixo-engine --example concurrent_transactions
CELETIXO_OCI_HOME=/reviewed/private/tooling cargo test --locked -p celetixo-execution --test oci --test source_flow -- --ignored --test-threads=1
```

The measured/runtime source passed 339 workspace tests, formatting, strict
all-target linting, examples, assertions, full self-inspection and benchmark
smoke; actual OCI/retry groups passed 10/10. The loading-window correction passed
28/28 LSP groups locally, including actual rust-analyzer. Source patches cover
the declared consistency model, not universal semantic extraction or program
correctness. Unsupported platforms/backends/toolchains are not validated by mocks.

The [baseline](../benchmarks/goal-03.md) records three repetitions for 14 actual
repository T0 cases, three generated T1 cases and capacity/recovery probes. It
includes patch/replay/hash/copy/cleanup, backend/check resource observations,
evidence publication/read/Engine admission, disabled-cache keys and Busy/orphan
recovery. Filesystem placements differ and no caches were evicted. Provisional
tracking ceilings are observed per-case maxima, not production SLOs. Raw data,
source/tool/binary hashes, hardware/kernel/compiler, exact commands and summary
derivation are preserved; safety cannot be offset by speed.

## Milestone lineage and deferred boundaries

| Milestone | Published source |
|---|---|
| 03.1 canonical source patches | `e669a621ada936a886572eb53a19c1a50c486c7c` |
| 03.2 authoritative source attempts | `c23d940c4a31d2d86930ff779ecb79d32be33a42` |
| 03.3 configured verification plans | `3b78f3ecb3e85989ddc376083f20358285e67f8e` |
| 03.5 independent evidence gate | `4be2abfefcac82e97a0ee2753941943d98a86dfb` |
| 03.4a concrete execution/storage | `c7edb05ed7b4070418bde5513d06efa78c1053c4` |
| 03.4b neutral facts/copy crash recovery | `4c6d7baf16c26a98ca293a885ba56eb892504547` |
| 03.6a measurement runners/actual retry | `80dafd0a12b7a4c44b083b7bdf88a92cceed8fb4` |
| Explicit evidence v2 attestation | `fa888ed49cda2f151f64491986aa8ea9d7afbb36` |
| Hosted LSP loading-window correction | `8e64ee43a46b1969d32b40970fe67b13c3d5b831` |
| 03.6b measured baseline and acceptance publication | `456bb375108532d9503c82fa8f6e7acfdfedddaf` |
| Cross-goal regression audit and benchmark-reporting hardening | `9538e107a688e21aa0730a23a6cf00b3719ce73d` |

Rust retains all transaction and commit authority. This milestone does not
provide source revision publication, durable/distributed PostgreSQL authority,
Temporal/NATS orchestration, a remote cache/worker pool, T2 microVMs, automatic
dependency installation, broader language harnesses, public production readiness
or a training/UI substitute. These remain accepted architectural targets governed
by their own implementation and verification gates.
