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


## 03.5: immutable evidence contracts and Engine gate

The evidence gate is integrated as an independently verified contract milestone
before the executor integration is published. This records its actual dependency
and validation scope; the real execution-to-evidence proof remains required.

`source_evidence` contains versioned canonical evidence, per-check execution
records and scope/content/retention-bound artifact references. The trusted
embedding registers an artifact reader with a separate administrator capability.
An ordinary verifier session cannot select a reader or assert that persistence
succeeded. Evidence includes the exact source patch, base and resulting snapshots
and manifests, application report, admission, policy, verification plan and each
check definition, plus check logs, diagnostics and canonical execution records.

`record_source_evidence` authenticates the claimed executor and registered store,
validates the live admission, releases the Engine mutex while reading artifacts,
and then rechecks current policy, index, snapshot, authorization, owner, lease,
executor and storage authority before retaining one immutable result. Each read
checks length and content address against the same bytes used for canonical
comparison. A storage adapter cannot satisfy integrity with one read and replace
the bytes on another read.

The Engine derives Passed, Failed or Incomplete from the exact planned check set,
check definitions, outcomes, prerequisite ordering, budgets, source integrity,
whole-process termination, cleanup and mandatory persisted artifacts. The caller
cannot supply a green aggregate status. Only complete passing evidence moves a
source attempt to EvidenceSealed and its transaction to Validating. It does not
advance the source snapshot or revision, and the source commit path remains
unsupported.

Exact duplicate delivery returns the original receipt; conflicting delivery
cannot replace it. Failure and retry use distinct immutable attempt identities.
Fault injection covers loss before artifact reads, after artifact reads and after
retention, including recovery of the retained receipt without rerunning checks.
Scoped read access can inspect historical results without granting execution.

Adversarial tests cover forged or missing objects, truncated content, altered
scope/retention/media/provenance, changed commands/toolchains/environments/resources,
stale candidates, incomplete cleanup or termination, dependency failures, output
truncation, concurrent conflicting deliveries, and authority changes during I/O.
These finite reference readers establish the Engine contract. Filesystem
synchronization, actual containment, runtime recovery and the integrated check
path are validated separately with the concrete executor; they are not inferred
from these reference fixtures.


Milestone 03.3 is published as `3b78f3ecb3e85989ddc376083f20358285e67f8e`.
Its exact staged source passed 261 workspace tests and the explicit actual
rust-analyzer probe, strict all-target Clippy, formatting, example builds,
concurrent-transaction assertions, whole-tree capture/index/Engine inspection,
benchmark smoke and all 25 attribution checks. Self-inspection uses the default
credential policy without new reviewed exceptions. Both commit identities are
the established human contributor.


The evidence contract milestone is published as
`4be2abfefcac82e97a0ee2753941943d98a86dfb`. Its exact source passed 289 workspace
tests plus the explicit actual rust-analyzer probe, formatting, strict Clippy,
examples, whole-tree inspection and benchmark smoke. Its
[GitHub Actions run](https://github.com/Emiloart/Celetixo/actions/runs/42539193564)
passed. Milestone 03.3's
[Actions run](https://github.com/Emiloart/Celetixo/actions/runs/42538446776)
also passed.

## 03.4a: concrete isolated checks and local retained artifacts

The execution crate connects private exact-base materialization, the reviewed
rootless OCI T1 runner, confirmed local artifact publication and the Engine gate.
The [execution contract and reproduction instructions](goal-03-execution.md)
describe its actual boundary and limitations. This is a verified component
milestone; the complete Goal 03 exit gate remains open.

The current component passed 40 ordinary coordinator/filesystem/contract tests
and eight explicit actual-OCI integration groups, with strict all-target Clippy.
The actual fixture uses real immutable Git capture and repository JSON selection
of approved checks. Its actual structural analysis remains incomplete; positive
execution uses a separate, explicitly finite, manually established reference
model for the controlled inputs. No language-wide semantic completeness is
inferred from the successful run.

The real path proves patch application, an unchanged source snapshot, isolated
Go syntax checks, persisted canonical artifacts, Engine assessment, immutable
redelivery and rejection of source publication. Adversarial probes cover resource
and namespace boundaries, network/filesystem denial, cancellation, timeout,
controller loss, owned cleanup, partial publication and response loss.

A newly exposed startup race is handled within the existing deadline: the
trusted launcher can signal readiness before runc publishes its PID file.
Absence waits; an invalid marker or failed isolation check still fails. Real
delayed-publication and permanently-missing-marker probes establish this boundary.

[Validation metadata](goal-03-execution-validation/checks.json),
[raw test output](goal-03-execution-validation/checks.log) and
[toolchain provenance](goal-03-execution-validation/toolchain.json) bind these
checks to their exact source-file digests and reviewed tool closure. A benchmark
runner has passed a single smoke invocation; representative measurements and
performance targets are not claimed by that smoke run.

At 03.4a publication, remaining work included source-copy hard-kill ownership
recovery, the neutral executor-result interface audit, representative benchmarks
and the final clean-checkout/CI acceptance audit. The following milestone
addresses ownership and the common executor interface. T2/T3, unvalidated language harnesses and source publication
remain unsupported. No incomplete record may authorize a commit.


## 03.4b: common execution facts and scoped source-copy recovery

The coordinator now consumes backend-neutral check facts. OCI-specific projection
and error translation stay in the OCI adapter. Forged bindings and absent actual
observations cannot become passing evidence. The isolated backend requires a
registered workspace whose ownership was durable before copying began.

A scoped persistent workspace registry accounts for pending/live/retained copy
quotas. Recovery skips live owners, retains runtime-latched sources and rejects
changed identities, scopes and cleanup grants. Actual SIGKILL tests cover four
allocation/copy publication intervals and recovery after reference Engine loss.
The separate runtime claim protocol is exercised before and after launch claim
publication as well as after actual command start. These are component recovery
proofs; the complete Goal 03 benchmark and acceptance audit remain open.

Milestone 03.4a is published as `c7edb05ed7b4070418bde5513d06efa78c1053c4`.
Its [GitHub Actions run](https://github.com/Emiloart/Celetixo/actions/runs/42543267592)
passed. Its validation directory remains historical evidence for that exact
source. The next component's test output is recorded separately.

The exact component source passed 48 ordinary execution tests, including the
four hard-kill allocation/copy checkpoints, and all nine actual OCI groups.
Formatting and strict all-target execution Clippy passed. The
[recovery validation metadata](goal-03-recovery-validation/checks.json) and
[raw command output](goal-03-recovery-validation/checks.log) record source hashes,
commands, current kernel and the actually used tool-closure hashes. Tool origin
and signature records remain in the preceding component's
[toolchain provenance](goal-03-execution-validation/toolchain.json).


## 03.6a: reproducible measurement runners

The [benchmark protocol](../benchmarks/goal-03-protocol.md) separates actual Git
repository T0 measurements from generated finite-reference T1 executions.
Operation/file-size, replay/hash, copying, source cleanup, resource observations,
artifact publication/read, Engine admission, disabled-cache identity and actual
capacity/orphan recovery have explicit measurement fields. Equal repeated fixture
captures must retain their snapshot identity. CI also runs the T0 runner against
its selected immutable source tree.

Generated inputs include fixed Git commit dates, repository-owned check selection,
a retained module manifest and controlled source/asset sizes. The actual structural
analysis remains incomplete; only the separately identified reference model admits
these known checks. Benchmark preparation does not promote arbitrary real-source
analysis. Raw baseline measurements and targets will bind a committed runner
source and are still outstanding for the complete Goal 03 exit gate.

The preceding recovery milestone is `4c6d7baf16c26a98ca293a885ba56eb892504547`.

The new actual end-to-end test rejects a mismatched immutable base before claim,
recovers an interrupted first attempt without fabricating evidence, executes a
fresh attempt through isolated verification and Engine admission, and proves
failed history, duplicate delivery and source-publication rejection are preserved.
The source-bound runner checks passed seven T0 cases over three repetitions,
three generated T1 cases over three repetitions, and three actual bounded
capacity/recovery probes. Existing 48 execution tests and all nine prior live
OCI groups passed; the new live retry/base-mismatch group also passed. These are
runner/integration proofs, not the published representative baseline yet.
Milestone 03.4b's [CI run](https://github.com/Emiloart/Celetixo/actions/runs/42547309224)
passed.


## Explicit result-attestation contract

The acceptance audit found that §9's result-attestation level needed an explicit
sealed field. Evidence v2 now binds a typed level to its canonical identity and
media type. The Engine admits only its scoped-executor authentication contract;
metadata constructors do not authenticate callers. Unattested claims and legacy
v1 envelope media fail without retaining evidence or poisoning a valid delivery.
No hardware, remote-worker or microVM attestation is inferred.

Check records, OCI journals and artifact addressing retain their own v1 versions;
historical envelopes are not rewritten or silently promoted. This closes the
named contract gap before final benchmark collection. The complete Goal 03 audit
and measured baseline remain open until their records are published and checked.

The measurement-runner milestone is published as
`80dafd0a12b7a4c44b083b7bdf88a92cceed8fb4`.

The exact evidence-v2 source passed 339 workspace tests, formatting, strict
all-target Clippy and all ten explicit actual integration groups (nine OCI and
one retry/base-mismatch flow). Its [validation metadata](goal-03-evidence-v2-validation/checks.json)
and [raw commands/output](goal-03-evidence-v2-validation/checks.log) retain code,
log and actual tool-closure hashes. Historical v1 validation records remain
identified with their original source versions.
