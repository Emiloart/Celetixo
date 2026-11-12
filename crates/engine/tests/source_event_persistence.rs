use celetixo_engine::engine::{SourceAttemptEvent, SourceAttemptState};
use celetixo_engine::ids::*;
use celetixo_engine::persistence::*;
fn scope() -> Scope {
    Scope {
        organization: OrganizationId::new("o").unwrap(),
        workspace: WorkspaceId::new("w").unwrap(),
        repository: RepositoryId::new("r").unwrap(),
    }
}
fn id() -> SourceAttemptId {
    SourceAttemptId::new("sa").unwrap()
}
fn event() -> SourceAttemptEvent {
    SourceAttemptEvent {
        sequence: 0,
        at_ms: 10,
        from: SourceAttemptState::Proposed,
        to: SourceAttemptState::Admitted,
        reason: "admitted".into(),
    }
}
fn field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value);
}
#[test]
fn source_event_layout_is_frozen_and_cannot_move_between_contexts_or_namespaces() {
    let value = SourceEventRecord::new(SourceEventKind::Attempt, scope(), id(), event()).unwrap();
    let mut bytes = Vec::new();
    for v in [b"o".as_slice(), b"w", b"r", b"sa"] {
        field(&mut bytes, v);
    }
    field(&mut bytes, &0_u64.to_be_bytes());
    field(&mut bytes, &10_u64.to_be_bytes());
    for v in [b"Proposed".as_slice(), b"Admitted", b"admitted"] {
        field(&mut bytes, v);
    }
    assert_eq!(value.canonical_bytes(), bytes);
    assert_eq!(
        value.digest().to_string(),
        "2543d41cf6e5ac68002956331dddcadedff5b276b9c805495a703efa8c4b505c"
    );
    assert_eq!(
        restore_source_event(
            SourceEventKind::Attempt,
            &scope(),
            &id(),
            0,
            value.digest(),
            &bytes,
            Default::default()
        )
        .unwrap(),
        value
    );
    assert!(
        restore_source_event(
            SourceEventKind::Rejection,
            &scope(),
            &id(),
            0,
            value.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    assert!(
        restore_source_event(
            SourceEventKind::Attempt,
            &scope(),
            &id(),
            1,
            value.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    assert!(
        restore_source_event(
            SourceEventKind::Attempt,
            &scope(),
            &SourceAttemptId::new("other").unwrap(),
            0,
            value.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    let mut other = scope();
    other.organization = OrganizationId::new("other").unwrap();
    assert!(
        restore_source_event(
            SourceEventKind::Attempt,
            &other,
            &id(),
            0,
            value.digest(),
            &bytes,
            Default::default()
        )
        .is_err()
    );
    for length in 0..bytes.len() {
        let prefix = &bytes[..length];
        let digest = Digest::of(SourceEventKind::Attempt.format(), prefix);
        assert!(
            restore_source_event(
                SourceEventKind::Attempt,
                &scope(),
                &id(),
                0,
                digest,
                prefix,
                Default::default()
            )
            .is_err()
        );
    }
    let mut unknown = bytes.clone();
    let position = unknown
        .windows(b"Admitted".len())
        .position(|w| w == b"Admitted")
        .unwrap();
    unknown[position..position + 8].copy_from_slice(b"Unknown!");
    let digest = Digest::of(SourceEventKind::Attempt.format(), &unknown);
    assert!(matches!(
        restore_source_event(
            SourceEventKind::Attempt,
            &scope(),
            &id(),
            0,
            digest,
            &unknown,
            Default::default()
        ),
        Err(RestoreError::Encoding)
    ));
}
#[test]
fn rejected_engine_diagnostics_do_not_weaken_worker_event_limits_or_legal_edges() {
    let mut rejected = event();
    rejected.to = SourceAttemptState::Rejected;
    rejected.reason = "x".repeat(5000) + "\0diagnostic";
    let value = SourceEventRecord::new(SourceEventKind::Rejection, scope(), id(), rejected.clone())
        .unwrap();
    assert_eq!(
        restore_source_event(
            SourceEventKind::Rejection,
            &scope(),
            &id(),
            0,
            value.digest(),
            &value.canonical_bytes(),
            Default::default()
        )
        .unwrap()
        .event(),
        &rejected
    );
    let mut worker = rejected.clone();
    worker.to = SourceAttemptState::Admitted;
    assert!(SourceEventRecord::new(SourceEventKind::Attempt, scope(), id(), worker).is_err());
    let mut terminal = event();
    terminal.sequence = 1;
    terminal.from = SourceAttemptState::EvidenceSealed;
    assert!(SourceEventRecord::new(SourceEventKind::Attempt, scope(), id(), terminal).is_err());
    rejected.sequence = 1;
    assert!(SourceEventRecord::new(SourceEventKind::Rejection, scope(), id(), rejected).is_err());
    assert!(matches!(
        restore_source_event(
            SourceEventKind::Rejection,
            &scope(),
            &id(),
            0,
            value.digest(),
            &value.canonical_bytes(),
            DomainDecodeLimits {
                field_bytes: 128,
                ..Default::default()
            }
        ),
        Err(RestoreError::Limit)
    ));
}
