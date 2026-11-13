# Goal 04 native PostgreSQL lifecycle authority

**Status:** 04.1h validated bounded native SQL integration. Extends the
[registration adapter](goal-04-registration-authority.md) without changing
published migrations 0001–0003 or the native Engine semantic rules. Complete
captured-source recovery and Goal 04 remain incomplete.

## Authority and publication

`PostgresAuthority` now reconstructs and publishes native transaction intents,
all attempts, access provenance, patches/candidates, reference evidence and check
selection, reservations/fences/events, binary reasons, terminal outcomes,
committed history and exact prepared requests/results. The Rust Engine evaluates
the same admission, authorization, conflict, invariant, evidence and commit rules.
SQL projections preserve those checked decisions; no private Database blob or
second semantic decision service is introduced.

Every access uses a fresh authenticated connection, the shared migration fence,
the exact scoped repository lock and the checked normalized state. Updates retain
READ COMMITTED/FOR UPDATE, generation CAS and one publication transaction. Reads
retain REPEATABLE READ/FOR SHARE and never publish clocks or expiry events.
There is no callback retry or memory fallback. Current authorization still runs
before exact historical request recovery. An expired historical lease cannot
prevent an authorized caller from recovering an already recorded outcome.

The publisher preserves immutable/preexisting facts, writes changed projections,
then loads them again for exact comparison with checked intended `NativeRecords`.
Auxiliary relationships are validated independently. A new commit conditionally
advances the exact old revision/snapshot, stores its history/receipt, terminal
state, original request/result and stable scoped outbox fact, and releases claims
in the same transaction. Existing Engine domain rejections can retain an exact
typed result and revalidation state. Infrastructure failures withhold that result.

The adapter rechecks login identity/privileges/scope after publication, then samples
the database clock again while retaining the repository lock. Every new accepted
commit must still have its original live acquisition grant/fence, Commit permission
and the matching release. Expiry equality rejects. Failure rolls back all proposed
rows, including outcome/outbox and clock/event changes; the callback is not rerun.
This is a final database validation point inside the atomic SQL transaction, not
a promise that WAL flush completes before a wall-clock deadline. Backward time
fails explicitly. All authority writers must retain the same locking protocol.

No success response precedes confirmed COMMIT. COMMIT errors return
`CommitIndeterminate`; a fresh authenticated embedding resubmits the **original
prepared request**, whose retained body/result determines recovery. Re-preparing
from a later attempt or evidence selection is not equivalent. Legacy unprepared
`commit` is rejected in this backend profile. At this original milestone, non-commit mutation requests lacked exact durable
identities; diagnostic views alone could not resolve a lost response. The later
[original mutation journal](goal-04-mutation-recovery.md) adds typed keyed
transaction, repository and administrative outcomes. Callers must use that
explicit keyed API to reconcile its retained original request; blindly retrying
an ordinary unkeyed operation remains insufficient.

## Normalized recovery coverage

| Relations / fields | Recovery rule |
|---|---|
| Scope identities, role map, schema ledger | Existing authenticated scope helpers and exact schema validation; role map remains inaccessible to services; migration fence spans access |
| Repository generation, latest publication | Exact generation/clock/current snapshot linkage; requested actor is attribution, including rejected requests, not authentication |
| Grants and permissions | Session grants only; immutable principal/permissions, monotonic counter, revocation without reactivation; positive bounded publication generations |
| Snapshots and domain payloads | Exact format/version/digest/scope/manifest/revision and checked canonical decode; private candidates never become registered proposal bases |
| Repository/model heads | Native profile and exact current binding; positive bounded head generation linked to its publication; initial plus committed snapshots only form the registered set |
| Transactions and attempts | Exact original owner/proposal grant/base/intent; contiguous zero-based attempts and current pointer; binary reason bound to current attempt; no legacy unbound metadata |
| Access provenance | Exact explicit, derived, effective, invariant, predicate and analysis projections; fresh checked Rust reconstruction validates their semantic relationships |
| Evidence, admissions, signer, checks | Immutable envelope/original issuer and historical first-admission timestamps; current signer has an actual admission; all old attempt selections retained; active verification authority checked by Engine at use |
| Fence, reservations, events, claims | Exact event sequence/attribution and original acquisition attempt; normalized expiry/state match events; current pointer matches its fence; live claims match the **persisted** clock/grants |
| Commit history and terminal outcomes | Exact receipt/provenance/snapshot/patch/attempt; one terminal association; binary abort reason or accepted receipt payload |
| Operations | Original complete prepared request and typed accepted/rejected result, with exact scope/principal/body digest and external key; no legacy missing request |
| Outbox | One stable scoped repository/revision fact for each commit, exact accepted payload and identity; no extra/missing facts |
| Outbox delivery | Exact event relationships; delivery counters/claims/timestamps are dispatcher progress, preserved untouched by native publications |
| Policy/source/execution/artifact/inbox rows and source counters | Reject their presence; partial recovery cannot authorize a native operation |
| Immutable unused candidate payloads/catalog facts, older audit attribution, operational timestamps | Preserved; not promoted into authority or current credentials |

All row collections share a pre-allocation metadata budget. Payload lengths are
checked before fetching bytes; decoders and reconstructed records enforce entry,
individual value and aggregate canonical-byte budgets. Owned snapshot copies are
charged. SQL ordering explicitly qualifies underlying columns: selected `::text`
aliases must never sort numeric ordinals lexically. This bounded whole-scope
reconstruction is not a capacity or retention claim.

The outbox fact is an internal native storage record (`NativeCommitRecorded`,
schema 1), with identity derived from the scoped accepted canonical payload. It
is not yet a published cross-service wire protocol. Canonical Protobuf evolution,
dispatcher claims, JetStream delivery and consumers remain 04.3 work.

Runtime receives reviewed lifecycle insert/update/claim-delete privileges in the
test fixture, with no principal/grant/permission mutation. Immutable triggers and
scoped constraints remain enforced. Administration additionally manages scoped
grants. Neither service can read the scope map or mutate the migration ledger.
`inspect_native` is trusted coherent diagnostics; `inspect_registration` and its
`RegistrationView` return type remain compatibility names. Stored IDs never
restore a private Session without fresh embedding authentication.

## Verification and remaining work

Real PostgreSQL tests cover phase-by-phase reopen, original request replay after
evidence replacement/revalidation, original grant attribution, admissions and
binary values, publication faults, final-gate expiry, revocation, process races,
numeric ordinals, generated pair histories, corruption and actual database restart.
A test-only SCRAM-transparent Unix relay withholds a real server COMMIT response;
the durable target result is inspected before replay. No transport payload or
credential is logged. This proves the stated local transport boundary, not
replication/failover durability.

[Final source-bound checks](goal-04-native-sql-validation/checks.json), the
[log](goal-04-native-sql-validation/checks.log) and
[observed local profile](goal-04-native-sql-validation/profile.json) record 409
ordinary workspace tests (45 opt-in tests ignored), all 32 real PostgreSQL groups,
workspace formatting/strict all-target Clippy, example builds/run, protocol
integrity/four compatibility mutation groups, workflow semantics and 25 attribution
fixtures. The implementation/protocol/workflow manifest remained unchanged.

The [development findings](goal-04-native-sql-validation/development-findings.json),
[first complete run](goal-04-native-sql-validation/first-pass-checks.json) and
[interruption record](goal-04-native-sql-validation/interrupted-run.json) preserve
failures and incomplete observations; they are not relabeled as passing evidence. The
[log presentation record](goal-04-native-sql-validation/log-presentation.json)
identifies trailing-space normalization; original subprocess bytes are retained
privately.
The final suite includes 16 independent bitmask pair scenarios, three child-process
race cases, child process death before/after commit, real response loss and actual
database restart. These bounded tests complement the larger native Engine oracle;
they do not close the complete source/distributed oracle gate.

Actual Linux T1 and rust-analyzer probes were not rerun for this SQL-only change;
ordinary regressions passed and their earlier exact-profile evidence remains
separately scoped. Whole-tree/T0 smoke is recorded separately if run. Hosted CI is
unconfirmed; a local passing run does not establish a hosted result.
No complete 04.1/04.2 acceptance is claimed. Remaining work includes full source
policy/execution/evidence recovery, exact non-commit mutation requests, broader
multi-instance reference histories and failure testing, production service
identity/transport, NATS, Temporal, S3, ClickHouse, shared adapters and the full
security/recovery/backup/load programme in the unchanged [Goal 04](goal-04.md).
