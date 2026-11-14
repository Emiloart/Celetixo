# Fresh source revalidation validation

This bounded local record covers [fresh source revalidation](../goal-04-source-revalidation.md),
not complete Goal 04 or milestones 04.1/04.2. [checks.json](checks.json) records
commands, results, scope and limitations. [source-manifest.json](source-manifest.json)
binds all 282 implementation, test, protocol and workflow inputs to the same frozen
source set. [checks.log](checks.log) presents the output with private path prefixes
replaced and trailing whitespace normalized; original raw hashes remain in the
receipts and original development logs remain privately retained.

- 548 ordinary workspace tests passed; 114 environment-dependent cases were ignored.
- All 10 selected actual Linux T1 execution cases passed separately.
- Fourteen selected PostgreSQL parent cases passed serially in separate test
  processes. They include fresh epoch/commit/reopen, independent-process races,
  process exit before/after publication, original-lease expiry at final database
  time, forced RLS/immutability, v7-to-v8 migration rollback and affected native,
  source-publication and schema-fencing regressions. Child helper executions are
  nested under their parents and are not counted as extra passing cases.
- A graceful PostgreSQL process stop/start preserved all 256 logical rows across
  60 scoped tables for one committed source transaction with two epochs, including
  its revalidation context, original result, reservations, evidence and outbox.
- Strict workspace Clippy, formatting, example builds/reference execution,
  workflow semantics, protocol compatibility/mutation checks, attribution fixtures,
  benchmark-summary fixtures and diff checks passed.

The test profile disables debug symbols to limit disk use while retaining debug
assertions and overflow checks. Strict Clippy used the default symbol profile on
the same frozen source set. The fixture retained its 1 GiB data/20,000-file guard,
512 MiB backing-space reserve, ten-second idle transaction timeout, restricted
roles and network-denied configuration.

To reproduce, use the [reviewed PostgreSQL fixture](../../../scripts/goal04/README.md)
and [Linux T1 runbook](../../operations/local-linux-t1-runbook.md). Set the private
connection-file selector as described there, run the workspace/Clippy commands
in `checks.json`, then run each named PostgreSQL case with its recorded `--exact`
command. The selected case list is explicit; this record does not claim the whole
opt-in database suite passed. Published migrations 0001–0007 remain unchanged;
0008 adds the new context projections and deferred epoch bindings.

Finite analyzer and synthetic source check fixtures validate metadata and
transaction semantics. Separate actual T1 cases exercise the existing runtime.
They do not prove complete language extraction or an actual T1 revalidation
end-to-end scenario. The unchanged real rust-analyzer integration and hosted CI
were not rerun here. Full source generated-history oracle equivalence, shared
services and production transport, host-loss/failover, backup/restore and load
acceptance remain open under the full Goal 04 contract. A graceful local database
restart establishes only the stated local preservation result.
