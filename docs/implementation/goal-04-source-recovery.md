# Source authority reconstruction and fresh runtime recovery

**Status:** checked source reconstruction and fresh runtime recovery validated
through a normalized memory adapter; regression checks are recorded below.
Its original validation covers the memory adapter. The subsequent
[source SQL lifecycle](goal-04-source-sql-authority.md) integrates this graph
through evidence receipts. Source revision commit remains unsupported.

**Authority:** [Goal 04](goal-04.md), ADR-0001/0003, the
[transaction](../spec/transaction-semantics.md),
[identity](../spec/engineering-identity.md) and
[failure](../spec/failure-model.md) contracts. This is one consolidated source
recovery capability within 04.1, not an additional top-level goal or a reduction
of the seven accepted milestones.

## Preserved authority graph

`AuthorityState::from_source_records` checks the complete captured-source metadata
graph before exposing it to an Engine callback. `source_records` exports that
same graph. `BaseRecords` names shared typed rows; `NativeRecords` remains its
compatible API name. Native-only import/export still rejects source state.
Source reconstruction never strips source facts to pass native validation or
manufactures an integer patch to satisfy a native transaction invariant.

The source graph retains:

- Scope, authoritative clock, session counters/grants, current and initial
  repository snapshots, model, original intents, transaction attempts, semantic
  accesses/analysis, reservations, fences, lease events and terminal reasons.
- Full immutable source analysis and policy catalogs, independently selected
  heads, and the exact source analysis/policy used by each transaction attempt.
  Replacing a head does not erase history. A stale policy head after index
  replacement is valid stored state; current operations still reject it.
- Every source admission, original patch and optional selection, owner grant,
  admission fence and lease-event cutoff, replayed candidate/report, executor
  claim, state, evidence selection and ordered transition events.
- Every recorded admission request body, original typed result, key/digest, owner
  grant, resolved target attempt, observed analysis/policy/model context, clock,
  fence and lease-event cutoff. Original rejected requests are not reconstructed
  from today's policy or successful attempts. Identity collisions can retain a
  separate rejection result without creating another rejection-view row.
- Executor definitions, tiers, backend/resources, expiry and active status;
  cleanup definitions; artifact registration retention/provenance/active status;
  complete evidence envelopes, receipts, assessments and storage associations.

An invalid submitted attempt number is data in a rejected request, not an
actual foreign-key target. For example, a request naming attempt 999 can be
rejected while its resolved authoritative transaction attempt is 0. Both values
survive recovery. SQL integration must add a representation for this distinction
rather than inventing an attempt or editing published migrations.

Original request bodies, including rejected inputs, remain privileged repository
data. Retaining them for authority recovery does not authorize trajectory export,
retrieval/training reuse or disclosure in diagnostics. Durable integration must
apply the repository's access, retention and governance policies to these bodies.

The original admission format is unchanged. A separate lease-event cutoff records
which events preceded admission. This preserves same-clock-tick history: a later
revocation at the same millisecond cannot invalidate the structural record of an
earlier valid admission. It also cannot revive current lease authority.

## Checked reconstruction and decisions

Repository/model/source bindings, constructor invariants, semantic derivation,
policy path preimages and write authorization are checked. Plans are re-resolved
from original intent, impact/invariant requirements and optional selections.
Patches are replayed against original immutable bases; candidate, manifest,
application-report, plan, admission and provenance identities must agree.

Transition sequences, historical time ordering, owner/executor/store references,
lease associations, original request/result relationships and evidence selections
are validated. Historical receipt time must precede both admission and executor
expiry; equality rejects. Passed source evidence can legitimately put a transaction
into
`Validating` while its native patch/candidate fields remain absent. Historical
expired or revoked facts remain facts. Import does not perform lazy reconciliation
or invent expiry/cancellation events; explicit Engine update operations do that.

`SourceRequestRecord` stores the full existing patch plus exact sorted optional
checks under `celetixo/source-request-record/v1`. The existing semantic operation
identity remains unchanged: empty options still use the patch address; nonempty
options retain the existing `celetixo/source-attempt-request/v1` identity.

`SourceOutcomeRecord` uses `celetixo/source-outcome-record/v1` and binds scope,
transaction, resolved attempt, key, request digest and the original admission ID
or typed rejection. Source attempt/plan/patch/credential/evidence/artifact and
repository-analysis errors retain structured variants and exact payloads. Static
producer diagnostics have an explicit finite vocabulary; unknown values reject
without leaked allocations or string fallback. Existing native outcome bytes and
their source-family rejection behavior are preserved. Infrastructure, injection
and lost-response errors are not converted into durable domain decisions.

Existing unprepared source calls to the native commit API can have a retained
`SourceMutationUnsupported` result. These are preserved explicitly with their
original transaction/attempt selector. Recovery does not manufacture a prepared
request or accept a source commit through that legacy path.

A nonallocating preflight charges common and source data against aggregate entry,
per-value and total-value limits before cloning, canonical serialization,
resolution or patch replay. Definitions, rationales, nested collections, snapshots,
owned request/plan/evidence copies and replay bounds are included. Defaults match
the existing authority limits: 100,000 entries, 64 MiB per value and 256 MiB total.
These are conservative canonical-data budgets, not exact allocator RSS or CPU
quotas. They do not establish large-repository capacity.

## Fresh runtime boundary

All source administrative, session, executor and cleanup operations now use
explicit scoped authority callbacks. Executor and cleanup actor metadata is
attribution; Rust still checks the opaque capability and current scoped grant.
The PostgreSQL native profile rejects these source actors and source state rather
than falling back to process memory.

Import restores no live session, executor, cleanup capability or artifact reader.
Trusted embedding methods independently authenticate existing executor/cleanup
records outside state callbacks, then re-read exact current metadata, scope,
revocation and expiry before issuing a new embedding-bound capability. Old
process capabilities and numeric selectors alone fail. Fresh authentication does
not create a second execution claim or authorize a duplicate launch. The recovery
test continues evidence admission for already-claimed work; physical execution
and journal reconciliation remain governed by the existing Goal 03 runner and
future durable workflow recovery contract.

Artifact resolution is a separate trusted operation. It receives the exact scoped
registration/retention/provenance, resolves a reader outside state/runtime locks,
checks its retention and rechecks registration before local attachment. Readers
are keyed by `(Scope, registration ID)`, so independently allocated scoped IDs
cannot alias. An existing different reader is not silently replaced. Missing or
revoked readers fail closed; reader calls and destructors run outside locks.

Metadata publication and local adapter installation are separate boundaries.
A registration retained before attachment fails remains available for explicit
reconciliation; this is not cross-resource atomicity. Current legacy registration
metadata does not define a production adapter configuration or identity service.
The eventual SQL/service integration must provide its explicit trusted resolver
configuration and exact administrative request reconciliation.

Historical evidence validation reuses the live check metadata/assessment logic,
without storage I/O. It checks exact envelope/provenance addresses and bindings,
execution-record metadata and original assessment. This does not re-establish
current physical object availability. Live admission still independently retrieves
and verifies all objects and rechecks authority after I/O. A restored `Passed`
receipt is not a new verification run or permission to commit source.

## Scope and validation

[Source-bound checks](goal-04-source-recovery-validation/checks.json) and the
[log](goal-04-source-recovery-validation/checks.log) record exact source and
commands. Focused tests cover transaction/source lifecycle reopening, replaced
heads, exact optional requests, invalid attempt 999, identity collisions,
same-tick revocation, lazy expiry, corruption and aggregate budgets. Evidence
tests preserve Passed/Failed/Incomplete receipts despite missing readers and
current revocation; a fresh embedding explicitly authenticates/resolves resources
and resumes a claimed attempt through evidence admission and a second reopen.
Reentrant callbacks prove authentication, artifact reads and reader destruction
do not hold authority/runtime locks. Separate scopes reuse executor, cleanup and
storage ID 1 without aliasing. Independent frozen outcome layouts and rehashed
malformed requests test the new canonical formats; native fixtures remain intact.

The final Rust pass records **442 passed** ordinary workspace tests with 45
opt-in tests ignored, workspace formatting and strict all-target Clippy. The
integration run separately passed **10 actual Linux T1 execution tests** and
**32 real PostgreSQL test groups**, plus example builds/run, workflow checks,
protocol integrity/four compatibility mutation groups and 25 attribution
fixtures. Two ignored crash helpers are exercised by their parent tests; the
real rust-analyzer probe was not rerun. The validation record preserves the
original run's manifest changes and the subsequent correction pass over unchanged
final source, including the historical executor-expiry boundary regression.

Source reconstruction currently covers captured-source execution before source
publication, including empty pre-registration scopes. It explicitly rejects
source revision advancement, mixed native evidence/prepared commits and source
revalidation states that the current Engine cannot produce. The registered
source base remains revision zero; private applied candidates are not proposal
bases. These are explicit existing capability boundaries, not whole-04.1
acceptance.

The source backend used by recovery tests stores normalized rows in memory. It
proves checked reconstruction, scoped callback routing and fresh runtime behavior;
it does not prove PostgreSQL source durability. Actual native PostgreSQL regression
and existing local Linux T1 checks are separately reported by the validation
record. No hosted CI, failover, backup/restore or production-service claim follows.

The subsequent [SQL source profile](goal-04-source-sql-authority.md) integrates
these complete records with additive schema, load/publication/readback and real
restart/race/fault tests. Exact identities for
all other retriable mutations, source prepared commit/revalidation, NATS, Temporal,
S3, ClickHouse and the complete operational programme remain required by Goal 04.
