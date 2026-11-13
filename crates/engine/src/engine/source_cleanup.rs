//! Narrow trusted recovery authority. Cleanup cannot launch checks, claim an
//! attempt, admit evidence or advance repository state.
use super::*;

#[derive(Clone)]
pub struct SourceCleanupCapability {
    pub(super) store: Arc<StoreIdentity>,
    pub(super) token: u64,
    pub(super) principal: PrincipalId,
    pub(super) scope: Scope,
    pub(super) backend: Digest,
}

impl SourceCleanupCapability {
    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }
}

#[derive(Clone)]
pub(super) struct SourceCleanupGrant {
    pub(super) principal: PrincipalId,
    pub(super) scope: Scope,
    pub(super) backend: Digest,
    pub(super) expires_at_ms: u64,
    pub(super) active: bool,
}

impl Administrator {
    /// Issued by the trusted embedding/operator, separately from execution. A
    /// fresh operator grant can reconcile a journal after in-memory state loss;
    /// it cannot reconstruct or resume a lost transaction/admission.
    pub fn grant_source_cleanup(
        &self,
        principal: PrincipalId,
        scope: Scope,
        backend: Digest,
        expires_at_ms: u64,
    ) -> Result<SourceCleanupCapability, EngineError> {
        self.store.administration(&scope).update(|db| {
            let token = administration::grant_cleanup(
                db,
                principal.clone(),
                &scope,
                backend,
                expires_at_ms,
            )?;
            Ok(SourceCleanupCapability {
                store: self.store.identity.clone(),
                token,
                principal,
                scope,
                backend,
            })
        })
    }

    pub fn revoke_source_cleanup(
        &self,
        capability: &SourceCleanupCapability,
    ) -> Result<(), EngineError> {
        if !Arc::ptr_eq(&self.store.identity, &capability.store) {
            return Err(EngineError::Unauthorized);
        }
        self.store.administration(&capability.scope).update(|db| {
            administration::revoke_cleanup(
                db,
                &capability.scope,
                capability.token,
                &capability.principal,
                capability.backend,
            )
        })
    }
}

impl Engine {
    /// Authorizes only stopping/inspecting identity-validated owned resources in
    /// the given scope/profile. The backend must still validate its journal,
    /// directory identities and process/cgroup ownership before each side effect.
    pub fn validate_source_cleanup(
        &self,
        capability: &SourceCleanupCapability,
        scope: &Scope,
        backend: Digest,
    ) -> Result<(), EngineError> {
        if !Arc::ptr_eq(&self.store.identity, &capability.store) {
            return Err(EngineError::Unauthorized);
        }
        self.store
            .source_cleanup(&capability.principal, capability.token, scope)
            .read(|db| {
                let grant = db
                    .source_cleanup_grants
                    .get(&capability.token)
                    .ok_or(EngineError::Unauthorized)?;
                if !grant.active
                    || grant.expires_at_ms <= db.now
                    || grant.principal != capability.principal
                    || grant.scope != *scope
                    || capability.scope != *scope
                    || grant.backend != backend
                    || capability.backend != backend
                {
                    return Err(EngineError::Unauthorized);
                }
                Ok(())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repository").unwrap(),
        }
    }
    #[test]
    fn cleanup_is_scoped_store_bound_revocable_and_expiring() {
        let (store, admin) = InMemoryStore::new();
        let engine = Engine::new(store);
        let profile = Digest::of("test", b"profile");
        let capability = admin
            .grant_source_cleanup(PrincipalId::new("operator").unwrap(), scope(), profile, 100)
            .unwrap();
        assert_eq!(
            engine.validate_source_cleanup(&capability, &scope(), profile),
            Ok(())
        );
        let (foreign, _) = InMemoryStore::new();
        assert_eq!(
            Engine::new(foreign).validate_source_cleanup(&capability, &scope(), profile),
            Err(EngineError::Unauthorized)
        );
        let mut other = scope();
        other.repository = RepositoryId::new("other").unwrap();
        assert_eq!(
            engine.validate_source_cleanup(&capability, &other, profile),
            Err(EngineError::Unauthorized)
        );
        assert_eq!(
            engine.validate_source_cleanup(&capability, &scope(), Digest::of("test", b"other")),
            Err(EngineError::Unauthorized)
        );
        admin.revoke_source_cleanup(&capability).unwrap();
        assert_eq!(
            engine.validate_source_cleanup(&capability, &scope(), profile),
            Err(EngineError::Unauthorized)
        );
        let expiring = admin
            .grant_source_cleanup(PrincipalId::new("operator").unwrap(), scope(), profile, 1)
            .unwrap();
        admin.advance_time(1).unwrap();
        assert_eq!(
            engine.validate_source_cleanup(&expiring, &scope(), profile),
            Err(EngineError::Unauthorized)
        );
    }
}
