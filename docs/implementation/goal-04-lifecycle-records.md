# Goal 04 lifecycle records, exact requests and schema upgrade

**Status:** 04.1e prerequisite; lifecycle authority remains in the reference
Engine's memory. This does not complete 04.1 or distributed commit milestone 04.2.

**Authority:** [Goal 04](goal-04.md), ADR-0003, the
[identity contract](../spec/engineering-identity.md) and
[failure model](../spec/failure-model.md).

## Checked immutable values

The Engine now exposes bounded reconstruction of native intent, access sets,
reference patches, impact results, verification evidence and explicit commit
requests. These values supply immutable payloads for normalized lifecycle rows;
there is no whole-Database encoding or public API that restores transaction
or capability authority.

| Format | Binding and validation |
|---|---|
| `celetixo/native-intent-record/v1` | Scope and original snapshot; every native `TransactionIntent` field, including explicit accesses, ordered invariants/check definitions and transaction dependencies. Existing intent validation rejects missing objectives/checks and duplicate identities. |
| `celetixo/access-set/v1` | Existing identity and bytes, now available for checked reconstruction; reads, writes, invariant scopes and predicates remain distinct. |
| `celetixo/patch/v1` | Existing identity and bytes; exact base snapshot, before images, additions/deletions and full signed integer range. Existing patch validation rejects empty/no-op changes. |
| `celetixo/impact-analysis-record/v1` | Scope, snapshot, external model generation, complete analyzer stamp, derived accesses and explicit completeness/reasons. Original impact identity is unchanged. |
| `celetixo/verification-evidence-record/v1` | Scope/base snapshot, all original attempt/check/provenance/result bindings, diagnostics and full binary artifacts. The envelope has a separate content address; original evidence identity is also recomputed and checked. |
| `celetixo/commit-request/v1` | Scope, independently authenticated principal, transaction/attempt, positive grant/fence, exact base/candidate, patch, analysis, original intent and selected evidence map. The external idempotency key selects an outcome and is excluded from semantic identity. |

Each decoder checks the supplied domain-separated digest, exact field types,
validated identities/constructors, required external context and complete
canonical re-encoding. Duplicate/reordered sets/maps, trailing bytes, unknown
tags and invalid values reject even when the caller supplies a matching hash.
Shared limits bound encoded bytes, each field and aggregate nested collection
entries. Defaults are 64 MiB for bytes/fields and 100,000 collection entries.
These limits do not make an unbounded caller-controlled SQL result set safe;
scoped state loading still requires its own aggregate resource budget.

The native intent is the Goal 01 reference value. It does not replace the complete
canonical Protobuf `TaskIntent` or discard its budgets, policies and provenance at
a service boundary. Decoded evidence is data: current scoped verifier authority
and the Engine evidence gate remain necessary. An incomplete impact remains
incomplete after reconstruction.

Four frozen fixtures were encoded independently in Python from the specified
length-delimited layouts, without invoking the Rust encoder/decoder. Their
[metadata](../../crates/engine/tests/fixtures/lifecycle-codecs.json) records exact
addresses and file hashes. New layouts are versioned independently of unchanged
Goal 01 patch/access/evidence identities.

## Exact native commit requests and fencing

`prepare_commit_request` requires the current opaque session, transaction owner
and live lease. It prepares data rather than granting commit authority.
`commit_prepared` enters the existing Rust commit gate and rechecks the request's
attempt, snapshot, patch, analysis, intent, evidence selection and fence. The
current session must hold Commit permission and match the request's grant and
transaction principal before looking up an outcome.

Resolved outcomes retain the original request body and exact attempt. Exact
historical replay returns the same admitted outcome after evidence replacement,
revalidation or lease expiry, subject to current authorization. It does not
advance revision again. Reusing a key with different semantic content rejects.
Malformed or unauthorized requests do not reserve the owner's key. A fresh
request/key is required after changing evidence or revalidation inputs.

Reference reservations allocate a positive monotonically increasing fence per
repository, only after conflict checks succeed. Renewal retains it. Disjoint
leases can hold different live fences; allocating a newer disjoint fence does
not invalidate older valid work. Cancellation/replacement cannot revive an old
fence. Overflow and conflict leave the counter, lease and acquisition history
unchanged. Lease events expose the fence for attribution; IDs/views never
construct opaque authority.

Sessions and executor capabilities now bind their immutable scope explicitly.
Revocation also checks stored scope/principal/backend metadata, and transaction
views retain the original proposal grant separately from a later lease grant.
Cleanup revocation now uses the explicit update/publication callback; it was a
remaining direct state lock after 04.1d. Storage-reader registration/revocation
retain their coupled state/runtime boundary.

The older `commit` entry point remains for Goal 01 harness compatibility; it does
not have the full explicit-request contract. A future durable profile must
require exact prepared requests and must not use that legacy path to bypass
request binding. Neither request replay nor reference fences currently survive
Engine process loss. Database-clock expiry, scoped grant resolution, persisted
reservations and multi-process atomic commit remain pending.

## Schema v2 and supported upgrade

[Migration 0002](../../crates/postgres/migrations/0002_lifecycle_reconstruction.sql)
is additive. Published migration 0001 and its checksum remain unchanged. It:

- admits the Engine's zero-based attempt ordinal without translating its identity;
- retains a bounded transaction reason and original operation request payload;
- defines scoped Engine counters and model-head metadata with forced RLS;
- binds the model head to the current repository-head snapshot through a deferred
  foreign key, allowing atomic updates in either order;
- rejects partial source-model head metadata and cross-scope payload substitution.

These are SQL constraints and future reconstruction metadata, not installed Rust
lifecycle authority or evidence of a source commit capability.

The migration registry accepts only an exact contiguous supported prefix by
version, name and checksum. Empty databases apply v1 and v2 together. Existing
v1 databases apply only v2. All missing DDL and ledger rows publish in one
advisory-lock-serialized transaction; a failure or child process death before
commit leaves the previous schema/data/ledger intact. Current catalog operations
require the complete supported history; partial, missing, changed or future
history fails closed. Commit acknowledgement failure remains indeterminate.

Operate upgrades as trusted maintenance: stop affected applications, take and
verify an operator backup, apply `migrate`, validate the complete schema, then
reopen using the matching binary and recheck role privileges. New tables have no
public grants. The existing runtime login must receive only its reviewed
additional privileges. There is no automatic downgrade. If rollback is required,
restore the verified pre-upgrade backup and matching binary; do not edit an
applied SQL file or forge a ledger entry. A backup/restore drill and live
old-connection fencing have not been implemented or validated by this milestone.

An earlier unpublished local v2 draft had already been applied during development.
Its database and ledger were preserved. Final validation used a separate fresh
database for the final v2 bytes; no applied checksum was rewritten to conceal
that difference.

## Evidence and remaining boundary

[Validation](goal-04-lifecycle-validation/checks.json) and its
[raw log](goal-04-lifecycle-validation/checks.log) bind exact source, tools and
commands. Codec/request tests cover frozen layouts, full numeric/binary values,
truncated/malformed/rehashed/noncanonical input, resource budgets, incomplete
analysis, revoked evidence authority, sticky rejection and exact lost-response
replay. Reference tests cover scope aliases, original proposal attribution,
fence exhaustion/conflict/cancellation and revalidation. Real PostgreSQL tests
cover empty/v1 upgrade, fault rollback, actual child death, data preservation,
history skew, forced RLS and deferred head constraints.

A frozen-fixture check initially failed because the independent Python encoder
used a string completeness tag where the new impact-record specification uses a
numeric tag. Only that unpublished fixture was corrected; the Rust format and
its strict comparison were retained. Final source-bound checks supersede the
failed development run.

Normalized mutable-state reconstruction, typed outcome codecs, authenticated
capability resolution, SQL lifecycle execution, an authoritative database clock,
durable fences/idempotency/outbox and independent Engine-process races are still
required. NATS, Temporal, S3 and ClickHouse integration and operational proof also
remain pending. Development test elapsed times are not performance baselines,
capacity claims or production SLOs.
