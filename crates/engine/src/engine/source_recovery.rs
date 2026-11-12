//! Fresh trusted runtime resolution. Metadata import never calls these closures
//! or reconstructs allocation identity, credentials or live storage readers.
use super::*;
use crate::source_evidence::SourceArtifactReader;
use authority::{SourceArtifactStoreRecord, SourceCleanupRecord, SourceExecutorRecord};
use source_attempts::SourceExecutorGrant;
use source_evidence_gate::RegisteredSourceArtifactStore;

fn executor_record(g: &SourceExecutorGrant) -> SourceExecutorRecord {
    SourceExecutorRecord {
        principal: g.principal.clone(),
        backend: g.backend.clone(),
        tiers: g.tiers.clone(),
        resource_ceiling: g.resource_ceiling,
        expires_at_ms: g.expires_at_ms,
        active: g.active,
    }
}
fn store_record(g: &RegisteredSourceArtifactStore) -> SourceArtifactStoreRecord {
    SourceArtifactStoreRecord {
        retention: g.retention,
        provenance: g.provenance,
        active: g.active,
    }
}
impl Administrator {
    /// The embedding must independently authenticate the exact scoped identity
    /// and approved backend/resources. A stored numeric selector grants nothing.
    pub fn authenticate_existing_source_executor(
        &self,
        scope: &Scope,
        grant_id: u64,
        authenticate: impl FnOnce(&Scope, &SourceExecutorRecord) -> Result<(), EngineError>,
    ) -> Result<SourceExecutorCapability, EngineError> {
        let original = self.store.administration(scope).read(|db| {
            let g = db
                .source_executors
                .get(&grant_id)
                .ok_or(EngineError::Unauthorized)?;
            if g.scope != *scope || !g.active || g.expires_at_ms <= db.now {
                return Err(EngineError::Unauthorized);
            }
            Ok(executor_record(g))
        })?;
        authenticate(scope, &original)?;
        self.store.administration(scope).read(|db| {
            let g = db
                .source_executors
                .get(&grant_id)
                .ok_or(EngineError::Unauthorized)?;
            if g.scope != *scope
                || executor_record(g) != original
                || !g.active
                || g.expires_at_ms <= db.now
            {
                return Err(EngineError::Unauthorized);
            }
            Ok(SourceExecutorCapability {
                store: self.store.identity.clone(),
                token: grant_id,
                principal: g.principal.clone(),
                scope: scope.clone(),
                backend: g.backend.clone(),
            })
        })
    }
    pub fn authenticate_existing_source_cleanup(
        &self,
        scope: &Scope,
        grant_id: u64,
        authenticate: impl FnOnce(&Scope, &SourceCleanupRecord) -> Result<(), EngineError>,
    ) -> Result<SourceCleanupCapability, EngineError> {
        let read = |db: &Database| {
            let g = db
                .source_cleanup_grants
                .get(&grant_id)
                .ok_or(EngineError::Unauthorized)?;
            if g.scope != *scope || !g.active || g.expires_at_ms <= db.now {
                return Err(EngineError::Unauthorized);
            }
            Ok(SourceCleanupRecord {
                principal: g.principal.clone(),
                backend: g.backend,
                expires_at_ms: g.expires_at_ms,
                active: g.active,
            })
        };
        let original = self.store.administration(scope).read(read)?;
        authenticate(scope, &original)?;
        self.store.administration(scope).read(|db| {
            if read(db)? != original {
                return Err(EngineError::Unauthorized);
            }
            Ok(SourceCleanupCapability {
                store: self.store.identity.clone(),
                token: grant_id,
                principal: original.principal.clone(),
                scope: scope.clone(),
                backend: original.backend,
            })
        })
    }
    /// Resolve from trusted runtime configuration outside state/runtime locks.
    /// Attachment retains the exact registration provenance and retention policy;
    /// it neither retrieves nor proves durability of any required object.
    pub fn resolve_existing_source_artifact_store(
        &self,
        scope: &Scope,
        registration: u64,
        resolve: impl FnOnce(
            &Scope,
            &SourceArtifactStoreRecord,
        ) -> Result<Arc<dyn SourceArtifactReader>, EngineError>,
    ) -> Result<SourceArtifactStoreCapability, EngineError> {
        let original = self.store.administration(scope).read(|db| {
            let g = db
                .source_artifact_stores
                .get(&registration)
                .ok_or(EngineError::Unauthorized)?;
            if g.scope != *scope || !g.active {
                return Err(EngineError::Unauthorized);
            }
            Ok(store_record(g))
        })?;
        let reader = resolve(scope, &original)?;
        if reader.retention_policy() != original.retention {
            return Err(EngineError::Unauthorized);
        }
        // Keep a caller-owned Arc until after the callback/SQL boundary so no
        // user-defined reader destructor can run while either lock is held.
        self.store.administration(scope).read(|db| {
            let g = db
                .source_artifact_stores
                .get(&registration)
                .ok_or(EngineError::Unauthorized)?;
            if g.scope != *scope || store_record(g) != original || !g.active {
                return Err(EngineError::Unauthorized);
            }
            let mut readers = self.store.artifact_readers()?;
            match readers.entry((scope.clone(), registration)) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(reader.clone());
                }
                std::collections::btree_map::Entry::Occupied(entry)
                    if Arc::ptr_eq(entry.get(), &reader) => {}
                std::collections::btree_map::Entry::Occupied(_) => {
                    return Err(EngineError::AlreadyExists);
                }
            }
            Ok(SourceArtifactStoreCapability {
                store: self.store.identity.clone(),
                token: registration,
                scope: scope.clone(),
            })
        })
    }
}
