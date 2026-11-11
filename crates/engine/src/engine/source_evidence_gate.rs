//! Authenticated source evidence admission. Artifact I/O occurs outside the state
//! mutex, followed by fresh authority validation and one atomic retained outcome.
use super::*;
use crate::source_evidence::{
    SourceArtifactError, SourceArtifactReader, SourceArtifactReference, SourceEvidence,
    SourceEvidenceAssessment, SourceEvidenceError,
};
use source_attempts::{attempt_key, validate_executor_store, validate_source_admission};

#[derive(Clone)]
pub struct SourceArtifactStoreCapability {
    store: Arc<StoreIdentity>,
    token: u64,
    scope: Scope,
}

#[derive(Clone)]
// Registration metadata only; the reader is a process-local runtime resource.
pub(super) struct RegisteredSourceArtifactStore {
    scope: Scope,
    retention: Digest,
    provenance: Digest,
    active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidenceReceipt {
    pub admission: SourceAdmissionId,
    pub evidence: SourceEvidenceId,
    pub assessment: SourceEvidenceAssessment,
    pub recorded_at_ms: u64,
    pub envelope: SourceArtifactReference,
    pub storage_provenance: Digest,
}

#[derive(Clone)]
pub(super) struct RetainedSourceEvidence {
    evidence: SourceEvidence,
    receipt: SourceEvidenceReceipt,
    store_registration: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEvidenceFault {
    None,
    BeforeArtifactRead,
    AfterArtifactRead,
    AfterRetention,
}

impl Administrator {
    /// The trusted embedding installs the reader. An ordinary session cannot
    /// attach its own in-memory adapter to a delivered evidence envelope.
    pub fn register_source_artifact_store(
        &self,
        scope: Scope,
        reader: Arc<dyn SourceArtifactReader>,
    ) -> Result<SourceArtifactStoreCapability, EngineError> {
        let retention = reader.retention_policy();
        let mut db = self.store.lock()?;
        let token = db
            .next_source_artifact_store
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?;
        // Lock order is state then runtime. Acquire both before publishing any
        // registration so a poisoned runtime cannot leave partial state.
        let mut readers = self.store.artifact_readers()?;
        let mut c = Canonical::default();
        c.u64(token);
        c.str(scope.organization.as_str());
        c.str(scope.workspace.as_str());
        c.str(scope.repository.as_str());
        c.digest(retention);
        db.source_artifact_stores.insert(
            token,
            RegisteredSourceArtifactStore {
                scope: scope.clone(),
                retention,
                provenance: c.finish("celetixo/source-artifact-store/v1"),
                active: true,
            },
        );
        readers.insert(token, reader);
        db.next_source_artifact_store = token;
        Ok(SourceArtifactStoreCapability {
            store: self.store.identity.clone(),
            token,
            scope,
        })
    }

    pub fn revoke_source_artifact_store(
        &self,
        store: &SourceArtifactStoreCapability,
    ) -> Result<(), EngineError> {
        if !Arc::ptr_eq(&self.store.identity, &store.store) {
            return Err(EngineError::Unauthorized);
        }
        let reader = {
            let mut db = self.store.lock()?;
            let mut readers = self.store.artifact_readers()?;
            db.source_artifact_stores
                .get_mut(&store.token)
                .ok_or(EngineError::Unauthorized)?
                .active = false;
            readers.remove(&store.token)
        };
        // A reader's destructor can perform I/O or reenter the embedding, too.
        drop(reader);
        Ok(())
    }
}

impl Engine {
    pub fn record_source_evidence(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        storage: &SourceArtifactStoreCapability,
        evidence: SourceEvidence,
        envelope: SourceArtifactReference,
    ) -> Result<SourceEvidenceReceipt, EngineError> {
        self.record_source_evidence_with_fault(
            executor,
            admission,
            storage,
            evidence,
            envelope,
            SourceEvidenceFault::None,
        )
    }

    /// Deterministic embedding/test failure injection; it cannot confer authority.
    pub fn record_source_evidence_with_fault(
        &self,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        storage: &SourceArtifactStoreCapability,
        evidence: SourceEvidence,
        envelope: SourceArtifactReference,
        fault: SourceEvidenceFault,
    ) -> Result<SourceEvidenceReceipt, EngineError> {
        let (registered, reader, expected_provenance, artifact_budget) = {
            let db = self.store.lock()?;
            validate_executor_store(self, executor)?;
            source_attempts::validate_executor_for_admission(&db, executor, admission, true)?;
            let registered = self
                .source_storage(&db, storage, &admission.binding().scope)?
                .clone();
            let record = db
                .source_attempts
                .get(&attempt_key(admission))
                .ok_or(EngineError::NotFound)?;
            if record.admission != *admission {
                return Err(SourceEvidenceError::BindingMismatch.into());
            }
            if let Some(id) = record.evidence {
                return self.existing_source_evidence(
                    &db,
                    &admission.binding().scope,
                    id,
                    storage.token,
                    &evidence,
                    &envelope,
                );
            }
            validate_source_admission(&db, executor, admission, true)?;
            if record.state != SourceAttemptState::Verifying {
                return Err(SourceAttemptError::InvalidState.into());
            }
            if evidence.finished_at_ms() > db.now {
                return Err(SourceEvidenceError::BindingMismatch.into());
            }
            let base = &db.repositories[&admission.binding().scope].current;
            let mut expected = BTreeMap::from([
                ("source-patch".into(), record.patch.canonical_bytes()),
                ("base-snapshot".into(), base.canonical_bytes()),
                (
                    "result-snapshot".into(),
                    record.application.snapshot().canonical_bytes(),
                ),
                ("base-manifest".into(), base.manifest().canonical_bytes()),
                (
                    "result-manifest".into(),
                    record.application.snapshot().manifest().canonical_bytes(),
                ),
                (
                    "application-report".into(),
                    record.application.report().canonical_bytes(),
                ),
                (
                    "verification-plan".into(),
                    admission.plan().canonical_bytes(),
                ),
                ("execution-admission".into(), admission.canonical_bytes()),
                (
                    "execution-policy".into(),
                    db.source_policies[&admission.binding().scope].canonical_bytes(),
                ),
            ]);
            for check in admission.plan().checks() {
                expected.insert(
                    format!("check-definition/{}", check.definition().id()),
                    check.definition().canonical_bytes(),
                );
            }
            let budget = db.source_policies[&admission.binding().scope]
                .input()
                .resource_ceiling
                .artifact_bytes;
            let reader = self.store.artifact_reader(storage.token)?;
            (registered, reader, expected, budget)
        };
        let mut total = envelope.bytes;
        for object in evidence.provenance().values().chain(
            evidence
                .checks()
                .values()
                .flat_map(|check| check.input().artifacts.values()),
        ) {
            total = total
                .checked_add(object.bytes)
                .ok_or(SourceArtifactError::Limit)
                .map_err(SourceEvidenceError::from)?;
        }
        if total > artifact_budget {
            return Err(SourceEvidenceError::Artifact(SourceArtifactError::Limit).into());
        }
        if fault == SourceEvidenceFault::BeforeArtifactRead {
            return Err(
                SourceEvidenceError::Artifact(SourceArtifactError::Persistence(
                    "injected before artifact read".into(),
                ))
                .into(),
            );
        }
        // No authoritative mutex is held across a storage adapter call. A lease,
        // policy, transaction or executor may change here; all are rechecked below.
        if registered.retention != admission.retention_policy() {
            return Err(SourceEvidenceError::BindingMismatch.into());
        }
        let assessment = evidence.validate_persisted(
            admission,
            executor.principal(),
            &envelope,
            reader.as_ref(),
            &expected_provenance,
        )?;
        if fault == SourceEvidenceFault::AfterArtifactRead {
            return Err(
                SourceEvidenceError::Artifact(SourceArtifactError::Persistence(
                    "injected after artifact read".into(),
                ))
                .into(),
            );
        }
        let mut db = self.store.lock()?;
        validate_executor_store(self, executor)?;
        source_attempts::validate_executor_for_admission(&db, executor, admission, true)?;
        let current_store = self.source_storage(&db, storage, &admission.binding().scope)?;
        let current_reader = self.store.artifact_reader(storage.token)?;
        if !Arc::ptr_eq(&current_reader, &reader)
            || current_store.retention != registered.retention
            || current_store.provenance != registered.provenance
        {
            return Err(EngineError::Unauthorized);
        }
        let key = attempt_key(admission);
        let record = db.source_attempts.get(&key).ok_or(EngineError::NotFound)?;
        if let Some(id) = record.evidence {
            return self.existing_source_evidence(
                &db,
                &admission.binding().scope,
                id,
                storage.token,
                &evidence,
                &envelope,
            );
        }
        validate_source_admission(&db, executor, admission, true)?;
        if record.state != SourceAttemptState::Verifying || evidence.finished_at_ms() > db.now {
            return Err(SourceAttemptError::InvalidState.into());
        }
        let now = db.now;
        let next = match assessment {
            SourceEvidenceAssessment::Passed => SourceAttemptState::EvidenceSealed,
            SourceEvidenceAssessment::Failed { .. } => SourceAttemptState::ExecutionFailed,
            SourceEvidenceAssessment::Incomplete { .. } => SourceAttemptState::EvidenceIncomplete,
        };
        // Validate transitions on clones, then publish the transaction, source
        // attempt and evidence together. A failed transition cannot leave a row.
        let mut source_record = record.clone();
        source_record.transition(
            now,
            next,
            "Engine independently checked persisted execution evidence",
        )?;
        source_record.evidence = Some(evidence.id());
        let transaction_key = (
            admission.binding().scope.clone(),
            admission.binding().transaction.clone(),
        );
        let mut transaction = db
            .transactions
            .get(&transaction_key)
            .ok_or(EngineError::NotFound)?
            .clone();
        if assessment == SourceEvidenceAssessment::Passed {
            transaction.transition(TransactionState::Validating)?;
        }
        let receipt = SourceEvidenceReceipt {
            admission: admission.id(),
            evidence: evidence.id(),
            assessment,
            recorded_at_ms: now,
            envelope,
            storage_provenance: registered.provenance,
        };
        db.source_evidence_records.insert(
            (admission.binding().scope.clone(), evidence.id()),
            RetainedSourceEvidence {
                evidence,
                receipt: receipt.clone(),
                store_registration: storage.token,
            },
        );
        db.source_attempts.insert(key, source_record);
        db.transactions.insert(transaction_key, transaction);
        if fault == SourceEvidenceFault::AfterRetention {
            return Err(EngineError::ResponseLost);
        }
        Ok(receipt)
    }

    pub fn source_evidence_record(
        &self,
        session: &Session,
        scope: &Scope,
        id: SourceEvidenceId,
    ) -> Result<(SourceEvidence, SourceEvidenceReceipt), EngineError> {
        let db = self.store.lock()?;
        self.authorize(&db, session, scope, Permission::Read)?;
        let record = db
            .source_evidence_records
            .get(&(scope.clone(), id))
            .ok_or(EngineError::NotFound)?;
        Ok((record.evidence.clone(), record.receipt.clone()))
    }

    fn existing_source_evidence(
        &self,
        db: &Database,
        scope: &Scope,
        id: SourceEvidenceId,
        store_registration: u64,
        evidence: &SourceEvidence,
        envelope: &SourceArtifactReference,
    ) -> Result<SourceEvidenceReceipt, EngineError> {
        let record = db
            .source_evidence_records
            .get(&(scope.clone(), id))
            .ok_or(EngineError::NotFound)?;
        if record.evidence != *evidence
            || record.receipt.envelope != *envelope
            || record.store_registration != store_registration
        {
            return Err(EngineError::IdempotencyConflict);
        }
        Ok(record.receipt.clone())
    }

    fn source_storage<'a>(
        &self,
        db: &'a Database,
        storage: &SourceArtifactStoreCapability,
        scope: &Scope,
    ) -> Result<&'a RegisteredSourceArtifactStore, EngineError> {
        if !Arc::ptr_eq(&self.store.identity, &storage.store) || storage.scope != *scope {
            return Err(EngineError::Unauthorized);
        }
        let registered = db
            .source_artifact_stores
            .get(&storage.token)
            .ok_or(EngineError::Unauthorized)?;
        if !registered.active || registered.scope != *scope {
            return Err(EngineError::Unauthorized);
        }
        Ok(registered)
    }
}
