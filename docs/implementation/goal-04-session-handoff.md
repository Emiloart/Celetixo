# Goal 04 fresh-session handoff

**State:** resumed at the user's request on 2026-11-12 from published 04.1f
(`f1f2b83a4193f483c79216ed6e8140a75d9ecea6`). The following preserves the original
pause context; consult current [progress](goal-04-progress.md) and the
[04.1g PostgreSQL registration adapter](goal-04-registration-authority.md) for
subsequent work. Goal 04 is not complete; preserve the entire accepted
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
- Windows backing storage is nearly full. Reuse the shared target; avoid duplicate
  build trees/images. No project/programming data deletion is authorized by this
  implementation task. Stop dedicated fixtures after verification, retaining data.

## What exists and what does not

04.1a–04.1f establish schema/catalogs, runtime separation, checked immutable/native
lifecycle records, explicit evaluation/publication, exact requests/fences, fresh
session resolution, scoped backend routing and supported schema upgrades through
v3. At the pause, the adapter test double was memory-only and there was no
PostgreSQL `AuthorityBackend`. There is still no durable Engine lease/commit
service or distributed commit.
04.1g now supplies a bounded PostgreSQL `AuthorityBackend` for scoped grants and
native repository/model registration. It explicitly rejects all lifecycle rows;
the complete durable lease/commit backend remains pending. Its shared schema and
repository locking, independent restricted runtime/admin logins and checked
publication protocol are the starting point for the remaining SQL integration.

The bounded native reconstruction rejects captured source state and legacy
unbound commit records. Existing Goal 03 memory/T1 behavior remains separately
validated. Do not silently discard unsupported rows or treat partial reconstruction
as complete source recovery. All remaining 04.1–04.7 requirements still apply.

## Next implementation work

1. Implement a normalized, bounded PostgreSQL `AuthorityBackend` using the existing
   Rust transitions and checked facts. Do not serialize the private Database blob
   or introduce a separate transaction decision service.
2. Independently authenticate nonowner/non-superuser/non-BYPASSRLS runtime roles;
   verify both role name and OID against scope mapping. Administrative writes use
   a separate trusted role. Requested actor fields are attribution only.
3. Coordinate schema validation with migration locking, then use the common scoped
   repository lock/generation protocol for every relevant mutation. Reads need a
   coherent snapshot. Sample the documented database clock after locking; reject
   backward clocks. Reads must not manufacture durable lease events.
4. Retain intentional domain-error mutations, all exact historical request bodies,
   evidence admissions/selection and lease acquisition attribution. Preserve legal
   NUL reasons through binary records. Keep private candidates out of registered
   proposal bases. Require prepared commit requests in the backend profile.
5. Publish current authority, committed history, receipt, exact idempotent result
   and required outbox fact atomically. Confirm SQL commit before acknowledging.
   Reconcile ambiguous responses through durable authenticated lookup.
6. Add real independent-process races/restart/fault tests and reference-oracle
   equivalence. Integrate complete captured-source/model/Goal 03 execution and
   evidence recovery before claiming 04.1 complete. Continue 04.2–04.7 afterward.

Published migrations 0001/0002/0003 must not be edited. Future changes require a new
migration. Do not rewrite applied checksums or promote SQL row shape into semantic
admissibility. Live schema fencing, backup/restore and production identity services
remain pending and need their own implementation/evidence.

## Local verification resources

Use one Cargo invocation at a time with the previously validated local environment:

```sh
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/mnt/c/dev/celetixo/target \
CRATE_CC_NO_DEFAULTS=1 CFLAGS='-target x86_64-linux-gnu -fPIC' \
cargo test --workspace --locked --offline
```

Read the [Linux T1 runbook](../operations/local-linux-t1-runbook.md) before operating
its execution profile. Private tools are under
`/home/emiloart/.local/share/celetixo-goal03`; the source-bound validation records
name their exact hashes. The private Goal 04 validation directory retains command
runners, source manifests and preservation metadata outside the public repository.

The dedicated PostgreSQL fixture is under
`/home/emiloart/.local/share/celetixo-goal04/pg17-ci`.
`connection-lifecycle.json` selects the supported published-schema database;
`connection.json` selects an older unpublished-v2 development database whose
checksum mismatch is intentional and preserved. Never rewrite that ledger.
Configuration/credential files are private: do not print their contents or place
credential values in arguments, logs, environment or Git. Fixture start/verify/stop
is through `scripts/goal04/postgres_fixture.py --help`; no automatic image pulls.
The unrelated host PostgreSQL instance must be left alone.

The last complete published parent CI run (04.1e) was
[43149440742](https://github.com/Emiloart/Celetixo/actions/runs/43149440742).
It does not establish CI success for a later commit. Consult GitHub for the resumed
revision and retain the distinction between local validation and hosted results.
