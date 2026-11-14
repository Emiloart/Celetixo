# Milestone 04.1 acceptance

**Status:** accepted for the implemented PostgreSQL authority adapter on the
reviewed Linux fixture, at implementation commit
`0918af94bcfa3351d902139069d31a221b76e15e` and schema v8.
Goal 04 remains active; milestone 04.2 and the later service/operational milestones
are incomplete. This audit establishes durable native and captured-source state
under the stated profile. It does not establish a production service deployment.

The original [04.1 criterion](goal-04.md#041--durable-state-model-and-postgresql-authority)
is schema, constraints, migrations, transaction persistence, revision heads,
attempts, idempotency, reservations/fencing and repository scope, with Engine
semantics preserved and restart/reopen demonstrated. The table below evaluates
those criteria. Source distributed-commit oracle expansion and cross-service
operations continue under their original 04.2–04.7 criteria.

| Criterion | Implementation and validation |
|---|---|
| Scoped durable schema and constraints | [Canonical schema](goal-04-postgres-schema.md), immutable migrations 0001–0008 and checked normalized SQL adapters. Scoped foreign keys, forced RLS, immutable payload/history/context rows and exact unsigned values retain organization/workspace/repository identity. The Engine checks semantic validity independently of SQL shape. |
| Repository revision heads | [Native authority](goal-04-native-sql-authority.md) and [source publication](goal-04-source-publication.md) recover typed history and exact head/model associations. Atomic publication includes revision advancement, terminal state, reservation release, original outcome and outbox fact. Outbox delivery is separate remaining work. |
| Transactions and attempts | [Complete source recovery](goal-04-source-recovery.md), [SQL lifecycle](goal-04-source-sql-authority.md) and [fresh source revalidation](goal-04-source-revalidation.md) preserve immutable intents, numbered attempts, explicit/derived accesses, policies, source events, evidence and artifact associations. Revalidation creates an empty execution frame and requires a new patch/checks; old workers cannot authorize it. |
| Original idempotent outcomes | [Mutation recovery](goal-04-mutation-recovery.md), [executor recovery](goal-04-executor-recovery.md), prepared source commits and fresh source revalidation retain exact typed requests/results and reject changed-body key reuse. Recovery reauthenticates the current original actor without renewing leases or launching work. |
| Durable reservations and fencing | Native/source SQL records preserve acquire/renew/release events, owner grants, expiry and monotonic fences. Final database-time checks reject expired original leases, including expiry during source revalidation. New epochs atomically release the old fence and acquire the replacement. |
| Preserved Engine semantics | The frozen workspace run passes the independently structured 192-seed reference-history suite and its shrinking regression. The supplemental SQL run passes all 16 generated native pair histories against its independent bitmask oracle. Nine source revalidation tests cover fresh epochs, late evidence, authorization/expiry/cancellation, historical request recovery and forged records. Finite source fixtures do not prove language extraction completeness. |
| Reopen and actual process restart | The supplemental real PostgreSQL restart test checks native/source reconstruction, fresh authentication/reader resolution and original receipt recovery. The primary revalidation proof separately preserves all 256 logical rows across 60 scoped tables for one committed two-epoch source transaction over a graceful database stop/start. |
| Supported migrations | The selected source validation database was created empty and migrated through v8. Current checks cover upgrades from every supported prior schema, v1–v7, with checksum/history preservation, interruption rollback and process-death cases. Live schema-skew detection and concurrent migrator regressions pass. Published migration bytes are unchanged. |

## Evidence and checks

[checks.json](goal-04-authority-acceptance/checks.json) binds this audit to the
[frozen implementation manifest](goal-04-source-revalidation-validation/source-manifest.json)
and [primary validation](goal-04-source-revalidation-validation/checks.json).
The implementation inputs are unchanged between those checks and this audit.
The primary record establishes 548 workspace passes (114 environment-dependent
cases ignored), 10 separately run actual Linux T1 cases and 14 selected SQL parents,
plus strict Clippy, formatting, examples, workflow/protocol and attribution checks.

[The supplemental log](goal-04-authority-acceptance/checks.log) records seven more
SQL parents on the same sources: v2/v3/v4/v6 upgrades, actual PostgreSQL process
restart, empty-schema process-death recovery and the 16-seed native reference-oracle comparison. The primary SQL set
already covers v1/v5/v7 upgrades and source revalidation process races/crashes.
The 21 parent cases are selected validation, not a whole database-suite result.
No hosted CI success is inferred from local checks.

The separate optional committed-tree repository inspector was interrupted after
257.603 seconds; the subsequent patch benchmark did not run. Neither is counted
as passing. Exact reviewed-input rejection/admission regressions and a complete
public-file audit passed with the unchanged default credential policy.

## Remaining Goal 04 work

04.2 still requires complete source independent-process commit races and broader
generated-history equivalence through source commit/revalidation. Existing native
process races and source revalidation process races establish only their named
cases. The existing local artifact adapter's exclusive writer ownership is not a
distributed artifact service; shared readers/storage must retain independent
verification and current scoped authority.

The original 04.3–04.7 work remains: canonical transactional event delivery and
NATS consumers, Temporal workflows/reconciliation, immutable S3-compatible storage
and governance, replayable ClickHouse/shared adapters, production service
identity/transport, and complete security/failure/backup/restore/load proof.
A local graceful PostgreSQL restart is not failover, host power loss or a tested
backup/restore procedure. No strict serializability, universal correctness,
complete language harness, T2/T3 or public deployment claim is made.

The [full Goal 04 contract](goal-04.md) remains unchanged at SHA-256
`339c328bf93bb512b8f90a8006d3dc07b8ed031399a93c3a3c151f45940b8743`.
