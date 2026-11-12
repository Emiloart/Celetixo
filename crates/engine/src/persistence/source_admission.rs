//! Checked reconstruction of existing immutable source-admission metadata.
//! Recovery of this value neither restores an executor nor admits evidence.
use super::source_execution::{Cost, charge, definition_cost};
use super::*;
use crate::engine::{SourceAdmission, SourceAuthorizationProvenance};
use crate::ids::{PrincipalId, SourceAdmissionId};
use crate::source_execution::{SourceExecutionPolicy, VerificationPlan};
use crate::source_patch::{PatchOperation, SourcePatch};

/// Exact historical dependencies, restored from independently checked records.
/// `effective_writes` and the plan requirements must originate in Engine
/// analysis; persisted admission bytes cannot choose either dependency.
pub struct SourceAdmissionDependencies<'a> {
    pub base: &'a Snapshot,
    pub patch: &'a SourcePatch,
    pub policy: &'a SourceExecutionPolicy,
    pub plan: &'a VerificationPlan,
    pub effective_writes: &'a BTreeSet<ElementId>,
    /// Historical provenance derived from the original scoped owner grant.
    /// This remains attribution and does not restore that grant's authority.
    pub authorization: &'a SourceAuthorizationProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAdmissionRecord(SourceAdmission);

impl SourceAdmissionRecord {
    pub const FORMAT: &str = "celetixo/source-admission/v1";
    pub fn new(admission: SourceAdmission) -> Self {
        Self(admission)
    }
    pub fn admission(&self) -> &SourceAdmission {
        &self.0
    }
    pub fn digest(&self) -> Digest {
        self.0.id().0
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.0.canonical_bytes()
    }
}

/// Replay the patch and check every admission field against the original
/// dependency graph. Historical expiry is retained, without consulting today's
/// clock or converting historical authorization provenance into a session.
pub fn restore_source_admission(
    dependencies: SourceAdmissionDependencies<'_>,
    expected: SourceAdmissionId,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceAdmission, RestoreError> {
    let mut r = Reader::new(expected.0, SourceAdmissionRecord::FORMAT, bytes, limits)?;
    let patch = r.digest()?;
    let operations = r.digest()?;
    let candidate = r.digest()?;
    let manifest = r.digest()?;
    let report = r.digest()?;
    let policy = r.digest()?;
    let generation = r.number()?;
    let plan = r.digest()?;
    let authorization = SourceAuthorizationProvenance {
        principal: PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        capability_reference: r.digest()?,
    };
    let admitted_at_ms = r.number()?;
    let expires_at_ms = r.number()?;
    let retention_policy = r.digest()?;
    r.end()?;
    if patch != dependencies.patch.id().0
        || operations != dependencies.patch.operation_digest()
        || policy != dependencies.policy.digest()
        || generation != dependencies.policy.input().generation
        || plan != dependencies.plan.id().0
        || expires_at_ms != dependencies.patch.binding().expires_at_ms
        || retention_policy != dependencies.policy.input().retention_policy
        || authorization != *dependencies.authorization
    {
        return Err(RestoreError::BindingMismatch);
    }
    preflight(&mut r, bytes.len(), &dependencies)?;
    let admission = SourceAdmission::restore_checked(
        dependencies.base,
        dependencies.patch,
        dependencies.policy,
        dependencies.plan.clone(),
        authorization,
        admitted_at_ms,
        dependencies.effective_writes,
    )?;
    if candidate != admission.resulting_snapshot().0
        || manifest != admission.resulting_manifest()
        || report != admission.application_report()
    {
        return Err(RestoreError::BindingMismatch);
    }
    if admission.id() != expected || admission.canonical_bytes() != bytes {
        return Err(RestoreError::NonCanonical);
    }
    Ok(admission)
}

/// Conservatively charge the checked plan plus base and candidate upper bound
/// before cloning/replay. The candidate cannot contain more source payload than
/// the base plus patch payloads. This is a canonical-data work budget, not an
/// assertion about the allocator's process RSS or temporary-map overhead.
fn preflight(
    r: &mut Reader<'_>,
    mut bytes: usize,
    d: &SourceAdmissionDependencies<'_>,
) -> Result<(), RestoreError> {
    for check in d.plan.checks() {
        bytes = charge(r, bytes, definition_cost(check.definition())?)?;
    }
    let mut cost = Cost::default();
    cost.count(d.plan.checks().len())?;
    for check in d.plan.checks() {
        cost.count(check.rationales().len())?;
        for rationale in check.rationales() {
            cost.number()?;
            match rationale {
                crate::source_execution::CheckRationale::Invariant(id) => cost.text(id.as_str())?,
                crate::source_execution::CheckRationale::DependencyOf(id) => {
                    cost.text(id.as_str())?
                }
                _ => {}
            }
        }
    }
    cost.count(d.plan.invariant_contracts().len())?;
    for id in d.plan.invariant_contracts().keys() {
        cost.text(id.as_str())?;
        cost.digest()?;
    }
    cost.count(d.effective_writes.len())?;
    for id in d.effective_writes {
        cost.text(id.as_str())?;
    }
    let binding = d.patch.binding();
    for s in [
        binding.scope.organization.as_str(),
        binding.scope.workspace.as_str(),
        binding.scope.repository.as_str(),
        binding.principal.as_str(),
        binding.transaction.as_str(),
        binding.source_attempt.as_str(),
        &binding.producer,
    ] {
        cost.text(s)?;
    }
    // Bound both original inputs and the candidate/report input graph. Charge
    // the entire layered base; selecting effective inputs cannot enlarge it.
    for _ in 0..2 {
        snapshot_cost(&mut cost, d.base)?;
        cost.count(d.patch.operations().len())?;
        for operation in d.patch.operations() {
            cost.text(operation.source_path())?;
            if let Some(path) = operation.destination_path() {
                cost.text(path)?;
            }
            // Report outcomes and preimages contain fixed-size tags/digests.
            for _ in 0..10 {
                cost.digest()?;
            }
            if let PatchOperation::Create { entry, .. } | PatchOperation::Replace { entry, .. } =
                operation
            {
                field_cost(&mut cost, entry.payload().bytes().len())?;
            }
        }
    }
    // Four fixed application headers, hex digest values and the bounded
    // generated provenance string fit in this conservative fixed allowance.
    cost.bytes(4096)?;
    cost.max_field = cost
        .max_field
        .max("celetixo/source-application/v1;base=;patch=;provenance=".len() + 3 * 64);
    cost.count(8)?;
    charge(r, bytes, cost)?;
    Ok(())
}

fn field_cost(cost: &mut Cost, length: usize) -> Result<(), RestoreError> {
    cost.max_field = cost.max_field.max(length);
    cost.bytes(8)?;
    cost.bytes(length)
}

pub(crate) fn snapshot_cost(cost: &mut Cost, snapshot: &Snapshot) -> Result<(), RestoreError> {
    for s in [
        snapshot.scope().organization.as_str(),
        snapshot.scope().workspace.as_str(),
        snapshot.scope().repository.as_str(),
    ] {
        cost.text(s)?;
    }
    let m = snapshot.manifest();
    cost.text(&m.base_commit)?;
    cost.text(&m.provenance)?;
    cost.count(m.inclusion_policy.captured_layers.len())?;
    for _ in &m.inclusion_policy.captured_layers {
        cost.number()?;
    }
    cost.count(m.inclusion_policy.excluded_paths.len())?;
    for (path, reason) in &m.inclusion_policy.excluded_paths {
        cost.text(path)?;
        cost.text(reason)?;
    }
    cost.count(m.inputs.len())?;
    for input in &m.inputs {
        cost.text(&input.path)?;
        for _ in 0..4 {
            cost.number()?;
        }
        if let Some(payload) = &input.bytes {
            field_cost(cost, payload.len())?;
        }
    }
    cost.count(m.environment.len())?;
    for (key, value) in &m.environment {
        cost.text(key)?;
        cost.text(value)?;
        // Captured environment keys gain this origin prefix in a candidate.
        let prefix = "celetixo.source_application.origin.".len();
        cost.bytes(prefix)?;
        cost.max_field = cost
            .max_field
            .max(key.len().checked_add(prefix).ok_or(RestoreError::Limit)?);
    }
    cost.count(snapshot.state().len())?;
    for id in snapshot.state().keys() {
        cost.text(id.as_str())?;
        cost.number()?;
    }
    Ok(())
}
