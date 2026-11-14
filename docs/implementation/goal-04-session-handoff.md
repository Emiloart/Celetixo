# Goal 04 fresh-session handoff

**State:** resumed at the user's request on 2026-11-12 from published 04.1f
(`f1f2b83a4193f483c79216ed6e8140a75d9ecea6`). The following preserves the original
pause context; consult current [progress](goal-04-progress.md) and the
[04.1g PostgreSQL registration adapter](goal-04-registration-authority.md) for
subsequent work, followed by the [04.1h native SQL profile](goal-04-native-sql-authority.md)
and [04.1i checked source codecs](goal-04-source-codecs.md), followed by
[complete source metadata/fresh runtime recovery](goal-04-source-recovery.md) and
[source SQL lifecycle through receipts](goal-04-source-sql-authority.md) and
[original transaction/repository/admin outcomes](goal-04-mutation-recovery.md) and
[original executor transitions](goal-04-executor-recovery.md),
[source publication](goal-04-source-publication.md) and
[fresh source revalidation](goal-04-source-revalidation.md) and
[independent source commit process proof](goal-04-source-processes.md).
[Milestone 04.1 is accepted](goal-04-authority-acceptance.md) for the reviewed
PostgreSQL adapter. Goal 04 is not complete; preserve the entire accepted
[seven-milestone objective](goal-04.md). Its contract SHA-256 remains
`339c328bf93bb512b8f90a8006d3dc07b8ed031399a93c3a3c151f45940b8743`.

Read `AGENTS.md`, `docs/README.md`, `Dev.md`, accepted ADRs/specs and the
[progress record](goal-04-progress.md), then the
[04.1f boundary](goal-04-authority-boundary.md) and its validation. Inspect Git and
remote contributions before editing. The 04.1f commit is the history entry adding
this handoff; obtain its identity with Git. The previous published milestone was
`cbe0318b7248de49152877f307efaeced1678182` (04.1e).

## Workspace and publication rules

- Implementation branch: `goal04/durable-platform`, worktree
  `/home/emiloart/.local/share/celetixo-goal04/worktree`.
- Original checkout: `/mnt/c/dev/celetixo`, branch `main`. Preserve the user's
  modified `README.md` and untracked `.env`. Never read or publish the latter.
- Human author and committer: `Emiloart.` / `ilodubahemeka@gmail.com`.
  No AI author/co-author. Follow `CONTRIBUTING.md` and check raw metadata before
  each push. Commit/push each verified milestone and announce its identity.
- Remote transport previously used the installed Windows Git executable at
  `/mnt/c/Program Files/Git/cmd/git.exe`, with `-C C:/dev/celetixo`.
  Fetch and preserve remote work; never force-push over contributions.
- Monitor actual Windows backing space and retain private fixture data. Use one
  Cargo build at a time and the persistent compact target described below; avoid
  duplicate build trees/images. Stop dedicated fixtures after verification.

## What exists and what does not

04.1a–04.1f establish schema/catalogs, runtime separation, checked immutable/native
lifecycle records, explicit evaluation/publication, exact requests/fences, fresh
session resolution, scoped backend routing and supported schema upgrades through
v3. At the pause, the adapter test double was memory-only and there was no
PostgreSQL `AuthorityBackend` or durable lease/commit backend. The native SQL
library added later does not establish a production service deployment.
04.1g now supplies a bounded PostgreSQL `AuthorityBackend` for scoped grants and
native repository/model registration. It explicitly rejects all lifecycle rows;
that restriction describes the historical 04.1g profile. 04.1h adds normalized
native lifecycle/commit recovery. The source SQL profile now implements captured
source lifecycle through evidence receipts; the [mutation journal](goal-04-mutation-recovery.md)
adds original transaction/repository/admin outcomes with schema v5. The
[executor extension](goal-04-executor-recovery.md) adds all four keyed executor
transitions with schema v6. Source revision commit and fresh source revalidation now have bounded SQL
validation with schemas v7/v8. Full Goal 04 acceptance remains pending. Its shared schema and
repository locking, independent restricted runtime/admin logins and checked
publication protocol are the starting point for the remaining SQL integration.

The bounded native reconstruction rejects captured source state and legacy
unbound commit records. Existing Goal 03 memory/T1 behavior remains separately
validated. Do not silently discard unsupported rows or treat partial reconstruction
as complete source recovery. 04.1 is now accepted for the reviewed adapter;
the remaining 04.2–04.7
requirements still apply.

## Next implementation work

The original 04.1f-to-native integration tasks are now covered by 04.1g/04.1h.
Continue the entire accepted goal from these remaining gates:

1. Continue from verified [source publication](goal-04-source-publication.md) and
   [fresh source revalidation](goal-04-source-revalidation.md). Preserve every
   original request/result, historical definition, source attempt/event, receipt,
   grant and artifact association. The source integration checks, rather than
   native-only regressions or memory reconstruction, establish its bounded SQL
   behavior. Neither a source receipt nor private candidate advances revision.
2. Preserve the [original mutation journal](goal-04-mutation-recovery.md) and its
   [executor extension](goal-04-executor-recovery.md). Transaction, repository,
   administrative and all four executor transitions now have typed original-outcome
   paths. Preserve fresh executor validation and independent launch/workspace
   recovery; an old claim result cannot authorize another launch. Use a new
   migration after published schema v8; never edit an applied or published checksum.
3. Extend the verified same-transaction source commit process races/crashes to
   distinct-source-transaction histories; retain current authorization, semantic,
   evidence/fence checks and atomic history/head/outcome/outbox with exact original
   requests. Expand independent generated histories and shrinking against
   SQL and complete source-path proof before claiming 04.2 accepted.
4. Add authenticated versioned service boundaries from canonical Protobuf contracts,
   preserving complete TaskIntent and Rust Engine authority. The reviewed Unix
   embedding profile does not establish production network identity/transport.
5. Implement 04.3–04.6: canonical transactional events/JetStream, Temporal workflows,
   verified immutable S3 storage/admission/governance, ClickHouse replay and scoped
   shared language/execution services. None becomes transaction truth.
6. Execute 04.7: the real end-to-end service path, partitions/failover/crashes,
   security/isolation, backpressure/cancellation, backup/restore and measured load.
   Preserve exact evidence and distinguish target, implementation and validation.

Published migrations must not be edited. Future changes require a new
migration. Do not rewrite applied checksums or promote SQL row shape into semantic
admissibility. Live schema fencing is implemented for the SQL adapter; backup/restore and
production identity services remain pending and need their own implementation/evidence.

## Local verification resources

Use one Cargo invocation at a time with the validated compact test profile:

```sh
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
CARGO_TARGET_DIR="$HOME/.local/share/celetixo-goal04/cargo-fast-target" \
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC' \
cargo test --workspace --all-targets --locked --offline
```

This disables debug symbols, retaining debug assertions and overflow checks.
Strict Clippy was also run with the default symbol profile. The
[fresh revalidation validation](goal-04-source-revalidation-validation/README.md)
binds the exact final sources, commands and selected PostgreSQL parents.

Read the [Linux T1 runbook](../operations/local-linux-t1-runbook.md) before operating
its execution profile. Private OCI tools are under
`/home/emiloart/.local/share/celetixo-goal03`. PostgreSQL validation uses the retained
`pg17-mutation-next` fixture under the private Goal 04 root and the database
`celetixo_revalidation_0fbc6ed697bb`, selected by its private connection file. Earlier
source-publication and draft databases/checksum ledgers remain retained. Never
rewrite those ledgers or apply changed migration bytes to an existing database.

Configuration and credential files are private: never print their contents or
place values in arguments, logs, environment or Git. Fixture start/verify/stop is
through `scripts/goal04/postgres_fixture.py --help`; no automatic image pulls.
The unrelated host PostgreSQL instance must be left alone.

The last complete published parent CI run (04.1e) was
[43149440742](https://github.com/Emiloart/Celetixo/actions/runs/43149440742).
It does not establish CI success for a later commit. Consult GitHub for the resumed
revision and retain the distinction between local validation and hosted results.
