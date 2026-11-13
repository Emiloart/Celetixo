//! Original trusted administrative decisions. Allocation IDs are historical
//! metadata; fresh authentication/resolution is required for runtime capability.
use super::*;
use crate::persistence::{RecoverySize, RestoreError};
use crate::source_execution::{BackendIdentity, ExecutionTier, ResourceLimits};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdministrativeResource {
    Session,
    Executor,
    Cleanup,
    ArtifactStore,
}
impl AdministrativeResource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Session => "Session",
            Self::Executor => "Executor",
            Self::Cleanup => "Cleanup",
            Self::ArtifactStore => "ArtifactStore",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdministrativeMutation {
    GrantSession {
        principal: PrincipalId,
        permissions: BTreeSet<Permission>,
    },
    RevokeSession {
        grant: u64,
        principal: PrincipalId,
    },
    GrantExecutor {
        principal: PrincipalId,
        backend: BackendIdentity,
        tiers: BTreeSet<ExecutionTier>,
        resources: ResourceLimits,
        expires_at_ms: u64,
    },
    RevokeExecutor {
        grant: u64,
        principal: PrincipalId,
        backend: BackendIdentity,
    },
    GrantCleanup {
        principal: PrincipalId,
        backend: Digest,
        expires_at_ms: u64,
    },
    RevokeCleanup {
        grant: u64,
        principal: PrincipalId,
        backend: Digest,
    },
    RegisterArtifactStore {
        adapter: SourceArtifactAdapter,
        retention: Digest,
    },
    RevokeArtifactStore {
        registration: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdministrativeContext {
    /// Original allocator counter, not today's counter or a new allocation.
    pub counter_before: u64,
    /// None means the raw revoke selector did not resolve in this scope.
    pub target_active_before: Option<bool>,
}
impl AdministrativeMutation {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::GrantSession { .. } => "GrantSession",
            Self::RevokeSession { .. } => "RevokeSession",
            Self::GrantExecutor { .. } => "GrantExecutor",
            Self::RevokeExecutor { .. } => "RevokeExecutor",
            Self::GrantCleanup { .. } => "GrantCleanup",
            Self::RevokeCleanup { .. } => "RevokeCleanup",
            Self::RegisterArtifactStore { .. } => "RegisterArtifactStore",
            Self::RevokeArtifactStore { .. } => "RevokeArtifactStore",
        }
    }
    pub fn resource(&self) -> AdministrativeResource {
        match self {
            Self::GrantSession { .. } | Self::RevokeSession { .. } => {
                AdministrativeResource::Session
            }
            Self::GrantExecutor { .. } | Self::RevokeExecutor { .. } => {
                AdministrativeResource::Executor
            }
            Self::GrantCleanup { .. } | Self::RevokeCleanup { .. } => {
                AdministrativeResource::Cleanup
            }
            Self::RegisterArtifactStore { .. } | Self::RevokeArtifactStore { .. } => {
                AdministrativeResource::ArtifactStore
            }
        }
    }
    pub(crate) fn target(&self) -> Option<u64> {
        match self {
            Self::RevokeSession { grant, .. }
            | Self::RevokeExecutor { grant, .. }
            | Self::RevokeCleanup { grant, .. } => Some(*grant),
            Self::RevokeArtifactStore { registration } => Some(*registration),
            _ => None,
        }
    }
    fn counter(&self, db: &Database) -> u64 {
        match self.resource() {
            AdministrativeResource::Session => db.next_token,
            AdministrativeResource::Executor => db.next_source_executor,
            AdministrativeResource::Cleanup => db.next_source_cleanup,
            AdministrativeResource::ArtifactStore => db.next_source_artifact_store,
        }
    }
    fn active(&self, db: &Database, scope: &Scope, id: u64) -> Option<bool> {
        match self.resource() {
            AdministrativeResource::Session => db
                .grants
                .get(&id)
                .filter(|g| g.scope == *scope)
                .map(|g| g.active),
            AdministrativeResource::Executor => db
                .source_executors
                .get(&id)
                .filter(|g| g.scope == *scope)
                .map(|g| g.active),
            AdministrativeResource::Cleanup => db
                .source_cleanup_grants
                .get(&id)
                .filter(|g| g.scope == *scope)
                .map(|g| g.active),
            AdministrativeResource::ArtifactStore => db
                .source_artifact_stores
                .get(&id)
                .filter(|g| g.scope == *scope)
                .map(|g| g.active),
        }
    }
    pub(super) fn observe(&self, db: &Database, scope: &Scope) -> AdministrativeContext {
        AdministrativeContext {
            counter_before: self.counter(db),
            target_active_before: self.target().and_then(|id| self.active(db, scope, id)),
        }
    }
    pub(super) fn encode(&self, c: &mut Canonical) {
        c.str(self.kind());
        match self {
            Self::GrantSession {
                principal,
                permissions,
            } => {
                c.str(principal.as_str());
                c.u64(permissions.len() as u64);
                for permission in permissions {
                    c.str(permission_name(*permission));
                }
            }
            Self::RevokeSession { grant, principal } => {
                c.u64(*grant);
                c.str(principal.as_str());
            }
            Self::GrantExecutor {
                principal,
                backend,
                tiers,
                resources,
                expires_at_ms,
            } => {
                c.str(principal.as_str());
                encode_backend(c, backend);
                c.u64(tiers.len() as u64);
                for tier in tiers {
                    c.u64(match tier {
                        ExecutionTier::T0 => 0,
                        ExecutionTier::T1 => 1,
                        ExecutionTier::T2 => 2,
                        ExecutionTier::T3 => 3,
                    });
                }
                for n in [
                    resources.wall_time_ms,
                    resources.cpu_time_ms,
                    resources.memory_bytes,
                    resources.disk_bytes,
                    resources.process_count,
                    resources.file_descriptors,
                    resources.stdout_bytes,
                    resources.stderr_bytes,
                    resources.artifact_bytes,
                ] {
                    c.u64(n);
                }
                c.u64(*expires_at_ms);
            }
            Self::RevokeExecutor {
                grant,
                principal,
                backend,
            } => {
                c.u64(*grant);
                c.str(principal.as_str());
                encode_backend(c, backend);
            }
            Self::GrantCleanup {
                principal,
                backend,
                expires_at_ms,
            } => {
                c.str(principal.as_str());
                c.digest(*backend);
                c.u64(*expires_at_ms);
            }
            Self::RevokeCleanup {
                grant,
                principal,
                backend,
            } => {
                c.u64(*grant);
                c.str(principal.as_str());
                c.digest(*backend);
            }
            Self::RegisterArtifactStore { adapter, retention } => {
                c.str(adapter.identity.as_str());
                c.digest(adapter.configuration);
                c.digest(*retention);
            }
            Self::RevokeArtifactStore { registration } => c.u64(*registration),
        }
    }
    fn apply(&self, db: &mut Database, scope: &Scope) -> Result<MutationValue, EngineError> {
        let id = match self {
            Self::GrantSession {
                principal,
                permissions,
            } => administration::grant_session(db, principal.clone(), scope, permissions.clone())?,
            Self::RevokeSession { grant, principal } => {
                administration::revoke_session(db, scope, *grant, principal)?;
                *grant
            }
            Self::GrantExecutor {
                principal,
                backend,
                tiers,
                resources,
                expires_at_ms,
            } => administration::grant_executor(
                db,
                principal.clone(),
                scope,
                backend.clone(),
                tiers.clone(),
                *resources,
                *expires_at_ms,
            )?,
            Self::RevokeExecutor {
                grant,
                principal,
                backend,
            } => {
                administration::revoke_executor(db, scope, *grant, principal, backend)?;
                *grant
            }
            Self::GrantCleanup {
                principal,
                backend,
                expires_at_ms,
            } => administration::grant_cleanup(
                db,
                principal.clone(),
                scope,
                *backend,
                *expires_at_ms,
            )?,
            Self::RevokeCleanup {
                grant,
                principal,
                backend,
            } => {
                administration::revoke_cleanup(db, scope, *grant, principal, *backend)?;
                *grant
            }
            Self::RegisterArtifactStore { adapter, retention } => {
                administration::register_store(db, scope, Some(adapter.clone()), *retention)?
            }
            Self::RevokeArtifactStore { registration } => {
                administration::revoke_store(db, scope, *registration)?;
                *registration
            }
        };
        Ok(if self.target().is_some() {
            MutationValue::Revocation {
                resource: self.resource(),
                id,
            }
        } else {
            MutationValue::Allocation {
                resource: self.resource(),
                id,
            }
        })
    }
    fn input_error(&self, at: u64) -> Option<EngineError> {
        match self {
            Self::GrantExecutor {
                backend,
                tiers,
                resources,
                expires_at_ms,
                ..
            } => {
                if tiers.is_empty()
                    || tiers.contains(&ExecutionTier::T0)
                    || !source_attempts::valid_text(&backend.name)
                    || !source_attempts::valid_text(&backend.version)
                    || *expires_at_ms <= at
                {
                    return Some(SourceAttemptError::UntrustedExecutor.into());
                }
                resources
                    .validate()
                    .err()
                    .map(|error| EngineError::SourceAttempt(SourceAttemptError::Plan(error)))
            }
            Self::GrantCleanup { expires_at_ms, .. } if *expires_at_ms <= at => {
                Some(EngineError::InvalidTime)
            }
            _ => None,
        }
    }
    pub(super) fn domain_error(&self, error: &EngineError) -> bool {
        if self.target().is_some() {
            return *error == EngineError::Unauthorized
                || matches!(self, Self::RevokeExecutor { .. })
                    && matches!(
                        error,
                        EngineError::SourceAttempt(SourceAttemptError::InvalidState)
                    );
        }
        *error == EngineError::CounterExhausted
            || match self {
                Self::GrantExecutor { .. } => matches!(
                    error,
                    EngineError::SourceAttempt(
                        SourceAttemptError::UntrustedExecutor | SourceAttemptError::Plan(_)
                    )
                ),
                Self::GrantCleanup { .. } => *error == EngineError::InvalidTime,
                _ => false,
            }
    }
    pub(super) fn check_record(&self, r: &MutationRecord) -> Result<(), RestoreError> {
        let invalid = RestoreError::BindingMismatch;
        let context = r.administration.as_ref().ok_or(invalid.clone())?;
        if r.resolved_attempt.is_some()
            || r.state_before.is_some()
            || r.lease_events_before != 0
            || (self.target().is_none() && context.target_active_before.is_some())
        {
            return Err(invalid);
        }
        if let Some(error) = self.input_error(r.recorded_at_ms) {
            return if r.outcome == Err(error) {
                Ok(())
            } else {
                Err(invalid)
            };
        }
        match &r.outcome {
            Ok(MutationValue::Allocation { resource, id })
                if self.target().is_none()
                    && *resource == self.resource()
                    && context.counter_before.checked_add(1) == Some(*id) =>
            {
                Ok(())
            }
            Ok(MutationValue::Revocation { resource, id })
                if self.target() == Some(*id)
                    && *id > 0
                    && *id <= context.counter_before
                    && context.target_active_before.is_some()
                    && *resource == self.resource() =>
            {
                Ok(())
            }
            Err(error) if self.domain_error(error) => Ok(()),
            _ => Err(invalid),
        }
    }
    fn metadata_matches(&self, db: &Database, scope: &Scope, id: u64) -> bool {
        match self {
            Self::GrantSession {
                principal,
                permissions,
            } => db.grants.get(&id).is_some_and(|g| {
                g.scope == *scope && g.principal == *principal && g.permissions == *permissions
            }),
            Self::RevokeSession { principal, .. } => db
                .grants
                .get(&id)
                .is_some_and(|g| g.scope == *scope && g.principal == *principal),
            Self::GrantExecutor {
                principal,
                backend,
                tiers,
                resources,
                expires_at_ms,
            } => db.source_executors.get(&id).is_some_and(|g| {
                g.scope == *scope
                    && g.principal == *principal
                    && g.backend == *backend
                    && g.tiers == *tiers
                    && g.resource_ceiling == *resources
                    && g.expires_at_ms == *expires_at_ms
            }),
            Self::RevokeExecutor {
                principal, backend, ..
            } => db.source_executors.get(&id).is_some_and(|g| {
                g.scope == *scope && g.principal == *principal && g.backend == *backend
            }),
            Self::GrantCleanup {
                principal,
                backend,
                expires_at_ms,
            } => db.source_cleanup_grants.get(&id).is_some_and(|g| {
                g.scope == *scope
                    && g.principal == *principal
                    && g.backend == *backend
                    && g.expires_at_ms == *expires_at_ms
            }),
            Self::RevokeCleanup {
                principal, backend, ..
            } => db.source_cleanup_grants.get(&id).is_some_and(|g| {
                g.scope == *scope && g.principal == *principal && g.backend == *backend
            }),
            Self::RegisterArtifactStore { adapter, retention } => {
                db.source_artifact_stores.get(&id).is_some_and(|g| {
                    g.scope == *scope
                        && g.adapter.as_ref() == Some(adapter)
                        && g.retention == *retention
                        && g.provenance
                            == source_evidence_gate::artifact_store_provenance(
                                scope,
                                id,
                                *retention,
                                Some(adapter),
                            )
                })
            }
            Self::RevokeArtifactStore { .. } => db
                .source_artifact_stores
                .get(&id)
                .is_some_and(|g| g.scope == *scope),
        }
    }
    pub(super) fn validate(
        &self,
        db: &Database,
        scope: &Scope,
        r: &MutationRecord,
    ) -> Result<(), RestoreError> {
        self.check_record(r)?;
        let invalid = RestoreError::BindingMismatch;
        let context = r.administration.as_ref().ok_or(invalid.clone())?;
        if context.counter_before > self.counter(db) {
            return Err(invalid);
        }
        if let Some(id) = self.target() {
            let now = self.active(db, scope, id);
            if (context.target_active_before.is_some()
                && (id == 0 || id > context.counter_before || now.is_none()))
                || (context.target_active_before.is_none()
                    && now.is_some()
                    && id <= context.counter_before)
            {
                return Err(invalid);
            }
        }
        match &r.outcome {
            Ok(MutationValue::Allocation { id, .. }) if self.metadata_matches(db, scope, *id) => {
                Ok(())
            }
            Ok(MutationValue::Revocation { id, .. })
                if self.metadata_matches(db, scope, *id)
                    && self.active(db, scope, *id) == Some(false) =>
            {
                Ok(())
            }
            Err(EngineError::CounterExhausted) if context.counter_before == u64::MAX => Ok(()),
            Err(EngineError::Unauthorized)
                if context.target_active_before.is_none()
                    || self
                        .target()
                        .is_some_and(|id| !self.metadata_matches(db, scope, id)) =>
            {
                Ok(())
            }
            Err(error) if self.input_error(r.recorded_at_ms).as_ref() == Some(error) => Ok(()),
            Err(EngineError::SourceAttempt(SourceAttemptError::InvalidState))
                if matches!(self, Self::RevokeExecutor { .. }) =>
            {
                Ok(())
            }
            _ => Err(invalid),
        }
    }
    pub(super) fn recovery_size(&self) -> Result<RecoverySize, RestoreError> {
        let mut size = RecoverySize {
            entries: 1,
            bytes: 4096,
        };
        let mut add = |text: &str| -> Result<(), RestoreError> {
            size.entries = size.entries.checked_add(1).ok_or(RestoreError::Limit)?;
            size.bytes = size
                .bytes
                .checked_add(text.len())
                .ok_or(RestoreError::Limit)?;
            Ok(())
        };
        match self {
            Self::GrantSession { principal, .. }
            | Self::RevokeSession { principal, .. }
            | Self::GrantCleanup { principal, .. }
            | Self::RevokeCleanup { principal, .. } => add(principal.as_str())?,
            Self::GrantExecutor {
                principal, backend, ..
            }
            | Self::RevokeExecutor {
                principal, backend, ..
            } => {
                add(principal.as_str())?;
                add(&backend.name)?;
                add(&backend.version)?;
            }
            Self::RegisterArtifactStore { adapter, .. } => add(adapter.identity.as_str())?,
            Self::RevokeArtifactStore { .. } => {}
        }
        Ok(size)
    }
}
fn encode_backend(c: &mut Canonical, backend: &BackendIdentity) {
    c.str(&backend.name);
    c.str(&backend.version);
    c.digest(backend.image);
    c.digest(backend.profile);
}
fn permission_name(permission: Permission) -> &'static str {
    match permission {
        Permission::Register => "Register",
        Permission::ManageModel => "ManageModel",
        Permission::Read => "Read",
        Permission::Propose => "Propose",
        Permission::Analyze => "Analyze",
        Permission::Reserve => "Reserve",
        Permission::Execute => "Execute",
        Permission::Verify => "Verify",
        Permission::Commit => "Commit",
        Permission::Cancel => "Cancel",
    }
}

impl Administrator {
    /// Persist an administrative effect and original reply together. Returned
    /// IDs are selectors; this API never constructs a runtime capability.
    pub fn mutate(
        &self,
        scope: &Scope,
        key: IdempotencyKey,
        request: AdministrativeMutation,
    ) -> Result<MutationResponse, EngineError> {
        let request = MutationRequest::Administrative(Box::new(request));
        let response = self.store.administration(scope).atomic(|db| {
            apply_once(
                db,
                scope,
                MutationActor::EmbeddingAdministrator,
                key,
                request,
                |db, request| {
                    let MutationRequest::Administrative(request) = request else {
                        return Err(EngineError::Unauthorized);
                    };
                    request.apply(db, scope)
                },
            )
        })??;
        if let Ok(MutationValue::Revocation {
            resource: AdministrativeResource::ArtifactStore,
            id,
        }) = &response.outcome
        {
            let reader = self.store.artifact_readers()?.remove(&(scope.clone(), *id));
            drop(reader);
        }
        Ok(response)
    }
    pub fn mutation_outcome(
        &self,
        scope: &Scope,
        key: &IdempotencyKey,
    ) -> Result<Option<MutationResponse>, EngineError> {
        self.store.administration(scope).read(|db| {
            Ok(db
                .mutations
                .get(&(
                    scope.clone(),
                    MutationKey {
                        actor: MutationActor::EmbeddingAdministrator,
                        key: key.clone(),
                    },
                ))
                .map(|record| record.response(scope, &MutationActor::EmbeddingAdministrator, true)))
        })
    }
}
