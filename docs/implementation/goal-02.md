# Goal 02 — Repository intelligence and impact analysis

This milestone connects captured source bytes to the Rust Engine's dependency model. It does not turn a parser, a successful LSP query, a graph digest or a caller-supplied completeness flag into commit authority. Read [Goal 01](goal-01.md), [engineering identity](../spec/engineering-identity.md) and the accepted ADRs alongside this contract.

## Developer entry points

The workspace pins Rust 1.94.1 and commits `Cargo.lock`. Install Git and a C compiler for the Tree-sitter grammars. No Node, Python or Go runtime is required to structurally index their source files. From the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --examples --locked
cargo run --locked -p celetixo-engine --example concurrent_transactions
```

Inspect an actual workspace without executing its source, package scripts, Git hooks or filters:

```sh
cargo run --locked -p celetixo-repository --example inspect_repository -- /path/to/repository --changed src/example.ts
```

The JSON report contains snapshot/index/impact identities, affected elements/files, suggested checks, capture consistency and analysis gaps. It installs the exact artifact in a fresh, explicitly scoped in-memory Engine and queries it there. Default example identities are local inspection labels; callers can supply `--organization`, `--workspace` and `--repository`. This example is not a network authentication service.

Capture includes ignored files by default. Resource-limit errors are explicit. An optional `--exclude PATH` records an exact directory/path exclusion with an unresolved relevance warning; it never silently proves that a cache or generated directory is irrelevant. A build checkout may need `--exclude target` before live inspection. Do not use that policy for a repository where generated files under `target` participate in analysis or execution.

To inspect an immutable Git commit instead, pass `--git-tree FULL_OID` (the full SHA-1 or SHA-256 object ID). This is a distinct capture operation: it intentionally selects the verified Git tree and does **not** describe the live dirty workspace. Use the pre-change generation to inspect a proposed deletion. A path absent from the selected generation produces an error rather than an empty impact result.

On the development WSL machine, `/home/emiloart/bin/cc` delegates to Zig 0.15.2. `cc-rs`'s default Rust target spelling is incompatible with that wrapper. The verified task-local workaround is:

```sh
CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC' cargo test --workspace --locked --offline
```

It does not change global compiler settings. A normal GCC/Clang installation, including the Linux CI runner, uses the ordinary commands above.

## Trusted capture

`celetixo_repository::capture` freezes bytes in Goal 01's `Snapshot` contract. `CaptureRequest` explicitly supplies scope, authoritative revision, inclusion policy, required paths, overlays, external generated/mounted files, already immutable inputs and relevant non-secret environment/configuration values. Ambient process environment is not claimed as a captured execution environment. Callers must enumerate additional execution/toolchain inputs that matter to their checks.

Live `capture` records HEAD, the index (including staged/index flags), working content, tracked deletions, untracked and ignored files, local Git configuration, relevant filesystem metadata, overlay versions/tombstones, declared generated/mounted inputs, and submodule references and available recursive contents. Policy, exclusions, modes, symlinks and content digests participate in immutable identity. Symlinks remain target bytes and are never followed into source indexing. Missing inputs, uninitialized submodules, nested repositories, unresolved merges and exclusions remain visible.

On Unix, filesystem reads use directory-relative no-follow handles. Git runs with a cleared environment and constrained configuration: no repository filters/hooks, optional locks, replacement refs, lazy network object fetching or repository program execution. Commands, file count/size, total bytes, traversal depth and syntax nodes have finite budgets. Capture failures return errors; no partial result is published as successful completion.

The no-follow filesystem capture implementation is validated on Linux/WSL and Linux CI. Native Windows capture is not claimed. Canonical Git modes and relevant Unix mode metadata are retained; ownership, ACLs, extended attributes and a complete recreation of an execution filesystem are not provided. Repository capture is distinct from an isolated execution-environment materializer.

Two full observations compare filesystem content/metadata and Git state. An observed concurrent change rejects capture. Equal reads cannot prove that an adversarial writer did not change and restore inputs between observations. Accordingly **every live capture retains `LiveSourceNotAtomic`**, even if its census is complete and reproducible. There is no caller option that upgrades two observations to an atomic filesystem snapshot.

`capture_git_tree` reads immutable Git objects, checks object IDs, and parses commit/tree/blob contents directly. It can produce a complete capture of that explicitly selected immutable input set. Uninitialized submodules or missing required/generated inputs still make it incomplete. Live external paths cannot be attached to an immutable-tree completeness claim; supplied immutable generated/mounted bytes and immutable editor overlays are explicit alternatives.

`CaptureResult::validate` rechecks manifest/report consistency, policy and uncertainty bindings, input count/content digests and effective-layer collisions. Public report fields are inspectable data, not trusted declarations. Consumers validate before indexing/impact analysis. Snapshots and results remain in memory; durable artifact storage is a later adapter, not a property claimed here.

Credential admission runs before snapshot publication across input layers and explicit environment inputs. A versioned default policy rejects conventional credential paths and recognizable private-key/provider-token/credential material, with content withheld from errors. Reviewed public exceptions bind an exact logical path and content digest; they require the trusted capture/governance caller and cannot be a wildcard or blanket disable. The policy participates in snapshot identity. Scoped credential references are distinct from raw credential values. These bounded detectors cannot classify arbitrary secrets embedded in source or prove an entire repository public. The embedding must apply authorized data classification and input policy before capture, persistence or export; this library does not claim a deployed governance service or automatic approval to send source to a model. Exclusions retain uncertainty and never silently redact transaction inputs into a complete snapshot.

## Versioned indexing

`Indexer` parses only snapshot bytes. The adapter set is Tree-sitter TypeScript/TSX, JavaScript/JSX, Python, Go and Rust. Each generation binds scope, exact snapshot, Tree-sitter runtime and grammar versions, analyzer version, language configuration, resource policy, generated relationships, files, nodes, edges, occurrences and gaps.

The index represents files/modules, lexical scopes, declarations/parameters, references, calls, imports, structural type/interface relationships, packages/dependency specifications, manifest/configuration/lockfile inputs and explicitly configured generated inputs. Edges point **consumer → dependency**. Containment points child → containing file/scope, allowing file changes to reach contained declarations. Package/build relationships conservatively connect participating source. Package specifications are not proof of a fully resolved external build graph.

Declaration IDs combine scoped declaration structure, namespace and tokenized identity; source locations remain attributes. Comments and line shifts do not become declaration identity. File content and occurrences remain snapshot-specific. Exact-content rename/copy continuity reports retain ambiguity and are for navigation/analysis; they never collapse OCC identities or authorize a commit.

Unchanged file syntax trees are reused. Changed files are reparsed; cross-file resolutions and generation digests are recomputed. This is incremental indexing through unchanged-tree reuse, not an incremental parser claim for changed text. Publication replaces the index/cache only after successful construction and validation. Budget failures preserve the previous published generation, and a fresh retry can succeed. Incremental output must equal a clean rebuild of the same snapshot and configuration, including issues and provenance.

Syntax errors, unsupported languages/constructs, unresolved/ambiguous references, invalid encodings, symlinks/nonregular inputs, malformed configuration and missing generated inputs are explicit. Structural source analysis always records missing semantic-service coverage. Parsing is useful evidence; it is never a complete program-understanding claim.

## Semantic service boundary

`lsp` implements bounded Content-Length JSON-RPC over TCP, initialization/capabilities, immutable document synchronization, UTF-16 positions, shared snapshot-scoped sessions, connection-bound multiplexed tickets, definitions, references, type definitions, implementations, call hierarchy and version-bound diagnostics. Sessions bind scope, snapshot, analyzer/server generation, root and configuration. Cancellation/deadlines, stale responses, mismatched documents, external/unmapped locations and server failures are explicit.

The production library **does not spawn or trust arbitrary repository language servers**. The caller must provision an isolated, resource-bounded server against a materialization of the exact snapshot and supply its authenticated configuration/generation. An LSP server may execute build scripts/plugins; those operations belong behind the execution boundary, not an untrusted host shortcut.

Successful queries are `QueryOnly`. Supplemented edges retain exact server/query/document provenance and never erase unresolved regions or broad semantic coverage gaps. Reference results at a usage site require the exact resolved definition; the enclosing function is not substituted for the referenced symbol.

Responses are sealed to their originating connection and contents. Targets must already be bound when the response arrives; opening a file later cannot retroactively turn an unbound location into a verified edge. Mapping requires an exact declaration-name location rather than an unsupported nested declaration's enclosing function. Malformed responses, poisoned frames and aggregate buffering limits have explicit failure behavior.

Protocol fixtures exercise the initial language configurations. A separate ignored integration probe exercises actual rust-analyzer against a self-authored, temporary Rust project, with build scripts/proc macros disabled and ambient environment cleared:

```sh
rustup component add rust-analyzer
cargo test --locked -p celetixo-repository --test lsp -- --include-ignored
```

Actual TypeScript/Python/Go server processes are not validated by the protocol fixtures. This milestone provides structural adapters and supported standard semantic queries; it does not claim full language engineering harnesses, complete macro/dynamic resolution or exhaustive LSP/SCIP coverage. No SCIP importer or exporter is claimed; the index provides versioned definitions, occurrences, relationships and analyzer provenance that can support a future SCIP adapter.

## Impact and Engine authority

`ImpactAnalyzer::new` validates the capture/index binding and freezes forward/reverse adjacency. `ImpactRequest` names exact snapshot and index generation, changed elements, transaction accesses/predicates and invariants. The request digest binds all invariant conditions even if two conditions currently reach the same graph.

The deterministic result distinguishes directly changed roots, immediate consumers, transitive downstream consumers, prerequisite dependencies, affected files, reconsidered transaction reads/writes/predicates/invariants and justified verification suggestions. `downstream` includes immediate consumers but excludes directly changed roots; cycles terminate through visited identities. Check suggestions explain structural/type/test/package/generated impact. They are targets for repository-configured verification, not commands, successful evidence or automatic permission to execute.

The result binds its full request, exact snapshot, full index generation, source-edge provenance and gaps into a content digest. Stale snapshots/generations, unknown semantic identities and modified results are rejected. Unknown global coverage remains present even when a traversal predicts a small affected set. An empty predicted set is not evidence of isolation.

The Rust-owned `RepositoryAnalysis` contract binds scope, snapshot, index digest, configuration, dependencies, checks and completeness. Installation requires both scoped registration and model-administration authority. Later replacement uses compare-and-swap on the installed artifact digest; two competing replacements cannot both install. Ordinary analysis capabilities cannot install a graph, select an older generation or invent unknown elements. The Engine's monotonic installed generation is separate from the content-addressed index generation.

Installing an analysis never advances repository revision. It preserves Goal 01's immutable attempts, evidence bindings, authorization, lease, conflict, revalidation and commit gates. The legacy integer reference API rejects source-input snapshots, and reference-model replacement cannot overwrite source facts. **Integer patches do not edit source:** source mutation, source revalidation through integer patches and final integer commit are rejected explicitly. This is repository intelligence before worker execution, not a newly implemented source patch publisher or durable distributed commit adapter.

The adapter/administrator is a trusted fact producer. A self-consistent digest is not a cryptographic attestation of semantic completeness. Production ingestion must authenticate and authorize the analyzer and isolate its execution. No client or model can acquire Engine privileges by supplying a content hash.

## Validation and milestones

Correctness tests cover capture integrity and races, staged/unstaged/untracked changes, renames/deletions, overlays, modes/symlinks, submodules, configuration, generated/mounted inputs, resource exhaustion/recovery, version invalidation, identity continuity, scope/reference extraction, incremental/rebuild equivalence, cycles, transitive impact, unknown coverage, query binding and Engine authority. Manual labels include a known dynamic Python false negative so measured recall cannot hide that limitation.

The executable benchmark, raw results, environment, measured targets and quality rationale are under [Goal 02 benchmarks](../benchmarks/goal-02.md). Timings are exploratory measurements of the recorded fixture/host/build; there is no production latency or arbitrary-repository accuracy guarantee.

Verified milestones are committed separately: (1) authoritative Engine source-analysis contracts and anti-bypass tests; (2) capture/index/LSP/impact subsystem and regression fixtures; (3) reproducible measurements, developer instructions and completion evidence. The [Goal 02 validation record](goal-02-validation.md) records the actual checks and remaining boundaries. Planned tests or intended commits are not completion evidence.
