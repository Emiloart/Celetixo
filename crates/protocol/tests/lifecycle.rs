use celetixo_protocol::{
    MAX_WIRE_BYTES, ProtocolError, celetixo::platform::v1::*, decode_lifecycle_event,
    validate_lifecycle_event,
};
use prost::Message;

fn fixtures() -> Vec<LifecycleEvent> {
    let base = LifecycleEvent {
        contract_version: 1,
        scope: Some(ScopeSelector {
            organization_id: "org".into(),
            workspace_id: "ws".into(),
            repository_id: "repo".into(),
        }),
        event_id: vec![1; 32],
        aggregate_kind: AggregateKind::Repository as i32,
        aggregate_id: "repo".into(),
        aggregate_sequence: 4,
        event_kind: LifecycleEventKind::CommitRecorded as i32,
        authority_payload_digest: vec![2; 32],
        trace_id: "trace".into(),
        commit: Some(CommitRecorded {
            transaction_id: "tx".into(),
            revision: 4,
            snapshot_digest: vec![3; 32],
            patch_digest: vec![4; 32],
            profile: CommitProfile::Source as i32,
        }),
        ..Default::default()
    };
    let mut tx = base.clone();
    tx.commit = None;
    tx.aggregate_kind = AggregateKind::Transaction as i32;
    tx.aggregate_id = "tx".into();
    tx.event_kind = LifecycleEventKind::TransactionStateChanged as i32;
    tx.transaction = Some(TransactionStateChanged {
        state: TransactionLifecycleState::Reserved as i32,
        attempt: 1,
        base_snapshot_digest: vec![5; 32],
        lease_fence: 1,
    });
    let mut source = tx.clone();
    source.transaction = None;
    source.aggregate_kind = AggregateKind::SourceAttempt as i32;
    source.aggregate_id = "source".into();
    source.event_kind = LifecycleEventKind::SourceAttemptStateChanged as i32;
    source.source_attempt = Some(SourceAttemptStateChanged {
        transaction_id: "tx".into(),
        transaction_attempt: 1,
        state: SourceLifecycleState::EvidenceSealed as i32,
        event_count: 5,
        admission_fence: 1,
    });
    let mut mutation = tx.clone();
    mutation.transaction = None;
    mutation.aggregate_kind = AggregateKind::Mutation as i32;
    mutation.aggregate_sequence = 0;
    mutation.event_kind = LifecycleEventKind::EngineMutationRecorded as i32;
    mutation.mutation = Some(EngineMutationRecorded {
        kind: EngineMutationKind::Propose as i32,
        request_digest: vec![6; 32],
        outcome_digest: vec![7; 32],
        transaction_id: "tx".into(),
    });
    let mut authority = base.clone();
    authority.commit = None;
    authority.aggregate_kind = AggregateKind::Authority as i32;
    authority.event_kind = LifecycleEventKind::RepositoryAuthorityChanged as i32;
    authority.authority = Some(RepositoryAuthorityChanged {
        revision: 4,
        snapshot_digest: vec![3; 32],
        metadata_digest: vec![8; 32],
    });
    vec![base, tx, source, mutation, authority]
}

#[test]
fn every_fact_roundtrips_and_preserves_full_sequence_range() {
    for mut event in fixtures() {
        if let Some(fact) = &mut event.transaction {
            fact.attempt = 0;
        }
        if let Some(fact) = &mut event.source_attempt {
            fact.transaction_attempt = 0;
        }
        if event.mutation.is_none() {
            event.aggregate_sequence = u64::MAX;
        }
        if let Some(fact) = &mut event.commit {
            fact.revision = event.aggregate_sequence;
        }
        let bytes = event.encode_to_vec();
        let validated = decode_lifecycle_event(&bytes).unwrap();
        assert_eq!(validated.event(), &event);
        assert_eq!(validated.canonical_bytes(), bytes);
    }
}

#[test]
fn conflicting_facts_missing_fields_and_mismatched_aggregate_are_rejected() {
    type Mutation = Box<dyn Fn(&mut LifecycleEvent)>;
    let changes: Vec<Mutation> = vec![
        Box::new(|e| e.scope = None),
        Box::new(|e| e.scope.as_mut().unwrap().workspace_id = "wrong scope".into()),
        Box::new(|e| e.event_id.pop().map(|_| ()).unwrap()),
        Box::new(|e| e.authority_payload_digest.clear()),
        Box::new(|e| e.trace_id.clear()),
        Box::new(|e| e.aggregate_id = "other".into()),
        Box::new(|e| e.aggregate_sequence += 1),
        Box::new(|e| e.aggregate_kind = AggregateKind::Transaction as i32),
        Box::new(|e| e.event_kind = LifecycleEventKind::TransactionStateChanged as i32),
        Box::new(|e| e.commit = None),
        Box::new(|e| e.transaction = fixtures()[1].transaction.clone()),
        Box::new(|e| e.commit.as_mut().unwrap().snapshot_digest.clear()),
        Box::new(|e| e.commit.as_mut().unwrap().profile = 0),
        Box::new(|e| e.commit.as_mut().unwrap().profile = 99),
    ];
    for change in changes {
        let mut event = fixtures().remove(0);
        change(&mut event);
        assert!(validate_lifecycle_event(&event).is_err());
        assert!(decode_lifecycle_event(&event.encode_to_vec()).is_err());
    }
    for mut event in fixtures() {
        event.contract_version = 2;
        assert_eq!(
            decode_lifecycle_event(&event.encode_to_vec()),
            Err(ProtocolError::UnsupportedVersion(2))
        );
    }
}

#[test]
fn wire_intake_rejects_unknown_duplicate_truncated_and_oversized_frames() {
    for event in fixtures() {
        let bytes = event.encode_to_vec();
        let mut unknown = bytes.clone();
        unknown.extend_from_slice(&[0x78, 1]); // unknown field 15
        assert_eq!(
            decode_lifecycle_event(&unknown),
            Err(ProtocolError::UnknownField(15))
        );
        let mut duplicate = bytes.clone();
        duplicate.extend_from_slice(&[8, 1]);
        assert_eq!(
            decode_lifecycle_event(&duplicate),
            Err(ProtocolError::DuplicateField(1))
        );
        let mut noncanonical = vec![8, 0x81, 0];
        noncanonical.extend_from_slice(&bytes[2..]);
        assert_eq!(
            decode_lifecycle_event(&noncanonical),
            Err(ProtocolError::MalformedWire)
        );
        assert!(decode_lifecycle_event(&bytes[..bytes.len() - 1]).is_err());
    }
    let mut event = fixtures().remove(1);
    for state in [0, 99, -1] {
        event.transaction.as_mut().unwrap().state = state;
        assert!(decode_lifecycle_event(&event.encode_to_vec()).is_err());
    }
    // Unknown nested semantics cannot be silently stripped by prost either.
    let mut bytes = fixtures().remove(0).encode_to_vec();
    bytes.extend_from_slice(&[0x62, 3, 0x98, 6, 1]);
    assert_eq!(
        decode_lifecycle_event(&bytes),
        Err(ProtocolError::UnknownField(99))
    );
    assert_eq!(
        decode_lifecycle_event(&vec![0; MAX_WIRE_BYTES + 1]),
        Err(ProtocolError::PayloadTooLarge)
    );
}
