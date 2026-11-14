//! Shared source-policy installation decision, without nested backend I/O.
use super::*;
use crate::source_execution::{SourceExecutionPolicy, SourcePlanError};

impl Engine {
    pub(super) fn install_source_policy_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        expected_current: Option<Digest>,
        policy: SourceExecutionPolicy,
    ) -> Result<Digest, EngineError> {
        self.authorize(db, session, scope, Permission::ManageModel)?;
        let current = db.source_policies.get(scope);
        if current == Some(&policy) {
            return Ok(policy.digest());
        }
        if current.map(SourceExecutionPolicy::digest) != expected_current {
            return Err(SourceAttemptError::PolicyChanged.into());
        }
        if current.is_some_and(|old| policy.input().generation <= old.input().generation)
            || policy.input().expires_at_ms <= db.now
        {
            return Err(SourceAttemptError::PolicyChanged.into());
        }
        let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
        let analysis = repo
            .source_analysis
            .as_ref()
            .ok_or(SourceAttemptError::SourceRepositoryRequired)?;
        if !analysis.is_complete() {
            return Err(EngineError::IncompleteAnalysis);
        }
        validate_publication_capture(db, scope, &policy, &repo.current)?;
        policy
            .validate_snapshot(&repo.current)
            .map_err(SourceAttemptError::from)?;
        if policy.input().scope != *scope
            || policy.input().analysis_artifact != analysis.digest()
            || policy.input().index_generation != analysis.index_generation()
        {
            return Err(SourceAttemptError::BindingMismatch.into());
        }
        let files = crate::snapshot::effective_workspace(&repo.current)
            .map_err(|_| SourceAttemptError::BindingMismatch)?;
        for (path, rule) in &policy.input().paths {
            let actual = files
                .get(path)
                .map(crate::source_patch::Preimage::of)
                .transpose()
                .map_err(SourceAttemptError::from)?;
            if actual != rule.expected_base {
                return Err(
                    SourceAttemptError::Plan(SourcePlanError::PathPreimage(path.clone())).into(),
                );
            }
            if !rule.required_writes.is_subset(analysis.elements()) {
                return Err(SourceAttemptError::UnknownPolicyElement.into());
            }
        }
        if policy
            .invariants()
            .values()
            .any(|invariant| !invariant.scope.is_subset(analysis.elements()))
        {
            return Err(SourceAttemptError::UnknownPolicyElement.into());
        }
        let digest = policy.digest();
        let generation = repo
            .model
            .generation
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?;
        let next_model = analysis.model(generation);
        db.repositories
            .get_mut(scope)
            .ok_or(EngineError::NotFound)?
            .model = next_model;
        db.source_policy_records
            .insert((scope.clone(), digest), policy.clone());
        db.source_policies.insert(scope.clone(), policy);
        Ok(digest)
    }
}

/// Raw manifest provenance cannot authorize a derived capture. Only an exact
/// source receipt already published through the authoritative journal can.
pub(super) fn validate_publication_capture(
    db: &Database,
    scope: &Scope,
    policy: &SourceExecutionPolicy,
    snapshot: &Snapshot,
) -> Result<(), EngineError> {
    if policy.input().capture.consistency
        != crate::source_execution::SourceCaptureConsistency::EnginePublication
    {
        return Ok(());
    }
    let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
    if !repo.history.iter().any(|change|change.revision==snapshot.revision())
        || !db.mutations.iter().any(|((s,_),record)|s==scope && matches!(&record.outcome,Ok(MutationValue::SourceCommitted(receipt)) if receipt.publication_snapshot==snapshot.id() && receipt.digest()==policy.input().capture.capture_policy))
    {return Err(SourceAttemptError::Plan(SourcePlanError::IncompleteCapture).into())}
    Ok(())
}
