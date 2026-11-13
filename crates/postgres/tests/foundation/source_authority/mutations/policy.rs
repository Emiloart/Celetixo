//! Policy recovery retains historical results without reinstalling old heads.
use super::*;

fn request(policy: SourceExecutionPolicy, expected: Option<Digest>) -> MutationRequest {
    MutationRequest::Repository(Box::new(RepositoryMutation::InstallPolicy {
        expected,
        policy: Box::new(policy),
    }))
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_policy_original_results_survive_replacement_and_a_lost_response() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut h = Harness::new("source-policy-original-replies");
    let original_request = request(h.policy.clone(), None);
    let original = h
        .engine
        .mutate(
            &h.owner,
            h.scope(),
            key("original"),
            original_request.clone(),
        )
        .unwrap();
    assert_eq!(
        original.outcome,
        Ok(MutationValue::Policy {
            policy: h.policy.digest(),
            model_generation: 2
        })
    );
    let mut input = h.policy.input().clone();
    input.generation += 1;
    let next = SourceExecutionPolicy::new(input).unwrap();
    let stale_request = request(next.clone(), None);
    let stale = h
        .engine
        .mutate(&h.owner, h.scope(), key("stale"), stale_request.clone())
        .unwrap();
    assert_eq!(
        stale.outcome,
        Err(EngineError::SourceAttempt(
            SourceAttemptError::PolicyChanged
        ))
    );
    let next_request = request(next.clone(), Some(h.policy.digest()));
    let armed = Arc::new(AtomicBool::new(false));
    let active = armed.clone();
    let fault = Arc::new(authority().with_checkpoint(Arc::new(move |stage| {
        if active.load(Ordering::SeqCst) && stage == celetixo_postgres::AuthorityStage::AfterCommit
        {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (engine, admin) = Engine::with_authority_backend(fault);
    let owner = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    armed.store(true, Ordering::SeqCst);
    assert_eq!(
        engine.mutate(&owner, h.scope(), key("replace"), next_request.clone()),
        Err(EngineError::Authority(
            AuthorityFailure::CommitIndeterminate
        ))
    );
    h.reopen();
    let replaced = h
        .engine
        .mutate(&h.owner, h.scope(), key("replace"), next_request.clone())
        .unwrap();
    assert!(replaced.replayed);
    assert_eq!(
        replaced.outcome,
        Ok(MutationValue::Policy {
            policy: next.digest(),
            model_generation: 3
        })
    );
    let noop = h
        .engine
        .mutate(&h.owner, h.scope(), key("noop"), next_request.clone())
        .unwrap();
    assert_eq!(noop.outcome, replaced.outcome);
    let mut input = next.input().clone();
    input.generation += 1;
    let last = SourceExecutionPolicy::new(input).unwrap();
    h.engine
        .install_source_policy(&h.owner, h.scope(), Some(next.digest()), last.clone())
        .unwrap();
    h.reopen();
    for (operation, request, expected) in [
        ("original", original_request, original),
        ("stale", stale_request, stale),
        ("replace", next_request, replaced),
    ] {
        let replay = h
            .engine
            .mutate(&h.owner, h.scope(), key(operation), request)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, expected.outcome);
        assert_eq!(replay.recorded_at_ms, expected.recorded_at_ms);
    }
    let records = h.authority.inspect_source(h.scope()).unwrap().records;
    assert_eq!(records.policy_head, Some(last.digest()));
    assert_eq!(records.policies.len(), 3);
    assert_eq!(
        records.base.repository.as_ref().unwrap().model.generation,
        4
    );
    assert_eq!(records.base.mutations.len(), 4);
    assert!(records.base.transactions.is_empty());
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn delayed_policy_installation_cannot_publish_an_expired_new_head() {
    let h = Harness::new("source-policy-publication-expiry");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut input = h.policy.input().clone();
    input.generation += 1;
    input.expires_at_ms = u64::try_from(now).unwrap() + 20_000;
    let next = SourceExecutionPolicy::new(input).unwrap();
    let expiry = next.input().expires_at_ms;
    let before = h.authority.inspect_source(h.scope()).unwrap();
    let backend = Arc::new(authority().with_checkpoint(Arc::new(move |stage| {
        if stage == celetixo_postgres::AuthorityStage::BeforeCommit {
            let now = u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(
                expiry.saturating_sub(now) + 20,
            ));
        }
        Ok(())
    })));
    let (engine, admin) = Engine::with_authority_backend(backend);
    let owner = admin
        .authenticate_existing_session(h.scope(), 1, |_, _| Ok(()))
        .unwrap();
    // Keep the intentional delay within the unchanged SQL idle bound. The
    // initial preparation/authentication happen before the publication window.
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(
        expiry.saturating_sub(now).saturating_sub(5_000),
    ));
    let request = request(next, Some(h.policy.digest()));
    assert_eq!(
        engine.mutate(&owner, h.scope(), key("replace"), request.clone()),
        Err(EngineError::Authority(AuthorityFailure::Contention))
    );
    assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), before);
    let retry = h
        .engine
        .mutate(&h.owner, h.scope(), key("replace"), request)
        .unwrap();
    assert!(!retry.replayed);
    assert_eq!(
        retry.outcome,
        Err(EngineError::SourceAttempt(
            SourceAttemptError::PolicyChanged
        ))
    );
}
