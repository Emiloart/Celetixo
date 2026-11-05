# Goal 02 validation record

Goal 02 is implemented and validated within the explicit subsystem boundaries below. The final clean-checkout audit, exact-tree Engine inspection and measured performance records are complete. This does not claim complete language semantics, source mutation or a deployed distributed service.

## Milestone 1 — authoritative Engine contracts

Commit `53d50ebed98476878a48503a9d9d159d56974c14` contains the immutable artifact, scoped registration/query/CAS installation and source/reference separation. The coordinator reran all 75 Engine tests, strict package Clippy across all targets, owned-file formatting and diff whitespace checks before committing. The 13 added repository tests cover exact snapshot/index/configuration/content binding, unknown/dangling identities, transitive checks, capability scopes/revocation, incomplete analysis rejection, concurrent replacement CAS, predicate membership and integer mutation bypasses. The original 62 tests, including generated histories and atomic failure/recovery schedules, still pass.

The milestone was pushed under the repository owner's established human identity. [GitHub CI](https://github.com/Emiloart/Celetixo/actions/runs/42047298783) completed successfully for that exact commit.

Commands on the WSL/Zig development environment use task-local `CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC'`. No global compiler or authorization policy was changed.

## Published core and attribution milestones

`036d8b10f1766e9978799fc1f776ee8c1368aa70` strengthens the upstream human-attribution policy with author/committer checks and 25 independently rerun temporary-repository regression cases. It preserves the owner's remote policy documents. Local hooks and repository-local identity are configured; CI checks raw metadata and co-author declarations. Recognition cannot authenticate a person or replace required branch protection.

`fdc0bed50d5b2eddf331d9d6c681f4a86dd9d65b` publishes the capture/index/LSP/impact core, fixtures, inspection and measurement harnesses, developer instructions and CI integration after the 177-test workspace verification below. [Core CI](https://github.com/Emiloart/Celetixo/actions/runs/42050087402) and [attribution milestone CI](https://github.com/Emiloart/Celetixo/actions/runs/42049435137) both passed. Benchmark results and the final performance/clean-checkout audit were published in subsequent milestones. Each published milestone was checked for human attribution and pushed; no unverified performance claim is implied by the core commit.

## Scope-to-evidence audit

| Requirement | Implementation and executable evidence |
| --- | --- |
| Immutable real capture; clean/dirty, staged/unstaged/untracked, rename/delete/overlay identity | `capture.rs`; `tests/capture.rs` compares exact contents/layers and deterministic manifest identities |
| Race, missing input, policy and incomplete capture handling | Two observations, no-follow reads, bounded Git/object capture and report validation; between-pass mutation, missing/submodule/exclusion and tampering regressions |
| Modes, symlinks, Git/worktree configuration and external/generated inputs | Captured canonical modes/metadata, explicit generated/mounted inputs and Git metadata; capture/index configuration tests |
| Credential input admission | Versioned pre-publication path/content/environment policy, exact reviewed public exceptions and rejection/recovery tests; no universal secret-detection claim |
| Initial structural language adapters | Pinned TS/TSX, JS/JSX, Python, Go, Rust grammars; multi-file calls/imports/scopes/types/packages and unresolved/syntax/unsupported regressions |
| Versioned indexes; stable identities and continuity | Scope/snapshot/config/parser/analyzer generation, occurrences and edge provenance; comments/line shifts, rename/copy ambiguity, stale/corrupt structure tests |
| Incremental/rebuild equivalence and failure recovery | Unchanged-tree reuse and atomic cache publication; update/rebuild equality, deletions and budget-failure/recovery tests; benchmark asserts equivalence before recording samples |
| Semantic services | `lsp.rs`, versioned shared connections/documents, sealed response provenance and bounded transports; protocol fixtures plus actual rust-analyzer trusted-fixture probe |
| Direct/transitive/cyclic/configuration/generated impact | `impact.rs`; deterministic manually labeled graph tests with direct/derived/predicted separation and justified verification suggestions |
| Transaction assumptions and stale results | Full request digest, read/write/predicate/invariant reconsideration; stale snapshot/index/result/unknown element rejection and Engine artifact CAS tests |
| Quality and performance measurements | Independent manual reference, automated `benchmark_quality.rs`, reproducible release harness, raw phase/RSS/query samples and measured targets in `docs/benchmarks` |
| Engine authority and incomplete analysis | Scoped trusted installation only, no legacy source registration/model replacement, fail-closed analysis admission and source integer patch/revalidation/commit guards |
| Developer workflow and separate verified milestones | Goal 02 contract, CLI inspection, benchmark instructions, CI and milestone commits |

## Core integration evidence

After integrating the remote human-attribution policy without dropping any source changes, the coordinator ran:

```sh
cargo fmt --all -- --check
CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC' cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC' cargo test --workspace --locked --offline -- --include-ignored
CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC' cargo build --workspace --examples --locked --offline
CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC' cargo run --locked --offline -p celetixo-engine --example concurrent_transactions
git diff --check
```

All passed. The test total is **177**: 75 Engine, 38 capture, 26 index, 28 LSP (including actual rust-analyzer), 9 impact and 1 independently labeled quality regression. The transaction example retained compatible final state `x=10, y=20` at revision 2 and rejected the invalidated-read scenario with final `x=10, y=2` at revision 1.

The trusted actual-Rust probe builds a real snapshot/index from the materialized fixture, applies definition/reference responses and validates analyzer-proven caller relationships against that snapshot. Actual TS/JS, Python and Go server processes remain unrun; their protocol fixtures do not establish full language support.

## Baseline measurement evidence

Four release/native-O3 runs completed sequentially: generated 1/5/20-copy mixed-language repositories and the immutable actual Engine milestone `53d50ebed98476878a48503a9d9d159d56974c14`. Each has three iterations and 100 impact queries per iteration. The coordinator independently parsed all reports, checked the summary invariants and query counts, and reconstructed the compiled implementation fingerprint from the committed `fdc0bed` source and lockfile: `b4ddcec88f989e78f24f132663d25a88fb69fbf0e8afa4982f9bb78ebe2ffc70`. All four reports match. Each labeled iteration records 15 true positives, zero false positives and one false negative; actual Celetixo has no quality oracle.

The actual corpus contains 42 files and 462,750 effective bytes. Its baseline median cold/incremental indexing times were 3,591.137/3,651.163 ms. These observations justify investigating repeated whole-index reference scans; they are not final optimized targets or evidence of full semantic coverage. Raw reports and methodology are in [the benchmark record](../benchmarks/goal-02.md).

The accepted baseline reports were committed and pushed as `3187eaa468fbc421a95cd6827d92d650c9c3effd` under the owner's identity; [its CI passed](https://github.com/Emiloart/Celetixo/actions/runs/42053727929). The summary and comparison scripts reject failed equivalence and altered generations even with Python optimization enabled; their checks do not depend on Python assertions.

## Reference-lookup optimization evidence

Per-file ordered scopes, per-scope/name declarations, top-level declarations and Go package membership replace repeated global declaration scans. Lookups are rebuilt for every generation, and resolution rules and provenance remain unchanged. A retained test-only scan resolver independently checks exact ordered candidates and full index artifacts across 35 language/state transitions, plus shadowing, ambiguity, syntax errors, unsupported/generated inputs and package boundaries.

The coordinator reviewed the complete source diff and independently reran workspace formatting plus `cargo test --locked --offline -p celetixo-repository --lib --test index --test benchmark_quality` with the documented local compiler settings. All **29** focused tests passed: two scan oracles, 26 index regressions and the manual quality case. The implementation worker also passed strict focused Clippy. The verified source milestone was committed and pushed as `0e608cbe72254d7822e3389dad6c35788d0a6ab0`; [its CI passed](https://github.com/Emiloart/Celetixo/actions/runs/42054126264).

All four optimized measurement runs then completed sequentially with the same corpus and build settings. The coordinator independently ran `python3 -O docs/benchmarks/goal-02-compare.py docs/benchmarks`; exact recorded snapshots, index generations, graph/quality fields and environment matched the baseline. The implementation fingerprint reconstructed from `0e608cb` is `fd0a39e63eec257cc1ef16ae73b2f3cef7556fbbedac4deebbd7a43b2a100f8d`, matching every optimized report. Actual-source median cold/incremental indexing was 1,009.132/1,011.453 ms; the one-copy medians increased to 10.690/8.453 ms. Both outcomes are retained without claiming statistical significance or a universal improvement. These artifact-neutral reports predate the separately versioned identity correction.

The artifact-neutral measurements were published separately as `737d788fa8e8e326fd9389b0e7eabd06292f04f4`.

## Non-recycling identity correction

The final identity audit found that restoring exactly the same bytes after a later accepted deletion could reuse the previous content/shape-derived IDs. Version 2 binds every source and package identity to security scope, authoritative revision and Git base. Index/configuration encodings and the actual structural extractor version advance to v2. The full index digest includes the retained identity namespace; snapshot validation recomputes it. Signatures/content digests remain shape evidence, and exact historical-snapshot rebuilds remain reproducible. This conservative allocation does not claim a durable file-lineage registry or authorize heuristic rename continuity.

An independent reviewer traced every production element constructor, including manifest dependencies and synthetic Go packages. Seven new behavioral regressions cover all five initial languages, exact-byte file and symbol recreation, later revision with unchanged Git base, new Git base at the same revision, original-snapshot revisits, comment/line stability within one namespace, configuration/package nodes, namespace tampering and scope isolation. Every transition compares incremental and clean results. The existing profile-invalidation assertion now uses an explicit custom profile distinct from the new default.

The coordinator ran workspace formatting, strict all-target workspace Clippy, and `cargo test --locked --offline -p celetixo-repository --lib --test index --test identity --test benchmark_quality` with the documented local compiler settings. All **36** focused tests passed. No authorization, evidence, transaction or completeness check was removed. The source correction was committed and pushed as `72621aa786ca430ccf9f267644a6554d7bf77ea6`; [its CI passed](https://github.com/Emiloart/Celetixo/actions/runs/42056011228). [The separate comparison-record milestone CI](https://github.com/Emiloart/Celetixo/actions/runs/42055850628) also passed.

## Final v2 performance evidence

All four final release/native-O3 workloads completed sequentially against source `72621aa`. The coordinator independently validated every report with the summary/target script, reconstructed implementation fingerprint `8df921f0cc94df6aa778fbd0b8c5a7a522c507780d36f5430d1f128a7ff76b4d` from committed source and lockfile, and checked it against all four records. Corpus snapshot IDs and graph cardinalities match earlier observations. Configuration, semantic IDs and index generations change for v2; all other labeled quality outcomes remain equal. Incremental/rebuild equivalence holds on every sample. The separately preserved v1 comparison still passes its strict artifact-equality verifier.

The actual-source medians are 3,934.212 ms capture, 1,013.859 ms cold index, 888.466 ms incremental index and 967.605 ms rebuild; pooled query p95 is 15.096 ms and observed lifetime process peak is 114.99 MiB. These apply to the specified 42-file, 462,750-byte immutable corpus, not arbitrary repositories. All raw samples, phase checkpoints, manually labeled quality counts and mechanically derived local investigation targets are in [the benchmark report](../benchmarks/goal-02.md). Broader language accuracy, deployment capacity and statistical significance remain unestablished.

The final measurement records and derived targets were committed and pushed as `0518053511b1481055eb5a615d64d7c4f000e7ce`.

## Final clean-checkout and Engine inspection audit

A separate detached worktree at `72621aa786ca430ccf9f267644a6554d7bf77ea6` was clean before and after verification. Existing Cargo dependency/build caches were reused; this was a clean source checkout, not a fresh machine or hermetic build. The coordinator reran formatting, strict all-target workspace Clippy, locked/offline workspace tests including ignored tests, workspace example builds, the transaction example, all 25 attribution regressions, full reachable-history attribution checks and whitespace checks. All passed. The later measurement milestone changes only documentation and raw JSON, preserving the tested Rust source and lockfile.

The final Rust total is **186**: 75 Engine, 38 capture, 26 index integration, two lookup-oracle unit tests, seven identity, 28 LSP including the actual rust-analyzer probe, nine impact and one manual quality regression. The real server reports `rust-analyzer 1.94.1 (e408947 2026-03-25)`. No check was skipped to obtain this count.

From that checkout, the coordinator also built the release inspection example with native `-O3` and inspected the exact immutable `72621aa` tree through the Engine. Capture completed with **112 files, 5,896 elements and 17,267 edges**. Snapshot `b5af92631d659e6e1686298245fa991fe6e3e289faf03c1de0086e66d4fd8e89` bound index generation `438ecf7e0171c99b4cc1b4a5f29b6f0404173a15cd6907e1439d75ee0217a59c` and Engine artifact `66ef1972c225ba3b37173d2d1060f3b638fe44f9f8af9d103fdc58cd7aab79e0`. The query included the changed Engine file, retained **9,903 coverage gaps**, reported incomplete impact in both analysis layers and kept source mutation disabled.

The [machine-readable audit](goal-02-audit.json) records the exact tested commit, commands, results, compiler settings and inspection bindings. This development record does not substitute for transaction-specific Engine verification evidence. Documentation links and both Python report scripts were checked; report rejection also passed under Python optimization. The earlier larger live-worktree debug attempt produced no accepted result and is not counted as successful validation.

Actual TypeScript/JavaScript, Python and Go language-server processes, native Windows capture, arbitrary-repository accuracy and production deployment performance remain unvalidated. The initial structural adapters and supported version-bound semantic queries are implemented; full language engineering harnesses are not claimed. See [implementation boundaries](goal-02.md) for live-source atomicity, durable storage, historical file lineage and source-publishing limits.
