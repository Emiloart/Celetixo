//! Typed, repository-scoped rows for the native lifecycle adapter. These are
//! normalization contracts, not a serialized Database or a service wire schema.
use super::super::*;
use crate::persistence::{CommitOutcomeRecord, RestoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseRecords {
    pub scope: Scope,
    pub clock_ms: u64,
    pub session_counter: u64,
    pub reservation_fence: u64,
    pub grants: BTreeMap<u64, SessionGrantRecord>,
    pub repository: Option<RepositoryRecord>,
    pub transactions: BTreeMap<TransactionId, TransactionRecord>,
    pub evidence: BTreeMap<EvidenceId, AdmittedEvidenceRecord>,
    pub operations: BTreeMap<IdempotencyKey, CommitOperationRecord>,
}
/// Compatibility name for the strictly native entry point. Base rows alone
/// never select a source profile; source reconstruction needs SourceRecords.
pub type NativeRecords = BaseRecords;
impl BaseRecords {
    pub fn empty(scope: Scope) -> Self {
        Self {
            scope,
            clock_ms: 0,
            session_counter: 0,
            reservation_fence: 0,
            grants: BTreeMap::new(),
            repository: None,
            transactions: BTreeMap::new(),
            evidence: BTreeMap::new(),
            operations: BTreeMap::new(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGrantRecord {
    pub principal: PrincipalId,
    pub permissions: BTreeSet<Permission>,
    pub active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRecord {
    pub initial: SnapshotId,
    pub current: SnapshotId,
    /// Only initial and committed snapshots, never private attempt candidates.
    pub registered: BTreeMap<SnapshotId, Snapshot>,
    pub model: EngineeringModel,
    pub history: Vec<CommittedRecord>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRecord {
    pub receipt: CommitReceipt,
    pub accesses: AccessSet,
    pub provenance: AccessProvenance,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRecord {
    pub owner: PrincipalId,
    pub proposal_grant: u64,
    pub intent: TransactionIntent,
    pub state: TransactionState,
    pub attempts: Vec<AttemptRecord>,
    pub lease: Option<LeaseRecord>,
    pub lease_history: Vec<LeaseEvent>,
    pub last_reason: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    pub number: u64,
    pub base: SnapshotId,
    pub model: EngineeringModel,
    pub effective: Option<AccessSet>,
    pub analysis: Option<crate::access::ImpactAnalysis>,
    pub patch: Option<Patch>,
    pub candidate: Option<Snapshot>,
    pub evidence: BTreeMap<CheckId, EvidenceId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRecord {
    pub fence: u64,
    pub owner_grant: u64,
    pub acquired_at: u64,
    pub acquired_attempt: u64,
    pub expires_at: u64,
    pub writes: BTreeSet<ElementId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedEvidenceRecord {
    pub evidence: VerificationEvidence,
    /// Current authenticated admission; immutable original evidence is separate.
    pub verifier_grant: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOperationRecord {
    pub request: CommitRequest,
    pub outcome: Result<CommitReceipt, EngineError>,
}

pub(super) fn export(db: &Database, scope: &Scope) -> Result<NativeRecords, RestoreError> {
    export_base(db, scope, false)
}
pub(super) fn export_base(
    db: &Database,
    scope: &Scope,
    source: bool,
) -> Result<BaseRecords, RestoreError> {
    if !source
        && (!db.source_executors.is_empty()
            || !db.source_policies.is_empty()
            || !db.source_analysis_records.is_empty()
            || !db.source_policy_records.is_empty()
            || !db.source_attempts.is_empty()
            || !db.source_operations.is_empty()
            || !db.source_rejections.is_empty()
            || !db.source_cleanup_grants.is_empty()
            || !db.source_artifact_stores.is_empty()
            || !db.source_evidence_records.is_empty())
    {
        return Err(RestoreError::UnsupportedDomainValue);
    }
    let mut records = NativeRecords::empty(scope.clone());
    records.clock_ms = db.now;
    records.session_counter = db.next_token;
    records.reservation_fence = db.reservation_fences.get(scope).copied().unwrap_or(0);
    for (id, grant) in &db.grants {
        if grant.scope == *scope {
            records.grants.insert(
                *id,
                SessionGrantRecord {
                    principal: grant.principal.clone(),
                    permissions: grant.permissions.clone(),
                    active: grant.active,
                },
            );
        }
    }
    if let Some(repo) = db.repositories.get(scope) {
        if !source && (repo.source_analysis.is_some() || captured_workspace(&repo.current)) {
            return Err(RestoreError::UnsupportedDomainValue);
        }
        let initial = repo
            .snapshots
            .values()
            .find(|snapshot| snapshot.revision() == Revision(0))
            .ok_or(RestoreError::BindingMismatch)?
            .id();
        let mut history = Vec::new();
        for change in &repo.history {
            let tx = db
                .transactions
                .get(&(scope.clone(), change.transaction.clone()))
                .ok_or(RestoreError::BindingMismatch)?;
            let attempt = tx
                .attempts
                .get(usize::try_from(change.attempt).map_err(|_| RestoreError::Limit)?)
                .ok_or(RestoreError::BindingMismatch)?;
            history.push(CommittedRecord {
                receipt: CommitReceipt {
                    transaction: change.transaction.clone(),
                    attempt: change.attempt,
                    revision: change.revision,
                    snapshot: attempt
                        .candidate
                        .as_ref()
                        .ok_or(RestoreError::BindingMismatch)?
                        .id(),
                    patch: attempt
                        .patch
                        .as_ref()
                        .ok_or(RestoreError::BindingMismatch)?
                        .id(),
                },
                accesses: change.accesses.clone(),
                provenance: change.provenance.clone(),
            });
        }
        records.repository = Some(RepositoryRecord {
            initial,
            current: repo.current.id(),
            registered: repo.snapshots.clone(),
            model: repo.model.clone(),
            history,
        });
    }
    for ((record_scope, id), tx) in &db.transactions {
        if record_scope != scope {
            continue;
        }
        records.transactions.insert(
            id.clone(),
            TransactionRecord {
                owner: tx.owner.clone(),
                proposal_grant: tx.owner_capability,
                intent: tx.intent.clone(),
                state: tx.state,
                attempts: tx
                    .attempts
                    .iter()
                    .map(|a| AttemptRecord {
                        number: a.number,
                        base: a.base.id(),
                        model: a.model.clone(),
                        effective: a.effective.clone(),
                        analysis: a.analysis.clone(),
                        patch: a.patch.clone(),
                        candidate: a.candidate.clone(),
                        evidence: a.evidence.clone(),
                    })
                    .collect(),
                lease: tx.lease.as_ref().map(|lease| LeaseRecord {
                    fence: lease.fence,
                    owner_grant: lease.owner,
                    acquired_at: lease.acquired_at,
                    acquired_attempt: lease.acquired_attempt,
                    expires_at: lease.expires_at,
                    writes: lease.writes.clone(),
                }),
                lease_history: tx.lease_history.clone(),
                last_reason: tx.last_reason.clone(),
            },
        );
    }
    for ((record_scope, id), evidence) in &db.evidence {
        if record_scope == scope {
            records.evidence.insert(
                *id,
                AdmittedEvidenceRecord {
                    evidence: evidence.clone(),
                    verifier_grant: *db
                        .evidence_sources
                        .get(&(scope.clone(), *id))
                        .ok_or(RestoreError::BindingMismatch)?,
                },
            );
        }
    }
    for ((record_scope, key), record) in &db.operations {
        if record_scope != scope {
            continue;
        }
        if source && record.request.is_none() {
            continue;
        } // SourceRecords retains these explicitly.
        records.operations.insert(
            key.clone(),
            CommitOperationRecord {
                request: record
                    .request
                    .clone()
                    .ok_or(RestoreError::UnsupportedDomainValue)?,
                outcome: record.outcome.clone(),
            },
        );
    }
    Ok(records)
}

pub(super) fn restore(records: NativeRecords) -> Result<Database, RestoreError> {
    restore_base(records, None)
}
pub(super) fn restore_base(
    records: BaseRecords,
    source: Option<&super::source_records::SourceRecords>,
) -> Result<Database, RestoreError> {
    let scope = records.scope.clone();
    let mut db = Database {
        now: records.clock_ms,
        next_token: records.session_counter,
        ..Default::default()
    };
    db.reservation_fences
        .insert(scope.clone(), records.reservation_fence);
    for (id, grant) in records.grants {
        if id == 0 || id > db.next_token {
            return Err(RestoreError::BindingMismatch);
        }
        db.grants.insert(
            id,
            Grant {
                principal: grant.principal,
                scope: scope.clone(),
                permissions: grant.permissions,
                active: grant.active,
            },
        );
    }
    let Some(repo) = records.repository else {
        if !records.transactions.is_empty()
            || !records.evidence.is_empty()
            || !records.operations.is_empty()
            || records.reservation_fence != 0
        {
            return Err(RestoreError::BindingMismatch);
        }
        return Ok(db);
    };
    repo.model.validate()?;
    let initial = repo
        .registered
        .get(&repo.initial)
        .ok_or(RestoreError::BindingMismatch)?;
    let current = repo
        .registered
        .get(&repo.current)
        .ok_or(RestoreError::BindingMismatch)?;
    if initial.revision() != Revision(0)
        || current.revision().0
            != u64::try_from(repo.history.len()).map_err(|_| RestoreError::Limit)?
        || repo.registered.len()
            != repo
                .history
                .len()
                .checked_add(1)
                .ok_or(RestoreError::Limit)?
    {
        return Err(RestoreError::BindingMismatch);
    }
    let mut revisions = BTreeSet::new();
    for (id, snapshot) in &repo.registered {
        if source.is_none() && captured_workspace(snapshot) {
            return Err(RestoreError::UnsupportedDomainValue);
        }
        if *id != snapshot.id()
            || snapshot.scope() != &scope
            || snapshot.revision() > current.revision()
            || !revisions.insert(snapshot.revision())
        {
            return Err(RestoreError::BindingMismatch);
        }
    }
    let current = current.clone();
    let registered = &repo.registered;
    let mut generations = BTreeMap::from([(repo.model.generation, repo.model.clone())]);
    let mut fences = BTreeSet::new();
    for (id, record) in records.transactions {
        validate_intent(&record.intent).map_err(|_| RestoreError::InvalidDomainValue)?;
        let proposer = db
            .grants
            .get(&record.proposal_grant)
            .ok_or(RestoreError::BindingMismatch)?;
        if proposer.principal != record.owner || record.attempts.is_empty() {
            return Err(RestoreError::BindingMismatch);
        }
        let mut attempts = Vec::new();
        for (offset, a) in record.attempts.into_iter().enumerate() {
            a.model.validate()?;
            if a.number != u64::try_from(offset).map_err(|_| RestoreError::Limit)?
                || a.model.generation > repo.model.generation
            {
                return Err(RestoreError::BindingMismatch);
            }
            if let Some(previous) = generations.insert(a.model.generation, a.model.clone())
                && previous != a.model
            {
                return Err(RestoreError::BindingMismatch);
            }
            let base = registered
                .get(&a.base)
                .ok_or(RestoreError::BindingMismatch)?
                .clone();
            if a.effective.is_some() != a.analysis.is_some()
                || a.patch.is_some() != a.candidate.is_some()
                || (a.analysis.is_none() && (a.patch.is_some() || !a.evidence.is_empty()))
            {
                return Err(RestoreError::BindingMismatch);
            }
            if let Some(patch) = &a.patch {
                if patch.base_snapshot() != base.id()
                    || patch.changes().keys().any(|e| {
                        !a.effective
                            .as_ref()
                            .is_some_and(|accesses| accesses.writes.contains(e))
                    })
                {
                    return Err(RestoreError::BindingMismatch);
                }
                let state = patch.apply(base.state())?;
                for invariant in &record.intent.invariants {
                    invariant
                        .evaluate(&state)
                        .map_err(|_| RestoreError::InvalidDomainValue)?;
                }
                let candidate = base.successor(
                    Revision(
                        base.revision()
                            .0
                            .checked_add(1)
                            .ok_or(RestoreError::InvalidDomainValue)?,
                    ),
                    state,
                )?;
                if a.candidate.as_ref() != Some(&candidate) {
                    return Err(RestoreError::BindingMismatch);
                }
            }
            let context = source.and_then(|s| s.attempt_contexts.get(&(id.clone(), a.number)));
            if source.is_some() && context.is_none() {
                return Err(RestoreError::BindingMismatch);
            }
            attempts.push(Attempt {
                number: a.number,
                base,
                model: a.model,
                effective: a.effective,
                analysis: a.analysis,
                source_analysis: context.map(|c| c.analysis),
                source_policy: context.and_then(|c| c.policy),
                patch: a.patch,
                candidate: a.candidate,
                evidence: a.evidence,
            });
        }
        let lease = record.lease.map(|a| Lease {
            fence: a.fence,
            owner: a.owner_grant,
            acquired_at: a.acquired_at,
            acquired_attempt: a.acquired_attempt,
            expires_at: a.expires_at,
            writes: a.writes,
        });
        let tx = Transaction {
            id: id.clone(),
            owner: record.owner,
            owner_capability: record.proposal_grant,
            scope: scope.clone(),
            intent: record.intent,
            state: record.state,
            attempts,
            lease,
            lease_history: record.lease_history,
            last_reason: record.last_reason,
        };
        if let Some(source) = source {
            super::source_records::validate_transaction(&tx, source)?;
        } else {
            validate_attempts(&tx)?;
        }
        validate_lease(&db, &tx, records.reservation_fence)?;
        for event in &tx.lease_history {
            if matches!(event.kind, LeaseEventKind::Acquired { .. }) && !fences.insert(event.fence)
            {
                return Err(RestoreError::BindingMismatch);
            }
        }
        db.transactions.insert((scope.clone(), id), tx);
    }
    let mut committed = BTreeSet::new();
    let mut committed_revisions = BTreeMap::new();
    let mut committed_generation = 0;
    let mut history = Vec::new();
    for (offset, change) in repo.history.into_iter().enumerate() {
        let revision = Revision(
            u64::try_from(offset)
                .map_err(|_| RestoreError::Limit)?
                .checked_add(1)
                .ok_or(RestoreError::Limit)?,
        );
        let tx = db
            .transactions
            .get(&(scope.clone(), change.receipt.transaction.clone()))
            .ok_or(RestoreError::BindingMismatch)?;
        let candidate = tx
            .attempt()
            .candidate
            .as_ref()
            .ok_or(RestoreError::BindingMismatch)?;
        if tx.state != TransactionState::Committed
            || change.receipt.attempt != tx.attempt().number
            || change.receipt.revision != revision
            || candidate.revision() != revision
            || change.receipt.snapshot != candidate.id()
            || registered.get(&candidate.id()) != Some(candidate)
            || change.receipt.patch
                != tx
                    .attempt()
                    .patch
                    .as_ref()
                    .ok_or(RestoreError::BindingMismatch)?
                    .id()
            || tx.attempt().effective.as_ref() != Some(&change.accesses)
            || provenance(tx).map_err(|_| RestoreError::BindingMismatch)? != change.provenance
            || !committed.insert(tx.id.clone())
            || tx.attempt().model.generation < committed_generation
        {
            return Err(RestoreError::BindingMismatch);
        }
        committed_generation = tx.attempt().model.generation;
        committed_revisions.insert(tx.id.clone(), revision);
        history.push(CommittedChange {
            transaction: tx.id.clone(),
            attempt: tx.attempt().number,
            revision,
            accesses: change.accesses,
            provenance: change.provenance,
        });
    }
    for ((_, id), tx) in &db.transactions {
        if (tx.state == TransactionState::Committed) != committed.contains(id) {
            return Err(RestoreError::BindingMismatch);
        }
        for dependency in &tx.intent.depends_on {
            if dependency == id
                || !db
                    .transactions
                    .get(&(scope.clone(), dependency.clone()))
                    .is_some_and(|other| other.state == TransactionState::Committed)
            {
                return Err(RestoreError::BindingMismatch);
            }
            if tx.state == TransactionState::Committed
                && committed_revisions[dependency] >= committed_revisions[id]
            {
                return Err(RestoreError::BindingMismatch);
            }
        }
    }
    db.repositories.insert(
        scope.clone(),
        Repository {
            current,
            snapshots: repo.registered,
            model: repo.model,
            history,
            source_analysis: source
                .and_then(|s| s.analysis_head.and_then(|id| s.analyses.get(&id)))
                .cloned(),
        },
    );
    for (id, record) in records.evidence {
        let evidence = record.evidence;
        if evidence.id() != id || evidence.provenance().finished_at > db.now {
            return Err(RestoreError::BindingMismatch);
        }
        let tx = db
            .transactions
            .get(&(scope.clone(), evidence.bindings().transaction.clone()))
            .ok_or(RestoreError::BindingMismatch)?;
        let tx = at_attempt(tx, evidence.bindings().attempt)?;
        let signer = db
            .grants
            .get(&record.verifier_grant)
            .ok_or(RestoreError::BindingMismatch)?;
        let check = tx
            .intent
            .required_checks
            .iter()
            .find(|check| check.id() == evidence.check().id())
            .ok_or(RestoreError::BindingMismatch)?;
        if signer.principal != evidence.provenance().verifier
            || !evidence.matches(
                &tx.bindings().map_err(|_| RestoreError::BindingMismatch)?,
                check,
            )
        {
            return Err(RestoreError::BindingMismatch);
        }
        db.evidence.insert((scope.clone(), id), evidence);
        db.evidence_sources
            .insert((scope.clone(), id), record.verifier_grant);
    }
    for tx in db.transactions.values() {
        for a in &tx.attempts {
            for (check, id) in &a.evidence {
                let evidence = db
                    .evidence
                    .get(&(scope.clone(), *id))
                    .ok_or(RestoreError::BindingMismatch)?;
                if evidence.bindings().transaction != tx.id
                    || evidence.bindings().attempt != a.number
                    || evidence.check().id() != check
                {
                    return Err(RestoreError::BindingMismatch);
                }
            }
        }
        if tx.state == TransactionState::Committed {
            for check in &tx.intent.required_checks {
                let id = tx
                    .attempt()
                    .evidence
                    .get(check.id())
                    .ok_or(RestoreError::BindingMismatch)?;
                if !db
                    .evidence
                    .get(&(scope.clone(), *id))
                    .is_some_and(VerificationEvidence::passed)
                {
                    return Err(RestoreError::BindingMismatch);
                }
            }
        }
    }
    let mut accepted = BTreeSet::new();
    for (key, record) in records.operations {
        let request = &record.request;
        if request.key != key {
            return Err(RestoreError::BindingMismatch);
        }
        let tx = db
            .transactions
            .get(&(scope.clone(), request.transaction.clone()))
            .ok_or(RestoreError::BindingMismatch)?;
        let old = at_attempt(tx, request.attempt)?;
        let signer = db
            .grants
            .get(&request.authorization_grant)
            .ok_or(RestoreError::BindingMismatch)?;
        let mut expected = old.clone();
        expected.attempt_mut().evidence = request.evidence.clone();
        // The original evidence selection is immutable even if the live attempt
        // has since selected different evidence. Validate each retained binding.
        for (check, id) in &request.evidence {
            let evidence = db
                .evidence
                .get(&(scope.clone(), *id))
                .ok_or(RestoreError::BindingMismatch)?;
            if evidence.bindings().transaction != tx.id
                || evidence.bindings().attempt != request.attempt
                || evidence.check().id() != check
            {
                return Err(RestoreError::BindingMismatch);
            }
        }
        let event = tx
            .lease_history
            .iter()
            .find(|event| {
                event.fence == request.fence
                    && matches!(event.kind, LeaseEventKind::Acquired { .. })
            })
            .ok_or(RestoreError::BindingMismatch)?;
        if signer.principal != tx.owner || event.authorization_grant != request.authorization_grant
        {
            return Err(RestoreError::BindingMismatch);
        }
        expected.lease = Some(Lease {
            fence: request.fence,
            owner: request.authorization_grant,
            acquired_at: event.at,
            acquired_attempt: event.acquisition_attempt,
            expires_at: match event.kind {
                LeaseEventKind::Acquired { expires_at } => expires_at,
                _ => return Err(RestoreError::BindingMismatch),
            },
            writes: event.scopes.clone(),
        });
        commit_request::validate_request(&expected, request)
            .map_err(|_| RestoreError::BindingMismatch)?;
        let outcome = CommitOutcomeRecord::new(
            scope.clone(),
            request.transaction.clone(),
            request.attempt,
            record.outcome.clone(),
        )?;
        if let Ok(receipt) = outcome.outcome() {
            if tx.state != TransactionState::Committed
                || request.evidence != tx.attempt().evidence
                || receipt.snapshot != request.candidate_snapshot
                || receipt.patch != request.patch
                || db.repositories[&scope].history.iter().all(|change| {
                    change.transaction != receipt.transaction
                        || change.attempt != receipt.attempt
                        || change.revision != receipt.revision
                })
            {
                return Err(RestoreError::BindingMismatch);
            }
            if !accepted.insert(tx.id.clone()) {
                return Err(RestoreError::BindingMismatch);
            }
        } else if let Err(EngineError::Conflict(reports)) = outcome.outcome() {
            let contender = old
                .attempt()
                .effective
                .as_ref()
                .ok_or(RestoreError::BindingMismatch)?;
            for report in reports {
                let committed_tx = db
                    .transactions
                    .get(&(scope.clone(), report.committed.clone()))
                    .ok_or(RestoreError::BindingMismatch)?;
                let committed_attempt = at_attempt(committed_tx, report.committed_attempt)?;
                let change = db.repositories[&scope]
                    .history
                    .iter()
                    .find(|change| {
                        change.transaction == report.committed
                            && change.attempt == report.committed_attempt
                    })
                    .ok_or(RestoreError::BindingMismatch)?;
                if change.revision != report.committed_revision
                    || provenance(&old).map_err(|_| RestoreError::BindingMismatch)?
                        != report.contender_provenance
                    || provenance(&committed_attempt).map_err(|_| RestoreError::BindingMismatch)?
                        != report.committed_provenance
                    || contender.conflicts(&change.accesses) != report.conflicts
                {
                    return Err(RestoreError::BindingMismatch);
                }
            }
        } else if let Err(EngineError::RevalidationRequired { current }) = outcome.outcome() {
            let current = db.repositories[&scope]
                .snapshots
                .get(current)
                .ok_or(RestoreError::BindingMismatch)?;
            if current.revision() <= old.attempt().base.revision() {
                return Err(RestoreError::BindingMismatch);
            }
        }
        db.operations.insert(
            (scope.clone(), key),
            RecordedOperation {
                transaction: request.transaction.clone(),
                attempt: request.attempt,
                request: Some(record.request),
                outcome: record.outcome,
            },
        );
    }
    if accepted != committed {
        return Err(RestoreError::BindingMismatch);
    }
    Ok(db)
}
fn at_attempt(tx: &Transaction, number: u64) -> Result<Transaction, RestoreError> {
    let number = usize::try_from(number).map_err(|_| RestoreError::Limit)?;
    if number >= tx.attempts.len() {
        return Err(RestoreError::BindingMismatch);
    }
    let mut old = tx.clone();
    old.attempts
        .truncate(number.checked_add(1).ok_or(RestoreError::Limit)?);
    Ok(old)
}
fn validate_attempts(tx: &Transaction) -> Result<(), RestoreError> {
    for (index, a) in tx.attempts.iter().enumerate() {
        if a.model != tx.attempts[0].model || (index + 1 < tx.attempts.len() && a.patch.is_none()) {
            return Err(RestoreError::BindingMismatch);
        }
        if a.analysis.is_some() {
            let mut expected = at_attempt(tx, a.number)?;
            analyze_attempt(&mut expected, None, None)
                .map_err(|_| RestoreError::BindingMismatch)?;
            if expected.attempt().effective != a.effective
                || expected.attempt().analysis != a.analysis
            {
                return Err(RestoreError::BindingMismatch);
            }
        }
    }
    let analyzed = tx.attempt().analysis.is_some();
    let patched = tx.attempt().patch.is_some();
    let leased = tx.lease.is_some();
    let valid = match tx.state {
        TransactionState::Proposed => !analyzed && !patched && !leased && tx.attempts.len() == 1,
        TransactionState::Analyzed => analyzed && !patched && !leased && tx.attempts.len() == 1,
        TransactionState::Reserved | TransactionState::Executing => {
            analyzed && !patched && leased && tx.attempts.len() == 1
        }
        TransactionState::Validating | TransactionState::RevalidationRequired => {
            analyzed && patched && leased
        }
        TransactionState::Committed => analyzed && patched && !leased,
        TransactionState::Aborted => {
            !leased
                && tx
                    .last_reason
                    .as_ref()
                    .is_some_and(|reason| !reason.trim().is_empty())
        }
    };
    if !valid {
        return Err(RestoreError::BindingMismatch);
    }
    Ok(())
}
fn validate_lease(db: &Database, tx: &Transaction, fence: u64) -> Result<(), RestoreError> {
    let mut acquired = BTreeMap::new();
    let mut previous_at = 0;
    let mut released = false;
    for event in &tx.lease_history {
        if released {
            return Err(RestoreError::BindingMismatch);
        }
        if event.fence == 0
            || event.fence > fence
            || event.at < previous_at
            || event.at > db.now
            || event.acquisition_attempt >= tx.attempts.len() as u64
        {
            return Err(RestoreError::BindingMismatch);
        }
        previous_at = event.at;
        let grant = db
            .grants
            .get(&event.authorization_grant)
            .ok_or(RestoreError::BindingMismatch)?;
        if grant.principal != event.owner || event.owner != tx.owner {
            return Err(RestoreError::BindingMismatch);
        }
        if let LeaseEventKind::Acquired { expires_at } = event.kind {
            if tx.attempts
                [usize::try_from(event.acquisition_attempt).map_err(|_| RestoreError::Limit)?]
            .effective
            .as_ref()
            .map(|a| &a.writes)
                != Some(&event.scopes)
            {
                return Err(RestoreError::BindingMismatch);
            }
            if expires_at <= event.at || acquired.insert(event.fence, (event, expires_at)).is_some()
            {
                return Err(RestoreError::BindingMismatch);
            }
        } else {
            let (first, expiry) = acquired
                .get_mut(&event.fence)
                .ok_or(RestoreError::BindingMismatch)?;
            if first.authorization_grant != event.authorization_grant
                || first.acquisition_attempt != event.acquisition_attempt
                || first.scopes != event.scopes
            {
                return Err(RestoreError::BindingMismatch);
            }
            match event.kind {
                LeaseEventKind::Renewed {
                    previous_expiry,
                    expires_at,
                } => {
                    if previous_expiry != *expiry || expires_at < *expiry || event.at >= *expiry {
                        return Err(RestoreError::BindingMismatch);
                    }
                    *expiry = expires_at;
                }
                LeaseEventKind::Expired => {
                    if event.at != *expiry {
                        return Err(RestoreError::BindingMismatch);
                    }
                }
                LeaseEventKind::Released { ref reason } => {
                    released = true;
                    if reason.trim().is_empty() {
                        return Err(RestoreError::InvalidDomainValue);
                    }
                }
                LeaseEventKind::Revoked => {}
                LeaseEventKind::Acquired { .. } => return Err(RestoreError::BindingMismatch),
            }
        }
    }
    if acquired.len() > 1 {
        return Err(RestoreError::BindingMismatch);
    }
    if tx.state == TransactionState::Committed {
        let final_event = tx
            .lease_history
            .last()
            .ok_or(RestoreError::BindingMismatch)?;
        let (_, expiry) = acquired
            .get(&final_event.fence)
            .ok_or(RestoreError::BindingMismatch)?;
        if !matches!(&final_event.kind,LeaseEventKind::Released{reason} if reason=="committed")
            || final_event.at >= *expiry
            || tx.lease_history.iter().any(|event| {
                matches!(
                    event.kind,
                    LeaseEventKind::Expired | LeaseEventKind::Revoked
                )
            })
        {
            return Err(RestoreError::BindingMismatch);
        }
    }
    if let Some(lease) = &tx.lease {
        let (first, expiry) = acquired
            .get(&lease.fence)
            .ok_or(RestoreError::BindingMismatch)?;
        if lease.owner != first.authorization_grant
            || lease.acquired_at != first.at
            || lease.acquired_attempt != first.acquisition_attempt
            || lease.expires_at != *expiry
            || lease.writes != first.scopes
            || tx.attempt().effective.as_ref().map(|a| &a.writes) != Some(&lease.writes)
        {
            return Err(RestoreError::BindingMismatch);
        }
        if tx.lease_history.iter().any(|event| {
            event.fence == lease.fence && matches!(event.kind, LeaseEventKind::Released { .. })
        }) {
            return Err(RestoreError::BindingMismatch);
        }
    }
    Ok(())
}
