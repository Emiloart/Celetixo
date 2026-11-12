# Goal 04 checked authority reconstruction and backend boundary

**Status:** 04.1f prerequisite. The native reference lifecycle has checked
reconstruction and a trusted backend interface. No PostgreSQL implementation of
that interface exists. Milestone 04.1 and Goal 04 remain incomplete.

**Authority:** [Goal 04](goal-04.md), ADR-0003, the
[identity contract](../spec/engineering-identity.md),
[failure model](../spec/failure-model.md) and
[previous lifecycle contract](goal-04-lifecycle-records.md).

## Scoped engineering facts

`AuthorityState::from_native_records` reconstructs one scope from typed normalized
facts: grants, repository/model head, registered snapshots, all attempts,
provenance, original proposal grant, reservations/events, admitted evidence,
original prepared requests, typed historical outcomes and committed history.
`native_records` exports these facts. There is no whole-Database byte encoding,
serialized mutex, persisted capability allocation identity or reader object.
The public interface belongs to trusted storage integration, never workers.

Reconstruction checks exact scope and associations, contiguous revisions and
zero-based attempts, consistent model generations, registered proposal bases,
patch before-images/candidates, committed receipts/evidence/dependencies and
original request/outcome binding. It recomputes analysis and effective accesses
through the existing Rust implementation and original intent/invariants.
Declared invariant scopes alone do not imply additional reads. Incomplete models
retain their uncertainty and cannot authorize a complete analysis.

Candidates remain private attempt values until committed. Evidence selected by an
old request is retained after replacement/revalidation. Historical revoked grants
remain facts; reopening them does not grant current authority. A new grant for the
same verifier principal can freshly admit identical evidence through the existing
gate, while the original request retains its original evidence selection.

Lease facts retain acquisition time, grant and attempt independently of the
current attempt. History is checked against acquisition, writes, expiry and
terminal release. The trusted adapter may advance an authority clock through the
shared expiry logic; a read-only observation does not invent persisted expiry
events. Backward observations fail. These helpers do not select a database clock
or provide durable expiry by themselves.

The current bounded profile accepts the native reference model with initial
revision zero. Captured source state, source-execution/evidence resources and
legacy operations without explicit original requests reject explicitly. This is
an integration stage of the same Engine, not an alternative semantic service or
full recovery of Goal 03 execution. Default aggregate budgets are 100,000 nested
entries, 256 MiB of accounted canonical values and 64 MiB per value. Loaders must
also bound SQL result sets, allocations and execution before constructing these
records; these checks are not a streaming loader or scalability benchmark.

## Authentication and explicit publication

`Administrator::authenticate_existing_session` requires fresh authentication of
the exact principal and scope by the trusted embedding. Authentication runs
outside state callbacks. The grant is checked again afterward for revocation or
scope/principal change, then a fresh private allocation-bound `Session` is issued.
An old embedding's session and a numeric grant ID cannot authenticate themselves.
There is no production identity provider in this milestone.

`Engine::with_authority_backend` selects a trusted `AuthorityBackend` explicitly.
Native operations carry private Engine-created scope, actor, permission and
read/update/administrative context. Actor metadata is attribution, not authority;
the existing Rust callback still checks the opaque session and live scoped grant.
Administrative writes require a separately authenticated storage role in a future
SQL implementation. An administrator handle must never be given to workers.

A backend must load checked state, invoke evaluation exactly once and confirm
publication before returning a domain response. A domain rejection may retain an
idempotency outcome or reconciliation transition and does not imply rollback.
Missing/repeated callbacks, scope substitution and unconfirmed publication reject.
The selected backend never falls back to the unused in-memory database. Unsupported
source operations fail closed. The memory harness retains its existing behavior.

The backend profile requires `commit_prepared`; the legacy `commit` entry point
returns `CommitRequestRequired`. Exact request replay still runs the original
Rust authorization/commit gate. Infrastructure failures remain separate from
persistable domain outcomes, including indeterminate commit acknowledgement.
Test adapters establish routing and failure handling only; their publication is
process-local and supplies no PostgreSQL or distributed durability evidence.

## Versioned typed metadata

The new checked formats are:

- `celetixo/access-provenance-record/v1` — original explicit/derived accesses,
  invariant scopes, completeness and exact analysis stamp.
- `celetixo/commit-outcome-record/v1` — exact accepted receipt or supported typed
  native rejection, including structured conflicts and provenance. Debug/Display
  strings never encode authority. Infrastructure/source-specific outcome families
  are explicitly unsupported by this profile.
- `celetixo/transaction-reason-record/v1` — exact scope/transaction/attempt and
  binary-safe UTF-8 reason, including legal NUL characters.
- `celetixo/lease-event-record/v1` — exact scope/transaction/history sequence,
  original grant/attempt/fence, owner, writes and typed event fields.

Decoders check digest, external context, canonical re-encoding, bounds, tags and
constructors. Independently encoded frozen fixtures are described by
[outcome metadata](../../crates/engine/tests/fixtures/outcome-codecs.json) and
[lease/reason metadata](../../crates/engine/tests/fixtures/metadata-codecs.json).
They supplement malformed, truncated, rehashed and resource-amplification tests.

## Schema v3 and upgrade

[Migration 0003](../../crates/postgres/migrations/0003_native_lifecycle.sql) preserves
published v1/v2 bytes and checksums. It adds an explicit optional native runtime
profile, per-attempt model generation, current lease pointer with scoped deferred
FK, binary reason payload reference, immutable reference-evidence admissions with
mutable current signer selection, and append-only publication audit rows.

Legacy rows keep null profile metadata and are not silently adopted as Engine
authority. New native writers must leave legacy reason TEXT null and retain the
binary record; PostgreSQL TEXT cannot preserve NUL. Audit stores the independently
authenticated database service identity separately from requested actor metadata.
The table cannot itself authenticate caller-supplied claims: the future trusted
writer must obtain service identity from its connection and authorize the callback.

New tables force RLS and revoke PUBLIC access. Evidence admissions and publication
facts reject update/delete. A terminal transaction cannot retain a current fence.
A current fence must reference that same transaction's reservation. SQL constraints
check row relationships; Rust still validates admissibility and payload content.

The registry upgrades an exact supported empty/v1/v2 prefix to v3 in one explicit
transaction. Tests inject errors after DDL and before commit, kill a child process
mid-upgrade, preserve earlier snapshot data, reject skew/future history and check
scoped immutable publication rows. Existing databases from unpublished drafts
remain preserved; no checksum ledger is repaired to hide changed migration bytes.
After publication, v3 is immutable too; further fields require another migration.

Use trusted maintenance: stop affected applications, verify an operator backup,
apply migrations, validate history, review narrowly scoped new role privileges and
reopen with the matching binary. No automatic downgrade, live old-writer fencing
or backup/restore drill is supplied here. Those remain Goal 04 acceptance gates.

## Validation and next boundary

[Source-bound checks](goal-04-reconstruction-validation/checks.json) and the
[raw log](goal-04-reconstruction-validation/checks.log) identify commands, exact
source and environment. This record is development validation, not Engine-admitted
verification evidence, a benchmark or a production readiness claim.

The two new public Rust sources contain numeric grant/counter assignments that
match the conservative credential-content detector. Their bytes were reviewed;
they contain no live credentials. The existing trusted CI smoke commands install
only those exact path/content exceptions. The default admission policy remains
unchanged. [The review record](goal-04-reconstruction-validation/public-input-review.json)
binds the reviewed addresses and workflow hash. The capture regression retains
default admission for the other public source literals, requires explicit frozen
reviews for the numeric grant assignments and rejects changed reviewed bytes.
Changed bytes still reject in the hosted smoke policy too.

The opt-in actual rust-analyzer probe and complete repository-inspection/patch
smoke commands were not rerun locally for this milestone; their hosted jobs remain
configured. No later hosted success is inferred from the passing parent run.
No T2/T3, failover, backup/restore or cross-service end-to-end claim is made.

The next implementation is the actual normalized PostgreSQL loader/publisher:
coherent bounded reads, independent scoped runtime/admin roles, common repository
lock/version protocol, schema fencing, database clock sampling after locking,
existing Rust evaluation, atomic state/receipt/idempotency/outbox publication and
confirmed-commit acknowledgement. Then validate independent-process restart/races,
reference-oracle equivalence and complete source-attempt recovery. NATS, Temporal,
S3, ClickHouse and the other full-goal gates also remain pending.

Work is paused at the user's request after publication of this piece. See the
[fresh-session handoff](goal-04-session-handoff.md).
