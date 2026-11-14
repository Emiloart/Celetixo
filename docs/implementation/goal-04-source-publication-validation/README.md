# Source publication validation

This record covers the bounded source-publication milestone, not complete Goal
04 or milestones 04.1/04.2. [checks.json](checks.json) names the checks and limits;
[checks.log](checks.log) presents their output with trailing whitespace normalized.
Original failed and passing development logs remain retained privately.

- 539 workspace tests passed; 107 environment-dependent tests were ignored.
- All 10 explicitly selected real Linux T1 cases passed.
- Fifteen named PostgreSQL parents passed in separate processes, including source
  publication/reopen, immutable rows, rollback, lost responses, independent Engine
  instances, final-time expiry, v3/v5/v6 upgrades, native process races and affected
  source regressions. This is a selected regression set, not a full SQL-suite pass.
- An actual graceful PostgreSQL process restart preserved 193 logical rows across
  57 scoped tables for one committed source revision. This establishes the stated
  local restart result, not host-loss or backup/restore recovery.
- Strict workspace Clippy, formatting, examples, reference execution, workflow,
  protocol compatibility, attribution fixtures and diff checks passed.

[sql-source-manifest.json](sql-source-manifest.json) binds the frozen SQL run.
[source-manifest.json](source-manifest.json) binds subsequent workspace and final
checks. Their only difference is two reviewed digest literals in one public-input
fixture; [reviewed-fixture-delta.json](reviewed-fixture-delta.json) records it.
Detector behavior, mismatch rejection assertions and SQL production sources did
not change. Matching workflow review arguments were refreshed separately.

Synthetic source check records validate Engine metadata semantics. Separate T1
cases exercise the existing isolated runtime. Hosted CI, unchanged real
rust-analyzer integration, fresh source revalidation, full source history/oracle
equivalence, service integrations and remaining operational gates are unclaimed.
