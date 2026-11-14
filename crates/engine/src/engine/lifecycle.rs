//! Lifecycle transitions evaluated inside one scoped Engine callback. These pure
//! helpers retain the entry points' guards and perform no nested backend I/O.
use super::*;

impl Engine {
    pub(super) fn register_in(
        &self,
        db: &mut Database,
        session: &Session,
        snapshot: Snapshot,
        model: EngineeringModel,
    ) -> Result<(), EngineError> {
        self.authorize(db, session, snapshot.scope(), Permission::Register)?;
        // The integer reference harness has no source inputs. Captured workspaces
        // must enter through the artifact-bound repository registration path.
        if captured_workspace(&snapshot) {
            return Err(EngineError::SourceAnalysisRequired);
        }
        validate_model(&model)?;
        if snapshot.revision() != Revision(0) {
            return Err(EngineError::InvalidIntent(
                "initial revision must be zero".into(),
            ));
        }
        if db.repositories.contains_key(snapshot.scope()) {
            return Err(EngineError::AlreadyExists);
        }
        db.repositories.insert(
            snapshot.scope().clone(),
            Repository {
                snapshots: BTreeMap::from([(snapshot.id(), snapshot.clone())]),
                current: snapshot,
                model,
                history: vec![],
                source_analysis: None,
            },
        );
        Ok(())
    }
    pub(super) fn register_repository_in(
        &self,
        db: &mut Database,
        session: &Session,
        snapshot: Snapshot,
        analysis: RepositoryAnalysis,
    ) -> Result<(), EngineError> {
        self.authorize(db, session, snapshot.scope(), Permission::Register)?;
        self.authorize(db, session, snapshot.scope(), Permission::ManageModel)?;
        analysis.validate(&snapshot, analysis.index_generation())?;
        if snapshot.revision() != Revision(0) {
            return Err(EngineError::InvalidIntent(
                "initial revision must be zero".into(),
            ));
        }
        if db.repositories.contains_key(snapshot.scope()) {
            return Err(EngineError::AlreadyExists);
        }
        db.source_analysis_records.insert(
            (snapshot.scope().clone(), analysis.digest()),
            analysis.clone(),
        );
        db.repositories.insert(
            snapshot.scope().clone(),
            Repository {
                snapshots: BTreeMap::from([(snapshot.id(), snapshot.clone())]),
                current: snapshot,
                model: analysis.model(1),
                history: vec![],
                source_analysis: Some(analysis),
            },
        );
        Ok(())
    }
    pub(super) fn install_repository_analysis_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        expected_artifact: Digest,
        analysis: RepositoryAnalysis,
    ) -> Result<(), EngineError> {
        self.authorize(db, session, scope, Permission::ManageModel)?;
        let repo = db
            .repositories
            .get_mut(scope)
            .ok_or(EngineError::NotFound)?;
        if repo.source_analysis.is_none() {
            return Err(EngineError::SourceAnalysisRequired);
        }
        analysis.validate(&repo.current, analysis.index_generation())?;
        if repo.source_analysis.as_ref() == Some(&analysis) {
            return Ok(());
        }
        if repo
            .source_analysis
            .as_ref()
            .is_none_or(|current| current.digest() != expected_artifact)
        {
            return Err(RepositoryAnalysisError::StaleGeneration.into());
        }
        let generation = repo
            .model
            .generation
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?;
        repo.model = analysis.model(generation);
        db.source_analysis_records
            .insert((scope.clone(), analysis.digest()), analysis.clone());
        repo.source_analysis = Some(analysis);
        Ok(())
    }
    pub(super) fn update_model_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        model: EngineeringModel,
    ) -> Result<(), EngineError> {
        self.authorize(db, session, scope, Permission::ManageModel)?;
        validate_model(&model)?;
        let repository = db
            .repositories
            .get_mut(scope)
            .ok_or(EngineError::NotFound)?;
        if repository.source_analysis.is_some() {
            return Err(EngineError::SourceAnalysisRequired);
        }
        if model.generation <= repository.model.generation {
            return Err(EngineError::ModelChanged);
        }
        repository.model = model;
        Ok(())
    }
    pub(super) fn propose_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: TransactionId,
        base: SnapshotId,
        intent: TransactionIntent,
    ) -> Result<TransactionView, EngineError> {
        self.authorize(db, session, scope, Permission::Propose)?;
        validate_intent(&intent)?;
        let key = (scope.clone(), id.clone());
        if let Some(existing) = db.transactions.get(&key) {
            if existing.intent == intent
                && existing.attempts[0].base.id() == base
                && existing.owner == session.principal
            {
                return Ok(existing.view());
            }
            return Err(EngineError::AlreadyExists);
        }
        let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
        let snapshot = repo
            .snapshots
            .get(&base)
            .ok_or(EngineError::NotFound)?
            .clone();
        for dependency in &intent.depends_on {
            if dependency == &id
                || !db
                    .transactions
                    .get(&(scope.clone(), dependency.clone()))
                    .is_some_and(|tx| tx.state == TransactionState::Committed)
            {
                return Err(EngineError::UnsatisfiedDependency);
            }
        }
        let tx = Transaction {
            id,
            owner: session.principal.clone(),
            owner_capability: session.token,
            scope: scope.clone(),
            intent,
            state: TransactionState::Proposed,
            attempts: vec![Attempt {
                number: 0,
                base: snapshot,
                model: repo.model.clone(),
                effective: None,
                analysis: None,
                source_analysis: repo
                    .source_analysis
                    .as_ref()
                    .map(RepositoryAnalysis::digest),
                source_policy: None,
                patch: None,
                candidate: None,
                evidence: BTreeMap::new(),
            }],
            lease: None,
            lease_history: vec![],
            last_reason: None,
        };
        let view = tx.view();
        db.transactions.insert(key, tx);
        Ok(view)
    }
    pub(super) fn analyze_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<AccessSet, EngineError> {
        self.authorize(db, session, scope, Permission::Analyze)?;
        let key = (scope.clone(), id.clone());
        let mut tx = db
            .transactions
            .get(&key)
            .ok_or(EngineError::NotFound)?
            .clone();
        // The compound source revalidation path may pass through Analyzed only
        // after constructing a fresh epoch. Generic Analyze cannot reopen an
        // existing frame or retain its old patch, evidence or reservation.
        if tx.state != TransactionState::Proposed {
            return Err(EngineError::IllegalTransition {
                from: tx.state,
                to: TransactionState::Analyzed,
            });
        }
        tx.transition(TransactionState::Analyzed)?;
        let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
        if tx.attempt().model != repo.model {
            return Err(EngineError::ModelChanged);
        }
        analyze_attempt(
            &mut tx,
            repo.source_analysis.as_ref(),
            db.source_policies.get(scope),
        )?;
        let accesses = tx
            .attempt()
            .effective
            .clone()
            .ok_or(EngineError::IncompleteAnalysis)?;
        db.transactions.insert(key, tx);
        Ok(accesses)
    }
    pub(super) fn execute_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<(), EngineError> {
        self.authorize(db, session, scope, Permission::Execute)?;
        let key = (scope.clone(), id.clone());
        let tx = db.transactions.get(&key).ok_or(EngineError::NotFound)?;
        require_lease(db, tx, session)?;
        db.transactions
            .get_mut(&key)
            .ok_or(EngineError::NotFound)?
            .transition(TransactionState::Executing)
    }
    pub(super) fn submit_patch_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        patch: Patch,
    ) -> Result<EvidenceBindings, EngineError> {
        self.authorize(db, session, scope, Permission::Execute)?;
        if db
            .repositories
            .get(scope)
            .ok_or(EngineError::NotFound)?
            .source_analysis
            .is_some()
        {
            return Err(EngineError::SourceMutationUnsupported);
        }
        let key = (scope.clone(), id.clone());
        let mut tx = db
            .transactions
            .get(&key)
            .ok_or(EngineError::NotFound)?
            .clone();
        require_lease(db, &tx, session)?;
        // An existing validating/revalidation attempt is immutable. The
        // revalidation path appends a fresh attempt; patch submission cannot
        // rewrite the patch or evidence bindings of the original one.
        if tx.state != TransactionState::Executing {
            return Err(EngineError::IllegalTransition {
                from: tx.state,
                to: TransactionState::Validating,
            });
        }
        tx.transition(TransactionState::Validating)?;
        if patch.base_snapshot() != tx.attempt().base.id() {
            return Err(EngineError::WrongPatchBase);
        }
        let writes = &tx
            .attempt()
            .effective
            .as_ref()
            .ok_or(EngineError::IncompleteAnalysis)?
            .writes;
        if patch
            .changes()
            .keys()
            .any(|element| !writes.contains(element))
        {
            return Err(EngineError::UndeclaredWrite);
        }
        let state = patch.apply(tx.attempt().base.state())?;
        for invariant in &tx.intent.invariants {
            invariant.evaluate(&state)?;
        }
        let revision = next_revision(tx.attempt().base.revision())?;
        let candidate = tx.attempt().base.successor(revision, state)?;
        tx.attempt_mut().patch = Some(patch);
        tx.attempt_mut().candidate = Some(candidate);
        let bindings = tx.bindings()?;
        db.transactions.insert(key, tx);
        Ok(bindings)
    }
    pub(super) fn record_evidence_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        evidence: VerificationEvidence,
    ) -> Result<EvidenceId, EngineError> {
        self.authorize(db, session, scope, Permission::Verify)?;
        let key = (scope.clone(), id.clone());
        let tx = db.transactions.get(&key).ok_or(EngineError::NotFound)?;
        if tx.state != TransactionState::Validating {
            return Err(EngineError::IllegalTransition {
                from: tx.state,
                to: TransactionState::Validating,
            });
        }
        if evidence.provenance().verifier != session.principal
            || evidence.bindings() != &tx.bindings()?
        {
            return Err(EngineError::EvidenceMismatch);
        }
        let required = tx
            .intent
            .required_checks
            .iter()
            .find(|check| check.id() == evidence.check().id())
            .ok_or(EngineError::EvidenceMismatch)?;
        if !evidence.matches(&tx.bindings()?, required) {
            return Err(EngineError::EvidenceMismatch);
        }
        if evidence.provenance().finished_at > db.now {
            return Err(EngineError::EvidenceMismatch);
        }
        let evidence_id = evidence.id();
        let check_id = evidence.check().id().clone();
        db.evidence.insert((scope.clone(), evidence_id), evidence);
        db.evidence_sources
            .insert((scope.clone(), evidence_id), session.token);
        db.transactions
            .get_mut(&key)
            .ok_or(EngineError::NotFound)?
            .attempt_mut()
            .evidence
            .insert(check_id, evidence_id);
        Ok(evidence_id)
    }
    pub(super) fn abort_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        reason: String,
    ) -> Result<(), EngineError> {
        self.authorize(db, session, scope, Permission::Cancel)?;
        if reason.trim().is_empty() {
            return Err(EngineError::InvalidIntent("abort needs reason".into()));
        }
        let now = db.now;
        let tx = db
            .transactions
            .get_mut(&(scope.clone(), id.clone()))
            .ok_or(EngineError::NotFound)?;
        if tx.state == TransactionState::Aborted && tx.last_reason.as_ref() == Some(&reason) {
            return Ok(());
        }
        tx.transition(TransactionState::Aborted)?;
        if let Some(lease) = &tx.lease {
            tx.lease_history.push(lease.event(
                now,
                tx.owner.clone(),
                LeaseEventKind::Released {
                    reason: reason.clone(),
                },
            ));
        }
        tx.lease = None;
        tx.last_reason = Some(reason);
        source_attempts::cancel_transaction_attempts(db, scope, id)?;
        Ok(())
    }
    pub(super) fn revalidate_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
    ) -> Result<EvidenceBindings, EngineError> {
        self.authorize(db, session, scope, Permission::Analyze)?;
        if db
            .repositories
            .get(scope)
            .ok_or(EngineError::NotFound)?
            .source_analysis
            .is_some()
        {
            return Err(EngineError::SourceMutationUnsupported);
        }
        let key = (scope.clone(), id.clone());
        let mut tx = db
            .transactions
            .get(&key)
            .ok_or(EngineError::NotFound)?
            .clone();
        require_lease(db, &tx, session)?;
        tx.transition(TransactionState::Validating)?;
        let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
        check_intervening(&tx, repo)?;
        if tx.attempt().model != repo.model {
            return Err(EngineError::ModelChanged);
        }
        let old = tx.attempt();
        let old_patch = old.patch.as_ref().ok_or(EngineError::WrongPatchBase)?;
        let patch = Patch::new(repo.current.id(), old_patch.changes().clone())?;
        let candidate = repo.current.successor(
            next_revision(repo.current.revision())?,
            patch.apply(repo.current.state())?,
        )?;
        for invariant in &tx.intent.invariants {
            invariant.evaluate(candidate.state())?;
        }
        let number = old
            .number
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?;
        tx.attempts.push(Attempt {
            number,
            base: repo.current.clone(),
            model: repo.model.clone(),
            effective: None,
            analysis: None,
            source_analysis: repo
                .source_analysis
                .as_ref()
                .map(RepositoryAnalysis::digest),
            source_policy: None,
            patch: Some(patch),
            candidate: Some(candidate),
            evidence: BTreeMap::new(),
        });
        analyze_attempt(
            &mut tx,
            repo.source_analysis.as_ref(),
            db.source_policies.get(scope),
        )?;
        tx.last_reason = Some("compatible revision advance; fresh evidence required".into());
        let bindings = tx.bindings()?;
        db.transactions.insert(key, tx);
        Ok(bindings)
    }
}
