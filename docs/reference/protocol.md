# Protocol reference

**Status:** canonical TaskIntent v1 messages and strict Rust intake implemented; no RPC service
**Validation:** [Goal 04 foundation record](../implementation/goal-04-progress.md) (2026-11-11)
**Authority:** ADR-0001 and ADR-0002, read with ADR-0003 and ADR-0004

## Canonical source and consumers

Protocol Buffers v3 is the internal typed contract. The sole wire source is
[proto/celetixo](../../proto/celetixo). The initial schema preserves every
TaskIntent field and nested message from [ADR-0001 section 8](../adr/0001-celetixo-engineering-state-coordination.md#8-chief-agent-dsl), including field types, numbers and repetition.

| Contract | Implemented consumer | Scope |
|---|---|---|
| `celetixo.orchestration.v1.TaskIntent` | Generated Rust/prost bindings | Full task dependencies, reads/writes, invariants, impact/verification policy, budgets, retry/rollback/escalation and provenance |
| `celetixo.platform.v1.TaskIntentEnvelope` and `ScopeSelector` | Generated Rust bindings and `celetixo_protocol::decode_task_intent` | Contract version, organization/workspace/repository selector, complete intent, idempotency key, trace ID and required capabilities |

The smaller Rust Engine `TransactionIntent` remains an in-process domain admission
record. It is not a competing TaskIntent wire schema. No adapter currently executes
this wire request against the Engine. There are no gRPC services, deployed RPC
endpoints, Go/Python bindings, schema registry or workload-identity service in this
foundation. Their absence must not be hidden by successful message decoding.

## Strict intake and identity

The public intake caps wire data at 1 MiB and 32,768 fields. It checks wire shape
before prost decoding, using field metadata generated from the canonical descriptor.
Unknown fields at any nesting level, unknown versions/required capabilities,
duplicate singular fields, malformed/noncanonical varints, scalar overflow and
unknown sandbox tier values reject. Repeated scalar policy sets are bounded and
must not contain duplicates. All policy messages must be present; tier zero is
valid T0, not a missing-value error. Text/identity lengths, repository agreement,
self-dependencies, conflicting allowed/forbidden domains and nonzero time/attempt
budgets are checked.

These are syntactic and static checks. A tier request does not establish backend
support; the Engine and execution policy must still reject unsupported execution.
A decoded request remains untrusted and unauthorized. Scope IDs, actor provenance,
reads/writes, requested checks and model identity cannot grant permissions, prove
complete impact or establish evidence/commit authority.

`request_digest()` hashes a versioned, length-delimited encoding of the complete
TaskIntent plus scope, contract version and capability set. Set ordering and
Protobuf field order do not affect it. Trace ID and idempotency key are excluded:
the operation's authenticated scope/kind/key will select an idempotency record;
the semantic request digest detects a different request under that identity.
This crate does not implement that durable lookup. It does not bind an actual
Engine admission, patch or verification receipt until a later integration does so.

Strict consumers reject unknown future fields instead of allowing prost to discard
unknown safety semantics. A relay must preserve original binary bytes; it cannot
use this decoder to strip and retransmit an unknown newer request. Additive schema
compatibility does not imply that an older strict consumer accepts a newer message.

## Generation and compatibility

[proto/README.md](../../proto/README.md) documents regeneration. Checked-in Rust,
descriptor and golden wire artifacts are derived using pinned protoc 33.3 and
prost-build 0.14.4. Ordinary Rust builds do not execute an ambient protoc.
[Generation provenance](../../proto/generation.json) records source/output hashes,
compiler identity and generator lockfile digest.

The integrity check validates those artifacts and the accepted ADR fields. The
bounded descriptor checker compares against the checked-in v1 baseline and, in CI,
a published base descriptor when that Git revision contains one. Existing package,
file, message, field name/number/type/repetition/default/oneof semantics cannot be
removed or changed. V1 intentionally rejects removal even with reservations,
because mandatory TaskIntent semantics must survive. Future generations must still
reserve removed field numbers and names; never reuse their meaning. Enums/services
are not supported by this initial checker and require an explicit checker extension
before introduction. Wire checks cannot establish semantic compatibility by themselves.

CI includes integrity/ADR checks, negative compatibility mutations, pinned
regeneration and strict Rust intake tests. Golden fixtures exercise all ADR fields;
unknown nested fields, malformed payloads, version/capability rejection and request
identity mutations have regression coverage. This is message-level validation, not
an old/new deployed service or authenticated authorization test.

## Remaining service boundary

ADR-0001/0002 retain gRPC for synchronous commands/queries, NATS for asynchronous
events, Temporal for durable workflows and authenticated client APIs. Before any
such service is advertised, implement workload authentication, scoped authorization,
version/capability negotiation, typed Engine admission/evidence/results, bounded
operations and explicit error/retry/idempotency behavior. Future generated Go and
Python consumers must derive from this same canonical tree and join reproducibility
and compatibility tests. Neither workflow completion nor an event can decide commit.
