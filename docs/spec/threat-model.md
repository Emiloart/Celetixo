# Celetixo Threat Model

**Status:** Normative security model and review baseline  
**Version:** 1.0  
**Reviewed against:** `main` at `456bb375108532d9503c82fa8f6e7acfdfedddaf` (2026-11-11)  
**Authority:** ADR-0001 through ADR-0004; this document consolidates security requirements and does not weaken or replace them.  
**Implementation evidence:** Goal 01–03 acceptance and validation records, linked per control; subsequent [Goal 04.1g registration integration](../implementation/goal-04-registration-authority.md) adds bounded SQL registration/grant authority, without durable transaction/source recovery or production-isolation claims.

This model distinguishes required security properties from controls demonstrated by the current implementation. A requirement is not implemented merely because it appears here or in an ADR. Validation status is limited to the exact scope and environment in the linked evidence.

## 1. Security objectives and scope

Celetixo accepts repository content and proposed changes from workers, derives engineering impact, executes selected checks, evaluates evidence and coordinates repository state transitions. Its security objectives are:

1. Prevent unauthorized actors from reading, changing, executing against or publishing repository state.
2. Prevent untrusted source, dependencies, tools and worker output from escaping their execution boundary or obtaining host credentials.
3. Prevent stale, forged, incomplete or misbound analysis and verification evidence from authorizing a state transition.
4. Preserve organization, workspace, repository, snapshot, transaction and evidence separation.
5. Preserve integrity and recoverability across crashes, retries, duplicate delivery and storage faults.
6. Prevent unauthorized collection, disclosure, retention or training use of repository and trajectory data.
7. Make security failures observable, attributable and fail-closed where correctness or confidentiality is at stake.

**Current operating envelope:** a local Rust reference Engine, repository capture/index/impact tooling and a reviewed Linux x86_64 rootless-runc T1 execution profile. This is not a production distributed service, public multi-tenant deployment, remote execution fleet or complete language harness. See the [capability matrix](../reference/capability-support-matrix.md) and [operations runbook](../operations/local-linux-t1-runbook.md).

## 2. Assets and security properties

| Asset | Examples | Required property |
|---|---|---|
| Source and workspace data | Git objects, staged/unstaged/untracked files, overlays, generated inputs | Confidentiality, integrity, explicit scope and provenance |
| Engineering identity/state | Organization, principal, workspace, repository, snapshot, index, transaction, attempt | Unambiguous scoped identity; no ID reuse or cross-scope substitution |
| Transaction authority | Engine state, capabilities, reservations, leases, commit admissibility | Only the authoritative Engine can decide admissibility |
| Analysis artifacts | Capture manifests, indexes, semantic edges, impact sets and coverage gaps | Exact snapshot/generation binding; uncertainty cannot be erased |
| Execution boundary | Runtime, namespaces, cgroups, mounts, entrypoint, toolchain and scratch | Isolation, resource bounds, no implicit credential/network access |
| Verification evidence | Check plans/results, output, artifacts, environment and provenance | Integrity, completeness, freshness and durable binding |
| Credentials and integrations | Repository tokens, provider credentials, signing keys | Least privilege, scoped delivery, revocation and secrecy |
| Operational state | Ownership records, runtime journals, artifact store, logs and backups | Integrity, recoverability, access control and bounded retention |
| Trajectory/dataset data | Prompts, patches, logs, artifacts, outcomes and derived datasets | Rights, purpose limitation, minimization, lineage and retention |
| Protocol and build supply chain | Schemas, generated bindings, compilers, runtimes and dependencies | Version integrity, reproducibility and compatibility |

## 3. Trust boundaries

| Boundary | Untrusted side | Trusted responsibility | Required enforcement |
|---|---|---|---|
| Client/API → platform | Client-supplied IDs, claims, payloads and approvals | Authentication, authorization, scope resolution and input validation | Authenticate principal; authorize each action; identifiers are selectors, not authority |
| Brain/model/worker → Engine | Model output, task intent, actor labels, proposed reads/writes and patches | Engine admission, impact, leases, evidence and commit decisions | Treat proposals as data; independently derive/validate facts; no worker-granted authority |
| Repository → capture/indexer | Source files, Git metadata, symlinks, hooks, filters, configuration and generated inputs | Bounded capture and analysis | No source execution during capture; no-follow reads; resource budgets; explicit gaps and uncertainty |
| Analyzer/LSP → Engine | Parser results, language-server responses and claimed completeness | Trusted registration and exact-generation validation | Bind response to session/document/snapshot; query-only evidence cannot promote completeness |
| Engine → executor | Candidate, policy plan, executable, toolchain and environment | Scoped executor capability and backend policy | Exact-base admission; pinned tools; no host fallback; isolated resource-bounded execution |
| Sandbox → host | Process behavior, syscalls, output, exit status and termination | Runtime supervisor and cleanup authority | Namespace/seccomp/cgroup/mount checks; independent deadline; verify termination and cleanup |
| Evidence store → Engine | Persisted objects and delivery envelopes | Independent evidence reader and Engine admission | Read back and hash objects; verify bindings and current authority; missing evidence is not success |
| Event/workflow → state | Duplicate, delayed, reordered or replayed messages | Durable authoritative state | Idempotency, state/revision checks, outbox/reconciliation; event delivery is not commit |
| Tenant/workspace → shared infrastructure | Cross-scope references, cache keys, event subjects, logs and artifacts | Policy and isolation layer | Scope all records and operations; test cross-scope access; no namespace-only security claim |
| Operator/host → runtime | Host administrator, kernel, user manager, runtime binaries and storage | Deployment owner | Pin and verify tools; restrict operator access; document host compromise as residual risk |

## 4. Attacker model

The model considers these capabilities, independently or in combination:

- **Malicious repository contributor:** can submit crafted source, symlinks, malformed manifests, unusual paths, build configuration, generated inputs or dependency metadata.
- **Compromised or adversarial worker/model:** can lie about intent, request excessive scope, emit malicious patches, replay requests, forge actor labels, attempt prompt/tool manipulation or leak data through output.
- **Malicious dependency or build tool:** can execute lifecycle scripts, exploit parsers/compilers, consume resources, access files or attempt network communication.
- **Unauthorized client or compromised integration credential:** can call APIs, enumerate identifiers, request cross-scope data or perform actions outside granted scope.
- **Compromised execution process:** can attempt namespace escape, privilege escalation, host filesystem access, resource exhaustion, process persistence or signal/PID confusion.
- **Network/storage adversary or fault:** can delay, duplicate, reorder, drop or replay messages; alter, truncate or withhold persisted objects; cause response loss or partial availability.
- **Insider/operator error:** can misconfigure policy, expose credentials, alter pinned tooling, delete artifacts, bypass local hooks or misuse cleanup authority.
- **Compromised host administrator/kernel:** can inspect or alter local processes, memory, files, runtime and evidence. The local T1 profile does not defend against a malicious host root or compromised kernel.

This model does not assume that a worker is trustworthy because it is operated internally, that repository content is safe because it is private, or that a successful process exit proves correct behavior.

## 5. Threats, mitigations and validation

Status labels:
- **Validated (bounded):** direct tests or actual integration evidence exist for the stated narrow scope.
- **Partial:** meaningful controls exist, but coverage or deployment is incomplete.
- **Target only:** architectural requirement; implementation evidence is not established.
- **Unsupported / fail closed:** the current implementation rejects the capability rather than claiming protection through a fallback.

| ID | Abuse case and impact | Required mitigation | Current implementation / validation status | Residual risk |
|---|---|---|---|---|
| T-01 | Worker or client forges principal, organization, repository or capability identifiers to gain authority or cross-scope access | Authenticate principals independently; authorize at service and Engine boundaries; separate attribution from capability; propagate authenticated scope | **Partial.** Engine capability and scoped admission contracts are tested in the in-memory reference Engine. There is no production authentication service or distributed scope propagation. See Goal 01 and Goal 03 acceptance. | Embedding owns authentication and trusted capability issuance. No production multi-tenant isolation claim. |
| T-02 | Crafted repository content exploits capture, parsing, indexing or LSP; hooks/filters/scripts execute during inspection | Treat repository as hostile; no-follow bounded capture; clear/constrain Git environment; do not execute repository code; version and bind analysis; preserve unknown coverage | **Validated (bounded).** Linux/WSL and Linux CI capture, no-follow behavior, Git command restrictions, budgets, identity and adversarial capture tests. Native Windows capture is not claimed. See Goal 02. | Parser/LSP implementation vulnerabilities remain possible. Semantic analysis is incomplete for dynamic/unsupported constructs; live capture is explicitly non-atomic. |
| T-03 | Analyzer or LSP lies, returns stale/mismatched locations, or omits dependencies to make a patch appear safe | Bind analysis to snapshot/generation; validate provenance; retain gaps; require trusted installation; incomplete impact blocks execution/commit | **Validated (bounded).** Versioned indexes, query binding, generation checks, Engine admission and known dynamic false-negative fixtures. Actual rust-analyzer probe only; other server processes are not validated. | A trusted analyzer administrator remains in the embedding boundary. A content digest proves binding, not semantic truth. |
| T-04 | Malicious patch exploits path traversal, symlink races, collisions or mutates original source | Canonical bounded patch format; exact preimages; portable path rules; private materialization; no-follow operations; source census and immutability checks | **Validated (bounded).** Goal 03 patch and private workspace tests, actual OCI source immutability checks and interrupted-copy recovery. Unsupported path/metadata forms reject. | Current portable profile is restrictive. Source revision publication is unsupported. |
| T-05 | Untrusted check or dependency escapes sandbox, reads host files/credentials, obtains privileges or exhausts host | Rootless execution; private namespaces; read-only source/image; no host home/socket/credentials; no-new-privileges; seccomp; cgroup and rlimit controls; independent timeout/kill; verify cleanup | **Validated (bounded T1).** Actual Linux x86_64 rootless runc 1.5.2 probes with delegated cgroup v2 and systemd user manager. T2/T3 and network allowlists/credential delivery fail closed. | Shared host kernel; malicious host root/administrator or host-UID process is outside boundary. T1 is not microVM isolation. |
| T-06 | Worker forges success, replays old evidence, swaps candidate/toolchain or delivers partial artifacts | Engine-owned check plans; exact snapshot/patch/environment bindings; persisted object readback/hash; scoped executor authority; idempotent delivery; reject stale/incomplete evidence | **Validated (bounded).** Evidence-v2 binding, independent persisted-object admission, actual OCI integration and lost-response/retry tests. No hardware attestation is claimed. | Engine authority is in-memory and embedding-owned. Local object storage is not distributed or administrator-proof. |
| T-07 | Crash, duplicate event, lost response or lease race causes double commit, stale execution or unsafe cleanup | Durable idempotency and atomic commit; explicit recovery state; independent runtime retention; exact cleanup grants; event outbox/reconciliation | **Partial.** Reference Engine idempotency and local execution ownership/recovery are tested. Goal 04 adds a checksum-checked PostgreSQL schema and immutable snapshot/semantic candidate catalogs with mapped-role RLS tests; Engine transaction state is still in-memory. Semantic candidate storage cannot install model heads or grant transaction authority. Distributed durable commit, multi-instance coordination and event publication are not implemented. | Process/store loss can lose in-memory transaction authority. No production distributed crash-recovery guarantee. |
| T-08 | Cross-workspace cache, index, event, artifact or log reference exposes another organization's data | Scope every identity and key; authorization on reads/writes; tenant-aware caches and storage; cross-scope adversarial tests | **Target only / partial contracts.** ADR-0004 and deployment specification define requirements; local Engine scope tests do not prove deployed multi-tenant isolation. | No public multi-tenant deployment or end-to-end isolation evidence. |
| T-09 | Secrets enter snapshots, model context, worker environment, logs or datasets | Credential admission, secret scanning/redaction, scoped secret references, least privilege, provider policy, access logging and revocation | **Partial.** Bounded credential detectors and exact digest-bound exceptions exist in capture. Managed secret service, policy decision service, secure model routing and credential delivery are not implemented. | Detectors cannot identify arbitrary embedded secrets or establish data-use rights. No automatic permission to send repository data to a model. |
| T-10 | Repository/trajectory data retained or used for training without rights, purpose, consent or deletion handling | Rights and purpose gates; data minimization; lineage; retention/deletion propagation; evaluation isolation; revocable training eligibility | **Target only.** Normative trajectory governance and ADR-0004 define controls; no production governance/retention service or training pipeline is claimed. | Jurisdictional legal requirements and model unlearning limitations require separate review. |
| T-11 | Toolchain/runtime substitution or supply-chain compromise changes execution semantics or breaks isolation | Pin runtime, entrypoint, formatter, libraries and control tools; verify origin/signatures and digests; bind image/profile generation into evidence | **Validated (bounded).** Goal 03 records exact runtime/formatter hashes and source-bound evidence; provenance checks are part of actual OCI validation. | Host kernel, systemd, runc supply chain and operator trust remain dependencies. Hash pinning alone does not prove upstream authenticity. |
| T-12 | Logs, diagnostics, artifacts or outputs disclose secrets/source or exhaust storage | Output limits; bounded artifact budgets; scoped storage; secret handling; retention and access policy; integrity checks | **Partial.** T1 output/artifact budgets and content-addressed local store are tested. Centralized access control, redaction pipeline, retention service and distributed backup controls are not implemented. | Host operators can access local data; arbitrary secrets may evade bounded detectors. |

## 6. Security invariants

The following are non-negotiable:

1. No model, worker, supervisor, client or event can grant itself authorization, reservation, verification-pass, executor or commit authority.
2. Actor identifiers are attribution, not authorization.
3. A parser success, LSP response, graph digest, small impact set or caller-supplied completeness flag is not proof of complete semantics.
4. Unknown or stale impact cannot be converted into safe impact by configuration convenience.
5. Repository inspection must not execute repository-controlled code.
6. Untrusted execution must not fall back to host execution when isolation prerequisites are absent.
7. Missing isolation observations, uncertain termination, incomplete cleanup or unpersisted required evidence cannot be reported as success.
8. A passing source verification result does not publish or commit source. Source publication and durable distributed commit remain separate capabilities.
9. Internal network location or internal user status does not bypass authorization, isolation, verification or audit controls.
10. Repository access does not imply permission for model processing, retention, evaluation or training.

## 7. Validation and change control

Security validation must include unit, integration, adversarial and fault-injection tests. Tests must preserve failing/unknown cases rather than converting them to warnings. Each newly discovered exploit or boundary failure becomes a regression fixture with a reproducible seed or artifact where possible.

Before changing a control, reviewers must identify:
- the threat IDs affected;
- the trust boundary and security invariant;
- whether the change alters authority, scope, isolation or evidence admissibility;
- the exact tests and platform/runtime profile that validate it;
- newly introduced residual risk.

Changes to accepted architectural security requirements require an ADR amendment. A documentation edit cannot mark a target control as implemented.

## 8. Current security posture

The repository has meaningful validated local controls for bounded capture, structural analysis, in-memory transaction admission, exact-base source patching, private materialization, Linux T1 execution, persisted local evidence and recovery.

It does **not** establish production readiness, public multi-tenant isolation, a production authentication/secret-management service, durable distributed transaction authority, T2/T3 isolation, a general language execution matrix, or end-to-end trajectory governance.

Use the [capability/support matrix](../reference/capability-support-matrix.md) for component-specific status and the [local Linux T1 runbook](../operations/local-linux-t1-runbook.md) for the exact supported operating profile.
