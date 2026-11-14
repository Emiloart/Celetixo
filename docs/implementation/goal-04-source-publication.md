# Source publication and prepared request bindings

**Status:** validated bounded Engine/PostgreSQL source publication and original
outcome recovery. [Checks](goal-04-source-publication-validation/checks.json) record
539 workspace tests, 10 actual Linux T1 tests and 15 selected PostgreSQL parent
cases. Fresh source revalidation and complete milestones 04.1/04.2 remain pending.
Published source application, admission, evidence and native request formats are
unchanged.

## Verification and publication identities

An existing source patch application retains the base revision. Its immutable
snapshot, manifest and application report are already bound by source admission
and verification. Advancing the repository revision changes snapshot identity,
even when all source bytes and modes stay the same.

`SourcePublicationCandidate` independently replays the full patch against the
immutable base and derives exactly one revision successor. It preserves the
verified manifest, content, modes and state. It retains both snapshots and the
original application report. Validation recomputes the entire relationship;
matching caller-supplied digests alone is insufficient. Revision overflow fails
explicitly. Constructing this value does not admit a commit.

`SourcePublicationAnalysis` binds complete analyzer output to that exact
successor. Analysis bound to the original base or verification snapshot fails,
as does analysis with unresolved gaps. Its checked decoder replays the original
base/patch dependencies, validates every publication selector and uses bounded
aggregate accounting before replay. Analyzer facts and storage reconstruction
remain data, not authenticated model installation or approval.

## Read and preparation boundaries

`Engine::source_publication_candidate` permits a current scoped reader to inspect
the proposed successor after Passed evidence has sealed the source attempt. It
checks the exact current base, transaction state, original owner grant, lease
and fence, model, complete impact, policy and original admission/application.
The existing execution context remains restricted to `Executing`; publication
preview uses a separate wrapper restricted to `Validating`.

`Engine::prepare_source_commit_request` independently checks two session roles:
the original lease owner needs `Commit`, and an analysis approver needs
`ManageModel`. Distinct principals can hold those permissions without receiving
the other role's permissions. Preparation bounds caller-owned analyzer facts
before entering a database callback or replaying source. It returns exact
request metadata, not an approval capability or a successful commit decision.

The request binds the complete original patch binding, owner grant and fence,
analysis approver, admission, source patch and operations, policy/generation,
verification plan, original application snapshot/manifest/report, proposed
publication snapshot/revision/proof, source evidence and full receipt identity,
store registration/provenance and complete prospective analysis. Source IDs
are not converted to native integer-patch or evidence IDs.

The canonical preimage separately prefixes the independently authenticated
scope/principal before the raw submitted binding. Structural recovery preserves
invalid raw selectors, including wrong scope/principal, attempt, zero grants or
fence and invalid times, so an eventual original rejection can be reconstructed
without inventing an authoritative target. The idempotency key is a separate
outcome selector and is excluded from the semantic request digest. These Rust
storage values do not replace the canonical service Protobuf contract.

Neither preview nor request preparation rereads external artifacts. They can
return metadata after a storage outage or revocation, and cannot establish
current artifact availability or admissibility for publication. The
commit gate independently resolves trusted storage, reads every required
object outside the authority callback and freshly rechecks all mutable
preconditions at atomic publication.

## Source publication and original outcomes

`Engine::commit_source_prepared` requires current opaque owner and analysis
approver sessions for a new decision. It independently reads the retained
envelope and all required objects through the original trusted registration,
outside authority locks. It then rechecks both roles, complete source bindings,
current head/model/policy/impact, lease/fence, original executor, storage and
evidence at the atomic boundary. Unavailable or corrupt objects create no
publication or memoized commit decision.

The same original mutation journal retains typed source success and domain
rejection outcomes. Exact recovery requires the original owner's current scoped
Commit authority and returns the historical result before object I/O or fresh
analysis approval. It cannot renew a lease or launch execution. Changed request
bodies conflict; raw rejected selectors remain independently reconstructible.
The one-session generic mutation interface cannot invoke source publication.

Publication advances exactly one snapshot revision, installs the independently
approved successor analysis/model, appends typed source history, commits the
transaction, releases its reservation and retains its original outcome together.
The sealed execution record and original verification snapshot remain immutable.
The previous policy stays explicitly bound to its original snapshot; subsequent
admissions require a fresh authorized policy. Checked recovery requires all head,
history, analysis, terminal, lease and original-outcome effects to agree. Source
patches and evidence never acquire fabricated native IDs.

Migration `0007_source_publication.sql` adds immutable scoped source journal
projections and committed history with forced RLS, checked nullable shapes and
deferred associations requiring both outcome context and publication effects.
The SQL adapter stages head/model CAS, history, terminal outcome, original result
and outbox together, checks readback and repeats Engine metadata validation at
the final database-time gate. The named local SQL checks cover publication, cold Engine reopen, rollback,
lost responses, independent Engine instances, final-time expiry and additive
upgrades. Migrations 0001–0006 retain their published bytes.

## Validation scope

The [validation record](goal-04-source-publication-validation/README.md) separates
15 selected SQL parents from the unrun full PostgreSQL suite. Actual PostgreSQL
process restart preserved all 193 logical rows across 57 scoped tables for a
committed source revision. Cold Engine reopening and original recovery are
separately tested. These checks do not establish host-loss recovery or production
readiness. Synthetic source checks establish metadata semantics; the 10 actual
Linux T1 cases exercise the existing isolated execution profile.

The SQL cases used a frozen source manifest. A later workspace run exposed stale
exact-review fixture digests for two changed public Rust files. Their digest
literals and workflow arguments were updated after review; the credential
detector and rejection assertions are unchanged. The [recorded delta](goal-04-source-publication-validation/reviewed-fixture-delta.json)
affects that fixture alone; SQL production sources stayed unchanged. Failed
development runs remain retained separately from the passing final checks.

## Remaining implementation

- Multiple transaction attempts with fresh source revalidation, patch adaptation
  and new verification evidence; old patches/evidence cannot satisfy a new attempt.
- Independent-process races and generated-history oracle equivalence for the
  complete source commit/revalidation path.
- Complete final workspace, real SQL and execution acceptance evidence for that
  source capability, followed by its own verified human-attributed commit/push.

Checks for this implementation are recorded separately
from the published [executor recovery evidence](goal-04-executor-recovery.md).
That earlier milestone's frozen source manifest does not cover these changes.
