# Deployment and Product Boundaries

**Status:** Normative architecture specification  
**Authority:** ADR-0004, read with ADR-0001, ADR-0002 and ADR-0003  
**Purpose:** Define how Celetixo can be operated internally first without creating an internal-only architecture or weakening its future public operating boundary.

This specification defines required contracts and validation expectations. It does not assert that the described mechanisms have already been implemented.

## 1. Governing principle

Celetixo is built to its complete accepted target architecture and operated internally first under Emilo Labs' data-governance policies.

Internal-first changes the initial operator, deployment configuration, approved integrations and data policies. It does not lower standards for correctness, security, isolation, scalability, reliability, observability, testing or performance. It does not create a reduced internal edition.

Public productization may stage customer-facing surfaces. It must not require a fundamental redesign of the Engine, transaction model, identity scopes, authorization boundaries, evidence model or execution isolation.

## 2. Separate the platform from its initial deployment

The architecture distinguishes these concerns:

| Concern | Responsibility | Initial operating posture |
|---|---|---|
| Core Engineering State Engine | Snapshot identity, semantic state, transaction admission, OCC, conflict analysis, evidence admissibility and commit authority | Full target contract |
| Brain and model integration | Provider adapters, routing, context assembly, planning, agent reasoning and evaluation | Full provider-neutral contract; Emilo Labs-approved providers and policies |
| Control plane | Authenticated APIs, review, administration, policy application and operational visibility | Internal operator/client access |
| Orchestration and event fabric | Durable workflows, task delegation, events, retries, cancellation and recovery | Full distributed semantics |
| Execution infrastructure | Isolated worker execution, resource controls, scoped credentials and evidence capture | Full security and isolation requirements |
| Data and evidence plane | Authoritative state, analytics, immutable artifacts, lineage, retention and deletion | Emilo Labs' initial data policies |
| Product experience | Onboarding, organization administration, billing, support and customer-facing workflows | May be staged until public release |

This separation is a logical ownership model. It does not require creating a separate microservice for every row. Physical deployment boundaries must follow the accepted architecture, threat model, operational needs and measured performance.

## 3. Resource identity and scope

### 3.1 Required scopes

The platform must represent the following scopes explicitly where applicable:

- **Organization:** an administrative and governance boundary.
- **Principal:** a human, service, agent, workload or other authenticated actor.
- **Workspace:** an independently governed engineering context within an organization.
- **Repository:** a versioned source and engineering-input boundary.
- **Snapshot:** an immutable identity for the exact repository/workspace input state.
- **Transaction:** an attempted state transition bound to a repository and snapshot.
- **Execution:** a bounded workload performed under an authenticated task and policy context.
- **Evidence/artifact:** a provenance-bearing record with access and retention policy.
- **Integration/credential:** an external connection with explicit owner, scope, purpose and revocation state.

The identity model in [engineering-identity.md](engineering-identity.md) remains authoritative for engineering identities. This specification adds organization, principal and governance scope; it does not redefine snapshot or transaction identity.

### 3.2 Scope propagation

Every request, workflow, event, execution and persisted record that crosses a trust boundary must carry or resolve an authenticated scope context. At minimum, the context must identify:

- authenticated principal and principal type;
- organization and workspace scope;
- requested repository and transaction where relevant;
- authorization decision or a verifiable reference to the policy evaluation;
- purpose and applicable data-handling policy;
- trace/correlation identity and provenance.

A scope identifier supplied by a client, model or worker is a selector, not proof of authority. The receiving boundary must authenticate the caller and authorize the requested action against authoritative policy state.

Scope context must survive delegation, retries, workflow replay, event delivery and artifact retrieval. It must not be reconstructed from untrusted prompt text or inferred from a worker's network location.

### 3.3 No global singleton assumptions

Core contracts must not assume one organization, user, workspace, repository, provider, policy, credential set or retention schedule. A deployment may configure one organization initially, but the model must preserve explicit scope and ownership.

## 4. Authorization and trust boundaries

Authorization is enforced at service boundaries and at the authoritative Engine boundary for repository-state operations.

Requirements:

1. Authenticate principals independently of their claimed identifiers.
2. Authorize actions using organization/workspace/repository scope, role or capability, resource state and applicable policy.
3. Apply least privilege and deny by default.
4. Separate attribution from authorization. Actor identity in a task or event does not grant permission.
5. Treat agents, generated code, tool output, repository content and external integrations as untrusted inputs.
6. Make privileged actions explicit, narrowly scoped, attributable and auditable.
7. Re-evaluate authorization when relevant policy, membership, credential or resource state changes; do not rely indefinitely on stale grants.
8. Fail closed when a required authorization decision cannot be established.

The Engine remains the final authority for transaction admissibility and commit. Control-plane approval, agent confidence, workflow state or UI state cannot independently authorize a repository mutation.

## 5. Isolation domains

Isolation is end-to-end. It must be considered across all planes, not only the API or database.

| Domain | Required isolation properties |
|---|---|
| API and clients | Authenticated requests; scoped resource lookup; no cross-scope enumeration or disclosure |
| Repository state | Scoped snapshots, worktrees, indexes, graph data and patch application |
| Transactions | Scoped intent, reads/writes, reservations, leases, conflict records and commit operations |
| Brain/context | Scope-aware retrieval; policy filtering before model transmission; no cross-workspace context leakage |
| Orchestration | Scoped workflow IDs and authorization on task, signal, query, cancellation and replay paths |
| Event fabric | Subject/stream permissions; authenticated publishers/consumers; tenant scope preserved in event envelopes |
| Execution | Isolated filesystem/process/network; scoped short-lived credentials; resource quotas; no ambient host authority |
| Evidence and artifacts | Scoped object access; immutable provenance; policy-aware export, retention and deletion |
| Analytics | Explicit data classification and access; controlled aggregation; prevent unauthorized row-level disclosure |
| Cache | Scope and complete correctness-relevant inputs included in cache identity; no cross-scope reuse unless explicitly proven safe |
| Backup and recovery | Scope-aware access, retention and restoration; no policy bypass through backup copies |
| Operations | Privileged access control, audited break-glass procedures and separation of duties where required |

Namespace prefixes alone do not establish isolation. Isolation claims require tests against the actual storage, service, event, execution and operational paths.

Shared compute or storage may be used only when the isolation controls are appropriate to the threat model and validated. Stronger physical separation may be required for specific data classes, customers or deployment environments.

## 6. Data governance and model use

### 6.1 Policy dimensions

The policy model must be able to express, at appropriate scope:

- data classification and sensitivity;
- permitted processing purposes;
- allowed model/provider destinations;
- residency and processing-location constraints;
- retention duration and deletion obligations;
- access, export and sharing permissions;
- logging and observability restrictions;
- trajectory storage and evaluation eligibility;
- training eligibility and consent/rights conditions;
- legal hold or other retention exceptions where applicable.

The initial effective policy is supplied by Emilo Labs. Policy is not inferred from the fact that a deployment is internal.

### 6.2 Enforcement points

Policy must be checked before data crosses a relevant boundary, including:

- retrieval into Brain context;
- transmission to an external model/provider;
- dispatch to a worker or execution environment;
- persistence into logs, analytics or trajectory storage;
- artifact access, export or sharing;
- retention expiry, deletion and backup lifecycle operations.

A policy decision must be attributable to the policy version or immutable policy reference that governed the operation when that is material to audit or reproducibility.

If a required policy decision is missing, unavailable or indeterminate, the operation must fail closed or enter an explicitly defined safe state. It must not silently fall back to a broader policy.

### 6.3 Training and evaluation separation

Data captured for operations, debugging, evaluation or trajectory reconstruction is not automatically eligible for model training. Follow [trajectory-governance.md](trajectory-governance.md) for rights, consent, purpose limitation, quality, contamination, lineage, retention and deletion.

The Brain must apply provider and purpose restrictions before assembling or transmitting context. Model adapters must not silently retain or repurpose data beyond the governing policy and provider contract.

### 6.4 Retention and deletion

Retention and deletion policies must account for primary records, replicas, indexes, caches, object artifacts, analytics, exports and backups. The system must define deletion propagation, verification evidence, legal exceptions and the limits of deletion from immutable or append-only records.

Where immutable audit records must remain, sensitive payloads should be separated from minimal tombstone/provenance records so that deletion obligations can be met without destroying necessary integrity evidence. The exact mechanism must be documented and tested before a deletion guarantee is claimed.

## 7. Credentials and external integrations

Credentials must have an explicit owner, purpose, scope, expiry/rotation policy and revocation path. Prefer short-lived credentials and scoped tokens. Secrets must not appear in:

- model prompts or context;
- task intent or general event payloads;
- source snapshots or patches;
- logs, analytics or trajectory content;
- worker images or persistent execution artifacts.

Workers receive only the minimum task-scoped capability required, through a controlled secret-delivery boundary. Production credentials are not exposed to untrusted generated code.

Repository, identity, model-provider and other integrations must use replaceable adapters with explicit contracts for authentication, authorization, scopes, rate limits, retries, revocation, error handling and audit. Initial integrations may be selected and administered internally without embedding their vendor-specific assumptions in the Engine.

## 8. Internal deployment requirements

The initial Emilo Labs deployment must:

1. use the accepted Engine transaction and verification path without internal bypasses;
2. use explicit organization, principal, workspace and repository scopes;
3. enforce authentication and authorization for human, service and agent principals;
4. apply Emilo Labs' approved data classification, provider, residency, retention, deletion and access policies;
5. use scoped credentials and isolated execution for untrusted workloads;
6. preserve immutable snapshots, transaction provenance and evidence;
7. exercise multi-workspace and cross-scope denial tests even if all workspaces belong to one organization;
8. maintain operational monitoring, backup/restore and incident handling appropriate to the deployment;
9. record which capabilities are architectural targets, implemented and validated separately.

"Internal" is not a trusted execution tier. An internal agent, repository, dependency or model-generated patch remains subject to the threat model and controls appropriate to its capabilities.

## 9. Public productization boundary

The following product surfaces may be implemented when the public release plan requires them:

- self-service registration, invitations and organization onboarding;
- customer-facing membership, role and workspace administration;
- customer-managed repository/provider connections and credential workflows;
- plan management, billing, invoices, usage views and quotas;
- customer support, service status and incident communications;
- customer-facing policy configuration and data export/deletion workflows.

The foundations that must precede those surfaces include scoped identities, authorization contracts, policy evaluation, integration interfaces, usage attribution where relevant, audit provenance, data lifecycle controls and end-to-end isolation.

Do not build a billing UI merely to claim public readiness. Conversely, do not defer usage attribution or resource accounting if the platform architecture needs those records to enforce quotas, investigate abuse or later calculate usage accurately.

## 10. Public release readiness gates

A public release requires explicit evidence for each applicable gate:

| Gate | Minimum evidence |
|---|---|
| Identity lifecycle | Tested registration/invitation, authentication, membership changes, revocation and account recovery |
| Authorization | Positive and negative tests for every resource scope and privileged action |
| Tenant/workspace isolation | Adversarial cross-scope tests across API, data, caches, indexes, events, workflows, execution and artifacts |
| Repository integrations | Tested least-privilege grants, token rotation/revocation, webhook validation and failure recovery |
| Model/provider governance | Tested provider allowlists, purpose restrictions, residency constraints and context filtering |
| Data lifecycle | Tested retention, deletion propagation, export, backup handling and auditable exceptions |
| Execution security | Threat-model review, sandbox escape testing appropriate to the environment, resource exhaustion and network policy tests |
| Reliability | Load, fault-injection, recovery, backup/restore and upgrade/migration evidence |
| Abuse and quotas | Rate/resource controls, cost attribution, abuse detection and operator response |
| Operations | Monitoring, alerting, incident response, access review and break-glass audit |
| Commercial controls | Metering and billing reconciliation if paid plans or usage charges are enabled |
| Legal and support | Reviewed privacy notices, terms, customer agreements, support and jurisdiction-specific obligations |

These are release gates, not claims about current implementation. The responsible release authority must record accepted risks, evidence, limitations and the exact operating envelope.

## 11. Validation strategy

Tests must validate boundaries, not merely helper functions.

Required classes include:

- principal cannot access another workspace's repository, transaction, evidence or artifact by changing an identifier;
- revoked membership or credential cannot authorize a new action;
- an agent cannot escalate authority by modifying task metadata or event scope;
- workflow replay, retry, signal and cancellation preserve authorization scope;
- event publishers and consumers cannot cross unauthorized subjects or streams;
- cache hits cannot return data from another scope or from an incompatible policy/version;
- Brain retrieval and provider routing reject disallowed data destinations before transmission;
- worker filesystem, network and credentials remain within task scope;
- analytics and logs enforce the intended data classification and access policy;
- deletion/retention behavior covers derived indexes, artifacts, exports and backups according to the documented policy;
- internal and future public operating modes use the same transaction, verification and commit boundary;
- policy-service outage or indeterminate policy state fails closed for protected operations.

Run adversarial tests against the deployed integration of each boundary. A passing unit test for a policy helper does not establish end-to-end isolation.

## 12. Status and claims discipline

For each requirement, distinguish:

- **Target:** required by accepted architecture.
- **Implemented:** code and configuration exist.
- **Validated:** relevant tests or operational evidence establish the stated behavior.
- **Released:** the capability is enabled within a documented operating envelope.

Do not claim multi-tenant isolation, public readiness, regulatory compliance, secure deletion, sandbox containment or production resilience solely because this specification requires it.

A capability may be intentionally unavailable in the internal product surface while its foundational contract is already implemented. Conversely, a documented target is not an implemented capability.

## 13. Change control

Changes that weaken product-independent core boundaries, scope propagation, authorization, data governance, execution isolation or release gates require an ADR amendment. Implementation details may evolve when the requirements and accepted correctness model remain intact.

This specification is subordinate to accepted ADRs. If a conflict is found, stop and resolve it through an ADR rather than silently changing either contract.
