//! Trusted storage-adapter boundary. Checked records are facts; no capability
//! identity, runtime adapter or authenticated session is reconstructed here.
mod backend;
mod records;
mod source_records;
use super::*;
use crate::persistence::{
    AccessProvenanceRecord, EngineeringModelRecord, ImpactAnalysisRecord, NativeIntentRecord,
    RestoreError, VerificationEvidenceRecord,
};
pub use backend::*;
pub use records::*;
pub(crate) use source_records::error_recovery_size;
pub use source_records::*;

/// Opaque evaluation state supplied by a trusted authority backend. Normalized
/// rows enter through checked construction; this is not a Database blob codec.
pub struct AuthorityState {
    scope: Scope,
    pub(super) db: Database,
}
impl AuthorityState {
    pub fn from_native_records(
        records: NativeRecords,
        limits: NativeRecordLimits,
    ) -> Result<Self, RestoreError> {
        check_budget(&records, limits)?;
        let scope = records.scope.clone();
        Ok(Self {
            scope,
            db: records::restore(records)?,
        })
    }
    pub fn native_records(
        &self,
        limits: NativeRecordLimits,
    ) -> Result<NativeRecords, RestoreError> {
        let scope = &self.scope;
        if self.db.repositories.keys().any(|stored| stored != scope)
            || self.db.grants.values().any(|grant| grant.scope != *scope)
            || self
                .db
                .transactions
                .keys()
                .any(|(stored, _)| stored != scope)
            || self.db.evidence.keys().any(|(stored, _)| stored != scope)
            || self.db.operations.keys().any(|(stored, _)| stored != scope)
            || self.db.mutations.keys().any(|(stored, _)| stored != scope)
            || self
                .db
                .reservation_fences
                .keys()
                .any(|stored| stored != scope)
        {
            return Err(RestoreError::BindingMismatch);
        }
        // Bound the new owned journal before export clones its requests/results.
        let mut journal_budget = Budget {
            limits,
            entries: 0,
            bytes: 0,
        };
        for ((stored, key), record) in &self.db.mutations {
            if stored == scope {
                let size = mutation::recovery_size(scope, key, record)?;
                journal_budget.entries(size.entries)?;
                journal_budget.bytes(size.bytes)?;
            }
        }
        let records = records::export(&self.db, scope)?;
        check_budget(&records, limits)?;
        Ok(records)
    }
    pub fn clock_ms(&self) -> u64 {
        self.db.now
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    /// Trusted backend samples its documented authority clock after locking.
    /// Backward time is explicit failure, never clamped or silently frozen.
    pub fn advance_authoritative_clock(&mut self, now_ms: u64) -> Result<(), EngineError> {
        advance_clock(&mut self.db, now_ms).map(|_| ())
    }
    /// Read-only clock observation checks expiry without inventing persisted
    /// lease-history events. The backend does not publish this observed clock.
    pub fn observe_authoritative_clock(&mut self, now_ms: u64) -> Result<(), EngineError> {
        if now_ms < self.db.now {
            return Err(EngineError::InvalidTime);
        }
        self.db.now = now_ms;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NativeRecordLimits {
    pub entries: usize,
    pub total_value_bytes: usize,
    pub value_bytes: usize,
}
impl Default for NativeRecordLimits {
    fn default() -> Self {
        Self {
            entries: 100_000,
            total_value_bytes: 256 * 1024 * 1024,
            value_bytes: 64 * 1024 * 1024,
        }
    }
}
struct Budget {
    limits: NativeRecordLimits,
    entries: usize,
    bytes: usize,
}
impl Budget {
    fn access(&mut self, access: &AccessSet) -> Result<(), RestoreError> {
        for count in [
            access.reads.len(),
            access.writes.len(),
            access.invariant_scopes.len(),
            access.predicates.len(),
        ] {
            self.entries(count)?;
        }
        Ok(())
    }
    fn completeness(&mut self, value: &crate::access::Completeness) -> Result<(), RestoreError> {
        if let crate::access::Completeness::Incomplete { reasons } = value {
            self.entries(reasons.len())?;
        }
        Ok(())
    }
    fn model(&mut self, model: &EngineeringModel) -> Result<(), RestoreError> {
        self.entries(model.dependencies.len())?;
        for dependencies in model.dependencies.values() {
            self.entries(dependencies.len())?;
        }
        self.completeness(&model.completeness)
    }
    fn provenance(&mut self, value: &AccessProvenance) -> Result<(), RestoreError> {
        self.access(&value.explicit)?;
        self.access(&value.derived)?;
        self.entries(value.invariant_scopes.len())?;
        self.completeness(&value.completeness)
    }
    fn snapshot(&mut self, value: &Snapshot) -> Result<(), RestoreError> {
        for count in [
            value.state().len(),
            value.manifest().inputs.len(),
            value.manifest().environment.len(),
            value.manifest().inclusion_policy.captured_layers.len(),
            value.manifest().inclusion_policy.excluded_paths.len(),
        ] {
            self.entries(count)?;
        }
        self.bytes(value.canonical_bytes().len())
    }
    fn check(&mut self, value: &crate::evidence::CheckSpec) -> Result<(), RestoreError> {
        self.entries(value.command().len())
    }
    fn entries(&mut self, count: usize) -> Result<(), RestoreError> {
        self.entries = self.entries.checked_add(count).ok_or(RestoreError::Limit)?;
        if self.entries > self.limits.entries {
            return Err(RestoreError::Limit);
        }
        Ok(())
    }
    fn bytes(&mut self, count: usize) -> Result<(), RestoreError> {
        self.bytes = self.bytes.checked_add(count).ok_or(RestoreError::Limit)?;
        if count > self.limits.value_bytes || self.bytes > self.limits.total_value_bytes {
            return Err(RestoreError::Limit);
        }
        Ok(())
    }
}
fn check_budget(records: &NativeRecords, limits: NativeRecordLimits) -> Result<(), RestoreError> {
    if limits.entries == 0 || limits.value_bytes == 0 || limits.total_value_bytes == 0 {
        return Err(RestoreError::Limit);
    }
    let mut budget = Budget {
        limits,
        entries: 0,
        bytes: 0,
    };
    budget.entries(records.grants.len())?;
    budget.entries(records.transactions.len())?;
    budget.entries(records.evidence.len())?;
    budget.entries(records.operations.len())?;
    budget.entries(records.mutations.len())?;
    for (key, record) in &records.mutations {
        let size = mutation::recovery_size(&records.scope, key, record)?;
        budget.entries(size.entries)?;
        budget.bytes(size.bytes)?;
    }
    for grant in records.grants.values() {
        budget.entries(grant.permissions.len())?;
        budget.bytes(grant.principal.as_str().len())?;
    }
    if let Some(repo) = &records.repository {
        budget.entries(repo.registered.len())?;
        budget.entries(repo.history.len())?;
        for snapshot in repo.registered.values() {
            budget.snapshot(snapshot)?;
        }
        let current = repo
            .registered
            .get(&repo.current)
            .ok_or(RestoreError::BindingMismatch)?;
        budget.model(&repo.model)?;
        budget.bytes(
            EngineeringModelRecord::new(current, repo.model.clone())?
                .canonical_bytes()
                .len(),
        )?;
        for change in &repo.history {
            budget.access(&change.accesses)?;
            budget.bytes(change.accesses.canonical_bytes().len())?;
            budget.provenance(&change.provenance)?;
            let base = repo
                .registered
                .get(&change.provenance.analysis.snapshot)
                .ok_or(RestoreError::BindingMismatch)?;
            budget.bytes(
                AccessProvenanceRecord::new(
                    base,
                    change.receipt.transaction.clone(),
                    change.receipt.attempt,
                    change.provenance.clone(),
                )?
                .canonical_bytes()
                .len(),
            )?;
        }
        for (id, tx) in &records.transactions {
            budget.entries(tx.attempts.len())?;
            budget.entries(tx.lease_history.len())?;
            if let Some(reason) = &tx.last_reason {
                budget.bytes(
                    crate::persistence::TransactionReasonRecord::new(
                        records.scope.clone(),
                        id.clone(),
                        tx.attempts
                            .last()
                            .ok_or(RestoreError::BindingMismatch)?
                            .number,
                        reason.clone(),
                    )?
                    .canonical_bytes()
                    .len(),
                )?;
            }
            for (sequence, event) in tx.lease_history.iter().enumerate() {
                budget.entries(event.scopes.len())?;
                budget.bytes(
                    crate::persistence::LeaseEventRecord::new(
                        records.scope.clone(),
                        id.clone(),
                        u64::try_from(sequence).map_err(|_| RestoreError::Limit)?,
                        event.clone(),
                    )?
                    .canonical_bytes()
                    .len(),
                )?;
            }
            budget.access(&tx.intent.accesses)?;
            budget.entries(tx.intent.invariants.len())?;
            budget.entries(tx.intent.required_checks.len())?;
            budget.entries(tx.intent.depends_on.len())?;
            for invariant in &tx.intent.invariants {
                if let crate::access::Condition::SumAtMost { elements, .. } = &invariant.condition {
                    budget.entries(elements.len())?;
                }
            }
            for check in &tx.intent.required_checks {
                budget.check(check)?;
            }
            let base = tx
                .attempts
                .first()
                .and_then(|a| repo.registered.get(&a.base))
                .ok_or(RestoreError::BindingMismatch)?;
            budget.bytes(
                NativeIntentRecord::new(base, tx.intent.clone())?
                    .canonical_bytes()
                    .len(),
            )?;
            for a in &tx.attempts {
                budget.entries(a.evidence.len())?;
                let base = repo
                    .registered
                    .get(&a.base)
                    .ok_or(RestoreError::BindingMismatch)?;
                budget.model(&a.model)?;
                budget.bytes(
                    EngineeringModelRecord::new(base, a.model.clone())?
                        .canonical_bytes()
                        .len(),
                )?;
                if let Some(value) = &a.effective {
                    budget.access(value)?;
                    budget.bytes(value.canonical_bytes().len())?;
                }
                if let Some(value) = &a.analysis {
                    budget.access(value.derived())?;
                    budget.completeness(value.completeness())?;
                    budget.bytes(
                        ImpactAnalysisRecord::new(base, value.clone())?
                            .canonical_bytes()
                            .len(),
                    )?;
                }
                if let Some(value) = &a.patch {
                    budget.entries(value.changes().len())?;
                    budget.bytes(value.canonical_bytes().len())?;
                }
                if let Some(value) = &a.candidate {
                    budget.snapshot(value)?;
                }
            }
        }
        for e in records.evidence.values() {
            budget.check(e.evidence.check())?;
            budget.entries(e.evidence.diagnostics().len())?;
            budget.entries(e.evidence.artifacts().len())?;
            let base = repo
                .registered
                .get(&e.evidence.bindings().base_snapshot)
                .ok_or(RestoreError::BindingMismatch)?;
            budget.bytes(
                VerificationEvidenceRecord::new(base, e.evidence.clone())?
                    .canonical_bytes()
                    .len(),
            )?;
        }
    }
    for op in records.operations.values() {
        budget.entries(op.request.evidence.len())?;
        if let Err(EngineError::Conflict(reports)) = &op.outcome {
            budget.entries(reports.len())?;
            for report in reports {
                budget.entries(report.conflicts.len())?;
                budget.provenance(&report.contender_provenance)?;
                budget.provenance(&report.committed_provenance)?;
            }
        }
        let owner = &records
            .transactions
            .get(&op.request.transaction)
            .ok_or(RestoreError::BindingMismatch)?
            .owner;
        budget.bytes(op.request.canonical_bytes(&records.scope, owner).len())?;
        budget.bytes(
            crate::persistence::CommitOutcomeRecord::new(
                records.scope.clone(),
                op.request.transaction.clone(),
                op.request.attempt,
                op.outcome.clone(),
            )?
            .canonical_bytes()
            .len(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
