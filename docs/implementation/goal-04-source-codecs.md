# Goal 04 checked source metadata codecs

**Status:** 04.1i verified source-recovery prerequisite. Complete source authority reconstruction remains pending.

**Authority:** [Goal 04](goal-04.md), ADR-0001/0003, the
[engineering identity](../spec/engineering-identity.md) and
[Goal 03 execution](goal-03-execution.md) contracts.

## Preserved formats and dependencies

The new persistence modules reconstruct existing immutable source values. They
do not change their canonical formats, domain-separated identities, Protobuf
contracts or published SQL migrations.

| Value | Existing digest domain / checked dependencies |
|---|---|
| Check specification | `celetixo/check-spec/v1`; full ID, argument vector, toolchain, environment and configuration |
| Patch policy | `celetixo/source-patch-policy/v1`; all ten existing version/profile/budget fields, validated through the existing path-policy validator |
| Source check definition | `celetixo/source-check/v1`; exact checked specification plus every execution, input/output, toolchain, resource, network, credential-reference, artifact and dependency field |
| Source execution policy | `celetixo/source-execution-policy/v1`; exact snapshot, repository analysis, generation, patch policy and checked definition registry; full capture, credential-governance, paths, invariants and execution policy |
| Verification plan | `celetixo/source-verification-plan/v1`; exact policy/binding and independently supplied Engine-derived requirements |
| Source admission | `celetixo/source-admission/v1`; original patch/base, historical policy/plan, semantic writes and original owner-grant provenance |
| Execution record | `celetixo/source-check-record/v1` bytes at their existing `celetixo/source-artifact/v1` address; expected admission/plan/candidate/check, scope and retention |
| Source evidence | Existing v2 envelope and identity; independently expected admission/executor/attestation/scope/retention binding |

The policy and plan formats refer to definitions by digest. Recovery therefore
requires complete checked dependencies; a digest is not a replacement definition.
Standalone CheckSpec and PatchPolicy records permit policies to recover before
the first patch exists. A verification plan is re-resolved from the caller's
Engine-derived requirements, then compared byte for byte. Stored required flags
or rationales cannot select a weaker plan.

Admission restoration independently applies the checked patch to its original
base, checks policy paths/preimages and semantic writes, applies the existing
credential policy to resulting inputs, and checks the complete plan binding and
candidate inputs. Candidate, manifest and application-report digests must match
the stored admission. Original principal, capability-reference provenance,
policy generation, retention and time ordering are retained. Historical expiry
is not compared with today's clock. This allows valid expired history to recover;
current authorization, leases and policy heads still require fresh checks.

## Parsing and resource boundaries

Every decoder requires an external expected digest and uses checked typed
constructors, strict tags/integers/UTF-8, explicit end of input and exact canonical
re-encoding. Duplicate or reordered sets/maps reject. Unsupported or incomplete
capture does not become complete through decoding. Credential reviews retain
their exact paths and content identities.

`DomainDecodeLimits` retains the existing defaults: 64 MiB encoded data, 100,000
aggregate collection entries and 64 MiB per field. Nested collections share the
budget. Referenced definitions are charged before hashing/cloning; plan recovery
conservatively preflights the complete registry and possible dependency/invariant
rationale fanout before resolution, including branches omitted by malformed
bytes. Admission recovery charges its plan and a conservative base/patch/candidate
data bound before cloning or patch replay, including generated provenance and
environment-key growth. These are canonical-data work budgets; measuring already
materialized dependencies remains linear in their size. They are not a process
RSS or CPU quota. Callers may choose smaller limits, and conservative accounting
may reject a value whose compact top-level payload alone fits.

The check and evidence decoders also preserve the existing 4 MiB execution-record
and 16 MiB envelope limits. The standalone execution object intentionally omits
its own `execution-record` reference to avoid recursive content addressing.
Restoration does not synthesize the omitted reference. Full envelope records
retain their original references and provenance.

## Authority and persistence limits

These are pure metadata decoders. They restore no session, executor, cleanup
capability, artifact reader, credential or store registration. An explicitly
expected unattested envelope remains unattested; the existing Engine gate still
rejects it. Evidence plan/check applicability, current executor authorization,
object retrieval, digest/length verification and durable admission remain the
responsibility of the existing evidence gate and `validate_persisted`.

The PostgreSQL adapter still rejects source state. Complete source AuthorityState
records, historical definitions/attempts/events/requests/outcomes, scoped runtime
resolution and SQL reconstruction/publication remain required. Exact mutation
reconciliation, source revalidation and atomic source commit remain pending;
`SourceMutationUnsupported` is preserved. This milestone does not close 04.1,
04.2 or any later Goal 04 service/recovery gate.

## Validation

[Source-bound checks](goal-04-source-codecs-validation/checks.json) and the
[log](goal-04-source-codecs-validation/checks.log) record 427 passing workspace
tests (45 opt-in tests ignored), strict all-target Clippy, formatting, example
build/run, workflow semantics, protocol integrity/four mutation groups and 25
attribution fixtures. The 18 added tests use independent layout encoders and frozen check,
patch-policy and execution-object addresses. They exercise exact fields and
bindings, rehashed malformed/truncated inputs, duplicate/order/tag attacks,
capture/credential and mandatory-plan substitution, aggregate dependency budgets,
expired/revoked historical admission, and missing-object rejection after metadata
restoration. Independent review caught and corrected omitted-branch/invariant
budget amplification and ineffective truncation fixtures before final validation.

```sh
cargo test --locked --offline -p celetixo-engine \
  --test source_execution_persistence --test source_evidence_persistence \
  --test source_attempts
```

Real PostgreSQL, live T1 and real rust-analyzer probes are not rerun for these pure
codecs; their earlier source-bound evidence remains separately scoped. Hosted CI
is unconfirmed. No distributed durability, object availability or new language/
execution support follows from these tests.
