# Fresh source revalidation

**Status:** validated bounded Engine/PostgreSQL fresh source revalidation.
[Checks](goal-04-source-revalidation-validation/checks.json) and the
[validation scope](goal-04-source-revalidation-validation/README.md) bind exact
sources to 548 ordinary workspace tests, 14 selected SQL parent cases and 10
actual Linux T1 cases. This record does not claim complete Goal 04 or milestone
04.2. The subsequent
[04.1 acceptance audit](goal-04-authority-acceptance.md) evaluates the original
durable-state criteria separately.

A stale source transaction can now request a new, explicitly numbered attempt.
The Engine uses current installed complete repository analysis and current
snapshot-bound execution policy, recomputes semantic impact from the immutable
intent, and replaces the owned live reservation with a new monotonically
increasing fence in one staged decision. New work requires Analyze, Reserve and
Execute authorization. Raw actor IDs and decoded requests grant no capability.

The new attempt starts in Executing with no patch, candidate or verification
results. Unfinished old source execution closes as RevalidationRequired; sealed
records remain immutable history. Old admissions, workers, patches and evidence
cannot satisfy the new attempt. A new explicit patch and all new verification
are required before the existing source publication gate can commit. This path
does not adapt an old patch, retain old read results, or certify compatibility.
Unknown declared identities and incomplete semantic analysis fail closed.

A revalidation receipt binds the previous/current attempt, base, model, complete
analysis and policy, intent selectors, old/new fence, expiry, recomputed accesses
and impact, intervening revisions and invalidated source executions. The shared
original-outcome journal stores the request, independently observed historical
metadata, and typed success or domain rejection. Checked restoration recomputes
the original decision and checks its attempt, reservation and invalidation
effects. Exact recovery authenticates current original-owner Analyze authority
and returns before an update or execution; it grants no renewed lease. Current
Reserve/Execute permissions and a live owned original lease remain necessary for
new effects. Expired, revoked and cancelled owners cannot revive work.

A published source revision has application provenance rather than the original
capture manifest. The additive EnginePublication capture kind binds a fresh
policy to the exact prior source commit receipt. Installation and complete
source restoration require that receipt in authoritative source revision
history. Manifest strings or caller-supplied digests cannot authorize a derived
capture. Existing capture kinds and their encoded values retain their meaning.

Migration 0008 adds immutable forced-RLS projections for original revalidation
metadata, observed lease/source event prefixes and accepted fresh epochs. Scoped
foreign keys require each noninitial source epoch to have one accepted original
revalidation. The SQL adapter publishes attempts, old-execution invalidation,
reservation replacement and original result in the same PostgreSQL transaction.
The final database-time gate rechecks the original live lease and current
metadata before commit. Migrations 0001–0007 remain byte-for-byte unchanged.

Validation includes finite complete analyzer fixtures, not proof of language
extraction completeness. The implemented deterministic policy always requires
fresh execution; evidence-equivalence reuse is not implemented. Full generated
source transaction histories against an independent oracle, all remaining
04.2 gates and later distributed services/operations remain Goal 04 work.

The generic Analyze operation remains restricted to Proposed. The compound
revalidation operation alone constructs a fresh epoch before analyzing it; merely
adding typed transition edges cannot reopen an old frame with its patch/evidence.
Regression assertions cover direct and keyed Analyze rejection without mutation.
