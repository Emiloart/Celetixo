# Executor mutation recovery validation

Final consolidated validation passed for
[original executor outcomes](../goal-04-executor-recovery.md). Focused development
checks have exercised normalized Engine recovery, coordinator faults and actual
PostgreSQL connections/processes. These are local development checks, not
Engine-admitted evidence or complete Goal 04 acceptance.

The final sequence binds exact source hashes and separately reports ordinary
workspace tests, actual Linux T1 probes and the serial PostgreSQL suite. It also
checks formatting, strict workspace lint, examples, workflows, protocol,
attribution and public-input admission. Hosted CI and the unchanged real
rust-analyzer probe are not inferred from local results.

The PostgreSQL fixture uses the existing reviewed isolated PostgreSQL 17 profile
with SCRAM Unix-socket authentication and durable database settings. Its data,
private connection configuration, artifact bytes and raw development failures
are retained privately. Source checks in SQL fixtures are synthetic.

To reproduce after configuring a dedicated fixture:

```sh
CELETIXO_PG_TEST_CONFIG=/reviewed/private/connection.json \
  cargo test --locked -p celetixo-postgres --test foundation -- \
  --ignored --skip migration_crash_child \
  --skip source_authority::recovery::source_process_child \
  --skip source_authority::lifecycle::mutations::executor::executor_replay_child \
  --test-threads=1
```

Parents invoke child helpers with explicit context. The suite restarts its
dedicated database. Stop the fixture when finished and retain its data.

The first frozen-source sequence in [checks.json](checks.json) and
[checks.log](checks.log) passed formatting, strict workspace all-target Clippy,
504 ordinary workspace tests (102 opt-in tests ignored) and 10 actual Linux T1
tests. Source hashes stayed unchanged. The PostgreSQL database-restart parent
then encountered the existing 512 MiB physical backing reserve: Windows C: had
about 498 MiB free. The fixture could not restart; subsequent groups failed to
connect. That full SQL attempt recorded 2 passed and 85 failed groups and is not
SQL acceptance evidence. The fixture was stopped with all its data retained.
No file deletion or reduction of the storage guard was used to continue.

Earlier focused SQL development checks passed original executor replies,
rollback/lost responses, independent-connection races, cold-process replay,
partial-row/duplicate-event constraints and missing/mismatched raw selectors.
Separate focused checks passed the published v5 original-row upgrade, migration
error/process-death rollback and keyed final-publication expiry rollback.
Seven Engine recovery tests include an independent byte layout and decode
budgets; 18 coordinator tests passed, including three new lost-reply cases.
[Development attempts](development-checks.json) distinguish these results and
retain failure provenance. Complete raw development outputs remain private.

[Remaining checks](remaining-checks.json) and [log](remaining-checks.log) passed
workflow semantics, protocol integrity/four compatibility mutation groups,
25 attribution fixtures, benchmark-summary rejection tests and diff checks.
These required no Cargo compilation. Deferred examples and all PostgreSQL parent cases passed after storage became
available, using the same frozen sources; their evidence is recorded below.

A separate [SQL predicate review](null-shape-review.json) evaluated the exact
eight migration predicates with SQLite 3.45.1 in memory. All four accepted-action
baselines and both original-rejection contexts were admitted; all 28,668 nonempty
NULL masks of mandatory source fields were rejected. This bounded review covers
SQL conditional behavior. PostgreSQL foreign keys, RLS, migration and durability
require the real database checks described above.

The resumed monolithic SQL process was later interrupted without a final result;
its [partial log](interrupted-checks.log) and [interruption record](interrupted-checks.json)
are retained and are not counted as a completed pass. Validation
then ran every one of the same 87 parent cases serially in separate processes,
with fsynced case receipts tied to the exact executable and frozen source hashes.
The [case report](checkpointed-sql-checks.json) and [log](checkpointed-sql-checks.log)
cover the complete enumerated parent list. No case or assertion was removed.
Parent-driven child processes are included in their parent case, not counted as
extra groups. This proves those cases in that execution arrangement; a successful
single-process whole-suite invocation or hosted CI run is not claimed.

The [example checks](example-checks.json) and [log](example-checks.log) passed
workspace example builds and the Engine concurrency example. The [combined
results](final-results.json) bind these gates to the same 255 source files as the
504 workspace and 10 actual Linux T1 checks. The selected PostgreSQL profile and
stop receipt are recorded separately; fixture data is retained.

The successful serial per-case SQL run took 2340.697 seconds wall time. This is
a local validation duration, not a throughput benchmark or production capacity
claim. [Fixture settings/ledger](postgres-profile.json) record the live reviewed
PostgreSQL 17.9 profile and migrations 0001–0006. The [stop receipt](postgres-stop.json)
confirms that this fixture was stopped with its data retained.

The combined SQL log removes extra blank lines at EOF for repository whitespace
checks. Its presentation hashes are recorded in the case report; individual raw
case logs and their hashes remain unchanged.
