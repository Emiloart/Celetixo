//! Checked original source publication outcomes, separate from live authority.
use super::*;
use crate::persistence::RestoreError;
use crate::source_evidence::{SourceEvidenceAssessment, SourceEvidenceError};

mod history;

pub(super) fn domain_error(error: &EngineError) -> bool {
    matches!(
        error,
        EngineError::NotFound
            | EngineError::Unauthorized
            | EngineError::InvalidLease
            | EngineError::ModelChanged
            | EngineError::CounterExhausted
            | EngineError::CommitRequestMismatch
            | EngineError::IncompleteAnalysis
            | EngineError::UnsatisfiedDependency
            | EngineError::RevalidationRequired { .. }
            | EngineError::SourceAttempt(
                SourceAttemptError::InvalidState
                    | SourceAttemptError::BindingMismatch
                    | SourceAttemptError::Expired
                    | SourceAttemptError::UntrustedExecutor
                    | SourceAttemptError::PolicyChanged
            )
            | EngineError::SourceEvidence(SourceEvidenceError::BindingMismatch)
    )
}

pub(super) fn check_record(
    request: &SourceCommitRequest,
    record: &MutationRecord,
    actor: &MutationActor,
) -> Result<(), RestoreError> {
    let invalid = RestoreError::BindingMismatch;
    let MutationActor::Session { principal, grant } = actor else {
        return Err(invalid);
    };
    let context = record.source_commit.as_ref().ok_or(invalid.clone())?;
    if *grant == 0
        || context.analysis_grant == 0
        || record.administration.is_some()
        || record.source_execution.is_some()
        || record.resolved_attempt.is_some() != record.state_before.is_some()
        || (record.resolved_attempt.is_none() && record.lease_events_before != 0)
        || context.head.is_some() != context.model_generation.is_some()
        || context.head.is_some() != context.analysis.is_some()
        || context.model_generation == Some(0)
        || (context.head.is_none() && context.policy.is_some())
    {
        return Err(invalid);
    }
    if let Some(source) = &context.source
        && (source.event_count == 0
            || source.admission != request.admission
            || source.executor_grant.is_some() != source.executor_active.is_some()
            || source.store_registration.is_some() != source.store_active.is_some()
            || source.executor_grant == Some(0)
            || source.store_registration == Some(0)
            || (source.state == SourceAttemptState::EvidenceSealed && source.evidence.is_none()))
    {
        return Err(invalid);
    }
    if let Some(artifacts) = &context.artifacts
        && (artifacts.registration == 0
            || artifacts.observed_at_ms > record.recorded_at_ms
            || artifacts.registration != request.store_registration
            || artifacts.evidence != request.evidence
            || artifacts.provenance != request.storage_provenance
            || context.source.as_ref().is_none_or(|s| {
                s.state != SourceAttemptState::EvidenceSealed
                    || s.evidence != Some(artifacts.evidence)
            }))
    {
        return Err(invalid);
    }
    match &record.outcome {
        Ok(MutationValue::SourceCommitted(receipt)) => {
            receipt.validate_request(&receipt.scope, principal, request)?;
            if record.resolved_attempt != Some(request.binding.transaction_attempt)
                || record.state_before != Some(TransactionState::Validating)
                || record.lease_events_before == 0
                || *grant != request.authorization_grant
                || context.analysis_principal != request.analysis_principal
                || context.analysis_grant != request.analysis_grant
                || context.head != Some(request.binding.base_snapshot)
                || context.analysis != Some(request.binding.analysis_artifact)
                || context.policy != Some(request.policy)
                || context.model_generation.and_then(|n| n.checked_add(1))
                    != Some(receipt.model_generation)
                || context.artifacts.is_none()
                || receipt.committed_at_ms != record.recorded_at_ms
                || context.source.as_ref().is_none_or(|s| {
                    s.source_attempt != request.binding.source_attempt
                        || s.state != SourceAttemptState::EvidenceSealed
                        || s.evidence != Some(request.evidence)
                        || s.executor_active != Some(true)
                        || s.store_active != Some(true)
                        || s.store_registration != Some(request.store_registration)
                })
            {
                return Err(invalid);
            }
            Ok(())
        }
        Err(error) if domain_error(error) => Ok(()),
        _ => Err(invalid),
    }
}

pub(super) fn validate(
    db: &Database,
    scope: &Scope,
    key: &MutationKey,
    request: &SourceCommitRequest,
    record: &MutationRecord,
) -> Result<(), RestoreError> {
    check_record(request, record, &key.actor)?;
    let invalid = RestoreError::BindingMismatch;
    let MutationActor::Session { principal, grant } = &key.actor else {
        return Err(invalid);
    };
    let context = record.source_commit.as_ref().ok_or(invalid.clone())?;
    let actor = db.grants.get(grant).ok_or(invalid.clone())?;
    let analyst = db
        .grants
        .get(&context.analysis_grant)
        .ok_or(invalid.clone())?;
    if request.key != key.key
        || record.recorded_at_ms > db.now
        || actor.scope != *scope
        || actor.principal != *principal
        || !actor.permissions.contains(&Permission::Commit)
        || analyst.scope != *scope
        || analyst.principal != context.analysis_principal
        || !analyst.permissions.contains(&Permission::ManageModel)
    {
        return Err(invalid);
    }
    if let Some(head) = context.head {
        let repo = db.repositories.get(scope).ok_or(invalid.clone())?;
        let snapshot = repo.snapshots.get(&head).ok_or(invalid.clone())?;
        let facts = db
            .source_analysis_records
            .get(&(scope.clone(), context.analysis.ok_or(invalid.clone())?))
            .ok_or(invalid.clone())?;
        facts.validate(snapshot, facts.index_generation())?;
        let generation = context.model_generation.ok_or(invalid.clone())?;
        if generation > repo.model.generation
            || (generation == repo.model.generation && repo.model != facts.model(generation))
        {
            return Err(invalid);
        }
        if let Some(policy) = context.policy
            && !db
                .source_policy_records
                .contains_key(&(scope.clone(), policy))
        {
            return Err(invalid);
        }
    }
    let native = db
        .transactions
        .get(&(scope.clone(), request.binding.transaction.clone()));
    if let Some(attempt) = record.resolved_attempt {
        let native = native.ok_or(invalid.clone())?;
        if attempt > native.attempt().number
            || record.lease_events_before > native.lease_history.len() as u64
            || record
                .lease_events_before
                .checked_sub(1)
                .is_some_and(|n| native.lease_history[n as usize].at > record.recorded_at_ms)
        {
            return Err(invalid);
        }
    }
    let source = if let Some(observed) = &context.source {
        let source = db
            .source_attempts
            .get(&(scope.clone(), observed.source_attempt.clone()))
            .ok_or(invalid.clone())?;
        let prefix = usize::try_from(observed.event_count).map_err(|_| RestoreError::Limit)?;
        if source.admission.id() != observed.admission
            || prefix > source.events.len()
            || source.events[prefix - 1].to != observed.state
            || source.events[prefix - 1].at_ms > record.recorded_at_ms
        {
            return Err(invalid);
        }
        if let Some(evidence) = observed.evidence {
            let retained = db
                .source_evidence_records
                .get(&(scope.clone(), evidence))
                .ok_or(invalid.clone())?;
            if retained.receipt.admission != observed.admission
                || retained.receipt.recorded_at_ms > record.recorded_at_ms
            {
                return Err(invalid);
            }
        }
        let claimed = source.events[..prefix]
            .iter()
            .any(|e| e.to == SourceAttemptState::Patching)
            .then_some(source.executor_capability)
            .flatten();
        if claimed != observed.executor_grant
            || (observed.state.is_terminal() && source.evidence != observed.evidence)
        {
            return Err(invalid);
        }
        if let Some(id) = observed.executor_grant {
            let executor = db.source_executors.get(&id).ok_or(invalid.clone())?;
            if executor.scope != *scope
                || (observed.executor_active == Some(false) && executor.active)
            {
                return Err(invalid);
            }
        }
        let registration = observed.evidence.and_then(|id| {
            db.source_evidence_records
                .get(&(scope.clone(), id))
                .map(|e| e.store_registration)
        });
        if registration != observed.store_registration {
            return Err(invalid);
        }
        if let Some(id) = observed.store_registration {
            let store = db.source_artifact_stores.get(&id).ok_or(invalid.clone())?;
            if store.scope != *scope || (observed.store_active == Some(false) && store.active) {
                return Err(invalid);
            }
        }
        Some(source)
    } else {
        None
    };
    if let Some(artifacts) = &context.artifacts {
        let retained = db
            .source_evidence_records
            .get(&(scope.clone(), artifacts.evidence))
            .ok_or(invalid.clone())?;
        let storage = db
            .source_artifact_stores
            .get(&artifacts.registration)
            .ok_or(invalid.clone())?;
        if retained.store_registration != artifacts.registration
            || retained.receipt.storage_provenance != artifacts.provenance
            || storage.scope != *scope
            || storage.provenance != artifacts.provenance
            || retained.receipt.assessment != SourceEvidenceAssessment::Passed
            || retained.receipt.recorded_at_ms > artifacts.observed_at_ms
        {
            return Err(invalid);
        }
    }
    let historical = history::at_observation(db, scope, key, request, record)?;
    let analyst_actor = MutationActor::Session {
        principal: context.analysis_principal.clone(),
        grant: context.analysis_grant,
    };
    let expected = super::super::source_publication::validate_source_commit_metadata(
        &historical,
        scope,
        &key.actor,
        &analyst_actor,
        request,
    );
    let selected = match (&record.outcome, expected) {
        (Err(actual), Err(expected)) if *actual == expected => return Ok(()),
        (Ok(MutationValue::SourceCommitted(_)), Ok(selected)) => selected,
        _ => return Err(invalid),
    };
    match &record.outcome {
        Ok(MutationValue::SourceCommitted(receipt)) => {
            receipt.validate_request(scope, principal, request)?;
            let source = source.ok_or(invalid.clone())?;
            let native = native.ok_or(invalid.clone())?;
            let repo = db.repositories.get(scope).ok_or(invalid.clone())?;
            let base = repo
                .snapshots
                .get(&request.binding.base_snapshot)
                .ok_or(invalid.clone())?;
            let proof = &selected.publication;
            proof
                .validate(base, &source.patch)
                .map_err(|_| invalid.clone())?;
            let change = repo
                .history
                .iter()
                .find(|c| c.revision == receipt.revision)
                .ok_or(invalid.clone())?;
            let retained = db
                .source_evidence_records
                .get(&(scope.clone(), request.evidence))
                .ok_or(invalid.clone())?;
            let evidence_receipt = crate::persistence::SourceEvidenceReceiptRecord::new(
                scope.clone(),
                retained.store_registration,
                retained.receipt.clone(),
            )?;
            let release = native
                .lease_history
                .get(usize::try_from(record.lease_events_before).map_err(|_| RestoreError::Limit)?)
                .ok_or(invalid.clone())?;
            if native.state != TransactionState::Committed
                || **receipt != selected.receipt
                || native.attempt().number != receipt.attempt
                || native.owner != *principal
                || context.model_generation != Some(native.attempt().model.generation)
                || receipt.model_generation > repo.model.generation
                || (receipt.model_generation == repo.model.generation
                    && repo.model != request.analysis.model(receipt.model_generation))
                || native.lease_history.len() as u64
                    != record
                        .lease_events_before
                        .checked_add(1)
                        .ok_or(invalid.clone())?
                || release.at != receipt.committed_at_ms
                || release.fence != request.fence
                || release.authorization_grant != *grant
                || !matches!(&release.kind, LeaseEventKind::Released {reason} if reason == "committed")
                || source.admission_fence != request.fence
                || evidence_receipt.digest() != request.evidence_receipt
                || source.admission.binding() != &request.binding
                || source.owner_capability != *grant
                || source.admission.patch() != request.patch
                || source.admission.operation_digest() != request.operations
                || source.admission.policy() != request.policy
                || source.admission.policy_generation() != request.policy_generation
                || source.admission.plan().id() != request.plan
                || source.admission.resulting_snapshot() != request.verified_snapshot
                || source.admission.resulting_manifest() != request.verified_manifest
                || source.admission.application_report() != request.application_report
                || proof.digest() != request.publication_proof
                || proof.candidate().snapshot().id() != request.publication_snapshot
                || repo.snapshots.get(&receipt.publication_snapshot)
                    != Some(proof.candidate().snapshot())
                || db
                    .source_analysis_records
                    .get(&(scope.clone(), receipt.analysis))
                    != Some(&request.analysis)
                || change.transaction != receipt.transaction
                || change.attempt != receipt.attempt
                || native.attempt().effective.as_ref() != Some(&change.accesses)
                || provenance(native).map_err(|_| invalid.clone())? != change.provenance
            {
                return Err(invalid);
            }
            Ok(())
        }
        _ => Err(invalid),
    }
}
