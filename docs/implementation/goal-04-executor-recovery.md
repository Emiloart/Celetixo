# Original source-executor outcomes and launch recovery

**Status:** validated original executor outcomes and coordinator recovery in the
bounded local captured-source PostgreSQL profile. This extends the
[original mutation journal](goal-04-mutation-recovery.md). Complete 04.1/04.2 acceptance and the full
[Goal 04](goal-04.md) remain in progress.

`Engine::mutate_source_executor` journals Claim, PatchApplied,
BeginVerification and Finish through the same atomic boundary as other original
mutations. The key includes scope, independently derived executor principal and
grant, and an idempotency key. Fresh opaque capability/store, current scoped
grant, backend, active status and expiry checks precede original-outcome lookup.
A session cannot invoke this executor interface.

The complete original admission selector, PatchBinding and action are retained,
including the original Finish outcome and reason. Exact retries return the
original typed result or rejection and decision time. Changed bodies conflict.
Original missing selectors stay missing in their historical reply after later
admission. Rejected raw transaction/admission selectors remain separate from the
independently resolved source admission and native transaction context.

Historical results contain an immutable view header and exact original event
prefix. They refer to the independently checked admission/plan graph; they cannot
create a live executor, launch permission, evidence receipt or commit authority.
Restore checks original executor attribution, binding, transition/rejection
precedence, event order, reason and time. One journal record may produce a source
event; distinct keys cannot claim the same produced event.

Migration `0006_source_executor_mutations.sql` extends immutable, forced-RLS
mutation rows. Published migrations 0001–0005 remain unchanged. Scoped foreign
keys bind resolved admission, original event state, produced event/time and
accepted executor claim. Tagged nullable fields reject partial context/results
without treating SQL UNKNOWN as approval. Existing administrator privilege and
runtime least-privilege boundaries stay in force. Source events and their journal
rows publish in one SQL transaction, with checked readback and final database-time
authority gates. Rollback retains neither effect nor key; lost replies recover
the original result.

The execution coordinator uses deterministic admission/action keys. A replayed
claim stops before materialization; a lost claim reply also stops execution.
Workspace/OCI reconciliation remains independent. Within an invocation that owns
its private workspace, patch, verification and terminal metadata may recover one
indeterminate response by the exact original request. Subsequent effects still
require current admission validation. Retrying an original reply cannot create
another check, process, workspace or terminal event.

The [validation record](goal-04-executor-validation/README.md) distinguishes
focused development checks from the final exact-source gates, including retained
failed/interrupted SQL attempts and all 87 SQL parent cases checked separately.
Synthetic check fixtures establish metadata/transition behavior only; separate
Linux T1 tests cover the existing isolated runtime. Source prepared
commit/revalidation, broader independent generated-history SQL oracle tests,
real event/workflow/object/analytics services, authenticated service transport,
failover, backup/restore, host loss and measured load remain required.
