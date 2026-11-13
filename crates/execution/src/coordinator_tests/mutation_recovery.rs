//! Normalized fault fixture; starts no process and establishes no isolation.
use super::*;
use celetixo_engine::engine::authority::*;
use std::sync::Mutex;

struct FaultBackend {
    stored: Mutex<SourceRecords>,
    lose: Mutex<Option<&'static str>>,
}
impl AuthorityBackend for FaultBackend {
    fn access(
        &self,
        access: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure> {
        let mut stored = self.stored.lock().unwrap();
        if access.scope() != &stored.base.scope {
            return Err(AuthorityFailure::ScopeDenied);
        }
        let mut state = AuthorityState::from_source_records(stored.clone(), Default::default())
            .map_err(|_| AuthorityFailure::Integrity)?;
        evaluate(&mut state)?;
        let after = state
            .source_records(Default::default())
            .map_err(|_| AuthorityFailure::Integrity)?;
        AuthorityState::from_source_records(after.clone(), Default::default())
            .map_err(|_| AuthorityFailure::Integrity)?;
        if matches!(
            access.mode(),
            AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
        ) {
            let mut lose = self.lose.lock().unwrap();
            let lost=after.base.mutations.iter().any(|(key,r)| {
                !stored.base.mutations.contains_key(key)&&matches!(&r.request,celetixo_engine::engine::MutationRequest::SourceExecutor(request) if Some(request.action.kind())==*lose)
            });
            *stored = after;
            if lost {
                *lose = None;
                return Err(AuthorityFailure::CommitIndeterminate);
            }
        }
        Ok(())
    }
}
fn durable(lose: &'static str) -> (Fixture, Arc<FaultBackend>) {
    let mut h = Fixture::new(false);
    let stored = h
        .admin
        .inspect_source_records(&scope(), Default::default())
        .unwrap();
    let backend = Arc::new(FaultBackend {
        stored: Mutex::new(stored),
        lose: Mutex::new(Some(lose)),
    });
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    h.owner = admin
        .authenticate_existing_session(&scope(), 1, |_, _| Ok(()))
        .unwrap();
    h.observer = admin
        .authenticate_existing_session(&scope(), 2, |_, _| Ok(()))
        .unwrap();
    h.executor = admin
        .authenticate_existing_source_executor(&scope(), 1, |_, _| Ok(()))
        .unwrap();
    h.storage = admin
        .resolve_existing_source_artifact_store(&scope(), 1, |_, _| Ok(h.artifacts.clone()))
        .unwrap();
    h.admin = admin;
    h.engine = engine;
    (h, backend)
}

#[test]
fn a_lost_claim_reply_and_original_replay_never_materialize_or_launch() {
    let (h, stored) = durable("ClaimSourceAttempt");
    let backend = FiniteBackend::new(0);
    let coordinator = h.coordinator(&backend);
    let first = coordinator.execute(h.request(), &mut Continue).unwrap_err();
    assert_eq!(first.stage, ExecutionStage::Admission);
    assert!(matches!(
        first.cause,
        SourceRunError::Engine(EngineError::Authority(
            AuthorityFailure::CommitIndeterminate
        ))
    ));
    assert_eq!(h.state(), SourceAttemptState::Patching);
    let before = stored.stored.lock().unwrap().clone();
    let second = coordinator.execute(h.request(), &mut Continue).unwrap_err();
    assert_eq!(second.stage, ExecutionStage::Admission);
    assert!(matches!(
        second.cause,
        SourceRunError::Engine(EngineError::SourceAttempt(
            SourceAttemptError::AlreadyClaimed
        ))
    ));
    assert!(first.terminal_recording.is_none() && second.terminal_recording.is_none());
    assert!(backend.calls.borrow().is_empty() && backend.materializations.borrow().is_empty());
    assert_eq!(objects(&h.artifacts), 0);
    assert_eq!(stored.stored.lock().unwrap().clone(), before);
    assert_eq!(before.base.mutations.len(), 1);
    assert_eq!(
        before.attempts[&h.admission.binding().source_attempt]
            .events
            .len(),
        2
    );
}

#[test]
fn original_patch_and_verification_replies_recover_with_one_owned_execution() {
    for lose in ["SourcePatchApplied", "BeginSourceVerification"] {
        let (h, stored) = durable(lose);
        let backend = FiniteBackend::new(0);
        let coordinator = h.coordinator(&backend);
        let result = coordinator.execute(h.request(), &mut Continue).unwrap();
        assert_eq!(result.receipt.assessment, SourceEvidenceAssessment::Passed);
        assert_eq!(backend.calls.borrow().len(), 1);
        assert_eq!(backend.materializations.borrow().len(), 1);
        backend.assert_cleaned();
        let before = stored.stored.lock().unwrap().clone();
        assert_eq!(before.base.mutations.len(), 3);
        assert_eq!(
            before.attempts[&h.admission.binding().source_attempt]
                .events
                .len(),
            5
        );
        let replay = coordinator.execute(h.request(), &mut Continue).unwrap_err();
        assert_eq!(replay.stage, ExecutionStage::Admission);
        assert_eq!(backend.calls.borrow().len(), 1);
        assert_eq!(stored.stored.lock().unwrap().clone(), before);
    }
}

#[test]
fn an_original_failure_reply_recovers_without_repeating_the_check_or_terminal_event() {
    let (h, stored) = durable("FinishSourceAttempt");
    let backend = FiniteBackend {
        forged: Some(ForgedBinding::Plan),
        ..FiniteBackend::new(0)
    };
    let failure = h
        .coordinator(&backend)
        .execute(h.request(), &mut Continue)
        .unwrap_err();
    assert_eq!(failure.stage, ExecutionStage::Verification);
    assert_eq!(failure.terminal_recording, Some(Ok(())));
    assert_eq!(h.state(), SourceAttemptState::ExecutionFailed);
    assert_eq!(backend.calls.borrow().len(), 1);
    backend.assert_cleaned();
    let before = stored.stored.lock().unwrap().clone();
    assert_eq!(before.base.mutations.len(), 4);
    assert_eq!(
        before.attempts[&h.admission.binding().source_attempt]
            .events
            .len(),
        5
    );
    assert!(
        h.coordinator(&backend)
            .execute(h.request(), &mut Continue)
            .is_err()
    );
    assert_eq!(backend.calls.borrow().len(), 1);
    assert_eq!(stored.stored.lock().unwrap().clone(), before);
}
