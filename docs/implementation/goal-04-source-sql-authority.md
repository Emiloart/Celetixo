# Captured-source PostgreSQL lifecycle authority

**Status:** validated bounded local SQL integration. [Source-bound checks](goal-04-source-sql-validation/checks.json) and [output](goal-04-source-sql-validation/checks.log) record the results. This profile extends the checked
[source recovery graph](goal-04-source-recovery.md) into PostgreSQL through
evidence receipt publication. Source revision commit/revalidation and complete
04.1/04.2 acceptance remain pending. The accepted [Goal 04](goal-04.md) is unchanged.

## Authority and publication

`PostgresAuthority::connect_source` explicitly selects `source-execution-v1`.
The native constructor retains `native-reference-v1`. A repository binds its
profile before the first model registration, including grant-only scopes;
neither constructor can reinterpret another profile's existing facts.

Every operation authenticates a restricted Unix-socket database login, checks
schema and role/scope privileges, and holds the repository row lock while loading
normalized records. Rust reconstructs and validates the complete source graph
before running the Engine callback. Updates validate the resulting graph,
publish the changes, load and compare every normalized fact, and recheck live
database identity and the authoritative clock before committing. A result is
returned only after confirmed SQL commit. Failed publication rolls back; a lost
commit acknowledgement requires reconciliation. There is no memory fallback or
automatic callback retry.

The source profile supports repository/analysis registration, source policy
installation/replacement, proposal and analysis, reservations/renewal/fencing,
execution admission, original admission requests/results, claims, patch and
verification transitions, cancellation/reconciliation, source evidence and
receipts, executor/cleanup grants, and configured artifact registrations. Actual
semantic decisions remain in the Engine. Shared native SQL machinery writes
only shared base facts; source graphs never pass through native-only validation.

Admission, claim, execution and receipt changes recheck the original active
execution grant, lease/fence, admission expiry, claimed executor and selected
storage registration at final database time. Historical replay does not grant
another launch. Cancellation, timeout and revalidation reconciliation can remove
authority after expiry. A stored expired or revoked definition remains history.

## Immutable graph and additive migration

Migration `0004_source_lifecycle.sql` leaves published migrations 0001–0003
unchanged. It adds scoped source analysis/policy/check catalogs, proposal and
admission contexts, original request/outcome associations, ordered admission and
rejection events, executor/cleanup definitions, configured store definitions and
artifact/evidence associations. New fact tables enforce row-level scope policies,
same-scope foreign keys, immutable rows and no public privileges.

Proposal analysis and later selected policy have separate immutable associations.
Historical catalog entries survive head replacement. A rejected submitted attempt
999 remains 999 in its original request, while a separate resolved-attempt
association points to actual attempt 0. Migration retains old rejection rows and
backfills their existing resolved selector before replacing the old constraint.
It does not invent missing original requests or turn legacy scaffolding into a
runnable source history. Unsupported legacy combinations fail closed.

Physical artifact content, per-registration location and logical reference are
separate identities. The same bytes may have stdout and stderr references, names,
media types or registrations with different identities. Evidence retains every
ordered envelope, provenance and check reference, plus the complete receipt and
Passed, Failed or Incomplete assessment. Legacy artifact rows and their locator
associations are retained and backfilled as metadata; no upload, logical name,
configured reader or evidence admission is inferred.

SQL metadata and domain payloads are bounded before materialization and checked
again against aggregate recovery budgets. The SQL loader preflights row counts
and uncompressed projected lengths before allocating result rows; nested typed
definitions and retained copies are also charged. These are conservative data
budgets, not allocator RSS, CPU limits or demonstrated large-repository capacity.
Unpublished source-only metadata cannot be adopted as generation-zero state.

## Runtime storage and execution boundary

Configured artifact registrations bind an explicit nonsecret adapter identity
and configuration digest to scope, registration, retention and provenance. The
trusted embedding resolves that selector and independently authenticates fresh
sessions, executors and cleanup grants. Restored metadata contains no live
capability, credentials or reader. Legacy local registration identity remains
unchanged and is unavailable as a configured SQL source store.

Artifact reads occur outside SQL transactions. The Engine rereads and rechecks
authority before publishing a receipt. Revocation during reads cannot admit the
result. Reader attachment is keyed by scope and registration; two repositories
can safely reuse numeric grant/store IDs inside the same embedding identity.

The integration fixture uses retained local filesystem objects and synthetic
check records. It exercises actual object bytes and SQL metadata, but executes
no repository commands and establishes no S3, remote execution or production
storage durability. Linux T1 execution has its separate validated profile.

## Validation and remaining work

The source SQL integration checks cover fresh Engine reconstruction at lifecycle
phases, all three evidence assessments and logical references, historical policy
replacement, original rejection replay, response loss, rollback, final-clock
expiry, reused scoped IDs, revocation during artifact reads, independent process
claim races, process death before/after SQL commit, migration interruption and
actual database restart with fresh reader resolution. Validation passed 452 workspace tests (59 opt-in tests ignored there),
10 actual Linux T1 tests and all 45 PostgreSQL groups, plus formatting,
strict workspace all-target Clippy, example builds/run, workflow checks,
protocol integrity/four compatibility mutation groups and 25 attribution
fixtures. The source manifest stayed unchanged during verification.
The validation directory retains the development failures and corrections.

This is a trusted local Unix embedding with PostgreSQL 17 and an isolated fixture.
It does not establish network service identity, failover, host-loss durability,
backup/restore, production isolation, load capacity or hosted CI success.
Source prepared commit/revalidation, exact request identities for every other
retriable mutation, broader generated-history oracle equivalence and all later
service/operational milestones remain required. Source revision stays at its
registered base; a passing source receipt does not publish a revision.
