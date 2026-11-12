# Goal 04 explicit state evaluation and publication

**Status:** 04.1d prerequisite verified; reference Engine only. PostgreSQL
transaction lifecycle and distributed commit remain pending.

**Authority:** [Goal 04](goal-04.md), ADR-0003 and the
[failure model](../spec/failure-model.md).

## Implemented boundary

State-only Engine operations now enter a private `read` or `update` callback.
Reads receive an immutable state reference. Updates evaluate against a private,
adapter-free stage, explicitly invoke publication, install that stage together,
then return the domain outcome. Dropping a stage never commits it. Publication
failure withholds both a proposed success and a proposed rejection. A panic
leaves the previous state intact and the in-memory mutex poisoned; subsequent
operations fail closed without a poison-recovery fallback.

Domain errors do not automatically discard their state changes. Existing Engine
operations intentionally retain rejected idempotency outcomes, revalidation,
expiry reconciliation or an evidence receipt whose response was lost. The update
boundary publishes those changes before returning their existing domain error.
Named pre-publication faults still leave no partial publication; the existing
commit gate's private staging remains intact.

The public Engine/session APIs and Rust admission, impact, invariant, evidence,
lease, conflict, revalidation and commit checks retain their existing contracts.
The source-attempt observation methods that reconcile expiry remain updates,
even though they return a view or clock reading. Actual read-only operations do
not publish a stage.

## Artifact and runtime resources

Evidence admission prepares either a previously retained receipt or a bound
artifact-read context in a read callback. It performs artifact I/O outside state
callbacks, then enters an update callback that rechecks current grant, claim,
attempt, policy, lease, storage registration and exact reader handle before
retaining evidence. Exact replay still acknowledges immutable history through its
existing authorization/claim/registration checks.

Trusted artifact-reader registration/revocation retain their explicit coupled
state/runtime lock boundary from 04.1b. They update process-local adapter handles
and metadata together, and drop removed adapters outside the locks. Runtime
objects and opaque capabilities never enter the staged engineering records.

## Limits and durable integration requirements

The reference adapter still serializes local operations with one mutex. It copies
the reference Database for updates and uses a local publication callback, so the
copy cost grows with retained reference state. This is an explicit reference
harness cost, not a production scalability design or a whole-Database persistence
format. Immutable snapshot/model/analysis codecs and candidate catalogs are
unchanged. No public pluggable-storage API is introduced by this prerequisite.

A PostgreSQL backend must load checked, scoped normalized records, authenticate
the request, acquire documented locks and authoritative clock/versions, invoke
the same Rust decisions, persist intended outcomes and outbox facts, conditionally
advance revision, and confirm SQL commit before acknowledgement. It must retain
intentional domain-error state changes, reject stale leases/fences and reconcile
ambiguous commit by complete request-bound idempotency. It must not use local
staging or a callback return as proof of durable commit, reconstruct capability
identity from IDs, or fall back to memory when PostgreSQL is unavailable.

## Recovery-fixture correction

The first workspace run observed a zero quarantine count at the corrupted
ownership check in `registry_retention_corruption_symlinks_and_limits_fail_closed`.
The isolated test and 20 subsequent parallel fixture runs passed. The initial
failure did not capture the full recovery report, so its exact scheduling cause
is not established.

A deterministic regression now duplicates the owner's open file description,
quarantines the controller, corrupts ownership, and verifies that the still-held
descriptor keeps recovery live without deletion. Once that alias closes,
recovery must quarantine the corrupt record and retain the source. This models
the shared flock lifetime that a concurrently created pre-exec child can briefly
retain, even for close-on-exec descriptors. See the Linux
[flock](https://man7.org/linux/man-pages/man2/flock.2.html) and
[open](https://man7.org/linux/man-pages/man2/open.2.html) manuals for shared open
description locks and the close-on-exec boundary. The regression proves the
descriptor-alias case; it does not establish that case as the initial failure's
observed cause.

The original fixture now waits at most three seconds for a reported live owner
to end, asserting the exact live path, no removal/quarantine and source existence
on every wait. It retains the final corrupted-record quarantine assertion and
all retention, symlink and limit checks. It fails if the owner remains live or any
unexpected result appears. Production recovery behavior and unsafe-code policy
are unchanged; descriptor-duplication helpers compile only in unit tests.

## Verification

The [progress record](goal-04-progress.md) links source-bound checks, preserved
failure/reproduction output and limitations. Tests cover domain-error retention,
publication failure before acknowledgement, panic rollback and poisoned-store
rejection, alongside the existing independent generated-history oracle,
idempotency, concurrent handles, source-evidence races and actual Linux T1 flow.
Local publication tests establish neither PostgreSQL rollback behavior nor
multi-process correctness; those remain required Goal 04 tests.
