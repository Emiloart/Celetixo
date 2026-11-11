# Goal 04 progress and foundation validation

**Status:** active; 04.1a foundation verified; Goal 04 and milestone 04.1 remain incomplete.

The complete [Goal 04 contract](goal-04.md) remains the objective. It was retrieved
from `origin/main` at `117479221eb0830fdcab6918ef76a7ed293cabe7` on 2026-11-11;
its SHA-256 is `339c328bf93bb512b8f90a8006d3dc07b8ed031399a93c3a3c151f45940b8743`.
This record does not narrow that contract or promote schema presence into durable
Engine authority. ADR-0001–0004 and the existing Goal 01–03 gates remain in force.

## Milestones

| Milestone | Current evidence / remaining work |
|---|---|
| 04.1 — PostgreSQL state and recovery | **Partial.** 04.1a below supplies migrations, checked immutable snapshot storage and protocol prerequisites. Routing the complete Engine lifecycle through durable typed state, authorization, reservations/leases/fencing, process recovery and oracle equivalence is pending. |
| 04.2 — Multi-instance atomic commit | Pending: independent Engine processes, authoritative SQL commit/CAS, complete request-bound idempotency, races and crash-ambiguity reconciliation. |
| 04.3 — Transactional events | Schema placeholders exist. No outbox publisher, inbox consumer or real NATS integration yet. |
| 04.4 — Temporal workflows | Pending: Go workflows/activities, replay, timeout reconciliation, cancellation and worker recovery. |
| 04.5 — Immutable object storage | Pending: S3 adapter, verified durable admission, scoped authorization, orphan reconciliation and governance/restore. Existing local Goal 03 storage is unchanged. |
| 04.6 — Derived analytics and shared adapters | Pending: real ClickHouse ingestion/rebuild, scoped shared-service lifecycle and integration with the durable path. Existing bounded LSP/T1 capabilities remain as recorded for Goals 02/03. |
| 04.7 — End-to-end operational proof | Pending: complete real-service path, multi-instance/fault/security/load tests, backup/restore drills and measured operational baselines. |

## 04.1a — Schema, immutable snapshots and typed request prerequisites

This bounded prerequisite is committed separately before changing authoritative
Engine transaction execution. The canonical request schema is included because a
durable operation identity must preserve the complete accepted TaskIntent rather
than projecting it to the smaller reference Engine admission record.

Implemented components:

- `celetixo-postgres`: explicit authenticated local Unix-socket PostgreSQL 17
  configuration, bounded connection/statement/lock deadlines, required primary
  durability settings, and diagnostics restricted to SQLSTATE classifications.
- Transactional, advisory-lock-serialized migration with an exact checksum ledger;
  unsupported migration history/version skew rejects. Interrupted DDL and ledger
  publication roll back together. Commit acknowledgement errors are conservatively
  indeterminate; no in-memory backend fallback is provided.
- [Scoped authority schema](goal-04-postgres-schema.md): composite foreign keys,
  full unsigned revision/fence bounds, immutable rows and forced RLS based on
  independently authenticated login name plus role OID. Mutable authority tables
  are a future integration contract, not an implemented lifecycle service.
- Snapshot catalog: content-addressed candidate storage and checked reopen;
  initial registration cannot advance an existing head. It does not install
  semantic analysis or authorize Engine transactions. The Engine decoder restores
  the existing canonical format through validated constructors with explicit byte,
  field and collection budgets, exact canonical ordering and unchanged identities.
- [Canonical TaskIntent v1](../reference/protocol.md), generated Rust bindings,
  descriptor/golden fixtures, strict bounded intake and full semantic request
  digest. There is no RPC service or authenticated Engine adapter yet.
- Trusted [local PostgreSQL fixture](../../scripts/goal04/README.md) and CI jobs
  for the real database tests and reproducible protocol generation.

### Validation performed

[checks.json](goal-04-foundation/checks.json) and
[checks.log](goal-04-foundation/checks.log) record the actual development run.
[followup.json](goal-04-foundation/followup.json) and
[followup.log](goal-04-foundation/followup.log) cover the final protocol-integrity,
workflow/documentation and attribution checks.
[Source hashes](goal-04-foundation/source-manifest.json) bind the executable source,
fixtures, lockfiles, schema and workflow to these results. The validation record
is not an Engine verification receipt or production evidence admission.

| Check | Observed result |
|---|---|
| Workspace fmt and Clippy across all targets, warnings denied | Passed |
| Locked/offline Rust workspace tests | 353 passed, 18 ignored, zero failed |
| Checked snapshot decoding | Four new groups passed: layered binary/maximum revisions, malformed/identity substitution, resource budgets and constructor/canonical-order rejection |
| Actual PostgreSQL integration | Five groups passed: forged payload/scope/format/manifest rejection; concurrent migrations and ledger skew; child process death before migration commit; actual database restart/reopen; mapped-role isolation, immutability and full unsigned revision |
| Protocol Rust intake | Nine groups passed, including full-field identity mutations and malformed/unknown nested safety semantics |
| Protocol tooling | Integrity/ADR checks, independent compatibility/integrity mutations and exact regeneration with pinned protoc/prost passed; final mutation suite has four groups |
| Workspace/example build and concurrent reference transaction example | Passed; still the in-memory reference Engine |
| LSP tests including actual rust-analyzer | 28 passed |

The PostgreSQL fixture used actual PostgreSQL 17.9 with checksums, fsync,
full-page writes and synchronous commit enabled, no TCP listener or network
connectivity, and private SCRAM-authenticated Unix sockets. Its administrator and
migration owner are trusted; two separate nonowner/non-superuser/non-BYPASSRLS
runtime logins received explicit scope mappings and minimal catalog privileges.
`SELECT ... FOR UPDATE` also requires `UPDATE(state_generation)` on repositories;
no runtime access to the role map or general owner role is granted.

The recorded environment is Linux x86_64 under WSL, Rust 1.94.1, using the existing
shared build target. Tree-sitter C compilation used the documented local compiler
workaround `CRATE_CC_NO_DEFAULTS=1` and
`CFLAGS='-target x86_64-linux-gnu -fPIC'`; no repository C flags were changed.
Build/test elapsed times in the raw record are observations of this development
run, not cold benchmarks, performance targets or capacity measurements.

### Hosted workflow correction

Foundation commit `7eba8338bf5f097e941fb846eab47dd8b80077bf` was pushed with
human author/committer attribution. Its [first hosted run](https://github.com/Emiloart/Celetixo/actions/runs/43105839130)
failed before starting jobs: `runner.temp` was used in a job-level `env` context.
The local YAML parser had not checked GitHub expression semantics. The database
job now selects the fixture configuration from `RUNNER_TEMP` inside its running
step. No Rust, SQL or protocol behavior changed in this correction.

The [workflow checker](../../.github/scripts/check-workflows.py) pins the official
[actionlint 1.7.12 release](https://github.com/rhysd/actionlint/releases/tag/v1.7.12)
by archive SHA-256, parses every workflow without executing jobs, and verifies
rejection of the invalid-context regression. ShellCheck/Pyflakes subprocesses are
disabled; this check covers workflow semantics, not arbitrary shell/Python code.
It runs locally before publication and in CI. The separate
[correction record](goal-04-foundation/ci-correction.json) binds its exact source
and results. The original foundation source manifest and raw test logs remain
unchanged. A configured job is not a passing hosted run.

The [corrected hosted run](https://github.com/Emiloart/Celetixo/actions/runs/43106365291)
started its jobs: protocol generation/intake and attribution passed, while the
database fixture failed to resolve a hardcoded local image ID after successfully
pulling the selected repository digest. The fixture now selects the same immutable
repository digest directly, checks its cached platform metadata and prohibits
automatic pulls. Local daemon image IDs remain provenance/reopen checks. A fresh
fixture creation and the five real database groups were rerun for this change;
[image-selection validation](goal-04-foundation/image-selection.json) records the
source and results. This changes neither the schema nor Engine semantics.

### Public input review in repository smoke checks

The hosted Engine job passed Rust tests, Clippy and the actual rust-analyzer probe,
then correctly rejected credential-like text during capture of Celetixo itself.
The four reviewed public inputs are the PostgreSQL Rust connection implementation,
the trusted fixture script, the foundation follow-up log and image-selection
record. The source contains declarations and generated/private-file references;
the records contain the public `scram-sha-256` setting. They contain no fixture
credential values. Their exact content digests are installed explicitly in the CI
inspection and T0 patch-benchmark commands using the existing credential policy.

`inspect_repository` and `source_patch_bench` accept repeated
`--reviewed-public-input PATH=CONTENT_DIGEST` arguments from their trusted operator.
The digest is the Engine's domain-separated `celetixo/source-content/v1` address,
not raw SHA-256. A shared bounded argument parser validates canonical paths,
lowercase digest encoding, policy version and duplicate reviews before mutating
the policy. It never discovers or approves policy from repository files. Capture
retains these inputs and binds the installed policy into snapshot provenance;
changed bytes or a different path still reject. The default policy is unchanged.

Regression tests cover exact admission, changed bytes/path rejection, invalid
arguments without partial policy mutation, duplicate reviews, multiple explicit
versions and unsupported policies. [Public-review validation](goal-04-foundation/public-review.json)
records the source and checks. This is trusted read-only tool configuration;
transaction execution and evidence admission remain subject to their existing
Engine-owned policy boundaries.

### Reproduce the database tests

Provision the exact cached image explicitly as an operator if it is absent, then
follow the [fixture instructions](../../scripts/goal04/README.md). Docker access
is privileged operator access and must never be passed to workers.

```sh
python3 scripts/goal04/postgres_fixture.py start
python3 scripts/goal04/postgres_fixture.py verify
export CELETIXO_PG_TEST_CONFIG="$HOME/.local/share/celetixo-goal04/service-fixtures/postgres17/connection.json"
cargo test --locked -p celetixo-postgres --test foundation -- --ignored --skip migration_crash_child --test-threads=1
```

Use one test thread because the restart scenario stops the dedicated database.
The child crash helper runs only under its parent; invoking it independently is
an error. Credential files stay private, outside Git, and are never printed.
The fixture has bounded resource settings and a disk-usage guard, not a hard disk
quota. Stop it after use; do not infer available Windows backing space from WSL's
virtual filesystem capacity.

### Unvalidated and unsupported boundaries

No durable Engine transaction, lease operation, fencing enforcement, SQL semantic
commit gate, lost commit-response reconciliation, process-level commit race,
object-store evidence admission, NATS event publication, Temporal workflow or
ClickHouse analytics exists in this foundation. The reference Engine still owns
in-memory transaction state and retains its full Goal 01–03 checks. Captured source
publication remains `SourceMutationUnsupported`.

Database isolation evidence covers the catalog operations and tested roles, not
end-to-end tenant isolation across future services. Schema constraints on future
lifecycle tables are not a proof of Rust transition semantics. Migration skew
checks cover migration/reopen; live upgrade and old-connection fencing remain
pending. A database process restart is not host power loss, failover, backup/restore
or a zero-loss guarantee. A runtime RLS boundary does not constrain a trusted
PostgreSQL administrator, host root or the operator's Docker privileges.

The ordinary workspace suite deliberately does not run the opt-in OCI/service
checks. The actual PostgreSQL tests and rust-analyzer probe were run separately.
The unchanged Goal 03 OCI profile was not rerun for this foundation; its earlier
source-bound evidence remains linked from Goal 03 acceptance. No production
readiness, whole-goal completion or service-level performance claim is made.

## Next implementation boundary

Separate serializable, checked engineering state from transient reader/capability
objects. Route the existing Rust transition logic through an explicit database
transaction with normalized mutable authority, shared repository lock/version
rules and a database clock read after lock acquisition. Preserve mutations that
intentionally persist a rejection/revalidation outcome, keep artifact reads outside
long transactions, and require confirmed SQL commit before acknowledgement.

The subsequent gate is equivalence with the independently structured reference
oracle plus process restart, grant/policy revocation, lease expiry/fencing and
complete source-attempt recovery tests. Only then can 04.1 be marked complete.
