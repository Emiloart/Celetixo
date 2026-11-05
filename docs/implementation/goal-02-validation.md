# Goal 02 validation record

Goal 02 remains active until the remaining performance measurements and final clean-checkout audit are recorded. This record distinguishes completed checks from pending work.

## Milestone 1 — authoritative Engine contracts

Commit `53d50ebed98476878a48503a9d9d159d56974c14` contains the immutable artifact, scoped registration/query/CAS installation and source/reference separation. The coordinator reran all 75 Engine tests, strict package Clippy across all targets, owned-file formatting and diff whitespace checks before committing. The 13 added repository tests cover exact snapshot/index/configuration/content binding, unknown/dangling identities, transitive checks, capability scopes/revocation, incomplete analysis rejection, concurrent replacement CAS, predicate membership and integer mutation bypasses. The original 62 tests, including generated histories and atomic failure/recovery schedules, still pass.

Commands on the WSL/Zig development environment use task-local `CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC'`. No global compiler or authorization policy was changed.

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

## Final integration evidence

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

An immutable-tree CLI smoke over the Engine milestone passed capture, structural indexing, Engine artifact installation and impact query. Its semantic gaps remained explicit and integer source mutation remained disabled. The larger live-worktree debug smoke exposed reference-resolution scaling cost; the performance milestone must measure and address that before final completion. Release measurements and clean-checkout verification remain pending. See [implementation boundaries](goal-02.md) for limitations that cannot be replaced by a green test count.
