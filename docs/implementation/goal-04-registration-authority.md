# Goal 04 PostgreSQL registration authority

**Status:** 04.1g bounded SQL integration. PostgreSQL now stores native grants,
repository/model registration and model updates through the existing Rust Engine.
Transaction lifecycle publication, source recovery and distributed commit remain
unsupported. Milestone 04.1 and the full [Goal 04](goal-04.md) remain incomplete.

## Scope and authority

`PostgresAuthority` implements the trusted `AuthorityBackend` boundary from
[04.1f](goal-04-authority-boundary.md). An embedding explicitly selects it through
`Engine::with_authority_backend`; no unused memory store becomes a fallback.
The same Rust `grant`, `revoke`, `register`, `update_model`, session authorization
and current-snapshot operations evaluate checked `AuthorityState` records.

The first SQL profile admits native registration at revision zero, scoped session
grants and one registered snapshot. It retains model uncertainty and binary-safe
canonical values, including NUL within a legal incomplete-model reason. Immutable
candidate snapshots/models remain catalog facts and never become registered bases.
The loader rejects legacy/unbound heads, captured-source state and any lifecycle,
policy, execution, evidence, event or analytics-consumer rows it cannot reconstruct.
The publisher rejects a callback that introduces transactions or unsupported facts
before committing anything. It never filters such facts into an apparently empty
supported state.

This is the first working SQL path of the existing Engine, not full transaction
recovery. The native lifecycle codecs and reference oracle remain unchanged.
The smaller in-process native intent does not replace canonical Protobuf TaskIntent.

## Identity and privileges

The trusted embedding supplies separate runtime and application-administration
`UnixConnection` configurations for the same socket directory/database. Both are
independently authenticated and must be nonowner, nonsuperuser, non-BYPASSRLS,
non-CREATEROLE, non-CREATEDB and non-replication logins. Neither may be a member of
the other, a database/schema/table/function owner, or a reviewed broad data/server
role. The migration/security owner is a third identity, outside this runtime path.

The adapter pins authenticated role name and OID at construction, rechecks them
and role/privilege restrictions on every access, and resolves exact repository
scope with the existing `session_user`/name/OID RLS helpers. Runtime cannot mutate
principals/grants/permissions. Neither service login may read the role map or
mutate it/the migration ledger. Privilege checks include column grants and
reachable memberships, including memberships without inheritance. Requested actor
metadata remains attribution; the private Session and current Rust grant still
authorize the domain operation.

Operators explicitly map both service roles to permitted repository scopes and
grant reviewed table privileges. The integration fixture's
`authorize_registration` lists those privileges; it is test setup, not a production
provisioner. Unsupported-state detection requires scoped SELECT on the relevant
authority tables. Runtime writes cover immutable payload/snapshot insertion,
repository/model heads, counters, generation and publication audit. Administration
additionally inserts principals/grants/permissions and revokes grants. Workers
receive neither database configuration nor the embedding's Administrator handle.

Reopening does not deserialize capabilities. `authenticate_existing_session` runs
fresh embedding authentication outside storage callbacks, then rechecks the live
grant. Sessions from the previous embedding remain invalid.

## Transaction, schema and clock protocol

Every access opens a dedicated authenticated connection with the existing bounded
connection/statement/lock/idle-transaction deadlines and required PostgreSQL 17
primary/durability settings. Before BEGIN it takes a shared **session** advisory
lock on the migrator's existing exclusive lock key. Thus a waiting repeatable-read
transaction cannot retain a pre-migration ledger snapshot. Schema validation and
all subsequent use remain under that lock until connection teardown. Published
migrations 0001–0003 are unchanged; this milestone adds no schema migration.

Writes use READ COMMITTED, authorize scope before bootstrap, create missing
organization/workspace/repository identities in that order, then lock the exact
repository row FOR UPDATE. Conflicting creators wait and reload; ON CONFLICT does
not replace existing identities. Reads use REPEATABLE READ and FOR SHARE on the
repository row. Their SQL transaction allows row locking but publishes no writes.
A waiting reader may return explicit `Contention` after a concurrent publication.
The adapter never retries a callback automatically.

After locking, the adapter loads normalized counters, grants/permissions and the
exact current snapshot/model. Metadata row counts and payload lengths are bounded
before bulk allocation, then canonical decoding and `NativeRecordLimits` check
aggregate entries/bytes. Smaller embedding limits are supported; larger-than-
default limits reject. Candidate catalogs and append-only audit growth are outside
the reconstructed authority set; this bounded loader is not a capacity benchmark
or retention service.

The clock is `floor(extract(epoch FROM clock_timestamp()) * 1000)` sampled after
the repository lock. A value older than the last published clock fails with
`ClockRegression`; it is never clamped. Updates use the shared Engine clock
transition. Reads observe time without publishing a clock or inventing lease
events. This is a database wall-clock policy with explicit regression rejection,
not a claim that clock synchronization or failover has been validated.

The adapter evaluates once, exports/checks the intended facts, persists the allowed
changes and reads them back for exact equality before commit. Grant/model state,
counters, repository generation CAS and immutable publication audit share one SQL
transaction. Audit separates the authenticated database role/OID from requested
actor fields. A zero-row CAS fails; overflow cannot publish. A Rust domain error
does not itself roll back publication. A storage error or panic cannot acknowledge
the callback's proposed result.

No acknowledgement precedes confirmed COMMIT. Any publication COMMIT error, or an injected
failure after confirmed commit, returns `CommitIndeterminate`. Connection teardown
releases the schema lock; there is no implicit commit on Drop. These registration
operations do **not yet have exact durable request keys**. `inspect_registration`
provides checked state and generation to trusted administration for investigation;
it returns the stored clock for diagnosing regression and does not infer an
uncertain request's outcome. Do not blindly repeat a grant
after uncertainty. Exact administrative-request reconciliation remains required
work, alongside the prepared commit operation's lifecycle/outcome integration.

## Validation and remaining gates

The [source-bound record](goal-04-registration-validation/checks.json) and
[log](goal-04-registration-validation/checks.log) identify actual commands, exact
source, environment, passing checks and unrun checks. This is development evidence,
not an Engine verification receipt or performance baseline.

Final checks passed: 409 ordinary workspace tests (33 opt-in tests ignored), all
20 real PostgreSQL groups, workspace formatting/strict Clippy, example builds/run,
workflow semantics, protocol integrity/four mutation groups and 25 attribution
fixtures. The [fixture verification](goal-04-registration-validation/fixture.json)
records PostgreSQL 17.9 and the required durability/authentication settings; the
[stop record](goal-04-registration-validation/fixture-stopped.json) retains data.
The CI capture exception for the reviewed public PostgreSQL entry point was
[refreshed to its exact bytes](goal-04-registration-validation/public-input-review.json).
The public fixture report's mechanism-name setting also has an exact reviewed
exception. [Final policy checks](goal-04-registration-validation/followup.json)
retain changed-byte/path rejection and workflow validation. New adapter/test
source passes the unchanged default credential detector.

Actual PostgreSQL tests cover native registration/model/grant reopening, fresh
session resolution, revocation, incomplete binary model values, read purity,
private candidates, unsupported proposal rejection, decoding budgets, backward
clock, independent-instance bootstrap/registration contention, process death before
commit, actual database restart, coherent waiting readers, scope/OID mapping
revocation, privilege escalation and migration-lock/future-schema fencing.
An injected post-commit acknowledgement failure is not a real network failure
during COMMIT. Independent instances in the registration race use separate
connections/Engine allocations in threads; the crash test uses a child process.
No independent-process transaction-commit race is claimed.

Next implement normalized transactions/attempts, all exact historical prepared
requests and typed outcomes, evidence admissions/selection, leases/fences/events,
binary reasons, committed history, atomic head advancement and required outbox
facts through this same protocol. Then prove native oracle equivalence, exact
uncertain-outcome recovery and complete captured-source/Goal 03 recovery. NATS,
Temporal, S3, ClickHouse, isolation, backup/restore and operational proof remain
the full goal's later gates.

PostgreSQL's [isolation documentation](https://www.postgresql.org/docs/17/transaction-iso.html),
[lock semantics](https://www.postgresql.org/docs/17/explicit-locking.html) and
[identity/privilege functions](https://www.postgresql.org/docs/17/functions-info.html)
describe the database mechanisms used here; the actual tests establish this
adapter's bounded behavior.
