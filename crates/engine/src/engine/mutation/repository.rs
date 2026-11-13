//! Repository requests have no synthetic transaction or attempt context.
use super::*;
use crate::persistence::{RecoverySize, RestoreError};
use crate::source_execution::SourceExecutionPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryMutation {
    InstallPolicy {
        expected: Option<Digest>,
        policy: Box<SourceExecutionPolicy>,
    },
    RegisterRepository {
        snapshot: Box<Snapshot>,
        analysis: Box<RepositoryAnalysis>,
    },
    InstallAnalysis {
        expected: Digest,
        analysis: Box<RepositoryAnalysis>,
    },
    Register {
        snapshot: Box<Snapshot>,
        model: Box<EngineeringModel>,
    },
    UpdateModel {
        model: Box<EngineeringModel>,
    },
}
impl RepositoryMutation {
    pub(super) fn permission(&self) -> Permission {
        match self {
            Self::Register { .. } | Self::RegisterRepository { .. } => Permission::Register,
            Self::UpdateModel { .. }
            | Self::InstallAnalysis { .. }
            | Self::InstallPolicy { .. } => Permission::ManageModel,
        }
    }
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::InstallPolicy { .. } => "InstallPolicy",
            Self::RegisterRepository { .. } => "RegisterRepository",
            Self::InstallAnalysis { .. } => "InstallAnalysis",
            Self::Register { .. } => "Register",
            Self::UpdateModel { .. } => "UpdateModel",
        }
    }
    fn model(&self) -> Option<&EngineeringModel> {
        match self {
            Self::Register { model, .. } | Self::UpdateModel { model } => Some(model),
            _ => None,
        }
    }
    pub(super) fn encode(&self, c: &mut Canonical) {
        c.str(self.kind());
        if let Self::InstallPolicy { expected, policy } = self {
            match expected {
                None => c.u64(0),
                Some(id) => {
                    c.u64(1);
                    c.digest(*id);
                }
            }
            crate::persistence::encode_original_policy(c, policy);
        }
        if let Self::Register { snapshot, .. } | Self::RegisterRepository { snapshot, .. } = self {
            c.digest(snapshot.id().0);
            c.bytes(&snapshot.canonical_bytes());
        }
        if let Some(model) = self.model() {
            crate::persistence::encode_model_fields(c, model);
        }
        if let Self::InstallAnalysis { expected, .. } = self {
            c.digest(*expected);
        }
        if let Self::RegisterRepository { analysis, .. } | Self::InstallAnalysis { analysis, .. } =
            self
        {
            c.digest(analysis.digest());
            c.bytes(&analysis.canonical_bytes());
        }
    }
    pub(super) fn apply(
        &self,
        engine: &Engine,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
    ) -> Result<MutationValue, EngineError> {
        match self {
            Self::InstallPolicy { expected, policy } => {
                let policy = engine.install_source_policy_in(
                    db,
                    session,
                    scope,
                    *expected,
                    *policy.clone(),
                )?;
                return Ok(MutationValue::Policy {
                    policy,
                    model_generation: db
                        .repositories
                        .get(scope)
                        .ok_or(EngineError::NotFound)?
                        .model
                        .generation,
                });
            }
            Self::RegisterRepository { snapshot, analysis } => {
                engine.register_repository_in(db, session, *snapshot.clone(), *analysis.clone())?
            }
            Self::InstallAnalysis { expected, analysis } => engine.install_repository_analysis_in(
                db,
                session,
                scope,
                *expected,
                *analysis.clone(),
            )?,
            Self::Register { snapshot, model } => {
                if snapshot.scope() != scope {
                    return Err(EngineError::Unauthorized);
                }
                engine.register_in(db, session, *snapshot.clone(), *model.clone())?;
            }
            Self::UpdateModel { model } => {
                engine.update_model_in(db, session, scope, *model.clone())?
            }
        }
        let repo = db.repositories.get(scope).ok_or(EngineError::NotFound)?;
        Ok(MutationValue::Repository {
            snapshot: repo.current.id(),
            model: Box::new(repo.model.clone()),
        })
    }
    pub(super) fn domain_error(&self, error: &EngineError) -> bool {
        match self {
            Self::InstallPolicy { .. } => {
                matches!(
                    error,
                    EngineError::NotFound
                        | EngineError::IncompleteAnalysis
                        | EngineError::CounterExhausted
                ) || matches!(
                    error,
                    EngineError::SourceAttempt(
                        SourceAttemptError::PolicyChanged
                            | SourceAttemptError::SourceRepositoryRequired
                            | SourceAttemptError::BindingMismatch
                            | SourceAttemptError::UnknownPolicyElement
                            | SourceAttemptError::Plan(_)
                            | SourceAttemptError::Patch(_)
                    )
                )
            }
            Self::RegisterRepository { .. } => {
                matches!(
                    error,
                    EngineError::AlreadyExists | EngineError::RepositoryAnalysis(_)
                ) || matches!(error, EngineError::InvalidIntent(reason) if reason == "initial revision must be zero")
            }
            Self::InstallAnalysis { .. } => matches!(
                error,
                EngineError::NotFound
                    | EngineError::SourceAnalysisRequired
                    | EngineError::RepositoryAnalysis(_)
                    | EngineError::CounterExhausted
            ),
            Self::Register { .. } => {
                matches!(
                    error,
                    EngineError::AlreadyExists
                        | EngineError::SourceAnalysisRequired
                        | EngineError::IncompleteAnalysis
                ) || matches!(error, EngineError::InvalidIntent(reason) if reason == "initial revision must be zero")
            }
            Self::UpdateModel { .. } => matches!(
                error,
                EngineError::NotFound
                    | EngineError::ModelChanged
                    | EngineError::SourceAnalysisRequired
                    | EngineError::IncompleteAnalysis
            ),
        }
    }
    pub(super) fn check_record(&self, record: &MutationRecord) -> Result<(), RestoreError> {
        if record.resolved_attempt.is_some()
            || record.state_before.is_some()
            || record.lease_events_before != 0
        {
            return Err(RestoreError::BindingMismatch);
        }
        let determined = if matches!(self, Self::Register {snapshot, ..} if captured_workspace(snapshot))
        {
            Some(EngineError::SourceAnalysisRequired)
        } else if self.model().is_some_and(|model| model.validate().is_err()) {
            Some(EngineError::IncompleteAnalysis)
        } else if matches!(self, Self::Register {snapshot, ..} if snapshot.revision() != Revision(0))
        {
            Some(EngineError::InvalidIntent(
                "initial revision must be zero".into(),
            ))
        } else {
            None
        };
        if let Some(error) = determined {
            return if record.outcome == Err(error) {
                Ok(())
            } else {
                Err(RestoreError::BindingMismatch)
            };
        }
        if let Self::RegisterRepository { snapshot, analysis } = self {
            if let Err(error) = analysis.validate(snapshot, analysis.index_generation()) {
                return if record.outcome == Err(EngineError::RepositoryAnalysis(error)) {
                    Ok(())
                } else {
                    Err(RestoreError::BindingMismatch)
                };
            }
            if snapshot.revision() != Revision(0) {
                return if record.outcome
                    == Err(EngineError::InvalidIntent(
                        "initial revision must be zero".into(),
                    )) {
                    Ok(())
                } else {
                    Err(RestoreError::BindingMismatch)
                };
            }
        }
        match &record.outcome {
            Ok(MutationValue::Policy {
                policy: result,
                model_generation,
            }) => {
                if !matches!(self, Self::InstallPolicy {policy, ..} if policy.digest() == *result && *model_generation > 0)
                {
                    return Err(RestoreError::BindingMismatch);
                }
                Ok(())
            }
            Ok(MutationValue::Repository { snapshot, model }) => {
                model
                    .validate()
                    .map_err(|_| RestoreError::InvalidDomainValue)?;
                if matches!(self, Self::InstallPolicy { .. })
                    || self.model().is_some_and(|input| model.as_ref() != input)
                    || matches!(self, Self::Register {snapshot: input, ..} if input.id() != *snapshot || input.revision() != Revision(0) || captured_workspace(input))
                    || matches!(self, Self::RegisterRepository {snapshot: input, analysis} if input.id() != *snapshot || input.revision() != Revision(0) || model.generation != 1 || analysis.validate(input, analysis.index_generation()).is_err())
                    || matches!(self, Self::RegisterRepository {analysis, ..} | Self::InstallAnalysis {analysis, ..} if analysis.model(model.generation) != **model || analysis.snapshot() != *snapshot)
                {
                    return Err(RestoreError::BindingMismatch);
                }
                Ok(())
            }
            Err(error) if self.domain_error(error) => Ok(()),
            _ => Err(RestoreError::BindingMismatch),
        }
    }
    pub(super) fn validate(
        &self,
        db: &Database,
        scope: &Scope,
        record: &MutationRecord,
    ) -> Result<(), RestoreError> {
        self.check_record(record)?;
        let invalid = RestoreError::BindingMismatch;
        if let (
            Self::InstallPolicy { policy, .. },
            Ok(MutationValue::Policy {
                model_generation, ..
            }),
        ) = (self, &record.outcome)
        {
            let repo = db.repositories.get(scope).ok_or(invalid.clone())?;
            let analysis = db
                .source_analysis_records
                .get(&(scope.clone(), policy.input().analysis_artifact))
                .ok_or(invalid.clone())?;
            let snapshot = repo
                .snapshots
                .get(&policy.input().snapshot)
                .ok_or(invalid.clone())?;
            if db
                .source_policy_records
                .get(&(scope.clone(), policy.digest()))
                != Some(policy.as_ref())
                || *model_generation > repo.model.generation
                || (*model_generation == repo.model.generation
                    && repo.model != analysis.model(*model_generation))
                || analysis.scope() != scope
                || analysis.index_generation() != policy.input().index_generation
                || policy.validate_snapshot(snapshot).is_err()
            {
                return Err(invalid);
            }
            for ((stored, _), tx) in &db.transactions {
                if stored == scope
                    && tx.attempts.iter().any(|attempt| {
                        attempt.model.generation == *model_generation
                            && attempt.model != analysis.model(*model_generation)
                    })
                {
                    return Err(invalid);
                }
            }
            for ((stored, _), previous) in &db.mutations {
                if stored == scope {
                    match &previous.outcome {
                        Ok(MutationValue::Policy {
                            policy: previous_id,
                            model_generation: generation,
                        }) if generation == model_generation && *previous_id != policy.digest() => {
                            return Err(invalid);
                        }
                        Ok(MutationValue::Repository { model, .. })
                            if model.generation == *model_generation
                                && **model != analysis.model(*model_generation) =>
                        {
                            return Err(invalid);
                        }
                        _ => {}
                    }
                }
            }
        }
        if let Self::Register { snapshot, .. } | Self::RegisterRepository { snapshot, .. } = self
            && snapshot.scope() != scope
        {
            return Err(invalid);
        }
        if let Ok(MutationValue::Repository { snapshot, model }) = &record.outcome {
            let repo = db.repositories.get(scope).ok_or(invalid.clone())?;
            let original = repo.snapshots.get(snapshot).ok_or(invalid.clone())?;
            if (self.model().is_some() && repo.source_analysis.is_some())
                || repo.model.generation < model.generation
                || (repo.model.generation == model.generation && repo.model != **model)
                || matches!(self, Self::Register {snapshot: input, ..} if input.as_ref() != original)
            {
                return Err(invalid);
            }
            for ((stored, _), tx) in &db.transactions {
                if stored == scope
                    && tx.attempts.iter().any(|attempt| {
                        attempt.model.generation == model.generation && attempt.model != **model
                    })
                {
                    return Err(invalid);
                }
            }
            for ((stored, _), prior) in &db.mutations {
                if stored == scope
                    && let Ok(MutationValue::Repository {
                        model: previous, ..
                    }) = &prior.outcome
                    && previous.generation == model.generation
                    && previous != model
                {
                    return Err(invalid);
                }
            }
            if let Self::RegisterRepository { analysis, .. }
            | Self::InstallAnalysis { analysis, .. } = self
                && db
                    .source_analysis_records
                    .get(&(scope.clone(), analysis.digest()))
                    != Some(analysis.as_ref())
            {
                return Err(invalid);
            }
        }
        Ok(())
    }
    pub(super) fn recovery_size(&self) -> Result<RecoverySize, RestoreError> {
        if let Self::InstallPolicy { policy, .. } = self {
            let cost = crate::persistence::source_policy_cost(policy)?;
            return Ok(RecoverySize {
                entries: cost.entries,
                bytes: cost.bytes.checked_add(4096).ok_or(RestoreError::Limit)?,
            });
        }
        let mut size = if let Some(model) = self.model() {
            model_size(model)?
        } else {
            RecoverySize {
                entries: 0,
                bytes: 1024,
            }
        };
        if let Self::Register { snapshot, .. } | Self::RegisterRepository { snapshot, .. } = self {
            merge(&mut size, snapshot_size(snapshot)?)?;
        }
        if let Self::RegisterRepository { analysis, .. } | Self::InstallAnalysis { analysis, .. } =
            self
        {
            merge(&mut size, analysis_size(analysis)?)?;
        }
        Ok(size)
    }
}
fn analysis_size(analysis: &RepositoryAnalysis) -> Result<RecoverySize, RestoreError> {
    let mut cost = RecoverySize {
        entries: 0,
        bytes: 1024,
    };
    for text in [
        analysis.scope().organization.as_str(),
        analysis.scope().workspace.as_str(),
        analysis.scope().repository.as_str(),
        analysis.analyzer_version(),
    ] {
        field(&mut cost, text.len())?;
    }
    for element in analysis.elements() {
        field(&mut cost, element.as_str().len())?;
    }
    for (consumer, dependencies) in analysis.dependencies() {
        field(&mut cost, consumer.as_str().len())?;
        for dependency in dependencies {
            field(&mut cost, dependency.as_str().len())?;
        }
    }
    for (element, checks) in analysis.checks() {
        field(&mut cost, element.as_str().len())?;
        for check in checks {
            field(&mut cost, check.as_str().len())?;
        }
    }
    for gap in analysis.gaps() {
        field(&mut cost, gap.stage.len())?;
        if let Some(path) = &gap.path {
            field(&mut cost, path.len())?;
        }
        field(&mut cost, gap.detail.len())?;
    }
    Ok(cost)
}
fn merge(cost: &mut RecoverySize, extra: RecoverySize) -> Result<(), RestoreError> {
    cost.entries = cost
        .entries
        .checked_add(extra.entries)
        .ok_or(RestoreError::Limit)?;
    cost.bytes = cost
        .bytes
        .checked_add(extra.bytes)
        .ok_or(RestoreError::Limit)?;
    Ok(())
}
fn field(cost: &mut RecoverySize, bytes: usize) -> Result<(), RestoreError> {
    merge(
        cost,
        RecoverySize {
            entries: 1,
            bytes: bytes.checked_add(64).ok_or(RestoreError::Limit)?,
        },
    )
}
pub(super) fn model_size(model: &EngineeringModel) -> Result<RecoverySize, RestoreError> {
    let mut cost = RecoverySize {
        entries: 0,
        bytes: 1024,
    };
    field(&mut cost, model.analyzer.len())?;
    field(&mut cost, model.version.len())?;
    for (consumer, dependencies) in &model.dependencies {
        field(&mut cost, consumer.as_str().len())?;
        for dependency in dependencies {
            field(&mut cost, dependency.as_str().len())?;
        }
    }
    if let crate::access::Completeness::Incomplete { reasons } = &model.completeness {
        for reason in reasons {
            field(&mut cost, reason.len())?;
        }
    }
    Ok(cost)
}
fn snapshot_size(snapshot: &Snapshot) -> Result<RecoverySize, RestoreError> {
    let mut cost = RecoverySize {
        entries: 0,
        bytes: 4096,
    };
    let m = snapshot.manifest();
    for text in [
        snapshot.scope().organization.as_str(),
        snapshot.scope().workspace.as_str(),
        snapshot.scope().repository.as_str(),
        &m.base_commit,
        &m.provenance,
    ] {
        field(&mut cost, text.len())?;
    }
    field(
        &mut cost,
        m.inclusion_policy
            .captured_layers
            .len()
            .checked_mul(32)
            .ok_or(RestoreError::Limit)?,
    )?;
    for (path, reason) in &m.inclusion_policy.excluded_paths {
        field(&mut cost, path.len())?;
        field(&mut cost, reason.len())?;
    }
    for input in &m.inputs {
        field(&mut cost, input.path.len())?;
        field(&mut cost, input.bytes.as_ref().map_or(0, Vec::len))?;
    }
    for (key, value) in &m.environment {
        field(&mut cost, key.len())?;
        field(&mut cost, value.len())?;
    }
    for element in snapshot.state().keys() {
        field(&mut cost, element.as_str().len())?;
    }
    Ok(cost)
}
