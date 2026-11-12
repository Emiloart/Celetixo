# Source PostgreSQL lifecycle validation

These are development checks of the bounded local source SQL profile, rather
than Engine-admitted verification evidence or full Goal 04 acceptance.
`checks.json` binds commands and results to exact source hashes and environment;
`checks.log` contains their output. Private credentials, connection configuration,
database contents, local objects and raw logs remain outside the repository.

Source tests use actual PostgreSQL transactions and filesystem artifact bytes,
with synthetic check definitions/results. They execute no repository commands
and establish no S3 behavior. Separate real Linux T1 checks exercise the existing
validated local execution profile. Native SQL regressions run on schema v4.

## Retained implementation findings

- Initial SQL publication omitted the separate source counters. Readback equality
  rejected the staged publication; executor, cleanup and storage counters now
  publish atomically with the shared clock/session counter and generation.
- Cold local-store reopening initially retained the previous Engine's reader.
  The fixture now drops its old Engine and reader before opening the exclusive
  filesystem adapter. The adapter's ownership lock remains enforced.
- `development-checks.log` preserves a seven-group source run with six passes
  and a failed final-clock test. Renewal intentionally preserves a longer live
  lease; requesting five seconds after a five-minute lease did not shorten it.
  The test waited past the connection's ten-second idle-transaction deadline and
  received Unavailable. The corrected fixture starts with nine seconds, asserts
  a live lease at the final checkpoint, crosses expiry within the SQL deadline,
  and retains exact Contention and rollback assertions. Its focused pass is in
  `final-clock-correction.log`.
- `isolation.log` preserves an overly generic expected error in the new
  cross-scope executor fixture. The Engine returned its existing typed
  UntrustedExecutor rejection. The fixture now requires that exact variant.
  `recovery-development.log` records the subsequent source suite passing,
  including revocation during artifact reads, reused scoped numeric IDs,
  independent process claim races and process death around SQL commit.
- Exact public-input reviews retain their original default-rejection and
  changed-path/changed-byte assertions. Three reviewed source digests change
  with their reviewed bytes; no credential policy is relaxed. An initial local
  audit rejected raw SHA-256 arguments; corrected reviews use the existing
  domain-separated source-content digest required by that API.

The final source manifest is captured before the final verification sequence.
Only documentation and validation artifacts may be completed afterward; source
manifest differences are reported explicitly. Published migrations 0001–0003
and the complete accepted Goal 04 contract remain unchanged.

Source prepared commit/revalidation, exact identities for all other retriable
mutations, broader generated-history SQL oracle equivalence, production service
identity, failover, host loss, backup/restore, measured load and remaining service
milestones are outside this validation. Hosted CI and the real rust-analyzer
probe are not established by these local checks.

## Final results

The frozen source run passed 452 ordinary workspace tests with 59 opt-in tests
ignored, formatting, strict workspace all-target Clippy, all 10 actual Linux T1
tests and all 45 PostgreSQL parent groups (32 native/foundation and 13 source).
Six native child result summaries are part of their parent groups, rather than
six additional groups. Of the 59 ordinary ignored tests, 55 ran explicitly, three
crash helpers ran through parents, and the real rust-analyzer probe was not rerun.
Example builds/run, workflow checks, protocol integrity/four compatibility mutation
groups and 25 attribution fixtures also passed. Source hashes remained unchanged
throughout the final sequence. Documentation/status completion followed that run.

The PostgreSQL restart check now includes full source receipt metadata, fresh
runtime resolution and an independent read of retained envelope bytes after the
old embedding/reader was dropped. The source process race has one claim winner;
death before publication rolls back, and death after commit recovers the stored
claim and original admission without a second launch. Scope probes reuse token 1
for executors, cleanup and stores within one embedding, and a SQL foreign-key
probe rejects cross-scope object association.

`postgres-profile.json` records observed PostgreSQL 17.9 durability settings,
authentication outcomes and the exact schema-v1–v4 ledger. Full operator output
remains private with its hash retained. `postgres-stop.json` records successful
shutdown with all fixture data preserved. `public-input-review.json` records
unchanged default credential-policy admission with eight exact reviewed inputs,
including separate changed-path/changed-byte rejection and report self-admission.
