//! Original native patch/evidence/revalidation decisions. Historical results
//! reference original attempts and do not select current evidence or leases.
use super::*;
use crate::persistence::{RecoverySize, RestoreError};

struct Cost(RecoverySize);
impl Cost {
    fn new() -> Self {
        Self(RecoverySize {
            entries: 0,
            bytes: 512,
        })
    }
    fn add(&mut self, entries: usize, bytes: usize) -> Result<(), RestoreError> {
        self.0.entries = self
            .0
            .entries
            .checked_add(entries)
            .ok_or(RestoreError::Limit)?;
        self.0.bytes = self.0.bytes.checked_add(bytes).ok_or(RestoreError::Limit)?;
        Ok(())
    }
    fn field(&mut self, value: &str) -> Result<(), RestoreError> {
        self.add(1, value.len().checked_add(16).ok_or(RestoreError::Limit)?)
    }
    fn access(&mut self, access: &AccessSet) -> Result<(), RestoreError> {
        for set in [&access.reads, &access.writes, &access.invariant_scopes] {
            for element in set {
                self.field(element.as_str())?;
            }
        }
        for crate::access::Predicate::Prefix(prefix) in &access.predicates {
            self.field(prefix)?;
        }
        Ok(())
    }
    fn provenance(&mut self, p: &AccessProvenance) -> Result<(), RestoreError> {
        self.access(&p.explicit)?;
        self.access(&p.derived)?;
        for element in &p.invariant_scopes {
            self.field(element.as_str())?;
        }
        self.field(&p.analysis.analyzer)?;
        self.field(&p.analysis.version)?;
        if let crate::access::Completeness::Incomplete { reasons } = &p.completeness {
            for reason in reasons {
                self.field(reason)?;
            }
        }
        self.add(1, 256)
    }
}
pub(super) fn patch_size(patch: &Patch) -> Result<RecoverySize, RestoreError> {
    let mut cost = Cost::new();
    for element in patch.changes().keys() {
        cost.field(element.as_str())?;
        cost.add(1, 64)?;
    }
    Ok(cost.0)
}
pub(super) fn evidence_size(e: &VerificationEvidence) -> Result<RecoverySize, RestoreError> {
    let mut cost = Cost::new();
    cost.field(e.bindings().transaction.as_str())?;
    cost.field(e.check().id().as_str())?;
    cost.field(e.check().toolchain())?;
    for arg in e.check().command() {
        cost.field(arg)?;
    }
    let p = e.provenance();
    for text in [p.verifier.as_str(), &p.runner, &p.run_id] {
        cost.field(text)?;
    }
    if let crate::evidence::CheckResult::Incomplete { reason } = e.result() {
        cost.field(reason)?;
    }
    for diagnostic in e.diagnostics() {
        cost.field(diagnostic)?;
    }
    for artifact in e.artifacts() {
        cost.field(artifact.name())?;
        cost.field(artifact.media_type())?;
        cost.add(1, artifact.content().len())?;
    }
    Ok(cost.0)
}
pub(super) fn error_size(error: &EngineError) -> Result<RecoverySize, RestoreError> {
    if !matches!(error, EngineError::Conflict(_)) {
        return authority::error_recovery_size(error);
    }
    let mut cost = Cost::new();
    if let EngineError::Conflict(reports) = error {
        for report in reports {
            cost.field(report.contender.as_str())?;
            cost.field(report.committed.as_str())?;
            cost.provenance(&report.contender_provenance)?;
            cost.provenance(&report.committed_provenance)?;
            for conflict in &report.conflicts {
                cost.field(conflict.element.as_str())?;
                cost.add(1, 64)?;
                if let Some(crate::access::Predicate::Prefix(prefix)) = &conflict.predicate {
                    cost.field(prefix)?;
                }
            }
        }
    }
    Ok(cost.0)
}

pub(super) fn check_shape(r: &MutationRecord) -> Result<(), RestoreError> {
    let invalid = RestoreError::BindingMismatch;
    if !matches!(
        r.request,
        MutationRequest::SubmitPatch { .. }
            | MutationRequest::RecordEvidence { .. }
            | MutationRequest::Revalidate { .. }
    ) {
        return Err(invalid);
    }
    let Some(before) = r.state_before else {
        return if r.outcome == Err(EngineError::NotFound)
            || (matches!(
                r.request,
                MutationRequest::SubmitPatch { .. } | MutationRequest::Revalidate { .. }
            ) && r.outcome == Err(EngineError::SourceMutationUnsupported))
        {
            Ok(())
        } else {
            Err(invalid)
        };
    };
    let attempt = r.resolved_attempt.ok_or(invalid.clone())?;
    match &r.outcome {
        Ok(MutationValue::Bindings { bindings }) => {
            let expected = if matches!(r.request, MutationRequest::Revalidate { .. }) {
                attempt
                    .checked_add(1)
                    .ok_or(RestoreError::InvalidDomainValue)?
            } else {
                attempt
            };
            if !matches!(
                r.request,
                MutationRequest::SubmitPatch { .. } | MutationRequest::Revalidate { .. }
            ) || (matches!(r.request, MutationRequest::SubmitPatch { .. })
                && before != TransactionState::Executing)
                || !before.allows(TransactionState::Validating)
                || Some(&bindings.transaction) != r.request.transaction()
                || bindings.attempt != expected
            {
                return Err(invalid);
            }
            Ok(())
        }
        Ok(MutationValue::Evidence { evidence }) => {
            let MutationRequest::RecordEvidence {
                evidence: input, ..
            } = &r.request
            else {
                return Err(invalid);
            };
            if before != TransactionState::Validating
                || *evidence != input.id()
                || Some(&input.bindings().transaction) != r.request.transaction()
                || input.bindings().attempt != attempt
            {
                Err(invalid)
            } else {
                Ok(())
            }
        }
        Err(EngineError::IllegalTransition { from, to }) => {
            let denied = match r.request {
                MutationRequest::RecordEvidence { .. } => before != TransactionState::Validating,
                MutationRequest::SubmitPatch { .. } => before != TransactionState::Executing,
                _ => !before.allows(TransactionState::Validating),
            };
            if *from == before && *to == TransactionState::Validating && denied {
                Ok(())
            } else {
                Err(invalid)
            }
        }
        Err(error) if mutation_error(&r.request, error) && *error != EngineError::NotFound => {
            if let EngineError::Conflict(reports) = error {
                crate::persistence::validate_conflict_reports(
                    r.request.transaction().ok_or(invalid.clone())?,
                    attempt,
                    reports,
                )?;
            }
            Ok(())
        }
        _ => Err(invalid),
    }
}

fn active_before(tx: &Transaction, grant: u64, r: &MutationRecord) -> bool {
    r.lease_events_before
        .checked_sub(1)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| tx.lease_history.get(n))
        .is_some_and(|event| {
            let expiry = match event.kind {
                LeaseEventKind::Acquired { expires_at }
                | LeaseEventKind::Renewed { expires_at, .. } => expires_at,
                _ => return false,
            };
            event.authorization_grant == grant && expiry > r.recorded_at_ms
        })
}
fn binding(tx: &Transaction, attempt: &Attempt) -> Result<EvidenceBindings, RestoreError> {
    Ok(EvidenceBindings {
        transaction: tx.id.clone(),
        attempt: attempt.number,
        base_snapshot: attempt.base.id(),
        candidate_snapshot: attempt
            .candidate
            .as_ref()
            .ok_or(RestoreError::BindingMismatch)?
            .id(),
        patch: attempt
            .patch
            .as_ref()
            .ok_or(RestoreError::BindingMismatch)?
            .id(),
        analysis: attempt
            .analysis
            .as_ref()
            .ok_or(RestoreError::BindingMismatch)?
            .digest(),
    })
}
pub(super) fn validate(
    db: &Database,
    scope: &Scope,
    tx: Option<&Transaction>,
    principal: &PrincipalId,
    grant: u64,
    r: &MutationRecord,
) -> Result<(), RestoreError> {
    check_shape(r)?;
    let invalid = RestoreError::BindingMismatch;
    let source = db
        .repositories
        .get(scope)
        .is_some_and(|repo| repo.source_analysis.is_some());
    if r.outcome == Err(EngineError::SourceMutationUnsupported) {
        return if source { Ok(()) } else { Err(invalid) };
    }
    if r.resolved_attempt.is_none() {
        return if r.outcome == Err(EngineError::NotFound) {
            Ok(())
        } else {
            Err(invalid)
        };
    }
    let tx = tx.ok_or(invalid.clone())?;
    let original = tx
        .attempts
        .get(
            usize::try_from(r.resolved_attempt.ok_or(invalid.clone())?)
                .map_err(|_| RestoreError::Limit)?,
        )
        .ok_or(invalid.clone())?;
    let active = active_before(tx, grant, r);
    if matches!(
        r.request,
        MutationRequest::SubmitPatch { .. } | MutationRequest::Revalidate { .. }
    ) {
        if source {
            return Err(invalid);
        }
        if r.outcome == Err(EngineError::InvalidLease) {
            return if !active { Ok(()) } else { Err(invalid) };
        }
        if !active {
            return Err(invalid);
        }
    }
    match (&r.request, &r.outcome) {
        (MutationRequest::SubmitPatch { patch, .. }, Ok(MutationValue::Bindings { bindings })) => {
            if original.patch.as_ref() != Some(patch) || binding(tx, original)? != *bindings {
                Err(invalid)
            } else {
                Ok(())
            }
        }
        (MutationRequest::Revalidate { .. }, Ok(MutationValue::Bindings { bindings })) => {
            let result = tx
                .attempts
                .get(usize::try_from(bindings.attempt).map_err(|_| RestoreError::Limit)?)
                .ok_or(invalid.clone())?;
            let patch = original.patch.as_ref().ok_or(invalid.clone())?;
            if binding(tx, result)? != *bindings
                || result.model != original.model
                || result
                    .patch
                    .as_ref()
                    .is_none_or(|p| p.changes() != patch.changes())
            {
                Err(invalid)
            } else {
                Ok(())
            }
        }
        (
            MutationRequest::RecordEvidence { evidence, .. },
            Ok(MutationValue::Evidence { evidence: id }),
        ) => {
            if db.evidence.get(&(scope.clone(), *id)) != Some(evidence.as_ref())
                || evidence.provenance().verifier != *principal
                || evidence.provenance().finished_at > r.recorded_at_ms
                || evidence.bindings() != &binding(tx, original)?
                || !tx
                    .intent
                    .required_checks
                    .iter()
                    .any(|check| evidence.matches(evidence.bindings(), check))
            {
                Err(invalid)
            } else {
                Ok(())
            }
        }
        (MutationRequest::SubmitPatch { patch, .. }, Err(EngineError::WrongPatchBase)) => {
            if patch.base_snapshot() != original.base.id() {
                Ok(())
            } else {
                Err(invalid)
            }
        }
        (MutationRequest::SubmitPatch { patch, .. }, Err(EngineError::UndeclaredWrite)) => {
            if original
                .effective
                .as_ref()
                .is_some_and(|a| patch.changes().keys().any(|e| !a.writes.contains(e)))
            {
                Ok(())
            } else {
                Err(invalid)
            }
        }
        (MutationRequest::SubmitPatch { patch, .. }, Err(EngineError::Patch(error))) => {
            if patch.apply(original.base.state()) == Err(error.clone()) {
                Ok(())
            } else {
                Err(invalid)
            }
        }
        (MutationRequest::Revalidate { .. }, Err(EngineError::Conflict(reports))) => {
            let repo = db.repositories.get(scope).ok_or(invalid.clone())?;
            let accesses = original.effective.as_ref().ok_or(invalid.clone())?;
            let expected_provenance = provenance_at(tx, original).map_err(|_| invalid.clone())?;
            if reports
                .windows(2)
                .any(|pair| pair[0].committed_revision >= pair[1].committed_revision)
            {
                return Err(invalid);
            }
            for report in reports {
                let change = repo
                    .history
                    .iter()
                    .find(|change| {
                        change.transaction == report.committed
                            && change.attempt == report.committed_attempt
                            && change.revision == report.committed_revision
                    })
                    .ok_or(invalid.clone())?;
                if change.revision <= original.base.revision()
                    || report.contender_provenance != expected_provenance
                    || report.committed_provenance != change.provenance
                    || report.conflicts != accesses.conflicts(&change.accesses)
                {
                    return Err(invalid);
                }
            }
            Ok(())
        }
        (_, Err(error)) if mutation_error(&r.request, error) => Ok(()),
        _ => Err(invalid),
    }
}
