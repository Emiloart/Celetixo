//! Live embedding resources. None of these values belongs in persisted state.
use super::*;
use crate::source_evidence::SourceArtifactReader;

/// Allocation identity authenticates opaque capabilities within one embedding.
/// Stored grant IDs alone cannot reconstruct this identity after state recovery.
pub(super) struct StoreIdentity;

pub(super) type SourceArtifactReaders = BTreeMap<(Scope, u64), Arc<dyn SourceArtifactReader>>;

impl InMemoryStore {
    /// When both locks are needed, acquire state before readers. Never invoke an
    /// adapter while either lock is held. A missing adapter fails closed even if
    /// an active registration record exists.
    pub(super) fn artifact_readers(
        &self,
    ) -> Result<MutexGuard<'_, SourceArtifactReaders>, EngineError> {
        self.source_artifact_readers
            .lock()
            .map_err(|_| EngineError::StoreUnavailable)
    }

    pub(super) fn artifact_reader(
        &self,
        scope: &Scope,
        registration: u64,
    ) -> Result<Arc<dyn SourceArtifactReader>, EngineError> {
        self.artifact_readers()?
            .get(&(scope.clone(), registration))
            .cloned()
            .ok_or(EngineError::StoreUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_evidence::SourceArtifactError;
    use crate::source_execution::{BackendIdentity, ExecutionTier, ResourceLimits};

    #[derive(Default)]
    struct Reader {
        on_drop: Option<Box<dyn FnOnce() + Send + Sync>>,
    }
    impl Drop for Reader {
        fn drop(&mut self) {
            if let Some(callback) = self.on_drop.take() {
                callback();
            }
        }
    }
    impl SourceArtifactReader for Reader {
        fn read(&self, _: &Scope, _: Digest, _: u64) -> Result<Vec<u8>, SourceArtifactError> {
            panic!("runtime ownership test must not execute adapter I/O")
        }
        fn retention_policy(&self) -> Digest {
            Digest::of("runtime-test", b"retention")
        }
    }

    fn scope() -> Scope {
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("workspace").unwrap(),
            repository: RepositoryId::new("repository").unwrap(),
        }
    }

    #[test]
    fn scoped_grant_id_alias_cannot_rebind_an_existing_opaque_session() {
        let (store, admin) = InMemoryStore::new();
        let principal = PrincipalId::new("operator").unwrap();
        let session = admin
            .grant(principal, scope(), Permission::ALL.into())
            .unwrap();
        let mut other_scope = scope();
        other_scope.repository = RepositoryId::new("other-repository").unwrap();
        // Models loading a different scoped SQL row with the same numeric ID.
        // There is no production API to mutate a grant's established scope.
        store
            .lock()
            .unwrap()
            .grants
            .get_mut(&session.token)
            .unwrap()
            .scope = other_scope.clone();
        let engine = Engine::new(store.clone());
        for requested in [scope(), other_scope] {
            assert_eq!(
                engine.authorize(
                    &store.lock().unwrap(),
                    &session,
                    &requested,
                    Permission::Read
                ),
                Err(EngineError::Unauthorized)
            );
        }
        assert_eq!(admin.revoke(&session), Err(EngineError::Unauthorized));
        assert!(store.lock().unwrap().grants[&session.token].active);
    }

    #[test]
    fn copied_records_restore_neither_capabilities_nor_readers() {
        let (store, admin) = InMemoryStore::new();
        let principal = PrincipalId::new("operator").unwrap();
        let session = admin
            .grant(principal.clone(), scope(), Permission::ALL.into())
            .unwrap();
        let backend = BackendIdentity {
            name: "reference".into(),
            version: "v1".into(),
            image: Digest::of("runtime-test", b"image"),
            profile: Digest::of("runtime-test", b"profile"),
        };
        let executor = admin
            .grant_source_executor(
                principal.clone(),
                scope(),
                backend.clone(),
                BTreeSet::from([ExecutionTier::T1]),
                ResourceLimits {
                    wall_time_ms: 1000,
                    cpu_time_ms: 500,
                    memory_bytes: 1024 * 1024,
                    disk_bytes: 1024 * 1024,
                    process_count: 4,
                    file_descriptors: 32,
                    stdout_bytes: 4096,
                    stderr_bytes: 4096,
                    artifact_bytes: 1024 * 1024,
                },
                100,
            )
            .unwrap();
        let cleanup = admin
            .grant_source_cleanup(principal, scope(), backend.profile, 100)
            .unwrap();
        let reader: Arc<dyn SourceArtifactReader> = Arc::new(Reader::default());
        let weak = Arc::downgrade(&reader);
        let storage = admin
            .register_source_artifact_store(scope(), reader.clone())
            .unwrap();
        drop(reader);

        // Test-only record copying is deliberately not a public recovery API.
        // Every stored grant/registration is copied, including its numeric ID.
        let copied_records = store.lock().unwrap().clone();
        let registration = copied_records.next_source_artifact_store;
        let (copy, copy_admin) = InMemoryStore::new();
        *copy.lock().unwrap() = copied_records;
        let copied_engine = Engine::new(copy.clone());
        assert_eq!(
            copied_engine.authorize(&copy.lock().unwrap(), &session, &scope(), Permission::Read),
            Err(EngineError::Unauthorized)
        );
        assert_eq!(copy_admin.revoke(&session), Err(EngineError::Unauthorized));
        assert_eq!(
            source_attempts::validate_executor_store(&copied_engine, &executor),
            Err(EngineError::Unauthorized)
        );
        assert_eq!(
            copy_admin.revoke_source_executor(&executor),
            Err(EngineError::Unauthorized)
        );
        assert_eq!(
            copied_engine.validate_source_cleanup(&cleanup, &scope(), backend.profile),
            Err(EngineError::Unauthorized)
        );
        assert_eq!(
            copy_admin.revoke_source_cleanup(&cleanup),
            Err(EngineError::Unauthorized)
        );
        assert_eq!(
            copy_admin.revoke_source_artifact_store(&storage),
            Err(EngineError::Unauthorized)
        );
        assert!(matches!(
            copy.artifact_reader(&scope(), registration),
            Err(EngineError::StoreUnavailable)
        ));

        // Multiple handles in the original embedding still share live authority.
        let engine = Engine::new(store.clone());
        assert_eq!(
            engine.authorize(&store.lock().unwrap(), &session, &scope(), Permission::Read),
            Ok(())
        );
        assert_eq!(
            source_attempts::validate_executor_store(&engine, &executor),
            Ok(())
        );
        assert_eq!(
            engine.validate_source_cleanup(&cleanup, &scope(), backend.profile),
            Ok(())
        );
        assert!(store.artifact_reader(&scope(), registration).is_ok());

        // Retained capabilities and copied records do not retain runtime readers.
        drop(engine);
        drop(admin);
        drop(store);
        assert!(weak.upgrade().is_none());
        assert!(
            copy.lock()
                .unwrap()
                .source_artifact_stores
                .contains_key(&registration)
        );
        drop((session, executor, cleanup, storage));
    }

    #[test]
    fn revocation_releases_the_reader_and_keeps_the_registration_record() {
        let (store, admin) = InMemoryStore::new();
        let reader: Arc<dyn SourceArtifactReader> = Arc::new(Reader::default());
        let weak = Arc::downgrade(&reader);
        let capability = admin
            .register_source_artifact_store(scope(), reader)
            .unwrap();
        let registration = store.lock().unwrap().next_source_artifact_store;
        let in_flight = store.artifact_reader(&scope(), registration).unwrap();
        admin.revoke_source_artifact_store(&capability).unwrap();
        assert!(matches!(
            store.artifact_reader(&scope(), registration),
            Err(EngineError::StoreUnavailable)
        ));
        assert!(weak.upgrade().is_some());
        drop(in_flight);
        assert!(weak.upgrade().is_none());
        assert_eq!(store.lock().unwrap().source_artifact_stores.len(), 1);
        // Revocation remains idempotent even after releasing the adapter.
        admin.revoke_source_artifact_store(&capability).unwrap();
    }

    #[test]
    fn revocation_drops_adapter_resources_outside_state_and_runtime_locks() {
        let (store, admin) = InMemoryStore::new();
        let reenter = admin.clone();
        let (done, receiver) = std::sync::mpsc::sync_channel(1);
        let receiver = Mutex::new(receiver);
        let reader = Reader {
            on_drop: Some(Box::new(move || {
                let worker = std::thread::spawn(move || {
                    reenter.advance_time(1).unwrap();
                    // Reacquire both state and runtime locks, without adapter I/O.
                    reenter
                        .register_source_artifact_store(scope(), Arc::new(Reader::default()))
                        .unwrap();
                    done.send(()).unwrap();
                });
                receiver
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .expect("adapter destruction must run outside Engine locks");
                worker.join().unwrap();
            })),
        };
        let capability = admin
            .register_source_artifact_store(scope(), Arc::new(reader))
            .unwrap();
        admin.revoke_source_artifact_store(&capability).unwrap();
        assert_eq!(store.lock().unwrap().now, 1);
        assert_eq!(store.lock().unwrap().source_artifact_stores.len(), 2);
    }

    #[test]
    fn unavailable_runtime_cannot_partially_publish_a_registration() {
        let (store, admin) = InMemoryStore::new();
        let readers = store.source_artifact_readers.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = readers.lock().unwrap();
                panic!("controlled runtime lock poisoning");
            })
            .join()
            .is_err()
        );
        assert!(matches!(
            admin.register_source_artifact_store(scope(), Arc::new(Reader::default())),
            Err(EngineError::StoreUnavailable)
        ));
        let db = store.lock().unwrap();
        assert_eq!(db.next_source_artifact_store, 0);
        assert!(db.source_artifact_stores.is_empty());
    }
}
