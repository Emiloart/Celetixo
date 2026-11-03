# ADR-0004: Internal-First Operation and Public-Capable Architecture

- **Status:** Accepted
- **Date:** 2026-11-03
- **Depends on:** ADR-0001, ADR-0002 and ADR-0003
- **Decision scope:** Initial operating model, data governance boundaries, deployment posture and future productization
- **Relationship to prior decisions:** Additive. This ADR does not replace, relax or defer any accepted engineering, correctness, security or platform requirement.

## Context

Celetixo will initially be operated for internal use by Emilo Labs. The initial deployment will therefore run under Emilo Labs' data-governance policies, operational control and approved infrastructure.

"Internal-first" describes who initially operates the platform and which data policies govern that deployment. It does **not** describe a reduced engineering edition, prototype-grade security posture, temporary correctness model, restricted target architecture or permission to defer foundational capabilities.

The platform is intended to be capable of later public productization. That future may introduce external organizations, customer-managed repositories and credentials, organization administration, configurable governance, usage metering, billing, support and additional deployment options. Those product surfaces need not all be exposed in the initial internal deployment. Their absence must not force a redesign of the core platform or justify hardcoded single-organization assumptions in foundational contracts.

The architecture already establishes correctness-critical responsibilities across the Rust Engineering State Engine, Python Brain, orchestration, event fabric, storage, clients and isolated execution. Those boundaries apply equally to internal and future public operation.

## Decision

Celetixo will follow an **internal-first, full-standard, public-capable** operating strategy.

1. Build and validate the complete accepted target architecture to the highest technically justified standard. Build sequencing may be staged around dependencies and evidence; engineering quality, security, correctness and intended capability must not be reduced to meet an arbitrary date or to simplify internal deployment.
2. Initially operate the platform under Emilo Labs' data-governance policies and operational control.
3. Keep core Engine semantics and platform contracts product-independent. Initial deployment policy is configuration and deployment composition, not a separate internal-only implementation.
4. Establish explicit organization, workspace, repository, principal, authorization and data-governance boundaries in the architecture and resource model. Do not assume that one initial organization means global singleton ownership is safe.
5. Apply the same transaction admission, immutable snapshot, semantic impact, verification, sandbox, evidence, authorization and atomic commit path to internal work that would apply to public workloads.
6. Defer public-facing product surfaces until required by the release plan. Deferral of a surface does not authorize deferral of the foundational security, isolation, identity, policy or extensibility contracts on which it depends.
7. Treat public release as a productization, operational-readiness and governance milestone of the same platform, not as a second architecture or a rewrite.

## Scope boundaries

### Build to full standard now

The following remain part of the target architecture and must not be downgraded because initial use is internal:

- Engineering State Engine authority, transaction consistency, OCC, conflict analysis, revalidation and atomic commit admissibility.
- Immutable workspace snapshots and complete provenance for relevant repository inputs.
- Evidence bound to exact transactions, snapshots, patches, checks and execution environments.
- Strong isolation for untrusted generated code, dependencies and worker execution.
- Authentication, authorization, least privilege, credential scoping, auditability and explicit trust boundaries.
- Distributed operation, concurrent workers, multiple repositories and independent workspaces.
- Provider-neutral Brain and replaceable model/provider integrations.
- Durable orchestration, event delivery, idempotency, backpressure, cancellation and crash recovery.
- Data lineage, retention/deletion mechanisms, privacy controls and trajectory governance.
- Comprehensive correctness, security, failure-injection, performance and comparative evaluation.
- Desktop and mobile clients that use the authoritative platform APIs and do not become alternate transaction authorities.

### Initial deployment policy

The initial deployment may be configured for Emilo Labs' internal users, infrastructure, approved model providers, repository connections, data residency and retention rules, operational access and governance requirements.

These are policy and deployment choices. They must be represented at explicit boundaries and must not be scattered as hardcoded assumptions through the Engine or canonical contracts.

### Public product surfaces that may be staged

Subject to product and operational readiness, the initial internal deployment need not expose:

- public self-service registration and organization onboarding;
- customer-facing organization and tenant administration;
- customer-managed billing, plans, invoices and payment workflows;
- public support, service-status and customer incident interfaces;
- generalized customer-facing integration catalogs and self-service credential management;
- public marketing, distribution and marketplace surfaces.

Deferring these interfaces does not permit deferring tenant-aware identity and authorization boundaries, scoped integration contracts, usage attribution, governance enforcement points, or secure credential handling where those are foundational to the platform.

## Architectural requirements

### Product-independent core

The Engineering State Engine must not encode Emilo Labs as a privileged global owner or assume one organization, one repository, one workspace, one operator or one policy. It consumes authenticated, authorized, scoped identities and policy decisions through explicit contracts.

The Brain, orchestration and execution systems must preserve those scopes across task creation, delegation, events, retries, storage, evidence and worker execution. Context propagation must be authenticated and integrity-protected; a caller-supplied organization or tenant identifier is not authorization.

### Organization and workspace isolation

Organization, workspace and repository boundaries must be explicit in resource identity and authorization. Isolation must cover, as applicable:

- API and client access;
- repository snapshots, indexes, caches and worktrees;
- transaction records, reservations and commit operations;
- model context, prompts, retrieval and trajectory data;
- execution workers, temporary files, network access and credentials;
- evidence, logs, analytics and object storage;
- event subjects, workflow identifiers and operational tooling;
- backups, exports, retention and deletion.

Shared infrastructure is acceptable only where the isolation mechanism is explicit, enforceable, testable and appropriate to the threat model. Namespace separation alone is not proof of isolation. No cross-organization data exposure may be treated as an acceptable performance trade-off.

The initial single-organization deployment must exercise the same scoped authorization and isolation mechanisms. It must not create a privileged bypass that later has to be removed.

### Data governance and policy

Data governance is deployment-configurable and enforceable. The initial policy set is controlled by Emilo Labs. The architecture must support policy decisions for data classification, purpose, access, model/provider use, retention, deletion, export, audit and trajectory eligibility.

Storage does not imply permission to use data for model training or evaluation. Trajectory governance remains governed by [trajectory-governance.md](../spec/trajectory-governance.md). Provider routing must respect data-use and residency policy before context is transmitted.

Policy changes must be versioned or otherwise attributable where they affect authorization, processing, retention or evidence admissibility. Policy enforcement must fail closed for required decisions when policy state is unavailable or indeterminate.

### Credentials and integrations

Credentials are scoped to an organization/workspace/repository or another explicitly defined principal and purpose. They must be stored and delivered through a managed secret boundary, never embedded in prompts, general task payloads, source snapshots or untrusted worker environments.

Repository and model-provider integrations use replaceable adapters with explicit authorization, scope, audit, revocation and failure behavior. Internal configuration may select Emilo Labs-controlled integrations without hardcoding those providers into core Engine semantics.

### No internal fast path

Internal identity or network location does not automatically confer transaction authority. No internal-only path may bypass:

- authenticated principal and authorization checks;
- immutable snapshot and provenance requirements;
- semantic impact and conflict analysis;
- evidence admissibility and required verification;
- sandbox and resource-isolation policy;
- atomic commit validation and revision advancement;
- audit and governance controls.

Privileged operator actions must be explicit, authorized, scoped, auditable and subject to the same repository-state correctness boundary.

## Public-release readiness

Public release requires a separately reviewed readiness decision. At minimum, the release gate must establish:

1. organization/workspace identity lifecycle and authorization enforcement;
2. isolation tests across API, repository state, indexes, caches, workflows, events, execution, evidence, analytics and backups;
3. secure external repository and model-provider integration, credential management and revocation;
4. enforceable data classification, processing, residency, retention, deletion, export and trajectory-use policies;
5. abuse resistance, resource quotas, rate limits and fair-use controls;
6. usage attribution and billing correctness if commercial metering is introduced;
7. operational monitoring, incident response, backup/restore, disaster recovery and upgrade/migration procedures;
8. security assessment and documented threat-model review;
9. support, privacy, contractual and legal policies appropriate to the jurisdictions and customer segments served;
10. reproducible correctness, reliability and performance evidence for the released operating envelope.

Passing this list is not implied by the architecture. Each item requires implementation evidence and an explicit release decision. Public release must not be represented as ready merely because the internal deployment works.

## Consequences

### Positive

- Internal operation can begin under controlled data policies without creating a disposable architecture.
- Correctness and security guarantees remain consistent across internal and future public use.
- Product-specific features can be sequenced independently from foundational platform capability.
- Organization and workspace scopes are designed in before external customers depend on them.
- Future productization becomes a controlled extension and operational-readiness exercise.

### Costs and obligations

- The architecture must account for scoped identity, policy propagation and isolation from the beginning.
- Single-organization deployment still requires meaningful authorization and isolation tests.
- Some public-release requirements will need separate implementation, operational evidence and governance review.
- Policy configuration and versioning add complexity that must be justified and tested.

These costs are accepted. They are necessary to avoid a platform whose internal success depends on assumptions that fail when the operating boundary expands.

## Rejected interpretations

### Internal prototype with deferred correctness

Rejected. Internal use does not reduce the threat posed by untrusted generated code, concurrency, stale state, compromised credentials or accidental data exposure.

### Separate internal and public architectures

Rejected. The initial deployment and future public product must use the same authoritative transaction and verification path.

### Build every public-facing feature immediately

Rejected. Public onboarding, billing and customer-facing operations can be sequenced to actual release needs. Their foundations and security boundaries cannot be omitted where they affect core contracts.

### Hardcode one organization throughout the platform

Rejected. Initial ownership is an operational fact, not a valid reason to make organization identity a global singleton or to prevent independent workspaces.

## Implementation and change control

The requirements are operationalized in [deployment-and-product-boundaries.md](../spec/deployment-and-product-boundaries.md). That specification defines the resource scopes, policy enforcement points, isolation domains and release-readiness evidence.

This ADR does not claim those capabilities are already implemented. Architecture, implementation and validation status must remain distinct. Any proposal to weaken the requirements in this ADR requires an explicit ADR amendment supported by technical evidence and reviewed against the accepted correctness and security model.

> **Internal-first governs initial data policy and operational control. It does not lower the engineering standard or narrow the platform's intended capability.**
