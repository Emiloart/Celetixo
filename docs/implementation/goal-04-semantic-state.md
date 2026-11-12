# Goal 04 immutable semantic-state persistence

**Status:** 04.1c verified immutable-record milestone; the authoritative transaction lifecycle remains in-memory.

**Authority:** [Goal 04](goal-04.md), ADR-0002/0003, the
[engineering identity](../spec/engineering-identity.md) and
[PostgreSQL schema](goal-04-postgres-schema.md) contracts.

## Implemented records

`EngineeringModelRecord` freezes a validated finite engineering model against an
exact scope and snapshot. Its content address uses
`celetixo/engineering-model-record/v1`. Canonical fields are organization,
workspace, repository, snapshot digest, unsigned model generation, analyzer,
version, configuration digest, consumer/dependency map and completeness. Maps and
sets use their existing sorted order. Incomplete reasons retain their ordered
list, including an empty list; incomplete never becomes complete by decoding.
This retains the reference model; it does not discover source dependencies or
assert complete language semantics.

`RepositoryAnalysis::canonical_bytes()` exposes the existing
`celetixo/repository-analysis/v1` digest preimage. It retains scope, snapshot,
index generation, analyzer version, configuration, elements, dependencies,
justified checks and analysis gaps. Its field order and identities are unchanged.
The [frozen fixtures](../../crates/engine/tests/fixtures/semantic-codecs.json)
were independently encoded from the published v1 field layout, including both
forms of optional gap paths. They are not regenerated from the Rust codec under
test. The new model format has its own frozen fixture.

## Checked reconstruction

Both decoders require an external expected digest, exact snapshot and expected
model/index generation. They check domain-separated identity, bounded fields,
UTF-8 and typed IDs, supported tags, aggregate nested collection budgets,
uniqueness, constructor invariants, end of input and exact canonical re-encoding.
Duplicate or reordered sets/maps reject rather than being normalized. Repository
analysis also rejects dependency/check relationships with unknown elements. The
Engine and model-record constructor share positive-generation and nonempty
analyzer/version validation.

`DomainDecodeLimits` aliases the existing snapshot budgets, preserving that API.
Defaults remain 64 MiB encoded bytes, 100,000 aggregate collection entries and
64 MiB per field. Callers may supply smaller limits to pure decoders. The catalog
uses these defaults and the schema's existing 64 MiB payload constraint. This
does not establish large-index capacity or a performance target.

## PostgreSQL catalog boundary

`SemanticCatalog` uses the existing authenticated Unix-socket configuration,
schema validation and mapped-role RLS. It retains candidate model/analysis records
in immutable, scoped `domain_payloads`; there is no new migration or mutable
model-head table. The referenced snapshot must already exist and pass the checked
snapshot loader. Puts validate before insertion, use immutable insert/readback,
and acknowledge only after confirmed SQL commit. Conflicting existing format,
version or bytes reject. Ambiguous commit errors remain `CommitIndeterminate`;
retrying an exact immutable candidate does not advance a repository revision.

Reads load snapshot and semantic bytes in one read-only repeatable-read SQL
transaction, reject unsupported format/version, and reconstruct through checked
Rust decoders. Fresh connections and actual database restart can reopen records.
Incorrect scope is absent under the independent login's RLS policy.

An expected generation compares a record to a requested reference; it does not
establish which generation is currently installed. Historical candidate reads are
allowed. A durable Engine operation must obtain expected current model/index,
policy, authorization and attempt references from authoritative state under its
atomic validation protocol. Content addressing proves binding, not analyzer trust,
semantic completeness, caller authorization or freshness relative to a head.

Catalog writes cannot install model heads, create transactions, grant capabilities,
reserve/renew leases, admit verification or commit source. They do not serialize a
Database or process-local resources. Full normalized lifecycle reconstruction,
authenticated request/grant resolution, fencing and multi-process atomic commit
remain required for 04.1–04.2. Full structural index storage and later platform
services remain pending.

## Reproduction

```sh
cargo test --locked -p celetixo-engine --test persistence --test semantic_persistence
CELETIXO_PG_TEST_CONFIG=/private/reviewed/connection.json \
  cargo test --locked -p celetixo-postgres --test foundation -- \
  --ignored --skip migration_crash_child --test-threads=1
```

Use the [trusted fixture runbook](../../scripts/goal04/README.md), never repository
credentials or an implicit network database. The database suite restarts its
dedicated fixture and runs serially. The [progress record](goal-04-progress.md)
links source-bound results and unrun checks.

The actual-repository group creates a trusted, deterministic Git fixture, captures
and indexes five source files across the initial language scope, persists the
resulting analysis and reopens it through a fresh connection. It then installs the
restored facts through the existing trusted reference Engine administration path
and verifies that incomplete impact still rejects. This is evidence about
analysis persistence and the admission boundary, not a new language-support claim
or a database-backed transaction service.
