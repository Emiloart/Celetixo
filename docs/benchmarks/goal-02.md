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
target/release/examples/repository_bench --repository . --commit 0dd142e2567adc74f6e96b73a50579a74b0e0c73 --change-path crates/engine/src/engine.rs --iterations 3 --queries 100 > /tmp/goal-02-actual-celetixo.json
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

Final baseline measurements are recorded after the capture/index integrity audit
settles. The checked-in JSON and the table derived by `goal-02-summary.py` are the
authoritative observations; no unmeasured throughput claim is made here.

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
