//! Redundant SQL projections bind resolved source facts. The canonical original
//! request/outcome and complete checked source graph remain recovery authority.
use super::*;
use celetixo_engine::engine::MutationRecord;
use postgres::Row;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Columns {
    pub raw_attempt: Option<String>,
    pub raw_admission: Option<Vec<u8>>,
    pub resolved_attempt: Option<String>,
    pub admission: Option<Vec<u8>>,
    pub state_before: Option<String>,
    pub executor_before: Option<u64>,
    pub events_before: Option<u64>,
    pub events_after: Option<u64>,
    pub before_sequence: Option<u64>,
    pub effect_sequence: Option<u64>,
    pub result_state: Option<String>,
    pub result_sequence: Option<u64>,
    pub result_executor: Option<u64>,
}
impl Columns {
    pub(super) fn new(key: &MutationKey, record: &MutationRecord) -> SqlResult<Self> {
        let MutationRequest::SourceExecutor(request) = &record.request else {
            return if record.source_execution.is_none() {
                Ok(Self::default())
            } else {
                Err(AuthorityFailure::Integrity)
            };
        };
        let MutationActor::SourceExecutor { grant, .. } = key.actor else {
            return Err(AuthorityFailure::Integrity);
        };
        let context = record
            .source_execution
            .as_ref()
            .ok_or(AuthorityFailure::Integrity)?;
        let mut result = Self {
            raw_attempt: Some(request.binding.source_attempt.as_str().into()),
            raw_admission: Some(request.admission.0.as_bytes().to_vec()),
            resolved_attempt: context
                .admission
                .map(|_| request.binding.source_attempt.as_str().into()),
            admission: context.admission.map(|id| id.0.as_bytes().to_vec()),
            state_before: context.state_before.map(|state| state.name().into()),
            executor_before: context.executor_before,
            events_before: Some(context.events_before),
            events_after: Some(context.events_after),
            before_sequence: context.events_before.checked_sub(1),
            effect_sequence: (context.events_after > context.events_before)
                .then_some(context.events_before),
            ..Self::default()
        };
        match &record.outcome {
            Ok(MutationValue::SourceAttempt(value)) => {
                result.result_state = Some(value.state.name().into());
                result.result_sequence = context.events_after.checked_sub(1);
                result.result_executor = Some(grant);
            }
            Err(_) => {}
            _ => return Err(AuthorityFailure::Integrity),
        }
        Ok(result)
    }
    pub(super) fn load(row: &Row, offset: usize) -> SqlResult<Self> {
        Ok(Self {
            raw_attempt: row.get(offset),
            raw_admission: row.get(offset + 1),
            resolved_attempt: row.get(offset + 2),
            admission: row.get(offset + 3),
            state_before: row.get(offset + 4),
            executor_before: optional_number(row, offset + 5)?,
            events_before: optional_number(row, offset + 6)?,
            events_after: optional_number(row, offset + 7)?,
            before_sequence: optional_number(row, offset + 8)?,
            effect_sequence: optional_number(row, offset + 9)?,
            result_state: row.get(offset + 10),
            result_sequence: optional_number(row, offset + 11)?,
            result_executor: optional_number(row, offset + 12)?,
        })
    }
}
