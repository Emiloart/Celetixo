//! Complete typed source revision history, never a native PatchId projection.
use super::*;

pub(in crate::engine::authority) fn export(
    db: &Database,
    scope: &Scope,
) -> Result<Vec<SourceCommittedRecord>, RestoreError> {
    let mut receipts = BTreeMap::new();
    for ((s, _), record) in &db.mutations {
        if s == scope
            && let Ok(MutationValue::SourceCommitted(receipt)) = &record.outcome
            && receipts
                .insert(
                    (
                        receipt.transaction.clone(),
                        receipt.attempt,
                        receipt.revision,
                    ),
                    receipt.as_ref(),
                )
                .is_some()
        {
            return Err(RestoreError::NonCanonical);
        }
    }
    let mut result = Vec::new();
    if let Some(repo) = db.repositories.get(scope) {
        for change in &repo.history {
            let receipt = receipts
                .remove(&(change.transaction.clone(), change.attempt, change.revision))
                .ok_or(RestoreError::BindingMismatch)?;
            result.push(SourceCommittedRecord {
                receipt: receipt.clone(),
                accesses: change.accesses.clone(),
                provenance: change.provenance.clone(),
            });
        }
    }
    if !receipts.is_empty() {
        return Err(RestoreError::BindingMismatch);
    }
    Ok(result)
}

pub(super) fn validate_catalog(records: &SourceRecords) -> Result<(), RestoreError> {
    let invalid = RestoreError::BindingMismatch;
    let Some(repo) = &records.base.repository else {
        return if records.history.is_empty() {
            Ok(())
        } else {
            Err(invalid)
        };
    };
    let current = repo.registered.get(&repo.current).ok_or(invalid.clone())?;
    if current.revision().0 != records.history.len() as u64
        || repo.registered.len()
            != records
                .history
                .len()
                .checked_add(1)
                .ok_or(RestoreError::Limit)?
    {
        return Err(invalid);
    }
    let mut head = repo.initial;
    let mut model_generation = 0;
    for (index, record) in records.history.iter().enumerate() {
        let receipt = &record.receipt;
        let revision = Revision(
            u64::try_from(index)
                .map_err(|_| RestoreError::Limit)?
                .checked_add(1)
                .ok_or(RestoreError::Limit)?,
        );
        let snapshot = repo
            .registered
            .get(&receipt.publication_snapshot)
            .ok_or(invalid.clone())?;
        if receipt.scope != records.base.scope
            || receipt.revision != revision
            || snapshot.revision() != revision
            || receipt.model_generation <= model_generation
            || receipt.model_generation > repo.model.generation
            || receipt.committed_at_ms > records.base.clock_ms
        {
            return Err(invalid);
        }
        head = receipt.publication_snapshot;
        model_generation = receipt.model_generation;
    }
    if head != repo.current {
        return Err(invalid);
    }
    Ok(())
}

/// Restore structural history before admission/evidence graphs are installed.
/// Original request, full publication proof, artifact provenance and exact
/// outcome are checked after that graph by the shared mutation validator.
pub(in crate::engine::authority) fn restore_base(
    db: &Database,
    records: &SourceRecords,
    registered: &BTreeMap<SnapshotId, Snapshot>,
) -> Result<Vec<CommittedChange>, RestoreError> {
    validate_catalog(records)?;
    let invalid = RestoreError::BindingMismatch;
    let mut result = Vec::new();
    let mut generation = 0;
    for record in &records.history {
        let receipt = &record.receipt;
        let tx = db
            .transactions
            .get(&(records.base.scope.clone(), receipt.transaction.clone()))
            .ok_or(invalid.clone())?;
        if tx.state != TransactionState::Committed
            || tx.attempt().number != receipt.attempt
            || tx.attempt().model.generation < generation
            || tx.attempt().effective.as_ref() != Some(&record.accesses)
            || provenance(tx).map_err(|_| invalid.clone())? != record.provenance
            || registered
                .get(&receipt.publication_snapshot)
                .is_none_or(|s| s.revision() != receipt.revision)
        {
            return Err(invalid);
        }
        generation = tx.attempt().model.generation;
        result.push(CommittedChange {
            transaction: tx.id.clone(),
            attempt: receipt.attempt,
            revision: receipt.revision,
            accesses: record.accesses.clone(),
            provenance: record.provenance.clone(),
        });
    }
    Ok(result)
}

pub(super) fn cost(record: &SourceCommittedRecord) -> Result<Cost, RestoreError> {
    let mut cost = Cost::default();
    for text in [
        record.receipt.scope.organization.as_str(),
        record.receipt.scope.workspace.as_str(),
        record.receipt.scope.repository.as_str(),
        record.receipt.transaction.as_str(),
        record.receipt.source_attempt.as_str(),
    ] {
        cost.text(text)?;
    }
    cost.bytes(1024)?;
    provenance_cost(&mut cost, &record.accesses, &record.provenance)?;
    Ok(cost)
}

pub(super) fn change_cost(record: &CommittedChange) -> Result<Cost, RestoreError> {
    let mut cost = Cost::default();
    cost.text(record.transaction.as_str())?;
    cost.bytes(1024)?;
    provenance_cost(&mut cost, &record.accesses, &record.provenance)?;
    Ok(cost)
}

fn provenance_cost(
    cost: &mut Cost,
    accesses: &AccessSet,
    provenance: &AccessProvenance,
) -> Result<(), RestoreError> {
    for access in [accesses, &provenance.explicit, &provenance.derived] {
        access_cost(cost, access)?;
    }
    cost.count(provenance.invariant_scopes.len())?;
    for id in &provenance.invariant_scopes {
        cost.text(id.as_str())?;
    }
    cost.text(&provenance.analysis.analyzer)?;
    cost.text(&provenance.analysis.version)?;
    if let crate::access::Completeness::Incomplete { reasons } = &provenance.completeness {
        cost.count(reasons.len())?;
        for reason in reasons {
            cost.text(reason)?;
        }
    }
    Ok(())
}
