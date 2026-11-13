# Durable original mutation outcomes

**Status:** validated original transaction, repository and administrative mutation
recovery in the bounded native and captured-source PostgreSQL profiles. This extends the [source SQL lifecycle
authority](goal-04-source-sql-authority.md). The full [Goal 04](goal-04.md),
including complete 04.1/04.2 acceptance, remains in progress.

## Request identity and atomic recovery

`Engine::mutate` accepts typed transaction and repository requests under a fresh
opaque session. `Administrator::mutate` accepts typed administrative requests
through the independently authenticated embedding boundary. Both use one scoped
journal and the same atomic evaluation/publication path. Its key includes scope,
the derived actor identity and grant where applicable, and the idempotency key.
Constructing actor metadata does not authenticate a caller.

The original full request, typed outcome and decision time are immutable. An exact
retry returns that original outcome, including an intentional rejection; a
different body under the same key returns `IdempotencyConflict`. Current
authentication precedes lookup. Storage/publication failure rolls back both the
effect and its journal entry. A lost response after SQL commit is recovered by
the original key without producing another effect. Callbacks are not retried
automatically and there is no memory fallback.

Supported journal requests are:

| Caller | Request families | Original result |
|---|---|---|
| Session | Propose, Analyze, Reserve, Renew, Execute, Abort | Transaction state/attempt, effective accesses, or original lease expiry/fence/acquisition attempt |
| Session | SubmitPatch, RecordEvidence, Revalidate | Exact native patch/candidate/analysis bindings, evidence identity or typed rejection |
| Session | Register, UpdateModel, RegisterRepository, InstallAnalysis, InstallPolicy | Original snapshot/model or policy/model-generation identity |
| Embedding administrator | GrantSession, RevokeSession, GrantExecutor, RevokeExecutor, GrantCleanup, RevokeCleanup, RegisterArtifactStore, RevokeArtifactStore | Historical resource kind and allocation/revocation ID |

Repository and administrative requests have no synthetic transaction. Rejected
raw selectors remain distinct from independently resolved original targets. In
particular, a revoke rejected before grant 1 exists cannot later revoke grant 1
when the original request is retried. Initial native patch submission requires
`Executing`; revalidation creates a fresh attempt and retains the old bindings.
Native-only patch/evidence/revalidation requests remain unsupported for source
transactions. Source admission and evidence retain their existing dedicated
original-outcome contracts.

## Historical results and current authority

A replay does not renew a lease, reacquire a reservation, select old evidence,
reinstall an old model/policy, allocate another grant or reattach an artifact
reader. Historical allocation IDs remain available after revocation or expiry;
fresh independent session/executor/cleanup authentication and configured-reader
resolution are still mandatory. Current `validate_source_attempt` and
`source_execution_time` observations remain current observations.

Ordinary and journaled entry points share the existing pure transition helpers.
Final SQL gates examine actual changes, so ordinary entry points also recheck
new native candidate effects, lease authority, policy installation and expiring
executor/cleanup allocations. Historical no-op replies create no renewed
authority. Reader detachment follows confirmed storage-registration revocation
and occurs outside SQL callbacks.

## Normalized persistence and security

Migration `0005_mutation_recovery.sql` adds immutable, forced-RLS mutation rows,
original request/outcome payloads, optional original context and scoped foreign
keys for resolved attempts and produced effects. Published migrations 0001–0004
are unchanged. The administrator actor uses tagged nullable fields and a
canonical actor digest, without a fabricated principal or grant zero.

Database row constraints reject partial/null resource results and unbound grant,
store, reservation, snapshot, policy and evidence associations. An authenticated
runtime login cannot insert an administrator journal record: its RLS check
requires the actual corresponding database allocation or revocation privilege.
Runtime operations also reject newly acquired administrative write privileges,
including changes made after backend/session creation. These checks supplement
Engine authorization; SQL row shape never decides semantic admissibility.

Recovery validates complete typed original inputs, including full policy/check
dependencies, original access provenance and immutable historical effects.
Nonallocating size preflights and aggregate decode/recovery limits precede
materialization. Existing published canonical formats and the default credential
policy remain unchanged. Source journal publication follows source catalog and
execution publication within the same SQL transaction so foreign keys bind
already staged facts.

## Validation and remaining work

The accompanying [validation record](goal-04-mutation-validation/README.md)
distinguishes final checks from development evidence. PostgreSQL tests use the
reviewed local PostgreSQL 17.9 fixture with SCRAM Unix-socket authentication,
fsync/full-page writes/synchronous commit/checksums and restricted scoped roles.
Source check results are synthetic; separate Linux T1 checks exercise the
existing isolated runtime. Neither establishes S3 durability or production
service identity.

At this original milestone, keyed executor transitions remained pending. The
subsequent [executor extension](goal-04-executor-recovery.md) adds their complete
original replies while preserving current admission and launch/workspace controls.
The original validation above retains its stated scope. Source prepared
commit/revalidation, the broader independent generated-history SQL oracle,
real event/workflow/object/analytics services,
service transport, failover, backup/restore, host loss and measured load remain
required by the unchanged Goal 04 contract.
