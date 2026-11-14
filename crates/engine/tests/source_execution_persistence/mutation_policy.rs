//! Original policy requests include complete immutable registry inputs.
use super::*;
use celetixo_engine::engine::{authority::*, *};
use std::sync::{Arc, Mutex};

fn actor() -> MutationActor {
    MutationActor::Session {
        principal: PrincipalId::new("owner").unwrap(),
        grant: 1,
    }
}
fn key(label: &str) -> IdempotencyKey {
    IdempotencyKey::new(label).unwrap()
}
fn request(policy: SourceExecutionPolicy, expected: Option<Digest>) -> MutationRequest {
    MutationRequest::Repository(Box::new(RepositoryMutation::InstallPolicy {
        expected,
        policy: Box::new(policy),
    }))
}

#[test]
fn original_policy_codec_keeps_full_definitions_and_rejects_truncation_and_substitution() {
    let f = Fixture::new();
    let request = request(f.policy.clone(), None);
    let bytes = request.canonical_bytes(f.base.scope(), &actor());
    assert_eq!(
        restore_mutation_request(
            f.base.scope(),
            &actor(),
            request.digest(f.base.scope(), &actor()),
            &bytes,
            Default::default()
        )
        .unwrap(),
        request
    );
    for end in 0..bytes.len() {
        let prefix = &bytes[..end];
        assert!(
            restore_mutation_request(
                f.base.scope(),
                &actor(),
                Digest::of(MutationRequest::FORMAT, prefix),
                prefix,
                Default::default()
            )
            .is_err(),
            "accepted prefix {end}"
        );
    }
    for limits in [
        DomainDecodeLimits {
            collection_entries: 1,
            ..Default::default()
        },
        DomainDecodeLimits {
            field_bytes: 64,
            ..Default::default()
        },
        DomainDecodeLimits {
            encoded_bytes: bytes.len() - 1,
            ..Default::default()
        },
    ] {
        assert!(
            restore_mutation_request(
                f.base.scope(),
                &actor(),
                request.digest(f.base.scope(), &actor()),
                &bytes,
                limits
            )
            .is_err()
        );
    }
    let mut changed = bytes.clone();
    let literal = b"/usr/bin/check";
    let offset = changed
        .windows(literal.len())
        .position(|part| part == literal)
        .unwrap();
    changed[offset] = b'!';
    assert!(
        restore_mutation_request(
            f.base.scope(),
            &actor(),
            Digest::of(MutationRequest::FORMAT, &changed),
            &changed,
            Default::default()
        )
        .is_err()
    );
    let key = MutationKey {
        actor: actor(),
        key: key("policy"),
    };
    for outcome in [
        Ok(MutationValue::Policy {
            policy: f.policy.digest(),
            model_generation: 2,
        }),
        Err(EngineError::SourceAttempt(
            SourceAttemptError::PolicyChanged,
        )),
    ] {
        let value = MutationOutcomeRecord::new(
            f.base.scope().clone(),
            key.clone(),
            MutationRecord {
                request: request.clone(),
                recorded_at_ms: 9,
                resolved_attempt: None,
                state_before: None,
                lease_events_before: 0,
                source_execution: None,
                source_commit: None,
                source_revalidation: None,
                administration: None,
                outcome,
            },
        )
        .unwrap();
        assert_eq!(
            restore_mutation_outcome(
                f.base.scope(),
                &key,
                &request,
                value.digest(),
                &value.canonical_bytes(),
                Default::default()
            )
            .unwrap(),
            value
        );
        assert!(
            value.recovery_size().unwrap().bytes >= bytes.len() + value.canonical_bytes().len()
        );
    }
}

struct Backend(Mutex<SourceRecords>);
impl AuthorityBackend for Backend {
    fn access(
        &self,
        context: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure> {
        let mut records = self.0.lock().unwrap();
        if records.base.scope != *context.scope() {
            return Err(AuthorityFailure::ScopeDenied);
        }
        let limits = NativeRecordLimits::default();
        let mut state = AuthorityState::from_source_records(records.clone(), limits)
            .map_err(|_| AuthorityFailure::Integrity)?;
        evaluate(&mut state)?;
        if matches!(
            context.mode(),
            AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
        ) {
            *records = state
                .source_records(limits)
                .map_err(|_| AuthorityFailure::Integrity)?;
        }
        Ok(())
    }
}

#[test]
fn policy_original_replies_survive_new_heads_expiry_and_fresh_embedding_authentication() {
    let f = Fixture::new();
    let scope = f.base.scope();
    let backend = Arc::new(Backend(Mutex::new(SourceRecords::empty(scope.clone()))));
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .grant(
            PrincipalId::new("owner").unwrap(),
            scope.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    engine
        .register_repository(&owner, f.base.clone(), f.analysis.clone())
        .unwrap();
    let first_request = request(f.policy.clone(), None);
    let first = engine
        .mutate(&owner, scope, key("policy"), first_request.clone())
        .unwrap();
    assert_eq!(
        first.outcome,
        Ok(MutationValue::Policy {
            policy: f.policy.digest(),
            model_generation: 2
        })
    );
    let mut input = f.policy.input().clone();
    input.generation += 1;
    input.expires_at_ms += 1;
    let next = SourceExecutionPolicy::new(input).unwrap();
    let next_request = request(next.clone(), Some(f.policy.digest()));
    let rejected_request = request(next.clone(), None);
    let rejected = engine
        .mutate(&owner, scope, key("stale"), rejected_request.clone())
        .unwrap();
    assert_eq!(
        rejected.outcome,
        Err(EngineError::SourceAttempt(
            SourceAttemptError::PolicyChanged
        ))
    );
    let replaced = engine
        .mutate(&owner, scope, key("replace"), next_request.clone())
        .unwrap();
    assert_eq!(
        replaced.outcome,
        Ok(MutationValue::Policy {
            policy: next.digest(),
            model_generation: 3
        })
    );
    let noop = engine
        .mutate(&owner, scope, key("already-current"), next_request.clone())
        .unwrap();
    assert_eq!(noop.outcome, replaced.outcome);
    let (engine, admin) = Engine::with_authority_backend(backend.clone());
    let owner = admin
        .authenticate_existing_session(scope, 1, |_, _| Ok(()))
        .unwrap();
    for (operation, request, expected) in [
        ("policy", first_request, first),
        ("stale", rejected_request, rejected),
        ("replace", next_request, replaced),
    ] {
        let replay = engine
            .mutate(&owner, scope, key(operation), request)
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, expected.outcome);
        assert_eq!(replay.recorded_at_ms, expected.recorded_at_ms);
    }
    let state = backend.0.lock().unwrap().clone();
    assert_eq!(state.policy_head, Some(next.digest()));
    assert_eq!(state.base.repository.as_ref().unwrap().model.generation, 3);
    let mut expired = state.clone();
    expired.base.clock_ms = 50_000;
    assert!(AuthorityState::from_source_records(expired, Default::default()).is_ok());
    let mut invalid = state;
    let record = invalid
        .base
        .mutations
        .values_mut()
        .find(|record| matches!(record.outcome, Ok(MutationValue::Policy { .. })))
        .unwrap();
    record.outcome = Ok(MutationValue::Policy {
        policy: d("forged-policy"),
        model_generation: 2,
    });
    assert!(AuthorityState::from_source_records(invalid, Default::default()).is_err());
}
