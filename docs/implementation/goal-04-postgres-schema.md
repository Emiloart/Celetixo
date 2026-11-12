# Goal 04 PostgreSQL authority schema

**Status:** implementation foundation for milestone 04.1; not a completed durable Engine or a distributed-commit claim
**Canonical SQL:** immutable [0001_authority.sql](../../crates/postgres/migrations/0001_authority.sql), additive [0002_lifecycle_reconstruction.sql](../../crates/postgres/migrations/0002_lifecycle_reconstruction.sql)
**Authority:** [Goal 04](goal-04.md), ADR-0003 and ADR-0004

This schema separates mutable operational authority, immutable engineering values and
append-only outcomes. It provides database constraints and row isolation. Rust still
owns admission, semantic conflict analysis, state transitions, invariant evaluation,
evidence applicability and commit decisions. There is no SQL procedure that declares
a transaction admissible, and this migration does not route the existing in-memory
Engine through PostgreSQL.

The migration runner owns `celetixo.schema_migrations`, its migration checksum and
the migration advisory lock. It verifies an exact supported history prefix, then applies all missing files
and records their versions in the same explicit database transaction. See the
[v1-to-v2 upgrade and rollback contract](goal-04-lifecycle-records.md#schema-v2-and-supported-upgrade). The SQL creates no cluster-global login, group or
deployment owner. Application migration/reopen/race evidence belongs in the milestone
validation record; the existence of tables is not that evidence.

## Storage interfaces

Every repository row uses the same key order:

```text
organization_id, workspace_id, repository_id
```

All three are `celetixo.identity` (text, 1–256 bytes, no database-recognized
whitespace/control characters). Rust identity constructors remain authoritative for
their full Unicode rules. `celetixo.uint64` is `NUMERIC(20,0)` checked between zero
and 18446744073709551615, including its upper bound. The client must bind canonical
unsigned decimal values; SQL numeric coercion is not an integer request parser.
`celetixo.digest32` is exactly 32 bytes, never a human-readable hex string.

| Tables | Structural responsibility |
|---|---|
| `organizations`, `principals`, `workspaces`, `repositories` | Explicit ownership identities. Repositories exist before their snapshots or head. Principals are organization-scoped; grants are repository-scoped. |
| `domain_payloads` | Immutable, scoped, bounded typed values: `payload_digest`, `payload_format`, positive `payload_version`, `payload_bytes`. This is not a whole-Database JSON authority record. |
| `snapshots`, `repository_heads` | Immutable snapshot/manifest references and current revision. The head's composite foreign key includes snapshot identity and revision. Multiple private candidates at one revision are permitted. |
| `grants`, `grant_permissions` | Current grant metadata, generation, revocation, optional expiry and typed permissions. Kinds are `session`, `executor`, `cleanup`; numeric IDs are attribution, not credentials. |
| `policies`, `policy_heads` | Immutable versioned policy values and separately mutable current-generation pointers. Policy kinds are model, source execution, authorization and retention. |
| `artifact_store_registrations` | Stable adapter identity, current registration generation, retention/provenance digests and active status. No reader object, object-store secret or raw credential is serialized. |
| `engine_counters`, `engine_model_heads` | Scoped clock/grant counters and snapshot-bound current model metadata; source heads require both analysis and index-generation references. This metadata does not itself install Engine authority. |
| `transactions`, `attempts` | Scoped lifecycle/current attempt and preserved zero-based attempt records, snapshot/model/analysis/patch bindings. The initial transaction/attempt are inserted in one SQL transaction; the current-attempt foreign key is deferred until commit. |
| `access_provenance` | Separate explicit, derived, effective, invariant, predicate and analysis payloads per attempt. |
| `transaction_outcomes`, `committed_changes` | One immutable terminal outcome per transaction, unique committed transaction and repository revision, with provenance and receipt references. |
| `reservation_fences`, `reservations`, `reservation_claims`, `reservation_events` | Durable fencing counter, owner/attempt/expiry, current semantic-element claims and append-only lease history. The Engine allocates/increments fences and evaluates conflicts/expiry. |
| `operations` | Immutable resolved operation outcome, keyed by scope, operation kind and idempotency key; canonical request digest, original request-payload reference and exact transaction/attempt are retained. |
| `source_attempts`, `source_attempt_events`, `source_attempt_outcomes`, `source_rejections` | Source admission, exact patch/application, single active attempt per transaction, executor claim, history, terminal result and pre-admission rejection records. |
| `artifact_objects`, `evidence`, `evidence_receipts`, `attempt_checks` | Immutable object metadata and evidence, unique source-attempt admission receipt and mutable reference-check selection. Evidence bytes are not made trustworthy by inserting metadata. |
| `outbox`, `outbox_delivery`, `inbox` | Immutable event facts, separately mutable publication progress, immutable idempotent consumer results. Event delivery is not transaction authority. |

`domain_payloads` caps each value at 64 MiB and records its format and version.
The database enforces length/foreign-key structure, not the domain-separated content
digest. Rust must verify digest, exact bytes, supported format/version and checked
domain construction before using a value. Unsupported or oversized values must fail
explicitly; they cannot be truncated or silently omitted. Larger artifacts belong in
the object-storage contract, with separately admitted metadata.

Foreign keys include repository scope, preventing a row from substituting another
scope's snapshot, attempt, payload, grant or evidence. Audit/history/outcome references
use restrictive deletion; there are no audit cascades. Immutable tables reject
`UPDATE` and `DELETE` through triggers in addition to their unique keys.
`attempt_checks` is a mutable selection of immutable evidence because the reference
Engine permits replacing the selected evidence for a check before commit.

Allowed lifecycle strings mirror current Rust variants; SQL checks reject unknown
values. The Engine must still validate transitions and associations between related
values. A valid row shape is not evidence that a transition, source admission or
commit is admissible.

## Database identity and row isolation

All organization-, workspace- and repository-scoped authority tables have both
`ENABLE ROW LEVEL SECURITY` and `FORCE ROW LEVEL SECURITY`. Policies use both
`USING` and `WITH CHECK`. The narrow helper functions resolve access from
PostgreSQL's independently authenticated `session_user`, not a client-set tenant
GUC, an actor string, a task field or `current_role`.

The security administrator alone writes:

```text
celetixo.db_role_scopes(
    database_role NAME,
    database_role_oid OID,
    organization_id,
    workspace_id,
    repository_id,
    authorized_at
)
```

Each runtime mapping must match both the login's name and current `pg_roles.oid`.
Dropping and recreating a same-name login therefore does not inherit its previous
mapping. Renaming a login or restoring it with a different OID fails closed until
an administrator reviews and rebinds the mapping. PostgreSQL superusers, table/function
owners and the trusted migration/security administrator are outside the runtime RLS
threat boundary.

The three `SECURITY DEFINER` helpers have fixed `pg_catalog` search paths and use
qualified table names:

- `has_repository_scope(TEXT,TEXT,TEXT)`
- `has_workspace_scope(TEXT,TEXT)`
- `has_organization_scope(TEXT)`

The authenticated function-owner login has an explicit administrator branch in these
helpers. This allows initial identity/bootstrap operations under forced RLS; it does
not depend on PostgreSQL's automatic owner exemption. A runtime login must not own
the schema, tables or functions, be superuser/`BYPASSRLS`, or possess direct or
inherited membership/impersonation rights over the migration/security owner.
`SET ROLE` does not change `session_user`. Ordinary runtime roles cannot use
`SET SESSION AUTHORIZATION` to impersonate the owner.

The role map has no identity foreign key so trusted setup can install a scope
mapping before that scope exists. Its lack of a foreign key is not permission to
grant runtime writes. Neither runtime roles nor worker-facing services may insert,
update, delete or read this administrative table directly.

## Deployment/bootstrap contract

Create the migration/security owner and runtime logins through a separately reviewed,
deployment-specific bootstrap. Use independently authenticated connections. Do not
hardcode an organization, owner, runtime login or password in this migration.

A nonowner runtime login requires schema `USAGE`, explicit `EXECUTE` on the three
scope helpers and a reviewed list of table privileges. The migration revokes all
public schema/table/helper privileges. Do not use `GRANT ... ON ALL TABLES IN SCHEMA
celetixo`: it would include the role map and migration ledger.

For example, after authenticating as the migration/security administrator, a reviewed
psql bootstrap can bind a particular runtime login to a scope using psql variables:

```sql
INSERT INTO celetixo.db_role_scopes (
    database_role, database_role_oid,
    organization_id, workspace_id, repository_id
)
SELECT rolname, oid, :'organization_id', :'workspace_id', :'repository_id'
FROM pg_catalog.pg_roles
WHERE rolname = :'runtime_role';
```

Require exactly one inserted row, inspect the role's attributes and transitive role
memberships, and establish an independent connection as that login for isolation
probes. Connection startup parameters and tenant GUCs must not establish authority.
Do not pass login secrets in command arguments, repository files or logs.

Separate grant/policy/registration administration from ordinary Engine operation.
Default runtime privileges should permit only the reads and mutations required by
the selected service, with read-only access to policy/grant administration and
insert-only access to immutable outcomes where appropriate. Any additional service
permission needs its own scoped review. Workers never receive these database roles.

The root migration runner and current integration fixture define the selected
transport/authentication envelope. This schema does not advertise a public API,
network deployment, mTLS service mesh, tenant credential broker or production role
provisioner.

## Required Engine integration

The [semantic-record catalog](goal-04-semantic-state.md) (04.1c) retains immutable
snapshot-bound engineering models and repository-analysis facts in existing scoped
`domain_payloads`. It checks format, version, digest, expected snapshot/generation
and constructor invariants on reopen. It does not install current model heads or
write lifecycle/commit state. Schema v1 and its published checksum are unchanged.

An initial backend can serialize mutations by locking the scoped repository row.
Within an explicit PostgreSQL transaction, the Engine must load and validate current
state, invoke the same Rust semantic implementation, recheck mutable versions,
conditionally advance the head, and atomically write the terminal outcome, receipt,
idempotency record and outbox fact. A zero-row CAS is conflict/revalidation, not success.
All mutations affecting a checked grant, policy, reservation or attempt must share
the documented lock/version protocol.

No acknowledgement precedes confirmed SQL commit. Connection loss around commit
has an indeterminate result until authenticated idempotency lookup reconciles it.
An operation failure is not automatically a SQL rollback: existing Engine operations
sometimes intentionally persist a rejected outcome or revalidation state.

Artifact reads remain outside long-held database transactions. Evidence admission
must independently read/hash the persisted bytes and then recheck authoritative
grant, policy, attempt, store registration/generation and expiry before recording
the outcome. A registration row cannot resurrect an absent reader adapter. Cleanup
authority cannot become execution, evidence or commit authority.

The reference Engine now separates adapter-free records from the embedding's live
capability allocation identity and artifact-reader registry (04.1b). Copying those
records does not authenticate an old session, executor, cleanup grant or storage
capability, and does not restore a reader. This is a prerequisite for checked
reconstruction, not a public restore API or an implemented PostgreSQL lifecycle.
Runtime identity is not durable authorization. The database-backed path must still
authenticate each request, load current scoped grants and policy, and bind trusted
adapters to their current registration before using them.

Authoritative clock policy, lock ordering, bounded retries, statement deadlines,
database availability behavior, fencing use and terminal-state reconstruction must
be documented and tested with the routed backend. This migration alone does not
supply those implementations. Captured source publication remains unsupported
until a separately implemented and validated Engine contract permits it.

## Verification required for milestone claims

Use real PostgreSQL and independently authenticated, nonowner runtime connections.
At minimum verify:

- Empty migration, repeated checksum-verified reopen and failure rollback.
- Every scoped table has enabled and forced RLS with read and write predicates.
- Each runtime login can access its mapped scope and cannot read/write another
  scope, set tenant GUCs to cross scope, acquire owner privileges, modify the role
  map or disable RLS.
- Foreign keys reject cross-scope payload/snapshot/attempt/grant references.
- Unsigned bounds, digest length, payload bounds and unknown status values reject.
- Snapshot/head revision disagreement and incomplete initial transaction/attempt
  setup fail atomically.
- Immutable payload/outcome/audit mutation and duplicate terminal/idempotency
  identities reject.
- Restart preserves exact rows/bytes; an independently reopened client can verify
  the same typed payload and digest.

Independent Engine-process commit races, crash ambiguity, semantic oracle equivalence,
source-attempt recovery, actual object durability and service outage recovery remain
additional integration gates. Do not infer their success from schema tests.
