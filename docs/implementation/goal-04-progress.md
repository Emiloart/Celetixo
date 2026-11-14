# Goal 04 progress and foundation validation

**Status:** [04.1 durable state](goal-04-authority-acceptance.md) and
[04.2 atomic commit](goal-04-commit-acceptance.md) are accepted for the reviewed
PostgreSQL adapters. [04.3 event delivery](goal-04-events-acceptance.md) is accepted
for the reviewed local PostgreSQL/NATS profile. The complete Goal 04 objective
remains active and incomplete.

The [event validation record](goal-04-events-validation/checks.json) records 556
workspace passes, six selected real PostgreSQL parents, actual three-node broker
fault recovery and strict build/protocol checks. The supplemental
[acceptance record](goal-04-events-acceptance/checks.json) adds all 107 SQL parents
across an initial 103-pass/four-failure run and four corrected exact-parent passes,
plus standalone publisher/consumer readiness and graceful shutdown. The original
failed run is retained; no single green full-suite run is claimed.

The preceding source-history proof records 552 workspace passes, 48 generated Engine
source histories and 16 real PostgreSQL histories using independent commit
processes, plus three related SQL recovery/race parents. Its exact scope and
unrun checks are in [the final record](goal-04-source-history-validation/checks.json).
Earlier bounded records remain linked below.

The complete [Goal 04 contract](goal-04.md) remains the objective. It was retrieved
from `origin/main` at `117479221eb0830fdcab6918ef76a7ed293cabe7` on 2026-11-11;
its SHA-256 is `339c328bf93bb512b8f90a8006d3dc07b8ed031399a93c3a3c151f45940b8743`.
This record does not narrow that contract or promote schema presence into durable
Engine authority. ADR-0001–0004 and the existing Goal 01–03 gates remain in force.

## Milestones

| Milestone | Current evidence / remaining work |
|---|---|
| 04.1 — PostgreSQL state and recovery | **Accepted for the reviewed PostgreSQL adapter.** [Acceptance audit](goal-04-authority-acceptance.md) maps the original criteria to normalized native/source state, exact outcomes, leases/fences, schema v8, all supported prior upgrades and actual checked restart/reopen. Full Goal 04 remains active. |
| 04.2 — Multi-instance atomic commit | **Accepted for the reviewed PostgreSQL adapters.** [Acceptance audit](goal-04-commit-acceptance.md) maps conditional revision advancement, exact outcomes, independent-process native/source histories and crash ambiguity to actual evidence. Production service/transport and the original full-goal operations programme remain open. |
| 04.3 — Transactional events | **[Accepted for the reviewed local profile](goal-04-events-acceptance.md).** Typed lifecycle frames, fenced publisher, scoped SQL inbox/projections, three-node mutual-TLS JetStream, crash/replay/quorum/retention reconciliation, combined 107-parent SQL coverage and standalone worker readiness/shutdown. Full-platform operations remain in 04.7. |
| 04.4 — Temporal workflows | Pending: Go workflows/activities, replay, timeout reconciliation, cancellation and worker recovery. |
| 04.5 — Immutable object storage | Pending: S3 adapter, verified durable admission, scoped authorization, orphan reconciliation and governance/restore. The local writer/format is retained; the reader extension above does not establish S3 storage. |
| 04.6 — Derived analytics and shared adapters | Pending: real ClickHouse ingestion/rebuild, scoped shared-service lifecycle and integration with the durable path. Existing bounded LSP/T1 capabilities remain as recorded for Goals 02/03. |
| 04.7 — End-to-end operational proof | Pending: complete real-service path, multi-instance/fault/security/load tests, backup/restore drills and measured operational baselines. |

## Milestone 04.2 acceptance

[The atomic commit audit](goal-04-commit-acceptance.md) evaluates the original
04.2 criterion. A test-only numeric serial interpreter checks source footprints
and final file contents independently of Engine analysis/conflict code. Forty-eight
Engine histories and 16 real SQL histories cover compatible/conflicting original
work, dependencies/cycles, explicit reads, scopes, predicates and fresh attempts.
A separate injected stale-read regression checks the shrinker.

[Final checks](goal-04-source-history-validation/checks.json) record all 552
workspace passes and strict Clippy on the exact frozen sources, followed by the
complete 16-seed SQL parent, original-key races, pre/post-commit process exit and
fresh revalidation/cold reopen. The first interrupted SQL run is not a passing
parent. Per-check build profiles and the later CI-only profile delta are explicit.
Existing production Rust, canonical protocol and migrations 0001–0008 are unchanged.

The subsequent [04.3 event implementation](goal-04-events.md) adds the real
JetStream path and selected fault/security evidence. Its
[acceptance audit](goal-04-events-acceptance.md) closes the regression and worker
operation checks for the reviewed local profile. Continue with 04.4 Temporal. The original 04.4–04.7 workflow,
object storage, analytics/shared services and operational proof also remain active.

## Independent source commit processes

[The bounded process proof](goal-04-source-processes.md) adds independent local
artifact readers while preserving exclusive writer ownership and all existing
read verification. Two selected PostgreSQL parents check same/changed original
keys on one source transaction and process exits before/after commit. Each scope
retains one revision/history, terminal outcome and outbox/delivery fact.

[Checks](goal-04-source-process-validation/checks.json) record 550 workspace passes,
10 actual T1 regressions, strict Clippy, formatting and workflow validation.
The full 04.2 source history/oracle and remaining service/operational work stay open.

## Milestone 04.1 acceptance

[The acceptance audit](goal-04-authority-acceptance.md) closes the original 04.1
criteria on implementation `0918af9`, without changing the Goal 04 contract.
Seven supplemental SQL parents add explicit empty-schema recovery, upgrades from
v2/v3/v4/v6, actual checked native/source PostgreSQL restart and the 16-seed native
reference-oracle comparison to the primary source-revalidation evidence.
The primary and supplemental sets contain 21 selected SQL parents, not the whole
opt-in suite. No implementation input changed between these runs.

Continue with 04.2's remaining source independent-process commit and broader
source history/oracle proof, then the original service/operational milestones.
Production deployment and full-goal acceptance remain unclaimed.

## Fresh source revalidation

[Fresh revalidation](goal-04-source-revalidation.md) replaces an owned live source
reservation with a new fence and numbered attempt using current complete analysis
and policy. New work requires an explicit patch and fresh checks. Old workers and
evidence cannot authorize the new attempt. Original typed replies recover without
renewing authority. Schema v8 preserves every published v1–v7 migration.

[Checks](goal-04-source-revalidation-validation/checks.json) bind one frozen source
set to 548 ordinary workspace tests, 14 selected SQL parents and 10 actual Linux
T1 cases. SQL scenarios include independent processes racing one original result,
process death before/after commit, final-time expiry, immutable forced-RLS context
and v7 upgrade rollback. A graceful PostgreSQL restart preserved 256 logical rows
across 60 scoped tables for a committed transaction with two source epochs.
This is a bounded local proof. Full source generated-history equivalence and the
remaining seven-milestone service/operational contract stay active.

## Source revision publication

[Source publication](goal-04-source-publication.md) independently reads retained
objects, checks both current session roles and publishes the successor head,
analysis/model, typed history, terminal state, reservation release, original
outcome and outbox atomically. Schema v7 preserves published v1–v6. Original
receipts and rejections recover without executing checks or reviving authority.

[Checks](goal-04-source-publication-validation/checks.json) record 539 workspace
tests, 10 actual T1 tests and 15 named SQL parent cases, including source races,
final-time expiry and upgrade/process-death rollback. PostgreSQL restart preserved
193 committed-scope rows across 57 tables. The complete SQL suite was not rerun;
fresh source revalidation, source generated-history/process/crash equivalence and
full 04.1/04.2 acceptance remain required.

## Original source-executor outcomes

[Executor recovery](goal-04-executor-recovery.md) adds all four keyed execution
transitions to the same scoped atomic journal. Exact original results/rejections
survive terminal states, missing-selector reuse, process recovery and lost
responses. Current executor authentication precedes replay. The coordinator
rejects replayed claims before materialization and retains independent workspace
and OCI launch recovery. Migration v6 leaves published v1–v5 unchanged.

[Combined results](goal-04-executor-validation/final-results.json) record 504
workspace tests, 10 actual Linux T1 tests and 87 PostgreSQL parent groups, strict
lint/format, examples, workflow/protocol and attribution checks. The first SQL
attempt hit the existing physical storage reserve and is retained as failed
evidence; deferred gates passed after storage became available on the same 255
frozen source files. The full Goal 04 contract and incomplete 04.1/04.2 status stay
unchanged.

## Durable original mutation outcomes

[Mutation recovery](goal-04-mutation-recovery.md) adds one actor-scoped journal
for original transaction, repository and administrative requests. Exact retries
retain the original typed outcome, time and immutable effects; changed bodies
conflict. Current authentication precedes lookup. Effect and journal publication
are atomic, including rejected requests, rollback and lost-response recovery.
Historical results cannot renew leases, select old evidence or restore revoked
grants/readers. Additive schema v5 preserves published migrations 0001–0004.

[Final checks](goal-04-mutation-validation/checks.json) passed 494 workspace
tests, 10 actual Linux T1 tests and 80 PostgreSQL parent groups, with unchanged
source throughout the final run. Formatting, strict workspace lint, examples,
workflow/protocol and attribution checks passed.
The subsequent [executor extension](goal-04-executor-recovery.md) adds keyed source
transition replies. Source prepared commit/revalidation and broader independent
SQL oracle equivalence remain next work. These are bounded recovery capabilities,
not completion of 04.1 or Goal 04.

## Source authority reconstruction and fresh runtime recovery

The [consolidated recovery capability](goal-04-source-recovery.md) retains the
complete source metadata graph, historical policies/indexes and exact original
admission requests/results. Checked reconstruction replays patches/plans,
validates leases/events and preserves Passed/Failed/Incomplete receipts. Source
`Validating` transactions do not acquire fabricated native patches. A fresh
embedding resumes claimed work only after independent executor authentication
and explicit reader resolution; scoped allocation IDs cannot alias runtime
resources. Source operations now use scoped authority callbacks throughout.

[Checks](goal-04-source-recovery-validation/checks.json) and
[log](goal-04-source-recovery-validation/checks.log) record final source-bound
workspace, native PostgreSQL and local Linux T1 regression validation. The source
recovery adapter in these original tests holds normalized records in memory.
The subsequent [source SQL lifecycle](goal-04-source-sql-authority.md) supplies
SQL load/publication/restart integration through receipts. Complete request
reconciliation and source prepared commit/revalidation remain required.
No complete 04.1/04.2 or later-service acceptance claim is made.

## 04.1i — Checked source metadata recovery prerequisites

The [source codec contract](goal-04-source-codecs.md) adds bounded reconstruction
of existing full CheckSpec/PatchPolicy definitions, source checks/policies/plans,
historical admissions and execution/evidence metadata. Dependency definitions,
semantic requirements and original authorization provenance are checked
independently; canonical formats and default credential admission are unchanged.
Dependency budgets precede hashing/cloning/resolution/replay. No runtime
capability or durable-object availability follows from restored metadata.

[Checks](goal-04-source-codecs-validation/checks.json) and
[log](goal-04-source-codecs-validation/checks.log) record 427 passing workspace
tests (45 opt-in tests ignored), strict Clippy, formatting, examples, workflow/
protocol checks and 25 attribution fixtures. The 18 new tests cover independent
layouts, substitution/canonical attacks, dependency-budget amplification and
historical authority separation. Real PostgreSQL/T1/LSP probes are not rerun for
these pure codecs; earlier evidence remains separately scoped, hosted CI unconfirmed.
Complete normalized source state, scoped runtime resolution, SQL reopen and
atomic source publication are the next integration gates. Exact non-commit
mutation reconciliation and every remaining 04.2–04.7 requirement remain active.

## 04.1h — Native PostgreSQL lifecycle/commit integration

The [native SQL contract](goal-04-native-sql-authority.md) extends the registration
adapter through native attempts, original prepared requests and typed outcomes,
reference evidence/admissions/selection, lease acquisition attribution and events,
binary terminal reasons, committed history and atomic revision/outbox publication.
The same Rust Engine decides admissibility. A final database-clock/login check
follows exact SQL readback while the repository lock is retained. Published
migrations and default credential admission remain unchanged.

[Final source-bound checks](goal-04-native-sql-validation/checks.json) and
[log](goal-04-native-sql-validation/checks.log) record 409 ordinary workspace tests
(45 opt-in tests ignored), all 32 real PostgreSQL groups, strict workspace Clippy,
formatting, examples, protocol integrity/four mutation groups, workflow semantics
and 25 attribution fixtures. SQL coverage includes phase reopen, full native
database restart, 16 independent bitmask pairs, three process race cases,
process death around commit, real COMMIT reply loss, expiry and corruption.
Development failures and an interrupted run remain preserved separately. These
bounded tests do not establish full source/distributed oracle equivalence.
Actual T1/LSP probes and hosted CI remain separately scoped/unconfirmed. Complete captured-source recovery, exact non-commit mutation
requests, service identity and all remaining 04.2–04.7 requirements remain active.

## 04.1g — PostgreSQL registration authority (resumed)

The [registration adapter](goal-04-registration-authority.md) is the first actual
SQL implementation of the existing Engine backend boundary. It durably publishes
scoped native grants, registration and model updates through the same Rust
transitions, with restricted independent runtime/admin logins, live schema fencing,
repository locks/generation CAS, database-clock policy, checked normalized
reconstruction/readback and confirmed publication audit. Published migrations are
unchanged. Any lifecycle/source/policy state outside this profile rejects before
publication; no complete transaction or distributed-commit claim is made.

[Source-bound checks](goal-04-registration-validation/checks.json) and the
[raw log](goal-04-registration-validation/checks.log) record validation. Real
PostgreSQL coverage includes native reopen and actual database restart, process
death before commit, separate-instance registration races, coherent readers,
future-schema/migration fencing, scope/OID and privilege changes, publication
rollback, an injected lost acknowledgement, model uncertainty and read purity.
Exact registration-request idempotency and real transport loss during COMMIT are
not established. `inspect_registration` is trusted investigation, not automatic
uncertain-request recovery.

Final verification passed: 409 ordinary workspace tests (33 opt-in tests ignored),
all 20 real PostgreSQL groups, strict workspace Clippy, formatting, example
builds/run, workflow semantics, protocol integrity/four mutation groups and 25
attribution fixtures. The dedicated PostgreSQL fixture was stopped with data
preserved. Actual T1/LSP probes and whole-repository/T0 smoke were not rerun locally
for this bounded SQL change; earlier evidence remains separately scoped. Hosted
CI was not confirmed through the unauthenticated Linux GitHub CLI.

The full goal remains active. Next extend the normalized loader/publisher to
transaction attempts, historical prepared requests/outcomes, evidence selection,
leases/fencing/events, binary reasons and atomic commit/history/outbox facts.
Complete source recovery, native oracle equivalence and all 04.2–04.7 gates remain.

## 04.1f — Checked reconstruction and trusted backend boundary

The [authority boundary](goal-04-authority-boundary.md) reconstructs normalized
native lifecycle facts through the existing Rust analysis/patch/commit rules.
It retains original requests, evidence selection, lease acquisition attribution
and typed outcomes, and rejects forged associations and unsupported source state.
Fresh authentication resolves a new embedding-bound session; stored IDs cannot
restore capabilities. Explicit scoped backend callbacks reject protocol failures
and withhold outcomes until publication confirms. No SQL backend exists yet.

Schema v3 preserves published v1/v2, adds native-profile/attempt/lease/reason
metadata, evidence admission/signer relationships and scoped immutable publication
audit. Real upgrade tests preserve earlier data across injected errors and child
process death. Binary-safe reason and lease-event codecs preserve exact values.

[Source-bound checks](goal-04-reconstruction-validation/checks.json) and the
[raw log](goal-04-reconstruction-validation/checks.log) record final verification,
including 409 ordinary workspace tests passed (25 opt-in tests ignored), all 12
real PostgreSQL groups, ten actual Linux T1 tests, strict workspace Clippy,
formatting, example builds/run, workflow semantics, protocol integrity/four
mutation groups and 25 attribution fixtures. The dedicated database was stopped
with its data preserved.
The [session handoff](goal-04-session-handoff.md) preserves the remaining full-goal
work and local operating details. Work stops after this commit/push as requested;
no completion, SQL lifecycle durability or distributed authority is claimed.

## 04.1e — Lifecycle payloads, exact requests and supported schema upgrade

The [lifecycle contract](goal-04-lifecycle-records.md) defines checked native
intent/access/patch/impact/evidence/request records and frozen format fixtures.
Explicit native commit requests reuse the existing Rust gate, retain the original
body/outcome across revalidation and evidence changes, and reject key reuse with
changed semantic content. Current opaque authorization is still required.
Reference reservations now retain scoped monotonic fences; conflict/overflow
cannot consume a fence or partially acquire authority. Proposal and lease grants
retain separate attribution, and opaque sessions/executors bind scope explicitly.

Migration v2 preserves published v1 and admits zero-based attempts, bounded
reasons, exact original request references and scoped counter/model-head metadata.
The migration registry upgrades only exact supported history in one transaction.
Real database tests cover preserved v1 data, injected rollback, actual child
process death, history skew, RLS and deferred current-head binding. An earlier
unpublished development-schema database was retained; final tests used a separate
fresh database instead of rewriting its applied checksum ledger.

[Source-bound validation](goal-04-lifecycle-validation/checks.json) and the
[raw log](goal-04-lifecycle-validation/checks.log) record 392 ordinary workspace
tests passed (24 opt-in tests ignored), all 11 real PostgreSQL groups, ten actual Linux T1 tests, strict workspace Clippy,
formatting, example builds, protocol integrity/four mutation groups and 25
attribution fixtures. The dedicated database fixture was stopped with all data
retained. These records and reference APIs do not reconstruct mutable
Engine authority, install a SQL lifecycle backend, persist leases/fences or prove
multi-process commit. The complete 04.1–04.7 gates remain required.

## 04.1d — Explicit state evaluation and publication

The [state-access contract](goal-04-state-access.md) defines private read/update
callbacks, staged reference-state publication and separate domain/publication
outcomes. Domain rejection can retain intended idempotency/reconciliation state;
publication failure cannot acknowledge it. Panics discard the private stage and
leave the reference store unavailable. Evidence I/O remains outside callbacks,
followed by fresh authority validation. Coupled adapter registration/revocation
retains its separate explicit state/runtime lock boundary.

[Source-bound checks](goal-04-state-access-validation/checks.json) and the
[raw log](goal-04-state-access-validation/checks.log) record 374 workspace tests
passed (21 opt-in tests ignored), eight real PostgreSQL groups, ten actual Linux
T1 OCI/source-flow tests, strict Clippy, formatting, example builds and the
compatible/conflicting concurrent-transaction example. Four new publication/panic
tests accompany the existing generated-history oracle, idempotent rejection,
lost-response, authorization, lease and evidence-race tests.

The first workspace run failed a corrupted-ownership recovery-fixture assertion.
Its [original output](goal-04-state-access-validation/initial-failure.log) is
preserved. The exact initial scheduling cause was not captured. An isolated run
and [20 parallel diagnostic runs](goal-04-state-access-validation/recovery-reproduction.log)
passed before correction. A new deterministic descriptor-alias test demonstrates
that recovery must retain a still-held owner as live before classifying corrupted
ownership. The original fixture now waits boundedly for that owner to end and
retains its quarantine assertion and all security checks. Production recovery is
unchanged; see the contract and record for the precise evidence limits.

The [mechanical audit](goal-04-state-access-validation/mechanical-audit.json)
compares 38 wrapped operations to `8fae233` after accounting for callback wrapping,
reborrows and formatting. Its [script](goal-04-state-access-validation/mechanical-audit.py)
is a limited lexical aid, not a proof of Rust equivalence. The separately refactored
evidence preparation/retention path is covered by its existing adversarial/race
suite and actual T1 tests.

The reference adapter copies retained state on updates; this is an explicit
harness cost, not a whole-Database persistence design. One direct debug-history
test observation was 2.14 seconds and 5,468 KiB maximum resident memory; the
[raw measurement](goal-04-state-access-validation/histories-memory.txt) binds its
exact executable. It is not a capacity benchmark, performance target or SLO.
No SQL lifecycle, durable authorization/fencing, multi-process commit or later
service completion is claimed. The fixture was stopped with data preserved;
unrun checks are listed in the source-bound record.

## 04.1c — Checked semantic records and PostgreSQL candidate catalog

The [semantic persistence contract](goal-04-semantic-state.md) defines the new
snapshot-bound engineering-model record, unchanged repository-analysis v1 bytes,
bounded checked decoders and scoped immutable catalog. Decoding preserves model
completeness, dependency cycles, elements, checks and gaps. External snapshot and
generation requirements reject substituted records; candidate storage does not
install a current model head or authorize transactions. Schema v1 is unchanged.

[Initial source-bound checks](goal-04-semantic-persistence/checks.json) and the
[raw log](goal-04-semantic-persistence/checks.log) record:

- 369 workspace tests passed, with 20 opt-in tests ignored in the ordinary run.
- Seven real PostgreSQL groups passed, including semantic model/analysis reopening
  after actual database process restart, separate mapped runtime logins, forged
  content/format/version/scope/snapshot/generation rejection, immutable-conflict
  readback, absent-snapshot rejection, unchanged repository head and zero
  transaction rows.
- Ten actual Linux T1 OCI/source-flow tests and 28 LSP tests including the real
  rust-analyzer probe passed.
- Workspace formatting, strict Clippy, example builds, protocol integrity/four
  tooling mutation groups, workflow semantics and 25 attribution fixtures passed.

Eight new pure codec test groups cover frozen formats, full unsigned model
generation, cyclic/transitive model behavior, incomplete/complete facts, every
truncated prefix, forged digests, invalid tags/IDs/provenance, duplicate/reordered
collections and aggregate nested decoding budgets. The frozen analysis fixture
uses the published v1 layout; it does not assert full language understanding.

The final repository-flow test captures five trusted source files in an actual
Git repository (TypeScript, JavaScript, Python, Go and Rust), runs the structural
indexer and impact adapter, retains the exact analysis in PostgreSQL, closes and
reopens the catalog, and compares every restored fact/gap. A fresh, explicitly
administered reference Engine then installs those facts and rejects transaction
analysis with `IncompleteAnalysis`, leaving the transaction proposed and the
snapshot unchanged. This establishes that persistence does not erase gaps; it is
not a supported execution harness for all five languages. The final test and its
test-only repository dependency are bound by
[follow-up checks](goal-04-semantic-persistence/followup.json) and
[log](goal-04-semantic-persistence/followup.log): formatting, strict workspace
Clippy, 369 workspace tests (21 opt-in tests ignored) and all eight actual
PostgreSQL groups passed. Earlier source hashes and logs remain unchanged; the
follow-up records the exact changed test/lockfile hashes and unchanged runtime
implementation evidence.

The database fixture was stopped after validation with data retained. The record
states runtime/tool hashes and unrun distributed/capacity/recovery checks; elapsed
development-run times are not benchmarks or SLOs. Current-head authority, durable
request authentication, leases/fences and the full Engine lifecycle remain pending.

The public PostgreSQL Rust entry point now exports the catalog and generalizes
record-error text. Its exact CI public-input review digest was refreshed after
reviewing these public declarations. Default credential policy is unchanged; the
same four public paths still require exact reviews. No fixture credentials or
private configuration were added to Git.

## 04.1b — Engineering records and live embedding resources

The reference Engine separates its cloneable engineering records from live
capability identity and artifact readers. Sessions, executors, cleanup grants and
storage capabilities authenticate against the embedding's private allocation
identity, then retain their existing scoped grant, permission, expiry, revocation
and attempt checks. Multiple Engine handles sharing a store share that identity;
a new store with copied records does not.

Artifact registration metadata retains scope, retention, provenance and active
state. The reader lives in a separate process-local registry. New evidence
admission requires an installed reader; missing or poisoned runtime resources
return `StoreUnavailable`. Previously retained exact delivery receipts still
acknowledge history through the existing claim/registration checks. Reader I/O
remains outside Engine locks. Final admission rechecks current authority, metadata
and the exact reader handle after I/O.

Registration acquires state and runtime locks, in that order, before publishing
either record or handle. Revocation marks the registration inactive and removes
the reader together; it releases both locks before dropping adapter resources.
An in-flight read may retain its handle but cannot admit evidence after
revocation. This preserves the existing read/recheck boundary and permits adapter
destructors to reenter the embedding without a lock deadlock.

Record copying is confined to private tests. There is no public restore API,
whole-Database serialization, reconstructed authentication, SQL transaction
lifecycle or new source publication capability. Checked normalized reconstruction,
authenticated durable grant resolution, database-clock leases/fencing and atomic
multi-process commit remain required work for 04.1–04.2.

### Validation performed for 04.1b

[The source-bound record](goal-04-runtime-boundary/checks.json) and
[raw log](goal-04-runtime-boundary/checks.log) record 361 ordinary workspace tests
passed (18 opt-in tests ignored), 10 additional actual Linux T1 OCI/source-flow
tests passed, strict workspace Clippy, formatting, example builds, protocol
integrity and 25 attribution fixtures. Four new tests cover copied records across
all capability types, missing readers, resource release, destructor reentry and
poisoned-runtime publication failure. Existing authority changes during storage
I/O, concurrent delivery, exact replay and lost-response tests also pass.

The real T1 run uses the previously reviewed local Linux x86_64 rootless-runc
profile; the record includes exact runtime, launcher, formatter and library
hashes. This reruns current source-to-evidence behavior, not distributed recovery
or a new sandbox profile. PostgreSQL opt-in groups and the actual rust-analyzer
probe were not rerun locally for this Engine-only change; their implementations
are unchanged and prior evidence remains separately scoped. There are no new
performance or production-readiness claims.

## 04.1a — Schema, immutable snapshots and typed request prerequisites

This bounded prerequisite is committed separately before changing authoritative
Engine transaction execution. The canonical request schema is included because a
durable operation identity must preserve the complete accepted TaskIntent rather
than projecting it to the smaller reference Engine admission record.

Implemented components:

- `celetixo-postgres`: explicit authenticated local Unix-socket PostgreSQL 17
  configuration, bounded connection/statement/lock deadlines, required primary
  durability settings, and diagnostics restricted to SQLSTATE classifications.
- Transactional, advisory-lock-serialized migration with an exact checksum ledger;
  unsupported migration history/version skew rejects. Interrupted DDL and ledger
  publication roll back together. Commit acknowledgement errors are conservatively
  indeterminate; no in-memory backend fallback is provided.
- [Scoped authority schema](goal-04-postgres-schema.md): composite foreign keys,
  full unsigned revision/fence bounds, immutable rows and forced RLS based on
  independently authenticated login name plus role OID. Mutable authority tables
  are a future integration contract, not an implemented lifecycle service.
- Snapshot catalog: content-addressed candidate storage and checked reopen;
  initial registration cannot advance an existing head. It does not install
  semantic analysis or authorize Engine transactions. The Engine decoder restores
  the existing canonical format through validated constructors with explicit byte,
  field and collection budgets, exact canonical ordering and unchanged identities.
- [Canonical TaskIntent v1](../reference/protocol.md), generated Rust bindings,
  descriptor/golden fixtures, strict bounded intake and full semantic request
  digest. There is no RPC service or authenticated Engine adapter yet.
- Trusted [local PostgreSQL fixture](../../scripts/goal04/README.md) and CI jobs
  for the real database tests and reproducible protocol generation.

### Validation performed

[checks.json](goal-04-foundation/checks.json) and
[checks.log](goal-04-foundation/checks.log) record the actual development run.
[followup.json](goal-04-foundation/followup.json) and
[followup.log](goal-04-foundation/followup.log) cover the final protocol-integrity,
workflow/documentation and attribution checks.
[Source hashes](goal-04-foundation/source-manifest.json) bind the executable source,
fixtures, lockfiles, schema and workflow to these results. The validation record
is not an Engine verification receipt or production evidence admission.

| Check | Observed result |
|---|---|
| Workspace fmt and Clippy across all targets, warnings denied | Passed |
| Locked/offline Rust workspace tests | 353 passed, 18 ignored, zero failed |
| Checked snapshot decoding | Four new groups passed: layered binary/maximum revisions, malformed/identity substitution, resource budgets and constructor/canonical-order rejection |
| Actual PostgreSQL integration | Five groups passed: forged payload/scope/format/manifest rejection; concurrent migrations and ledger skew; child process death before migration commit; actual database restart/reopen; mapped-role isolation, immutability and full unsigned revision |
| Protocol Rust intake | Nine groups passed, including full-field identity mutations and malformed/unknown nested safety semantics |
| Protocol tooling | Integrity/ADR checks, independent compatibility/integrity mutations and exact regeneration with pinned protoc/prost passed; final mutation suite has four groups |
| Workspace/example build and concurrent reference transaction example | Passed; still the in-memory reference Engine |
| LSP tests including actual rust-analyzer | 28 passed |

The PostgreSQL fixture used actual PostgreSQL 17.9 with checksums, fsync,
full-page writes and synchronous commit enabled, no TCP listener or network
connectivity, and private SCRAM-authenticated Unix sockets. Its administrator and
migration owner are trusted; two separate nonowner/non-superuser/non-BYPASSRLS
runtime logins received explicit scope mappings and minimal catalog privileges.
`SELECT ... FOR UPDATE` also requires `UPDATE(state_generation)` on repositories;
no runtime access to the role map or general owner role is granted.

The recorded environment is Linux x86_64 under WSL, Rust 1.94.1, using the existing
shared build target. Tree-sitter C compilation used the documented local compiler
workaround `CRATE_CC_NO_DEFAULTS=1` and
`CFLAGS='-target x86_64-linux-gnu -fPIC'`; no repository C flags were changed.
Build/test elapsed times in the raw record are observations of this development
run, not cold benchmarks, performance targets or capacity measurements.

### Hosted workflow correction

Foundation commit `7eba8338bf5f097e941fb846eab47dd8b80077bf` was pushed with
human author/committer attribution. Its [first hosted run](https://github.com/Emiloart/Celetixo/actions/runs/43105839130)
failed before starting jobs: `runner.temp` was used in a job-level `env` context.
The local YAML parser had not checked GitHub expression semantics. The database
job now selects the fixture configuration from `RUNNER_TEMP` inside its running
step. No Rust, SQL or protocol behavior changed in this correction.

The [workflow checker](../../.github/scripts/check-workflows.py) pins the official
[actionlint 1.7.12 release](https://github.com/rhysd/actionlint/releases/tag/v1.7.12)
by archive SHA-256, parses every workflow without executing jobs, and verifies
rejection of the invalid-context regression. ShellCheck/Pyflakes subprocesses are
disabled; this check covers workflow semantics, not arbitrary shell/Python code.
It runs locally before publication and in CI. The separate
[correction record](goal-04-foundation/ci-correction.json) binds its exact source
and results. The original foundation source manifest and raw test logs remain
unchanged. A configured job is not a passing hosted run.

The [corrected hosted run](https://github.com/Emiloart/Celetixo/actions/runs/43106365291)
started its jobs: protocol generation/intake and attribution passed, while the
database fixture failed to resolve a hardcoded local image ID after successfully
pulling the selected repository digest. The fixture now selects the same immutable
repository digest directly, checks its cached platform metadata and prohibits
automatic pulls. Local daemon image IDs remain provenance/reopen checks. A fresh
fixture creation and the five real database groups were rerun for this change;
[image-selection validation](goal-04-foundation/image-selection.json) records the
source and results. This changes neither the schema nor Engine semantics.

### Public input review in repository smoke checks

The hosted Engine job passed Rust tests, Clippy and the actual rust-analyzer probe,
then correctly rejected credential-like text during capture of Celetixo itself.
The four reviewed public inputs are the PostgreSQL Rust connection implementation,
the trusted fixture script, the foundation follow-up log and image-selection
record. The source contains declarations and generated/private-file references;
the records contain the public `scram-sha-256` setting. They contain no fixture
credential values. Their exact content digests are installed explicitly in the CI
inspection and T0 patch-benchmark commands using the existing credential policy.

`inspect_repository` and `source_patch_bench` accept repeated
`--reviewed-public-input PATH=CONTENT_DIGEST` arguments from their trusted operator.
The digest is the Engine's domain-separated `celetixo/source-content/v1` address,
not raw SHA-256. A shared bounded argument parser validates canonical paths,
lowercase digest encoding, policy version and duplicate reviews before mutating
the policy. It never discovers or approves policy from repository files. Capture
retains these inputs and binds the installed policy into snapshot provenance;
changed bytes or a different path still reject. The default policy is unchanged.

Regression tests cover exact admission, changed bytes/path rejection, invalid
arguments without partial policy mutation, duplicate reviews, multiple explicit
versions and unsupported policies. [Public-review validation](goal-04-foundation/public-review.json)
records the source and checks. This is trusted read-only tool configuration;
transaction execution and evidence admission remain subject to their existing
Engine-owned policy boundaries.

All four [hosted CI jobs for `106935a`](https://github.com/Emiloart/Celetixo/actions/runs/43110552462)
passed: Engine regression/examples/smoke checks, canonical protocol generation and
intake, real PostgreSQL migration/restart/isolation, and attribution/workflow
validation. [The hosted record](goal-04-foundation/hosted-ci-106935a.json) binds this
claim to that exact commit. Milestone 04.1 remains partial; the database catalog
and schema have not become a durable Engine transaction backend.

### Import lookup cost during real-repository integration

A two-second CPU profile of the read-only inspection attributed 61.22% of 98
samples to the pending-import filter. Each reference scanned all pending imports
and references. The resolver now builds a fresh lookup by exact file and local
alias for each generation, retaining pending order and deduplicating repeated
bindings within one import. Scope visibility, lexical shadowing, ambiguity,
module resolution, occurrences and uncertainty handling keep their existing rules.

The test-only implementation retains the full scan as an independent oracle.
All language fixtures, rename/deletion/move/copy transitions and incremental/clean
rebuild comparisons pass, with a new order/multiplicity/file-boundary regression.
The complete inspection JSON for input commit `b31539c` is byte-identical before
and after the change: snapshot, generation, 237 files, 13,148 elements, 38,809 edges
and all 24,570 impact gaps remain unchanged. Semantic impact remains incomplete
and source mutation unsupported.

The two development commands took 429.283 seconds before and 209.275 seconds after,
including 13.60 and 14.67 seconds of compilation respectively. This single pair
includes capture, indexing and Engine queries under ambient development activity;
it is not an isolated cold-index benchmark, a service SLO or a capacity target.
[The lookup validation](goal-04-foundation/import-lookup.json) and
[raw check log](goal-04-foundation/import-lookup-checks.log) record exact source,
input, output digest and commands. Workspace Clippy, all 357 ordinary tests,
format checks and 28 LSP tests including actual rust-analyzer passed. This bounded
integration improvement does not complete any remaining durable-platform gate.

### Reproduce the database tests

Provision the exact cached image explicitly as an operator if it is absent, then
follow the [fixture instructions](../../scripts/goal04/README.md). Docker access
is privileged operator access and must never be passed to workers.

```sh
python3 scripts/goal04/postgres_fixture.py start
python3 scripts/goal04/postgres_fixture.py verify
export CELETIXO_PG_TEST_CONFIG="$HOME/.local/share/celetixo-goal04/service-fixtures/postgres17/connection.json"
cargo test --locked -p celetixo-postgres --test foundation -- --ignored --skip migration_crash_child --test-threads=1
```

Use one test thread because the restart scenario stops the dedicated database.
The child crash helper runs only under its parent; invoking it independently is
an error. Credential files stay private, outside Git, and are never printed.
The fixture has bounded resource settings and a disk-usage guard, not a hard disk
quota. Stop it after use; do not infer available Windows backing space from WSL's
virtual filesystem capacity.

### Unvalidated and unsupported boundaries

No durable Engine transaction, lease operation, fencing enforcement, SQL semantic
commit gate, lost commit-response reconciliation, process-level commit race,
object-store evidence admission, NATS event publication, Temporal workflow or
ClickHouse analytics exists in this foundation. The reference Engine still owns
in-memory transaction state and retains its full Goal 01–03 checks. Captured source
publication remains `SourceMutationUnsupported`.

Database isolation evidence covers the catalog operations and tested roles, not
end-to-end tenant isolation across future services. Schema constraints on future
lifecycle tables are not a proof of Rust transition semantics. Migration skew
checks cover migration/reopen; live upgrade and old-connection fencing remain
pending. A database process restart is not host power loss, failover, backup/restore
or a zero-loss guarantee. A runtime RLS boundary does not constrain a trusted
PostgreSQL administrator, host root or the operator's Docker privileges.

The ordinary workspace suite deliberately does not run the opt-in OCI/service
checks. The actual PostgreSQL tests and rust-analyzer probe were run separately.
The unchanged Goal 03 OCI profile was not rerun for this foundation; its earlier
source-bound evidence remains linked from Goal 03 acceptance. No production
readiness, whole-goal completion or service-level performance claim is made.

## Next implementation boundary

Implement 04.4 Temporal workflows against the same authoritative Engine path,
following [04.3 local-profile acceptance](goal-04-events-acceptance.md). Publishing an event
cannot create or reverse a commit. Continue the original 04.4–04.7 workflow,
object-store, analytics/shared-service and complete operational programme.

The [full Goal 04 contract](goal-04.md) remains the objective. Reviewed SQL adapter
acceptance does not establish production network identity, host-loss/failover,
backup/restore, load acceptance or the complete integrated platform.
