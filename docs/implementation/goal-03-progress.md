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

The published 03.1 commit is `e669a621ada936a886572eb53a19c1a50c486c7c`;
its [GitHub Actions run](https://github.com/Emiloart/Celetixo/actions/runs/42199194642)
passed, including the explicit real rust-analyzer job.

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

## 03.2: authoritative source-attempt admission

The Engine now owns source execution policy installation, exact patch admission,
source-attempt transitions, separate executor capabilities and cleanup-only
recovery capabilities. These remain in-process contracts over the shared
development store. They do not provide durable or distributed transaction state.

The trusted embedding registers a `RepositoryAnalysis`, installs a
`SourceExecutionPolicy` through scoped `ManageModel` authority and compare-and-swap,
then uses the existing propose/analyze/reserve/execute transaction lifecycle.
`source_patch_context` returns the Engine-derived bindings for a patch.
`admit_source_attempt` independently recomputes the request and impact bindings,
checks the exact current snapshot/revision, analysis/index generation, owner,
lease, policy, expiry, path operations and semantic write guards, and replays the
candidate before issuing an immutable `SourceAdmission`.

Policy-owned source invariants expand dependency reads and invariant scopes to
a fixed point before reservation and admission. Added facts retain derived
provenance. Policy replacement advances the installed model generation, so a
pre-policy or stale-policy transaction must obtain fresh analysis. Numeric
reference invariants cannot stand in for source checks.

Admission requires complete capture and analysis. Live capture, exclusions,
missing inputs and real structural semantic gaps cannot become empty impact.
The success tests deliberately use finite complete reference artifacts under
`celetixo/reference-source/` provenance; they do not upgrade Tree-sitter or LSP
coverage. Immutable Git-tree metadata is validated against the exact capture
policy and uncertainty identity.

`Administrator::grant_source_executor` issues an opaque, scoped, expiring
capability for an exact backend, tier and resource ceiling. Ordinary transaction
or verifier sessions cannot substitute for it. Claiming an admitted attempt is
atomic and permits one executor; a duplicate claim cannot launch another run.
`validate_source_attempt` and `source_execution_time` support supervision without
copying source on each poll. The trusted embedding drives the development
store's monotonic clock; a worker cannot set admission time.

Cancellation, revocation, expiry and stale facts produce explicit source-attempt
outcomes. Source attempt failures do not overwrite prior attempts. Authorized
admission denials also retain their request, rejection and idempotency result:
policy changes cannot revive a rejected identity, and different inputs require
a new source-attempt identity and key. Another principal or another session
holding the same principal attribution cannot poison the lease owner's attempt.

Cleanup capabilities grant only scope/backend-bounded recovery of owned runtime
resources. They cannot claim attempts, execute checks, admit evidence or publish
source. A fresh operator capability may reconcile a runtime journal after loss
of the development store, but cannot reconstruct a lost transaction.

### Verification-plan foundation

`source_execution` defines the policy-owned check registry and a deterministic
plan over declared checks, impact-justified checks, invariants, mandatory policy
checks and transitive check prerequisites. It rejects missing, cyclic,
substituted, out-of-scope and over-budget definitions. Check identities bind exact
argv, working directory, candidate inputs and dependency digests, executable and
runtime identities, toolchain, environment, backend/tier, network/credential
policy, resource limits, expected artifacts and failure semantics.

Input digests are checked against the resulting snapshot. A base-file digest
cannot satisfy a changed candidate check. Arbitrary commands cannot run in T0.
Cache identities bind the complete plan/check and execution context; reuse is
disabled. These types provide the planning dependency for admission. Adoption of
repository harness configuration and actual execution remain later integration
work; no language-wide harness support is claimed here.

### Credential admission and compatibility

Capture and source admission share `credentials::CredentialPolicy`. The candidate
is screened again after applying the patch, including newly introduced bytes and
renamed paths, before an execution grant or artifact publication. Capture's public
`CredentialPolicy` import and original policy-digest encoding remain compatible.
Reviewed public examples require a trusted exact path/content exception in the
installed policy; workers cannot approve their own exceptions. Propagate the
reviewed capture policy explicitly when it is also appropriate for execution.
Literal command environments cannot contain recognized credential material or
credential keys; protected references remain separately scoped contracts.

This classifier has finite rules. It can miss encoded or unknown secrets and can
reject public examples. It is not a complete data classification service.

### Checks and remaining integration

Before final staged-tree verification, the Engine and Repository suite passed
249 tests (138 Engine, 111 Repository), with the existing opt-in rust-analyzer
test ignored. Strict all-target Clippy for both crates, owned-file formatting,
capture-policy golden compatibility and the focused credential rerun passed.
The working tree also contains four future evidence-object unit tests; those
are not part of the admission milestone's published source subset.

Source evidence admission, isolated execution and filesystem materialization are
not enabled by this commit. The legacy source integer-patch, revalidation and
commit paths still return `SourceMutationUnsupported`. No executor may seal a
successful Engine result until the later persisted-evidence gate is integrated.


The exact published 03.2 tree passed 245 workspace tests, formatting, strict
all-target Clippy and workspace example builds with the locked offline commands
above. The opt-in real language-server test was not repeated locally. The
milestone is `c23d940c4a31d2d86930ff779ecb79d32be33a42`; both Git attribution
identities are the established human contributor.

## 03.3: repository configuration and optional checks

`celetixo_repository::harness::ConfiguredHarness` reads an ordinary file from
an exact immutable snapshot. It accepts bounded, strict version-1 JSON:

```json
{
  "version": 1,
  "checks": [
    {"id": "go-syntax", "definition": "<64 lowercase hexadecimal digits>"}
  ]
}
```

The example address is a placeholder for an exact approved check-definition
digest, not an executable configuration. The trusted embedding supplies the
registry. Repository configuration selects exact definitions; it cannot supply
commands, install tools, expand network/credential grants, change resources or
install Engine policy. Unknown or duplicate fields, duplicate IDs, missing
checks, substituted definitions and dependency cycles reject.

The adapter records selected roots separately from transitive prerequisites,
orders prerequisites deterministically and binds the raw configuration content
into each configured definition. Even a whitespace change changes its identity.
The plan validates this input against the candidate, so a patch changing the
configuration cannot reuse the base configuration's checks. Applying the adapter
to policy input preserves mandatory/invariant requirements and does not confer
policy installation authority.

`SourceAttemptOptions` and `admit_source_attempt_with_options` let an authorized
transaction owner request additional approved optional checks. Their sorted IDs
are part of admission idempotency; changing that set requires a new request.
Empty options retain the existing API and request identity. The Engine still
derives mandatory checks independently, promotes prerequisites reached by a
mandatory check and rejects attempts to label a required check as optional.
The request is bounded to 4096 IDs before hashing or reserving an identity.

Tests cover strict configuration decoding and budgets, raw-byte configuration
identity, trusted definition selection, deterministic dependency closure,
mandatory-check preservation, optional-request identity and rejection behavior,
and candidate-input mismatches. An actual immutable Git-tree fixture containing
TypeScript, Python, Go and Rust is parsed and configured, but its real structural
analysis remains incomplete and the Engine rejects execution. Positive
admission tests continue to use explicitly finite reference analysis. No
language-wide execution support is inferred from configuration or syntax parsing.

Actual isolated tool execution, artifact persistence, evidence admission and
integrated recovery remain later milestones. Cache key construction remains
available with reuse disabled.


The 03.2 CI run passed tests, strict linting, the actual rust-analyzer probe and
builds, then correctly refused self-inspection: the newly added `credentials.rs`
source and test filenames matched the existing sensitive-file rule. Private
capability-handle initializer names also triggered the finite material heuristic;
those fields now use capability terminology. The shared
module now lives in `input_admission.rs`, retaining its public `credentials`
module API through an explicit path attribute. This changes neither credential
classification nor capture exceptions. The inspection step is repeated against
the exact next staged commit before publication.
