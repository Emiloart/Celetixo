# Mutation recovery validation

This directory records local development validation for
[durable original mutation outcomes](../goal-04-mutation-recovery.md).
It is not Engine-admitted evidence or complete Goal 04 acceptance.

Final commands, source hashes and results are recorded in [checks.json](checks.json)
and [checks.log](checks.log). Private connection credentials, database contents, artifact bytes
and raw development output remain outside the repository. Each development
version of unpublished migration 0005 used a separate retained database;
applied migration checksums were never rewritten.

The final source manifest is captured before verification. Only documentation
and validation artifacts may change afterward. Published migrations 0001–0004
and the complete accepted Goal 04 contract remain unchanged.

Development findings retained privately include rejected incomplete fixtures,
an intentional delay exceeding the unchanged SQL idle-transaction bound,
constraint probes encountering an earlier valid uniqueness check, and test
assumptions about pre-bootstrap inspection and operation-time privilege checks.
Corrections preserved production admission, authority and rollback assertions.
Replay comparisons permit monotonic database time advancement while comparing
every other normalized fact exactly.

The scope includes exact original successful/rejected replies, changed-body
rejection, fresh authentication, cold recovery, rollback/lost replies, independent
process and connection races, final-time authority checks, malformed normalized
records and scoped SQL constraints. Actual PostgreSQL and Linux T1 checks are
reported separately from ordinary workspace tests. Hosted CI, the real
rust-analyzer probe and the remaining Goal 04 requirements are not inferred from
these local results.

The first final workspace attempt stopped before a result was recorded when its
process and tool handle disappeared during a continuation. Its raw output was
retained. The resumed run then rejected stale exact-review constants in the
repository capture test. Two frozen constants were refreshed for the inspected
Engine source bytes; default rejection, exact-path and changed-byte assertions
were preserved. `development-checks.json` and `development-checks.log` retain
that failed run. Final checks use a fresh source manifest after the correction.

To reproduce database checks, start/verify a dedicated fixture using
[scripts/goal04](../../../scripts/goal04/README.md), then supply its private
configuration to the serial test suite:

```sh
CELETIXO_PG_TEST_CONFIG=/reviewed/private/connection.json \
  cargo test --locked -p celetixo-postgres --test foundation -- \
  --ignored --skip migration_crash_child \
  --skip source_authority::recovery::source_process_child --test-threads=1
```

Parent tests invoke crash helpers with explicit private context. The suite also
restarts the selected dedicated database; use an isolated fixture. Stop it after
the run and retain its data. Linux T1 reproduction uses the existing
[operations runbook](../../operations/local-linux-t1-runbook.md).

Final verification passed 494 ordinary workspace tests (94 opt-in tests
ignored there), 10 actual Linux T1 tests and 80 PostgreSQL parent groups.
8 child-process result summaries belong to their parent groups and are not
added as additional groups. Formatting, strict workspace all-target Clippy,
example builds/run, workflow semantics, protocol integrity/four mutation groups,
25 attribution fixtures and benchmark-summary rejection tests passed. Source
hashes remained unchanged during this final sequence. The ordinary ignored
tests ran separately except explicit helper children and the unchanged real
rust-analyzer probe, whose execution is not claimed.

The full PostgreSQL run took 1930.158 seconds. After the frozen-source checks,
the CI database job allowance changed from 30 to 45 minutes and its standalone
source crash helper was excluded in favor of its parent-driven invocation,
matching the measured command. [Workflow addendum](workflow-addendum.json)
records the exact before/after workflow hashes and a passing semantic lint run.
Production code, tests, SQL deadlines and execution limits stayed unchanged;
`checks.json` retains the original run manifest and the workflow-only publication
override. Hosted CI execution remains unconfirmed.
