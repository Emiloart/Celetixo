# Goal 03 implementation record

The [accepted Goal 03 objective](goal-03.md) and updated [Dev.md](../../Dev.md)
were retrieved from GitHub at `2c42f67d3e9492c2dcd2cc49ab6a4870f608dec8`.
This record describes implemented behavior separately from that objective.
Goal 03 remains in progress until its integrated exit gate passes.

## 03.1: deterministic source patches

`celetixo_engine::source_patch` implements a pure T0 transformation over immutable
snapshots. It does not touch the filesystem or execute repository programs.
`SourcePatch::new` canonicalizes operations; `from_canonical_bytes` strictly
decodes the bounded version 1 format, checks payload and operation digests, and
rejects trailing bytes and alternative encodings. This is an in-process domain
format, not a replacement for the canonical cross-service TaskIntent protocol.

The envelope binds scope, principal attribution, transaction and source-attempt
identities, base snapshot and revision, analysis/index/impact identities, producer
and validity interval. These bindings are not authorization: Engine admission
must compare them with authoritative state in milestone 03.2.

Create, replace, delete, rename and regular/executable mode changes operate on
exact bytes. All preimages refer to the unchanged base, including simultaneous
rename cycles. Ambiguous overlapping operations and ineffective patches reject.
The candidate retains the authoritative base revision. It does not publish a
revision or enable the integer patch/commit path for source repositories.

The application report retains the full original layered manifest, environment,
exclusions and capture provenance. A normalized execution view contains the
effective file bytes; recognized captured Git administration is recorded as
non-operational provenance rather than installed as an execution Git directory.
Unknown reserved control content rejects. Capture uncertainty remains bound to
the original snapshot and report; application cannot promote it to completeness.
Independent replay validates both the complete report and resulting snapshot.

The initial mutation profile is explicitly restricted to portable ASCII paths.
Non-ASCII paths reject rather than undergo Unicode normalization. Absolute paths,
traversal, Windows reserved names, case-fold aliases (including inferred
directories), file ancestors and reserved control paths reject. Symlinks are
denied by default, including inherited links; the optional internal-relative
profile validates the final source graph and rejects unsupported resolution.
Submodule references are preserved but are not mutable patch payloads.

`snapshot::effective_workspace` is the shared layer-merging implementation used
by capture and application. Its precedence and collision behavior are unchanged
from Goal 02. The new manifest digest addresses complete source provenance
without changing Goal 01/02 snapshot identity encoding.

### Validation and remaining work

Behavioral tests cover binary bytes, operation ordering, exact base/preimages,
rename cycles, modes, malformed and oversized decoding, path aliases, symlink
safety, unchanged source/revision, retained provenance and forged report/result
rejection. Workspace regression checks are recorded with the milestone commit.

The 03.1 validation ran the full workspace suite (203 tests passed, one optional
real language-server test ignored), then reran all Engine tests after the final
environment and symlink-resolution fixes (95 passed). The final patch adds 18
tests and shared snapshot contracts add two. Formatting, strict workspace
Clippy across all targets, workspace/example builds and all 25 attribution
regression checks passed. The real rust-analyzer probe was not repeated for this
pure patch milestone; its existing explicit CI job is retained.

Filesystem materialization, Engine-governed source attempts, concrete verification
plans, isolated commands and persisted evidence are subsequent milestones.
No isolation, execution authority, semantic completeness, durable evidence or
source publication capability is claimed by 03.1.

Run the portable checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --examples --locked
```

This WSL development environment uses a Zig-backed C compiler for Tree-sitter.
Its local invocation prefixes Cargo with
`CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC'` and uses
`--offline`; these are environment-specific build settings, not sandbox controls.
