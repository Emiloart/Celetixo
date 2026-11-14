use super::*;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

struct ReadBarrier {
    backing: Arc<LocalSourceArtifactStore>,
    first_reads: Mutex<usize>,
    both_reading: Condvar,
}
impl SourceArtifactReader for ReadBarrier {
    fn read(
        &self,
        scope: &Scope,
        id: Digest,
        max_bytes: u64,
    ) -> Result<Vec<u8>, SourceArtifactError> {
        let mut count = self.first_reads.lock().unwrap();
        if *count < 2 {
            *count += 1;
            if *count == 2 {
                self.both_reading.notify_all();
            } else {
                let (next, timeout) = self
                    .both_reading
                    .wait_timeout_while(count, Duration::from_secs(20), |count| *count < 2)
                    .unwrap();
                if timeout.timed_out() {
                    return Err(SourceArtifactError::Persistence(
                        "competing source publication did not reach unlocked reads".into(),
                    ));
                }
                count = next;
            }
        }
        drop(count);
        self.backing.read(scope, id, max_bytes)
    }
    fn retention_policy(&self) -> Digest {
        self.backing.retention_policy()
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn independent_engine_instances_publish_one_revision_and_one_original_result() {
    let (h, _, request) = ready("source-publication-race");
    let reader = Arc::new(ReadBarrier {
        backing: h.artifacts.as_ref().unwrap().clone(),
        first_reads: Mutex::new(0),
        both_reading: Condvar::new(),
    });
    let mut workers = Vec::new();
    for _ in 0..2 {
        let (engine, admin) =
            Engine::with_authority_backend(Arc::new(super::super::super::authority()));
        let owner = admin
            .authenticate_existing_session(h.scope(), request.authorization_grant, |_, _| Ok(()))
            .unwrap();
        let analyst = admin
            .authenticate_existing_session(h.scope(), request.analysis_grant, |_, _| Ok(()))
            .unwrap();
        admin
            .resolve_existing_source_artifact_store(
                h.scope(),
                request.store_registration,
                |_, _| Ok(reader.clone()),
            )
            .unwrap();
        let scope = h.scope().clone();
        let request = request.clone();
        workers.push(std::thread::spawn(move || {
            engine.commit_source_prepared(&owner, &analyst, &scope, request)
        }));
    }
    let receipts = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(receipts[0], receipts[1]);
    let after = h.authority.inspect_source(h.scope()).unwrap();
    assert_eq!(after.records.history.len(), 1);
    assert_eq!(after.records.history[0].receipt, receipts[0]);
    assert_eq!(
        after
            .records
            .base
            .mutations
            .values()
            .filter(|r| matches!(r.outcome, Ok(MutationValue::SourceCommitted(_))))
            .count(),
        1
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn final_database_time_rejects_source_publication_after_original_lease_expiry() {
    let (h, _, request) = ready_with_ttl("source-publication-final-time", 18_000);
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let expires = before.records.base.transactions[&transaction_id()]
        .lease
        .as_ref()
        .unwrap()
        .expires_at;
    let injected = Arc::new(super::super::super::authority().with_checkpoint(Arc::new(
        move |stage| {
            if stage == AuthorityStage::BeforeCommit {
                assert!(
                    now() < expires,
                    "source commit must reach the checkpoint with a live original lease"
                );
                let remaining = expires.saturating_add(2).saturating_sub(now());
                assert!(remaining <= 9_002);
                std::thread::sleep(Duration::from_millis(remaining));
            }
            Ok(())
        },
    )));
    let (engine, admin) = Engine::with_authority_backend(injected);
    let owner = admin
        .authenticate_existing_session(h.scope(), request.authorization_grant, |_, _| Ok(()))
        .unwrap();
    let analyst = admin
        .authenticate_existing_session(h.scope(), request.analysis_grant, |_, _| Ok(()))
        .unwrap();
    admin
        .resolve_existing_source_artifact_store(h.scope(), request.store_registration, |_, _| {
            Ok(h.artifacts.as_ref().unwrap().clone())
        })
        .unwrap();
    // Wait outside SQL so expiry occurs inside the final checkpoint without
    // exceeding the unchanged ten-second idle-transaction timeout.
    let wait = expires.saturating_sub(now().saturating_add(9_000));
    std::thread::sleep(Duration::from_millis(wait));
    assert_eq!(
        engine.commit_source_prepared(&owner, &analyst, h.scope(), request),
        Err(EngineError::Authority(AuthorityFailure::Contention))
    );
    assert_eq!(
        h.authority.inspect_source(h.scope()).unwrap(),
        before,
        "head, history, original key, terminal outcome and outbox must all roll back"
    );
}
