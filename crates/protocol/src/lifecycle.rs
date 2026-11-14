//! Bounded lifecycle notifications. Validation confers no producer identity or
//! authority; consumers must authenticate and reconcile the referenced SQL fact.
use crate::celetixo::platform::v1::{
    AggregateKind, CommitProfile, EngineMutationKind, LifecycleEvent, LifecycleEventKind,
    SourceLifecycleState, TransactionLifecycleState,
};
use crate::intake::{CONTRACT_VERSION, ProtocolError, id, scan_lifecycle};
use prost::Message;

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedLifecycleEvent {
    event: LifecycleEvent,
}

impl ValidatedLifecycleEvent {
    pub fn event(&self) -> &LifecycleEvent {
        &self.event
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.event.encode_to_vec()
    }
}

pub fn decode_lifecycle_event(bytes: &[u8]) -> Result<ValidatedLifecycleEvent, ProtocolError> {
    scan_lifecycle(bytes)?;
    let event = LifecycleEvent::decode(bytes).map_err(|_| ProtocolError::MalformedWire)?;
    validate_lifecycle_event(&event)?;
    Ok(ValidatedLifecycleEvent { event })
}

fn require(condition: bool, field: &'static str) -> Result<(), ProtocolError> {
    if condition {
        Ok(())
    } else {
        Err(ProtocolError::InvalidField(field))
    }
}

fn digest(bytes: &[u8], field: &'static str) -> Result<(), ProtocolError> {
    require(bytes.len() == 32, field)
}

/// Validate generated types too, so a locally constructed frame cannot bypass
/// the same semantic rules used by the strict wire consumer.
pub fn validate_lifecycle_event(event: &LifecycleEvent) -> Result<(), ProtocolError> {
    if event.contract_version != CONTRACT_VERSION {
        return Err(ProtocolError::UnsupportedVersion(event.contract_version));
    }
    let scope = event
        .scope
        .as_ref()
        .ok_or(ProtocolError::InvalidField("scope"))?;
    id(&scope.organization_id, "scope.organization_id")?;
    id(&scope.workspace_id, "scope.workspace_id")?;
    id(&scope.repository_id, "scope.repository_id")?;
    id(&event.aggregate_id, "aggregate_id")?;
    id(&event.trace_id, "trace_id")?;
    digest(&event.event_id, "event_id")?;
    digest(&event.authority_payload_digest, "authority_payload_digest")?;
    let aggregate = AggregateKind::try_from(event.aggregate_kind)
        .map_err(|_| ProtocolError::InvalidField("aggregate_kind"))?;
    let kind = LifecycleEventKind::try_from(event.event_kind)
        .map_err(|_| ProtocolError::InvalidField("event_kind"))?;
    require(
        [
            event.commit.is_some(),
            event.transaction.is_some(),
            event.source_attempt.is_some(),
            event.mutation.is_some(),
            event.authority.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            == 1,
        "exactly_one_fact",
    )?;
    match kind {
        LifecycleEventKind::CommitRecorded => {
            let fact = event
                .commit
                .as_ref()
                .ok_or(ProtocolError::InvalidField("commit"))?;
            require(
                aggregate == AggregateKind::Repository && event.aggregate_id == scope.repository_id,
                "commit.aggregate",
            )?;
            id(&fact.transaction_id, "commit.transaction_id")?;
            require(
                fact.revision > 0 && fact.revision == event.aggregate_sequence,
                "commit.revision",
            )?;
            digest(&fact.snapshot_digest, "commit.snapshot_digest")?;
            digest(&fact.patch_digest, "commit.patch_digest")?;
            require(
                matches!(
                    CommitProfile::try_from(fact.profile),
                    Ok(CommitProfile::Native | CommitProfile::Source)
                ),
                "commit.profile",
            )?;
        }
        LifecycleEventKind::TransactionStateChanged => {
            let fact = event
                .transaction
                .as_ref()
                .ok_or(ProtocolError::InvalidField("transaction"))?;
            require(
                aggregate == AggregateKind::Transaction && event.aggregate_sequence > 0,
                "transaction.aggregate",
            )?;
            require(
                matches!(TransactionLifecycleState::try_from(fact.state), Ok(state) if state != TransactionLifecycleState::Unspecified),
                "transaction.state",
            )?;
            digest(
                &fact.base_snapshot_digest,
                "transaction.base_snapshot_digest",
            )?;
        }
        LifecycleEventKind::SourceAttemptStateChanged => {
            let fact = event
                .source_attempt
                .as_ref()
                .ok_or(ProtocolError::InvalidField("source_attempt"))?;
            require(
                aggregate == AggregateKind::SourceAttempt && event.aggregate_sequence > 0,
                "source_attempt.aggregate",
            )?;
            id(&fact.transaction_id, "source_attempt.transaction_id")?;
            require(
                fact.event_count > 0 && fact.admission_fence > 0,
                "source_attempt.epoch",
            )?;
            require(
                matches!(SourceLifecycleState::try_from(fact.state), Ok(state) if state != SourceLifecycleState::Unspecified),
                "source_attempt.state",
            )?;
        }
        LifecycleEventKind::EngineMutationRecorded => {
            let fact = event
                .mutation
                .as_ref()
                .ok_or(ProtocolError::InvalidField("mutation"))?;
            require(
                aggregate == AggregateKind::Mutation && event.aggregate_sequence == 0,
                "mutation.aggregate",
            )?;
            let mutation = EngineMutationKind::try_from(fact.kind)
                .map_err(|_| ProtocolError::InvalidField("mutation.kind"))?;
            require(mutation != EngineMutationKind::Unspecified, "mutation.kind")?;
            if matches!(
                mutation,
                EngineMutationKind::Administrative | EngineMutationKind::Repository
            ) {
                require(fact.transaction_id.is_empty(), "mutation.transaction_id")?;
            } else {
                id(&fact.transaction_id, "mutation.transaction_id")?;
            }
            digest(&fact.request_digest, "mutation.request_digest")?;
            digest(&fact.outcome_digest, "mutation.outcome_digest")?;
        }
        LifecycleEventKind::RepositoryAuthorityChanged => {
            let fact = event
                .authority
                .as_ref()
                .ok_or(ProtocolError::InvalidField("authority"))?;
            require(
                aggregate == AggregateKind::Authority
                    && event.aggregate_id == scope.repository_id
                    && event.aggregate_sequence > 0,
                "authority.aggregate",
            )?;
            require(
                fact.snapshot_digest.len() == 32
                    || (fact.revision == 0 && fact.snapshot_digest.is_empty()),
                "authority.snapshot_digest",
            )?;
            digest(&fact.metadata_digest, "authority.metadata_digest")?;
        }
        LifecycleEventKind::Unspecified => return Err(ProtocolError::InvalidField("event_kind")),
    }
    Ok(())
}
