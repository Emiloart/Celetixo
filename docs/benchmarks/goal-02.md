# Goal 02 repository-intelligence measurements

These are exploratory subsystem measurements. They do not establish full language
support, complete semantic impact, production capacity, or a comparative win over
other coding-agent workflows. Correctness and explicit uncertainty remain gates
independent of latency.

## Reproduce

Run on Linux from this checkout with the pinned toolchain and Cargo.lock:

```sh
CELETIXO_BENCH_RUSTC_VERSION="$(rustc --version)" CELETIXO_BENCH_CC_VERSION="$(cc --version | head -1)" cargo build --release -p celetixo-repository --example repository_bench --locked
target/release/examples/repository_bench --copies 1 --iterations 3 --queries 100 > /tmp/goal-02-copies-1.json
target/release/examples/repository_bench --copies 5 --iterations 3 --queries 100 > /tmp/goal-02-copies-5.json
target/release/examples/repository_bench --copies 20 --iterations 3 --queries 100 > /tmp/goal-02-copies-20.json
target/release/examples/repository_bench --repository . --commit 53d50ebed98476878a48503a9d9d159d56974c14 --change-path crates/engine/src/engine.rs --iterations 3 --queries 100 > /tmp/goal-02-actual-celetixo.json
python3 docs/benchmarks/goal-02-summary.py /tmp/goal-02-copies-1.json /tmp/goal-02-copies-5.json /tmp/goal-02-copies-20.json /tmp/goal-02-actual-celetixo.json
cargo test -p celetixo-repository --test benchmark_quality --locked
```

The measured WSL machine uses a local Zig C compiler wrapper. Its native grammars
were built with `CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC -O3'`
prefixed to Cargo commands; this is a local compiler compatibility setting. The
measurements use optimized Rust release binaries and explicitly optimized native
grammars. The raw JSON embeds `CFLAGS`, `CRATE_CC_NO_DEFAULTS`, the selected
rustup toolchain and explicitly supplied Rust/native compiler-version strings at example
build time via `option_env!`; it does not infer compiled flags from the benchmark's
runtime environment. Normal GCC/Clang environments
use the standard command above. Fixture creation and Rust/grammar compilation
are outside phase timings.

Run scales sequentially, and avoid concurrent builds when comparing performance.
`--help` documents bounded copy/iteration/query counts. Generated mode writes only
a disposable temporary Git repository. Actual-repository mode reads a full,
explicit Git commit OID, verifies the captured objects and constructs the changed
snapshot through an in-memory source overlay. It does not edit the target tree,
run its source code, spawn language servers, or use the live workspace as if it
were the named commit. There are no credentials or network-dependent package
installs in this harness.

## Corpora and methodology

The [mixed-language fixture](../../crates/repository/tests/fixtures/benchmark/README.md)
contains TypeScript, JavaScript, Python, Go and Rust, each with a changed base,
direct caller, transitive test caller and unrelated source. Python adds a dynamic
attribute consumer. Replication measures many small files and repeated graph
shapes; it does not represent a large production application. Package/build
manifests also participate. Generated dirty mode includes different staged and
unstaged versions of one TypeScript source and an untracked configuration input.

The second corpus is actual Celetixo source at the pinned Engine milestone above,
including its larger Rust implementation and regression suites. Its hypothetical
edit appends a comment to `crates/engine/src/engine.rs` through a versioned overlay.
There is no manually adjudicated impact oracle for that corpus, so no precision
or recall is assigned to it.

Every iteration records exact snapshot and index-generation identities. A repeated
capture must compare equal, the changed snapshot must differ, and incremental
index output must compare exactly equal to a new Indexer rebuild before a sample
is accepted. One changed language file must be reparsed and all other unchanged
language syntax trees must be reused. Failed equivalence aborts the measurement;
there is no timing-only success path.

"Cold" means an empty Indexer cache in that iteration. It does not mean a flushed
operating-system page cache, a fresh machine or a new process. Tree-sitter and
allocator initialization can therefore affect the first iteration differently.
Incremental indexing currently reuses unchanged syntax trees while rebuilding
the extracted/resolved graph; the timing includes that work. A new rebuild uses
an empty syntax cache. Cached impact timings reuse a validated immutable
ImpactAnalyzer adjacency graph, and include request/result binding, transitive
closure, uncertainty, provenance and verification suggestions. Graph preparation
is timed separately in the raw records.

Memory is sampled from `/proc/self/status`: `VmRSS` and lifetime `VmHWM`, in KiB.
The [Linux kernel proc documentation](https://www.kernel.org/doc/html/v6.13/filesystems/proc.html)
defines those as resident memory and its high-water mark. These process samples
exclude child Git RSS and are not an allocator census or phase-exclusive peak.
The process retains baseline/changed snapshots, indexes and the analyzer during
the measurement; later iterations may reuse allocator capacity. The raw data
preserves checkpoints rather than subtracting incompatible lifetime peaks.

Raw JSON pins fixture/reference digests, compiled capture/index/impact/harness
source plus Cargo.lock, parser versions, analyzer configuration, Git version,
CPU, kernel, every phase duration and every query sample. A content-addressed
snapshot is still required; neither the source commit nor the implementation
fingerprint stands in for a dirty-workspace snapshot identity.

## Results and limits

The completed **baseline** below predates the lexical-lookup optimization and the
separate identity non-recycling correction. The artifact-neutral optimization
comparison is now complete below. Measurements after the identity correction and
final performance targets are still pending. Each table identifies the measured
implementation rather than implying that earlier measurements cover later code.

All four baseline reports identify compiled implementation fingerprint
`b4ddcec88f989e78f24f132663d25a88fb69fbf0e8afa4982f9bb78ebe2ffc70`.
The machine is x86_64 WSL2, kernel `6.18.54.1-microsoft-standard-WSL2`, Intel
Core i7-7600U. The raw data records Rust 1.94.1 and the Zig-wrapper Clang 20.1.2
build metadata with native `-O3`. Generated repositories use the WSL ext4
filesystem (`stat` reports `ext2/ext3`); the actual repository uses the Windows
mount (`v9fs`). Storage differs, so capture timings are not a controlled comparison
between those two storage types. Other developer builds/semantic servers were
held during these sequential runs; this was not a dedicated benchmark machine.

Each workload has three iterations and 100 full impact queries per iteration.
Times below are milliseconds, with no outlier removal. Capture, cold/incremental
index and rebuild columns are medians. Query percentiles pool all 300 queries for
that workload. Peak memory is the maximum sampled lifetime process high-water
mark, in MiB, with the limits described above.

| Baseline corpus | Files / bytes | Clean capture median [min, max] ms | Changed capture median ms | Git-tree capture median ms | Cold index median ms | Incremental median ms | Rebuild median ms | Query p50 / p95 ms | Process peak MiB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| [1 copy](goal-02-baseline-copies-1.json) | 30 / 2064 | 691.077 [582.905, 764.463] | 641.778 | 158.283 | 7.785 | 5.379 | 5.231 | 0.378 / 0.830 | 9.81 |
| [5 copies](goal-02-baseline-copies-5.json) | 122 / 7716 | 2306.939 [2274.874, 2595.304] | 2244.208 | 592.588 | 33.426 | 34.154 | 32.475 | 0.560 / 1.022 | 16.97 |
| [20 copies](goal-02-baseline-copies-20.json) | 467 / 28911 | 9808.025 [8212.139, 12060.328] | 10189.207 | 2711.145 | 246.639 | 253.935 | 326.993 | 2.026 / 3.986 | 43.07 |
| [Actual Celetixo](goal-02-baseline-actual-celetixo.json) | 42 / 462750 | 4335.238 [4291.335, 4658.421] | 4820.095 | 4335.238 | 3591.137 | 3651.163 | 3610.057 | 12.430 / 22.438 | 115.14 |

For actual Celetixo, clean capture and Git-tree capture columns describe the same
measured operation, and changed capture is the overlay-based Git-tree operation.
The exact corpus is commit `53d50ebed98476878a48503a9d9d159d56974c14`: 16 Rust
files, 2025 indexed elements and 6354 edges. Its impact query remains incomplete
with 3725 gaps. Its predicted affected-file count is descriptive, with no
adjudicated precision or recall. All iterations passed repeat-capture identity,
changed-snapshot identity, exact incremental/rebuild equality and one-file parse
reuse checks. Actual-source incremental indexing reused 15 unchanged syntax
trees; generated 20-copy indexing reused 460.

The independently authored reference has five unique scenarios, totaling 16
potentially affected files across those scenarios. This file-level unit includes
the five directly changed root files and eleven expected consumer files; pooled
recall below is not downstream-only recall. Every repetition at every
generated scale produced these counts; repetitions do not enlarge the independent
reference set.

| Reference scenario | True positives | False positives | False negatives | Precision | Recall |
|---|---:|---:|---:|---:|---:|
| TypeScript base change | 3 | 0 | 0 | 100% | 100% |
| JavaScript base change | 3 | 0 | 0 | 100% | 100% |
| Python base change, including dynamic consumer | 3 | 0 | 1 | 100% | 75% |
| Go base change | 3 | 0 | 0 | 100% | 100% |
| Rust base change | 3 | 0 | 0 | 100% | 100% |
| Aggregate over these five scenarios | 15 | 0 | 1 | 100% | 93.75% |

Python's missed `dynamic.py` remains part of the reference denominator. Every
quality result remains incomplete, including scenarios with 100% observed recall.
Those percentages describe this small oracle, not a language-wide recall claim.

## Artifact-neutral lookup comparison

The optimized implementation precomputes path/scope/name lookup rather than
repeatedly scanning all nodes. Its original-scan test oracle and full index tests
passed before measurements. The frozen optimized executable reports implementation
fingerprint `fd0a39e63eec257cc1ef16ae73b2f3cef7556fbbedac4deebbd7a43b2a100f8d`.
It used the same corpus, fixture/reference bytes, compiler flags, toolchain,
parser/analyzer versions and machine as the baseline. Source edits for the later
identity correction began only after this executable and its embedded source
fingerprint were frozen; other builds remained held during its timings.

The [comparison verifier](goal-02-compare.py), also run with `python3 -O`, rejected
altered artifact data in a controlled check. It accepted all four real pairs with
exactly equal clean/changed snapshot IDs, index-generation digests, configuration,
graph/issue counts and full quality observations. Each generation binds the full
validated index, so equality is stronger than comparing counts alone. Every
iteration again matched its independent clean rebuild.

| Corpus and optimized raw data | Baseline / optimized cold median ms | Baseline / optimized incremental median ms |
|---|---:|---:|
| [1 copy](goal-02-optimized-copies-1.json) | 7.785 / 10.690 | 5.379 / 8.453 |
| [5 copies](goal-02-optimized-copies-5.json) | 33.426 / 28.771 | 34.154 / 26.719 |
| [20 copies](goal-02-optimized-copies-20.json) | 246.639 / 131.641 | 253.935 / 130.475 |
| [Actual Celetixo](goal-02-optimized-actual-celetixo.json) | 3591.137 / 1009.132 | 3651.163 / 1011.453 |

These exploratory medians each contain three observations and do not establish
statistical significance or overall performance. The smallest workload was slower. Capture and query
measurements also vary despite their algorithms being unchanged; all timings and
outliers remain in the linked raw data. The lookup change preserves the same known
dynamic-reference miss, semantic uncertainty and validated outputs as the original
scan oracle.

Recheck the complete artifact comparison with:

```sh
python3 -O docs/benchmarks/goal-02-compare.py docs/benchmarks
```

Final records after the separate identity-namespace correction will use
`goal-02-*.json`; their deliberately changed semantic generation must not be
presented as artifact-equal to this baseline. `goal-02-summary.py --targets`
derives local investigation ceilings from observed maxima, rounded upward, after
those final measurements. No production SLO or timing gate is inferred.

Live generated-workspace capture explicitly retains `LiveSourceNotAtomic`, even
when its two-pass census is complete. Verified immutable Git-tree capture can be
complete for its specified tree/overlay inputs. Structural indexes and all
benchmark impact results remain semantically incomplete. No result in this
report can silently authorize a source commit.

The hand-labeled suite is five small file-level impact scenarios, with reference
labels stored independently of computed output. Its Python dynamic consumer is
included in the recall denominator even when extraction misses it. Broader
real-world recall, generated code, macros, reflection, dependency ecosystems and
language-server coverage require separate labeled corpora and measurements. This
published regression fixture is not an unseen confirmatory evaluation set.
