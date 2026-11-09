# Goal 03 measurement protocol

The executable runners measure the implemented reference Engine and local Linux
execution component. Publish measured data only after committing the runner
source. Record that source commit, the selected input commits, exact commands,
build flags, compiler, OS/kernel, CPU and tool-closure hashes beside the raw JSON.
The benchmark does not download tools or install repository dependencies.

## Actual repository T0 cases

`source_patch_bench ROOT FULL_GIT_OID --iterations 3` captures the selected
immutable Git tree using the default capture/credential policy. It measures
replacement, deletion, rename and mode changes on an existing regular input,
plus binary creation at 4 KiB, 64 KiB and 1 MiB. It validates canonical patch
decoding, exact application, independent replay, candidate manifest hashing,
private no-follow copying and cleanup. The original snapshot must remain equal.

Run it on Celetixo and on the separately provisioned public Go repository
[`cespare/xxhash` at v2.3.0](https://github.com/cespare/xxhash/tree/v2.3.0), exact
commit `998dce232f17418a7a5721ecf87ca714025a3243`. The latter has 24 tracked
files, 12 Go files and 36,958 content bytes. Preserve its source/licence and
provisioning hashes. Git source acquisition is an operator action; these
programs do not execute that project's source, hooks, tests or build scripts.

These pure T0 bindings deliberately carry no Engine admission or complete-impact
claim. Actual inspection of the Go input reported incomplete impact, including
unsupported assembly and unresolved relationships. Retain that result; selecting
a real input for a benchmark does not authorize it for source execution.

Materialization includes its own patch replay and source census. Independent
timings overlap, so do not add them to infer an end-to-end duration. The reported
copy throughput includes these validation costs. Content reuse is zero. The T0
runner uses standalone disposable copies; the actual executor instead requires
the scoped persistent workspace registry.

## Generated finite-reference T1 cases

`source_execution_bench --iterations 3` creates deterministic Git source trees
with fixed fixture commit dates and repository JSON selecting the exact approved
check. The cases contain a 4 KiB Go input, a 64 KiB input, and a 64 KiB input with
128 additional public 256-byte assets. A `go.mod` input is retained. Source sizes
are controlled with comments around one explicitly enumerated function, rather
than pretending that byte count describes general language complexity.

The actual structural index is built and remains incomplete. Positive execution
uses a separate, explicitly finite reference dependency model for these known
inputs and permitted operation. The check parses source using pinned Go 1.26.2
`gofmt -l /src/main.go`. Exit zero establishes that syntax check; this is not a
format-conformance assertion, compiler/type check or language-wide Go harness.

The real rootless OCI backend runs the check, confirms cleanup, persists the
complete evidence, and submits it to the Engine. Each successful sample asserts
Engine admission, byte-identical artifact retrieval and the historical receipt.
Record application, replay/hash, materialization, backend preparation/startup,
command supervision, runtime/source cleanup, sealing/publication, artifact read,
Engine evidence admission and cache-key construction. The raw isolation record
also preserves actual controls and resource observations. Guest peak figures are
the backend's observations, not a claim of exact attribution to every phase.

A separate capacity probe overlaps two admitted attempts while one holds the
backend's actual single execution slot. The contender must receive `Busy`, with
no queue or host fallback. A controlled controller interruption after durable
claim publication, before process launch, leaves owned resources for measured
cleanup-only recovery. Repeated recovery must be idempotent and cannot produce
a check result. Actual SIGKILL/process-tree recovery is proved by the separate
live integration suite; this timing probe does not substitute for it.

Cache reuse remains disabled. Measure complete key construction and its disabled
result; there is no reusable-hit or distributed cache latency claim. Source
copies, evidence and failed attempts are never reused to skip current validation.

## Reproduction and interpretation

Use the reviewed tool directory and delegation prerequisites described in the
[execution instructions](../implementation/goal-03-execution.md). Build once:

```sh
cargo build --release --locked -p celetixo-execution --example source_patch_bench --example source_execution_bench
target/release/examples/source_patch_bench /path/to/celetixo FULL_GIT_OID --iterations 3 > celetixo-t0.json
target/release/examples/source_patch_bench /path/to/xxhash 998dce232f17418a7a5721ecf87ca714025a3243 --iterations 3 > xxhash-t0.json
CELETIXO_OCI_HOME=/reviewed/private/tooling target/release/examples/source_execution_bench --iterations 3 > generated-t1.json
```

This WSL environment builds with `CARGO_INCREMENTAL=0`,
`CRATE_CC_NO_DEFAULTS=1` and `CFLAGS='-target x86_64-linux-gnu -fPIC -O3'`.
Those flags are compiler configuration, not sandbox controls. Ordinary debug
checks use the same C flags without `-O3`.

Preserve every sample, including failures. Fresh process/case setup does not
prove cold caches; no cache eviction occurs. `/proc/self/status` records current
RSS and process-lifetime high water, not a per-operation allocation census.
Record environmental load and deviations before comparing runs.

Derive provisional tracking targets from the observed per-case distribution.
Keep the sample count, median and observed maximum with the proposed threshold;
do not extrapolate production throughput, queueing or universal SLOs from this
machine and corpus. A safety violation invalidates the run regardless of speed.

The runner milestone does not itself supply a measured baseline or complete
Goal 03. Raw measurements and the final acceptance audit are separate deliverables.
