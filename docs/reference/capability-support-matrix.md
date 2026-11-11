# Capability and Support Matrix

**Status date:** 2026-11-11  
**Repository revision reviewed:** `456bb375108532d9503c82fa8f6e7acfdfedddaf`  
**Evidence basis:** Goal 02 contract/validation/benchmarks and Goal 03 execution/acceptance/benchmarks. Recheck the linked records before relying on this matrix after a code change.

This matrix separates four questions that must not be conflated:

- **Parses:** a structural parser can process supported syntax and produce a syntax/index representation.
- **Analyzes:** the system can derive particular relationships or answer particular semantic queries. This does not imply complete program semantics.
- **Executes:** a concrete policy-owned check can run through the validated execution path.
- **Validated environment:** the exact host/platform/runtime and tool profile on which the stated behavior has been tested.

Status vocabulary:
- **Validated:** direct tests or integration evidence exist for the exact stated scope.
- **Partial:** a subset is implemented; gaps are explicit.
- **Not validated:** adapter/configuration may exist, but the relevant real integration has not been demonstrated.
- **Unsupported:** no supported capability; admission must reject or fail closed.
- **Target:** accepted architecture requirement, not current implementation.

## 1. Programming-language support

| Language | Parses / structural index | Semantic analysis | Execution / verification | Validation boundary |
|---|---|---|---|---|
| TypeScript / TSX | Tree-sitter structural adapter; declarations, scopes, imports/references and supported relationships | Structural resolution plus bounded LSP protocol/query support. Real TypeScript language-server process is **not validated**; dynamic and unsupported constructs remain gaps | No validated TypeScript compiler, type-check, lint or test harness in Goal 03 | Structural tests and protocol fixtures; no claim of end-to-end TypeScript engineering harness |
| JavaScript / JSX | Tree-sitter structural adapter | Structural resolution plus bounded LSP protocol/query support. Real JavaScript server process is **not validated**; dynamic resolution remains incomplete | No validated JavaScript lint/test/build harness | Structural tests and protocol fixtures only |
| Python | Tree-sitter structural adapter | Structural resolution plus bounded LSP protocol/query support. Known dynamic-reference false negative is retained in quality fixtures; real Python server process is **not validated** | No validated Pyright/Mypy, Ruff, pytest or general Python execution harness | Structural quality fixtures; no complete Python semantic or execution claim |
| Go | Tree-sitter structural adapter, package membership and supported references | Structural resolution plus bounded LSP protocol/query support. Real Go language-server process is **not validated** | Actual T1 check invokes pinned Go 1.26.2 `gofmt -e` on the controlled fixture. This is syntax/format processing only, not `go test`, type checking or compilation | Actual Linux rootless-runc T1 integration; finite manually established reference dependency model for the positive controlled fixture |
| Rust | Tree-sitter structural adapter | Structural resolution plus bounded LSP protocol/query support; actual rust-analyzer probe against a generated temporary project is validated with build scripts/proc macros disabled | Celetixo's own Rust workspace is built/tested with pinned Rust 1.94.1. This does **not** establish a general arbitrary-repository Rust execution harness | Linux CI and recorded WSL/Linux development checks; real rust-analyzer integration is opt-in in local validation and included in recorded CI |

**Important semantic limitation:** Goal 02 explicitly records structural coverage gaps, unresolved/ambiguous references, dynamic misses, unsupported constructs and LSP query-only provenance. A successful parser or isolated LSP query cannot promote an analysis to complete. Incomplete real-source analysis blocks Goal 03 execution admission.

## 2. Non-code engineering inputs

| Input class | Current handling | Limitation |
|---|---|---|
| Package manifests and lockfiles | Selected formats participate as captured/indexed engineering inputs and conservative dependency relationships | Not a complete resolved build graph; ecosystem-specific execution/cache adapters are not generally implemented |
| YAML, JSON, TOML, HCL, Dockerfiles, shell, SQL and CI configuration | Captured as repository inputs and may participate in conservative impact classification | Do not infer full language parsing, execution or semantic dependency coverage for every format |
| Generated/mounted inputs | Explicitly declared and bound into snapshot/capture policy | Missing or mutable generated inputs preserve incompleteness; no automatic generator execution during capture |
| Symlinks, Gitlinks and filesystem metadata | Explicit representation and policy checks | Unsupported path/metadata cases reject; the portable execution profile is intentionally restrictive |

## 3. Execution tiers

| Tier | Target definition | Current status | Evidence / restriction |
|---|---|---|---|
| T0 | Deterministic local Engine operations: parsing, graph queries, patch validation, hashing and private materialization | **Validated (bounded)** | Actual-repository T0 measurements and workspace tests. Copying/materialization is not isolation and does not execute repository source |
| T1 | Isolated process/container execution for selected checks | **Validated (specific Linux profile)** | Rootless runc 1.5.2, Linux x86_64, delegated cgroup v2, live systemd user manager, pinned tooling, namespaces/seccomp/resource controls. Actual check is Go 1.26.2 gofmt on a controlled fixture |
| T2 | Firecracker microVM / stronger isolation and untrusted dependency installation/integration tests | **Unsupported** | No validated backend; fail closed |
| T3 | Remote execution pools, large test matrices and GPU workloads | **Target only / unsupported** | No remote pool or distributed execution service |

T1 is not a microVM and shares the host kernel. The validated profile excludes malicious host root/administrator and compromised host kernel from its protection boundary.

## 4. Repository capture and platform support

| Capability / platform | Status | Boundary |
|---|---|---|
| Immutable Git-tree capture | **Validated (bounded)** | Reads verified Git objects and binds the selected tree; missing submodules/generated inputs remain incomplete |
| Live working-tree capture | **Validated with explicit uncertainty** | Tracks staged, unstaged, untracked, ignored, overlay and declared external inputs; two observations cannot prove atomicity, so `LiveSourceNotAtomic` remains |
| No-follow filesystem capture | **Validated on Linux/WSL and Linux CI** | Does not establish native Windows support |
| Native Windows capture/execution | **Not validated / unsupported** | No native Windows guarantee |
| Linux x86_64 T1 | **Validated for the exact profile** | Requires rootless runc 1.5.2, delegated cgroup v2 and live systemd user manager; different profiles require new validation |
| macOS | **Not validated** | No supported host claim |
| Other Linux architectures | **Not validated** | No portability claim beyond recorded profile |
| Distributed/multi-instance Engine | **Target only** | Current authoritative Engine state is in-memory; no durable distributed commit authority |
| Public multi-tenant deployment | **Target only** | ADR-0004 requirements are accepted, but production end-to-end isolation is not demonstrated |

## 5. Evidence and operational support

| Area | Current status | Limitation |
|---|---|---|
| Transaction admission/OCC reference semantics | **Validated in-memory** | Not a durable multi-instance database transaction service |
| Repository analysis installation | **Validated in-memory** | Embedding supplies trusted authentication/administration authority |
| Exact-base source patch application | **Validated** | In-process versioned domain format; source publication is unsupported |
| T1 evidence persistence/admission | **Validated (local)** | Content-addressed local artifact store; no distributed object store or durable Engine state |
| Runtime ownership/recovery | **Validated (local T1)** | Exact cleanup grant and ownership journal; does not restore lost transaction authority |
| Central metrics/traces/alerting | **Target only** | Local records and test/benchmark outputs exist; no production observability platform |
| Backup/restore automation | **Target only** | Local artifact store has persistence/reopen behavior; no coordinated platform backup/DR service |
| Secrets management and credential delivery | **Target only / unsupported in T1** | Do not mount host credentials or inject secrets into the runner |
| Protobuf RPC service and generated bindings | **Target only** | ADR describes TaskIntent semantics; no canonical .proto source, RPC service or generated bindings is present in the reviewed repository state |

## 6. Authoritative evidence

- [Goal 02 implementation contract](../implementation/goal-02.md): capture, parsers, index model, LSP boundary, impact semantics and explicit gaps.
- [Goal 02 validation](../implementation/goal-02-validation.md): tested behavior and source-bound results.
- [Goal 02 measured baseline](../benchmarks/goal-02.md): measured repository-intelligence performance and quality limitations.
- [Goal 03 execution component](../implementation/goal-03-execution.md): concrete T1 controls, toolchain, recovery and actual Go check.
- [Goal 03 acceptance record](../implementation/goal-03-acceptance.md): complete acceptance mapping, actual integration evidence and known limitations.
- [Goal 03 detailed contract comparison](../implementation/goal-03-contract-review.md): full-objective review, fresh focused/live checks and explicit distinctions between supported operations and unsupported conditional capabilities.
- [Goal 03 measured baseline](../benchmarks/goal-03.md): raw measurements, environment, provisional targets and scope.
- [Local Linux T1 runbook](../operations/local-linux-t1-runbook.md): preflight, reproduction, storage, recovery and upgrade procedures.
- [Protocol reference](protocol.md): current canonical schema and generated-binding status.

## 7. Update rule

Update this matrix whenever a parser, analyzer, LSP server, execution adapter, platform, isolation tier, evidence store or deployment boundary changes. A row may be promoted only when a reproducible validation record names the exact source revision, tool versions, environment, fixtures and checks. Do not promote a capability based on configuration presence, successful compilation alone, or a model's ability to generate code in that language.
