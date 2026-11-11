# Protocol Reference

**Status:** Reader reference; canonical wire schema is not yet implemented  
**Reviewed against:** `main` at `456bb375108532d9503c82fa8f6e7acfdfedddaf` (2026-11-11)  
**Architectural authority:** ADR-0001 and ADR-0002, read with ADR-0003 and ADR-0004

## 1. Current status: accepted semantics, no implemented wire source

Protocol Buffers v3 is the accepted target for Celetixo's internal typed cross-service contracts. The architectural TaskIntent example and required semantics are documented in [ADR-0001, section 8](../adr/0001-celetixo-engineering-state-coordination.md#8-chief-agent-dsl); ADR-0002 requires those semantics to be preserved.

The reviewed repository state does **not** contain:
- a canonical `.proto` schema source for `celetixo.orchestration.v1`;
- generated language bindings;
- a gRPC service definition or running RPC service;
- a schema registry or automated compatibility gate.

The ADR excerpt is therefore an architectural contract/example, not a compiled, versioned or deployed wire protocol. Do not generate production clients from a copied excerpt or describe the cross-service protocol as implemented.

The Rust `TransactionIntent` in `crates/engine` is explicitly an in-process Engine admission record. It is not the TaskIntent DSL and must not become a reduced replacement wire definition. Goal 01 states that there is no RPC service, cross-service schema, Brain or Chief DSL implementation in that milestone.

## 2. Architectural message semantics

The ADR-0001 example declares:

- Protobuf syntax: `proto3`
- Package: `celetixo.orchestration.v1`
- Primary message: `TaskIntent`

Its semantic fields are:

| Field number | Field | Meaning |
|---:|---|---|
| 1 | `task_id` | Task identity |
| 2 | `parent_task_id` | Parent/causal task identity |
| 3 | `repository_id` | Target repository identity |
| 4 | `base_revision` | Required base revision |
| 5 | `objective_description` | Human-readable objective |
| 6 | `explicit_reads` | Declared read set |
| 7 | `explicit_writes` | Declared write set |
| 8 | `invariant_contracts` | Invariants to preserve |
| 9 | `depends_on_task_ids` | Task dependency identities |
| 10 | `impact_policy` | Requested impact-analysis policy |
| 11 | `verification` | Verification requirements |
| 12 | `budget` | Resource budget |
| 13 | `retry_policy` | Retry/rebase behavior |
| 14 | `rollback_policy` | Failed artifact/patch preservation policy |
| 15 | `escalation_policy` | Escalation behavior |
| 16 | `provenance` | Model, prompt and agent provenance |

The nested example messages define impact depth/domain constraints, minimum verification tier and required checks, token/time/cost budgets, retry count and rebase permission, rollback artifact preservation, escalation target/count and model/prompt/agent version provenance.

This is a semantic summary of the ADR excerpt, not an independently authoritative schema. Consult the ADR for its exact illustrative field declarations.

## 3. Authority and security semantics

A TaskIntent requests work. It does not grant authority.

- The Engine derives transaction mechanics and impact from trusted repository state.
- Worker-provided reads/writes are explicit declarations, not proof of complete impact.
- Actor and model identifiers establish attribution, not authorization.
- Reservation, executor, verification and commit authority remain Engine/policy-owned.
- Verification requirements are requests to be evaluated against policy; a worker cannot self-certify a pass.
- Human approval, where required, must bind to the exact transaction, snapshot, diff and evidence.
- A stale base requires semantic revalidation; a force-rebase request never bypasses commit checks.

The schema must preserve these semantics without encoding a client-supplied field as an authorization grant.

## 4. Compatibility policy required before schema publication

When the canonical schema is introduced, it must have an explicit compatibility process. At minimum:

1. **Canonical ownership:** schema source files are the sole source of truth. ADR excerpts and generated documentation are references, not competing schemas.
2. **Package/versioning:** use versioned Protobuf packages for intentional breaking API generations. Do not change the meaning of an existing field in place.
3. **Field evolution:** never reuse removed field numbers or names; reserve both. Additive fields must have safe defaults and documented semantics.
4. **Semantic compatibility:** wire compatibility is insufficient if a change drops task dependencies, explicit reads/writes, invariants, impact policy, verification requirements, budgets, retry/rollback/escalation or provenance.
5. **Unknown fields/enums:** define forward-compatibility behavior. Consumers must not silently interpret unknown authority- or safety-relevant values as permissive defaults.
6. **Generated bindings:** generate from the canonical schema with pinned compiler/plugins. Commit generated output only under a documented policy and verify it is reproducible.
7. **Compatibility checks:** CI must compare schema revisions, reject field-number/name reuse and breaking changes outside an explicit new package/version, and compile all supported generated consumers.
8. **Protocol tests:** maintain golden wire fixtures, round-trip tests, unknown-field tests, old/new producer-consumer tests, malformed payload tests and authorization-boundary tests.
9. **Provenance:** record schema revision/digest and generator versions in builds and protocol-facing artifacts.
10. **Review:** schema changes affecting transaction authority or correctness require Engine/architecture review and, where architectural semantics change, an ADR.

These are requirements for future implementation. They are not claims that a compatibility checker, registry or generated binding pipeline currently exists.

## 5. Communication architecture: target

ADR-0001 and ADR-0002 define the following target separation:

| Mechanism | Intended responsibility | Current implementation status |
|---|---|---|
| Protobuf v3 | Canonical typed internal contracts | Target; no canonical schema source in reviewed tree |
| gRPC | Typed request/response service communication | Target; no Celetixo RPC service |
| NATS JetStream | Asynchronous event distribution | Target; no Celetixo event fabric |
| Temporal | Durable workflow execution, retries and compensation | Target; no Celetixo workflow deployment |
| HTTPS/JSON | External API/interoperability surface | Target; no public service asserted here |
| WebSocket / authenticated bidirectional channel | Interactive updates | Target; no deployed channel asserted here |

Do not collapse workflow orchestration into event delivery or treat event receipt as authoritative state transition.

## 6. Canonicalization implementation plan

The protocol is ready to become an implementation task only after the following are done:

1. Audit all existing domain types and ADR semantics; identify which fields belong in TaskIntent versus Engine-issued admission/evidence contracts.
2. Define the first canonical `.proto` source under a single repository-owned schema directory.
3. Define service boundaries and RPCs separately from messages; do not create speculative services solely to mirror the architecture diagram.
4. Specify authentication context and authorization references without treating caller-provided scope IDs as authority.
5. Add pinned code generation and reproducibility checks for each language actually consumed.
6. Add compatibility CI and golden/adversarial protocol tests.
7. Link the schema and generated bindings here only after they exist and pass validation.
8. Update the capability matrix and documentation index with exact supported consumers and validation scope.

## 7. References

- [ADR-0001, Chief Agent DSL and communication boundaries](../adr/0001-celetixo-engineering-state-coordination.md)
- [ADR-0002, protocol and platform extensions](../adr/0002-platform-brain-client-and-correctness-extensions.md)
- [Goal 01 implementation contract](../implementation/goal-01.md)
- [Goal 03 acceptance record](../implementation/goal-03-acceptance.md)
- [AGENTS.md](../../AGENTS.md), which prohibits competing protocol definitions and requires canonical-schema-first evolution
- [Capability/support matrix](capability-support-matrix.md)

Until a canonical schema and compatibility pipeline are implemented, describe the protocol as **architecturally specified, not implemented**.
