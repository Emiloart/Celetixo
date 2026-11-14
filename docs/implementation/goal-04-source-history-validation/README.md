# Generated source-history validation

[Milestone 04.2 acceptance](../goal-04-commit-acceptance.md) describes the reviewed
adapter scope. [checks.json](checks.json) records 552 workspace passes (118
opt-in cases ignored), strict Clippy, 48 generated Engine source histories and a
complete 16-history real PostgreSQL process corpus. Three additional SQL parents
check original-key races, process exit before/after commit and fresh source
revalidation/cold reopen. These four parents are a selected set, not the whole
SQL suite. Child invocations are not counted as additional parent tests.

[source-manifest.json](source-manifest.json) binds the exact 284 frozen inputs.
[checks.log](checks.log) retains SQL output and workspace group summaries, with
private path prefixes replaced and trailing whitespace removed. Per-check hashes
name retained original raw logs. The completed workspace/Clippy receipts use
unchanged sources with default optimization and disabled debug symbols. SQL uses
opt-level 2, explicit debug assertions/overflow checks and two build jobs. All
source hashes were checked before and after every stage.

The original SQL parent was interrupted after 14 seed messages and supplied no
passing parent receipt. A redundant optimized workspace rebuild was intentionally
stopped after its libraries completed. Neither interruption is counted as passing.
The completed workspace/Clippy checks were retained on exactly unchanged source;
the final run repeated all 16 SQL histories and all three selected regressions.

After those checks, the only workflow change selects the same tested optimized
SQL profile for CI. [workflow-profile-delta.json](workflow-profile-delta.json)
records its old/new hashes, exact environment and passing workflow validation.
It changes no test selector, assertion, fixture control or timeout. Every other
frozen input remains unchanged. Hosted CI for this commit is unclaimed.

Production Rust, canonical protocol and published migrations 0001–0008 are
unchanged from the parent implementation. Actual Linux T1 and rust-analyzer
probes, protocol generation and whole-repository indexing/patch-benchmark smoke
were not repeated for these test additions; their earlier evidence remains
separately scoped. The preceding source-process milestone ran all 10 actual T1
cases on unchanged production code.

SQL histories use actual PostgreSQL and retained objects with independent Engine
processes, finite analyzer facts and synthetic check evidence. They do not prove
source-extraction completeness, actual sandbox execution in those histories,
strict serializability or production network-service behavior. Goal 04 remains
active with its complete 04.3–04.7 service and operational requirements.
