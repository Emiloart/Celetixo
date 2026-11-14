//! Source publication: external evidence reads followed by a fresh atomic gate.
use super::*;
use crate::source_evidence::{SourceArtifactReader, SourceEvidenceError};
use authority::AuthorityFailure;

mod artifacts;
use artifacts::ArtifactPreparation;

impl Engine {
    pub fn commit_source_prepared(
        &self,
        owner: &Session,
        analysis_authority: &Session,
        scope: &Scope,
        request: SourceCommitRequest,
    ) -> Result<SourceCommitReceipt, EngineError> {
        self.commit_source_prepared_with_fault(
            owner,
            analysis_authority,
            scope,
            request,
            CommitFault::None,
        )
    }

    /// Deterministic failure injection. Original outcomes remain immutable and
    /// are recovered under current owner authentication, without new execution.
    pub fn commit_source_prepared_with_fault(
        &self,
        owner: &Session,
        analysis_authority: &Session,
        scope: &Scope,
        request: SourceCommitRequest,
        fault: CommitFault,
    ) -> Result<SourceCommitReceipt, EngineError> {
        let request = MutationRequest::SourceCommit(Box::new(request));
        request.check_recovery_limits()?;
        let actor = session_actor(owner);
        let analyst = session_actor(analysis_authority);
        let MutationRequest::SourceCommit(source_request) = &request else {
            unreachable!()
        };
        let preparation = self
            .store
            .session(owner, scope, Permission::Commit)
            .read(|db| {
                self.authorize(db, owner, scope, Permission::Commit)?;
                if let Some(original) = original_result(db, scope, &actor, source_request)? {
                    return Ok(Err(original));
                }
                self.authorize(db, analysis_authority, scope, Permission::ManageModel)?;
                // A domain rejection is re-evaluated and journaled at the final
                // boundary. No stale pre-read decision is published as authority.
                match validate_source_commit_metadata(db, scope, &actor, &analyst, source_request) {
                    Ok(_) => ArtifactPreparation::new(self, db, scope, source_request).map(Some),
                    Err(_) => Ok(None),
                }
                .map(Ok)
            })?;
        let preparation = match preparation {
            Err(original) => return original,
            Ok(preparation) => preparation,
        };
        if let Some(preparation) = &preparation {
            preparation.validate()?;
        }
        self.store
            .session(owner, scope, Permission::Commit)
            .atomic(|db| {
                if let Err(error) = self.authorize(db, owner, scope, Permission::Commit) {
                    return Ok(Err(error));
                }
                match original_result(db, scope, &actor, source_request) {
                    Ok(Some(original)) => return Ok(original),
                    Err(error) => return Ok(Err(error)),
                    Ok(None) => {}
                }
                if let Err(error) =
                    self.authorize(db, analysis_authority, scope, Permission::ManageModel)
                {
                    return Ok(Err(error));
                }
                if fault == CommitFault::BeforeValidation {
                    return Ok(Err(EngineError::InjectedFailure(fault)));
                }
                let selected =
                    validate_source_commit_metadata(db, scope, &actor, &analyst, source_request);
                if fault == CommitFault::DuringValidation {
                    return Ok(Err(EngineError::InjectedFailure(fault)));
                }
                let artifacts = if selected.is_ok() {
                    // The gate may have become valid while there was no artifact
                    // preparation. Retry the exact key; never publish an unchecked
                    // success or turn this preparation race into a domain outcome.
                    let prepared = preparation.as_ref().ok_or(AuthorityFailure::Contention)?;
                    prepared.recheck(self, db, scope, source_request)?;
                    Some(SourceCommitArtifactRead {
                        evidence: source_request.evidence,
                        registration: source_request.store_registration,
                        provenance: source_request.storage_provenance,
                        observed_at_ms: db.now,
                    })
                } else {
                    None
                };
                let context = observe(db, scope, source_request, analysis_authority, artifacts)?;
                if selected.is_ok() && fault == CommitFault::BeforePublication {
                    return Ok(Err(EngineError::InjectedFailure(fault)));
                }
                // Include the original reply in the same staged publication as
                // head, analysis, history, transaction and reservation release.
                let mut staged = db.clone();
                let response = mutation::apply_once(
                    &mut staged,
                    scope,
                    actor.clone(),
                    source_request.key.clone(),
                    request.clone(),
                    Some(Box::new(context)),
                    |db, _| match selected {
                        Ok(selection) => publish(db, scope, owner, source_request, selection)
                            .map(|receipt| MutationValue::SourceCommitted(Box::new(receipt))),
                        Err(error) => {
                            if matches!(error, EngineError::RevalidationRequired { .. }) {
                                let tx = db
                                    .transactions
                                    .get_mut(&(
                                        scope.clone(),
                                        source_request.binding.transaction.clone(),
                                    ))
                                    .ok_or(EngineError::NotFound)?;
                                tx.transition(TransactionState::RevalidationRequired)?;
                                tx.last_reason = Some(error.to_string());
                            }
                            Err(error)
                        }
                    },
                )?;
                let response = match response {
                    Ok(response) => response,
                    Err(error) => return Ok(Err(error)),
                };
                if response.outcome.is_ok() && fault == CommitFault::DuringPublication {
                    return Ok(Err(EngineError::InjectedFailure(fault)));
                }
                let result = receipt_result(response.outcome)?;
                *db = staged;
                if result.is_ok() && fault == CommitFault::AfterPublication {
                    return Ok(Err(EngineError::ResponseLost));
                }
                Ok(result)
            })?
    }
}

fn session_actor(session: &Session) -> MutationActor {
    MutationActor::Session {
        principal: session.principal.clone(),
        grant: session.token,
    }
}

fn original_result(
    db: &Database,
    scope: &Scope,
    actor: &MutationActor,
    request: &SourceCommitRequest,
) -> Result<Option<Result<SourceCommitReceipt, EngineError>>, EngineError> {
    let Some(record) = db.mutations.get(&(
        scope.clone(),
        MutationKey {
            actor: actor.clone(),
            key: request.key.clone(),
        },
    )) else {
        return Ok(None);
    };
    if record.request != MutationRequest::SourceCommit(Box::new(request.clone())) {
        return Err(EngineError::IdempotencyConflict);
    }
    receipt_result(record.outcome.clone())
        .map(Some)
        .map_err(EngineError::from)
}

fn receipt_result(
    result: Result<MutationValue, EngineError>,
) -> Result<Result<SourceCommitReceipt, EngineError>, AuthorityFailure> {
    match result {
        Ok(MutationValue::SourceCommitted(receipt)) => Ok(Ok(*receipt)),
        Err(error) => Ok(Err(error)),
        _ => Err(AuthorityFailure::Integrity),
    }
}

fn observe(
    db: &Database,
    scope: &Scope,
    request: &SourceCommitRequest,
    analyst: &Session,
    artifacts: Option<SourceCommitArtifactRead>,
) -> Result<SourceCommitContext, AuthorityFailure> {
    let repo = db.repositories.get(scope);
    let source = db.source_attempts.iter().find_map(|((s, _), a)| {
        (*s == *scope && a.admission.id() == request.admission).then_some(a)
    });
    let source = source.map(|a| {
        let storage = a
            .evidence
            .and_then(|id| db.source_evidence_records.get(&(scope.clone(), id)));
        SourceCommitSourceContext {
            admission: a.admission.id(),
            source_attempt: a.admission.binding().source_attempt.clone(),
            state: a.state,
            event_count: a.events.len() as u64,
            evidence: a.evidence,
            executor_grant: a.executor_capability,
            executor_active: a
                .executor_capability
                .and_then(|id| db.source_executors.get(&id))
                .map(|g| g.active),
            store_registration: storage.map(|e| e.store_registration),
            store_active: storage
                .and_then(|e| db.source_artifact_stores.get(&e.store_registration))
                .map(|g| g.active),
        }
    });
    let analysis = repo
        .map(|r| {
            r.source_analysis
                .as_ref()
                .map(RepositoryAnalysis::digest)
                .ok_or(AuthorityFailure::Integrity)
        })
        .transpose()?;
    Ok(SourceCommitContext {
        analysis_principal: analyst.principal.clone(),
        analysis_grant: analyst.token,
        head: repo.map(|r| r.current.id()),
        model_generation: repo.map(|r| r.model.generation),
        analysis,
        policy: db.source_policies.get(scope).map(|policy| policy.digest()),
        source,
        artifacts,
    })
}

fn publish(
    db: &mut Database,
    scope: &Scope,
    owner: &Session,
    request: &SourceCommitRequest,
    selection: gate::SourceCommitSelection,
) -> Result<SourceCommitReceipt, EngineError> {
    let tx_key = (scope.clone(), request.binding.transaction.clone());
    let tx = db.transactions.get(&tx_key).ok_or(EngineError::NotFound)?;
    let change = CommittedChange {
        transaction: tx.id.clone(),
        attempt: tx.attempt().number,
        revision: selection.receipt.revision,
        accesses: tx
            .attempt()
            .effective
            .clone()
            .ok_or(EngineError::IncompleteAnalysis)?,
        provenance: provenance(tx)?,
    };
    let candidate = selection.publication.candidate().snapshot().clone();
    let repo = db
        .repositories
        .get_mut(scope)
        .ok_or(EngineError::NotFound)?;
    repo.snapshots.insert(candidate.id(), candidate.clone());
    repo.current = candidate;
    repo.model = request.analysis.model(selection.receipt.model_generation);
    repo.source_analysis = Some(request.analysis.clone());
    repo.history.push(change);
    db.source_analysis_records.insert(
        (scope.clone(), request.analysis.digest()),
        request.analysis.clone(),
    );
    // Keep the prior snapshot-bound policy and its monotonic generation. New
    // admissions need a freshly authorized policy for the new analysis/head.
    let tx = db
        .transactions
        .get_mut(&tx_key)
        .ok_or(EngineError::NotFound)?;
    tx.transition(TransactionState::Committed)?;
    let lease = tx.lease.as_ref().ok_or(EngineError::InvalidLease)?;
    tx.lease_history.push(lease.event(
        db.now,
        owner.principal.clone(),
        LeaseEventKind::Released {
            reason: "committed".into(),
        },
    ));
    tx.lease = None;
    tx.last_reason = Some("atomic source validation and publication".into());
    Ok(selection.receipt)
}
