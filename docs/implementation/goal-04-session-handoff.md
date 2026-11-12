# Goal 04 fresh-session handoff

**State:** resumed at the user's request on 2026-11-12 from published 04.1f
(`f1f2b83a4193f483c79216ed6e8140a75d9ecea6`). The following preserves the original
pause context; consult current [progress](goal-04-progress.md) and the
[04.1g PostgreSQL registration adapter](goal-04-registration-authority.md) for
subsequent work, followed by the [04.1h native SQL profile](goal-04-native-sql-authority.md)
and [04.1i checked source codecs](goal-04-source-codecs.md), followed by
[complete source metadata/fresh runtime recovery](goal-04-source-recovery.md) and
[source SQL lifecycle through receipts](goal-04-source-sql-authority.md). Goal 04 is not complete; preserve the entire accepted
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
PostgreSQL `AuthorityBackend` or durable lease/commit backend. The native SQL
library added later does not establish a production service deployment.
04.1g now supplies a bounded PostgreSQL `AuthorityBackend` for scoped grants and
native repository/model registration. It explicitly rejects all lifecycle rows;
that restriction describes the historical 04.1g profile. 04.1h adds normalized
native lifecycle/commit recovery. The source SQL profile now implements captured
source lifecycle through evidence receipts with schema v4; source revision commit
and complete mutation reconciliation remain pending. Its shared schema and
repository locking, independent restricted runtime/admin logins and checked
publication protocol are the starting point for the remaining SQL integration.

The bounded native reconstruction rejects captured source state and legacy
unbound commit records. Existing Goal 03 memory/T1 behavior remains separately
validated. Do not silently discard unsupported rows or treat partial reconstruction
as complete source recovery. All remaining 04.1–04.7 requirements still apply.

## Next implementation work

The original 04.1f-to-native integration tasks are now covered by 04.1g/04.1h.
Continue the entire accepted goal from these remaining gates:

1. Continue from the verified [source SQL lifecycle](goal-04-source-sql-authority.md)
   through source prepared commit/revalidation. Preserve every
   original request/result, historical definition, source attempt/event, receipt,
   grant and artifact association. The source integration checks, rather than
   native-only regressions or memory reconstruction, establish its bounded SQL
   behavior. Neither a source receipt nor private candidate advances revision.
2. Add exact durable administrative request identities/digests/results for grants,
   revocation, registration and model/policy mutation, plus every externally
   retriable lifecycle mutation (including reservation, renewal and cancellation).
   Diagnostic state/generation does not resolve an uncertain request automatically.
3. Extend multi-instance source commit, authorization/index/policy/evidence/fence
   rechecks and fault recovery; retain atomic history/head/outcome/outbox and exact
   original requests. Expand independent generated histories and shrinking against
   SQL and complete source-path proof before claiming 04.1/04.2 accepted.
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
`connection-source-sql.json` selects the new source/native schema-v4 validation
database. Earlier draft-v4 databases and their checksum ledgers remain retained;
do not reuse them with different migration bytes. `connection-lifecycle.json`
selects the previous schema-v3 native validation database;
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
