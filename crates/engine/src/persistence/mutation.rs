//! Complete original mutation inputs/outcomes. Parsing metadata never resolves
//! capabilities or reapplies a historical mutation against today's state.
use super::*;
use crate::engine::{
    EngineError, MutationActor, MutationKey, MutationRecord, MutationRequest, MutationValue,
};
use crate::ids::{Canonical, PrincipalId, TransactionId};

fn text<'a>(r: &mut Reader<'a>, maximum: usize) -> Result<&'a str, RestoreError> {
    let bytes = r.field()?;
    if bytes.len() > maximum {
        return Err(RestoreError::Limit);
    }
    std::str::from_utf8(bytes).map_err(|_| RestoreError::Encoding)
}
fn scope(r: &mut Reader<'_>, expected: &Scope) -> Result<(), RestoreError> {
    for id in [
        expected.organization.as_str(),
        expected.workspace.as_str(),
        expected.repository.as_str(),
    ] {
        if r.field()? != id.as_bytes() {
            return Err(RestoreError::BindingMismatch);
        }
    }
    Ok(())
}
fn actor(r: &mut Reader<'_>) -> Result<MutationActor, RestoreError> {
    let kind = text(r, 64)?;
    if kind == "EmbeddingAdministrator" {
        return Ok(MutationActor::EmbeddingAdministrator);
    }
    if !matches!(kind, "Session" | "SourceExecutor") {
        return Err(RestoreError::Encoding);
    }
    let principal = PrincipalId::new(text(r, 256)?).map_err(|_| RestoreError::Identity)?;
    let grant = r.number()?;
    if grant == 0 {
        return Err(RestoreError::InvalidDomainValue);
    }
    Ok(if kind == "Session" {
        MutationActor::Session { principal, grant }
    } else {
        MutationActor::SourceExecutor { principal, grant }
    })
}
pub fn restore_mutation_request(
    expected_scope: &Scope,
    expected_actor: &MutationActor,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<MutationRequest, RestoreError> {
    restore_request_inner(
        expected_scope,
        expected_actor,
        None,
        expected,
        bytes,
        limits,
    )
}

/// Context-aware restoration for requests whose operation identity contains an
/// external idempotency selector. No placeholder key or capability is created.
pub fn restore_mutation_request_for_key(
    expected_scope: &Scope,
    expected_key: &MutationKey,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<MutationRequest, RestoreError> {
    restore_request_inner(
        expected_scope,
        &expected_key.actor,
        Some(&expected_key.key),
        expected,
        bytes,
        limits,
    )
}

fn restore_request_inner(
    expected_scope: &Scope,
    expected_actor: &MutationActor,
    operation_key: Option<&crate::ids::IdempotencyKey>,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<MutationRequest, RestoreError> {
    let mut r = Reader::new(expected, MutationRequest::FORMAT, bytes, limits)?;
    scope(&mut r, expected_scope)?;
    if actor(&mut r)? != *expected_actor {
        return Err(RestoreError::BindingMismatch);
    }
    let kind = text(&mut r, 64)?;
    if kind == "CommitSource" {
        let key = operation_key
            .ok_or(RestoreError::UnsupportedDomainValue)?
            .clone();
        let request =
            MutationRequest::SourceCommit(Box::new(source_commit::decode_request(&mut r, key)?));
        r.end()?;
        lifecycle::canonical(
            request.digest(expected_scope, expected_actor),
            expected,
            &request.canonical_bytes(expected_scope, expected_actor),
            bytes,
        )?;
        return Ok(request);
    }
    if source_mutation::is_kind(kind) {
        let request =
            MutationRequest::SourceExecutor(Box::new(source_mutation::request(&mut r, kind)?));
        r.end()?;
        lifecycle::canonical(
            request.digest(expected_scope, expected_actor),
            expected,
            &request.canonical_bytes(expected_scope, expected_actor),
            bytes,
        )?;
        return Ok(request);
    }
    if administrative_mutation::is_kind(kind) {
        let request = MutationRequest::Administrative(Box::new(administrative_mutation::decode(
            &mut r, kind,
        )?));
        r.end()?;
        lifecycle::canonical(
            request.digest(expected_scope, expected_actor),
            expected,
            &request.canonical_bytes(expected_scope, expected_actor),
            bytes,
        )?;
        return Ok(request);
    }
    if matches!(
        kind,
        "Register" | "UpdateModel" | "RegisterRepository" | "InstallAnalysis" | "InstallPolicy"
    ) {
        use crate::engine::RepositoryMutation;
        let request = match kind {
            "InstallPolicy" => {
                let expected = match r.number()? {
                    0 => None,
                    1 => Some(r.digest()?),
                    _ => return Err(RestoreError::Encoding),
                };
                RepositoryMutation::InstallPolicy {
                    expected,
                    policy: Box::new(original_policy::decode_original_policy(&mut r)?),
                }
            }
            "Register" => RepositoryMutation::Register {
                snapshot: Box::new(decode_original_snapshot(&mut r)?),
                model: Box::new(semantic::decode_model_fields(&mut r)?),
            },
            "RegisterRepository" => RepositoryMutation::RegisterRepository {
                snapshot: Box::new(decode_original_snapshot(&mut r)?),
                analysis: Box::new(decode_original_analysis(&mut r)?),
            },
            "InstallAnalysis" => RepositoryMutation::InstallAnalysis {
                expected: r.digest()?,
                analysis: Box::new(decode_original_analysis(&mut r)?),
            },
            _ => RepositoryMutation::UpdateModel {
                model: Box::new(semantic::decode_model_fields(&mut r)?),
            },
        };
        r.end()?;
        let request = MutationRequest::Repository(Box::new(request));
        lifecycle::canonical(
            request.digest(expected_scope, expected_actor),
            expected,
            &request.canonical_bytes(expected_scope, expected_actor),
            bytes,
        )?;
        return Ok(request);
    }
    if !matches!(
        kind,
        "Propose"
            | "Analyze"
            | "Reserve"
            | "Renew"
            | "Execute"
            | "Abort"
            | "SubmitPatch"
            | "RecordEvidence"
            | "Revalidate"
    ) {
        return Err(RestoreError::Encoding);
    }
    let transaction = TransactionId::new(text(&mut r, 256)?).map_err(|_| RestoreError::Identity)?;
    // Invalid TTL and blank cancellation reasons remain original input data.
    let request = match kind {
        "SubmitPatch" => MutationRequest::SubmitPatch {
            transaction,
            patch: decode_original_patch(&mut r)?,
        },
        "RecordEvidence" => {
            let base = crate::ids::SnapshotId(r.digest()?);
            MutationRequest::RecordEvidence {
                transaction,
                evidence: Box::new(lifecycle::decode_verification_fields(&mut r, base)?),
            }
        }
        "Revalidate" => MutationRequest::Revalidate { transaction },
        "Propose" => MutationRequest::Propose {
            transaction,
            base: crate::ids::SnapshotId(r.digest()?),
            intent: Box::new(lifecycle::decode_intent(&mut r)?),
        },
        "Analyze" => MutationRequest::Analyze { transaction },
        "Reserve" => MutationRequest::Reserve {
            transaction,
            ttl_ms: r.number()?,
        },
        "Renew" => MutationRequest::Renew {
            transaction,
            ttl_ms: r.number()?,
        },
        "Execute" => MutationRequest::Execute { transaction },
        "Abort" => MutationRequest::Abort {
            transaction,
            reason: r.text()?,
        },
        _ => return Err(RestoreError::UnsupportedDomainValue),
    };
    r.end()?;
    lifecycle::canonical(
        request.digest(expected_scope, expected_actor),
        expected,
        &request.canonical_bytes(expected_scope, expected_actor),
        bytes,
    )?;
    Ok(request)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationOutcomeRecord {
    scope: Scope,
    key: MutationKey,
    record: MutationRecord,
}
impl MutationOutcomeRecord {
    pub const FORMAT: &str = "celetixo/engine-mutation-outcome/v1";
    pub fn new(
        scope: Scope,
        key: MutationKey,
        record: MutationRecord,
    ) -> Result<Self, RestoreError> {
        record.check_shape(&key.actor)?;
        if let MutationRequest::SourceCommit(request) = &record.request {
            if request.key != key.key {
                return Err(RestoreError::BindingMismatch);
            }
            if let Ok(MutationValue::SourceCommitted(receipt)) = &record.outcome {
                let MutationActor::Session { principal, .. } = &key.actor else {
                    return Err(RestoreError::BindingMismatch);
                };
                receipt.validate_request(&scope, principal, request)?;
            }
        }
        if let Err(error) = &record.outcome {
            source_errors::encode_engine_error(&mut Canonical::default(), error)?;
        }
        Ok(Self { scope, key, record })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn key(&self) -> &MutationKey {
        &self.key
    }
    pub fn record(&self) -> &MutationRecord {
        &self.record
    }
    /// Nonallocating bound for owned normalized recovery metadata.
    pub fn recovery_size(&self) -> Result<RecoverySize, RestoreError> {
        self.record.recovery_size(&self.scope, &self.key)
    }
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(self.scope.organization.as_str());
        c.str(self.scope.workspace.as_str());
        c.str(self.scope.repository.as_str());
        self.key.actor.encode(&mut c);
        c.str(self.key.key.as_str());
        c.digest(self.record.request.digest(&self.scope, &self.key.actor));
        c.u64(self.record.recorded_at_ms);
        match self.record.resolved_attempt {
            None => c.u64(0),
            Some(a) => {
                c.u64(1);
                c.u64(a);
            }
        }
        match self.record.state_before {
            None => c.u64(0),
            Some(s) => {
                c.u64(1);
                outcome::encode_state(&mut c, s);
            }
        }
        c.u64(self.record.lease_events_before);
        if let Some(context) = &self.record.administration {
            c.u64(context.counter_before);
            c.u64(match context.target_active_before {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            });
        }
        if let Some(context) = &self.record.source_execution {
            source_mutation::encode_context(&mut c, context);
        }
        if let Some(context) = &self.record.source_commit {
            source_commit::encode_context(&mut c, context);
        }
        match &self.record.outcome {
            Ok(MutationValue::SourceCommitted(receipt)) => {
                c.str("Accepted");
                c.str("SourceCommitted");
                receipt.encode_fields(&mut c);
            }
            Ok(MutationValue::SourceAttempt(value)) => {
                c.str("Accepted");
                c.str("SourceAttempt");
                source_mutation::encode_result(&mut c, value);
            }
            Ok(MutationValue::Allocation { resource, id })
            | Ok(MutationValue::Revocation { resource, id }) => {
                c.str("Accepted");
                c.str(
                    if matches!(self.record.outcome, Ok(MutationValue::Allocation { .. })) {
                        "Allocation"
                    } else {
                        "Revocation"
                    },
                );
                c.str(resource.name());
                c.u64(*id);
            }
            Ok(MutationValue::Policy {
                policy,
                model_generation,
            }) => {
                c.str("Accepted");
                c.str("Policy");
                c.digest(*policy);
                c.u64(*model_generation);
            }
            Ok(MutationValue::Repository { snapshot, model }) => {
                c.str("Accepted");
                c.str("Repository");
                c.digest(snapshot.0);
                semantic::encode_model_fields(&mut c, model);
            }
            Ok(MutationValue::Bindings { bindings }) => {
                c.str("Accepted");
                c.str("Bindings");
                bindings.encode(&mut c);
            }
            Ok(MutationValue::Evidence { evidence }) => {
                c.str("Accepted");
                c.str("Evidence");
                c.digest(evidence.0);
            }
            Ok(MutationValue::Analysis { attempt, accesses }) => {
                c.str("Accepted");
                c.str("Analysis");
                c.u64(*attempt);
                accesses.encode(&mut c);
            }
            Ok(MutationValue::Lease {
                expires_at_ms,
                fence,
                acquisition_attempt,
            }) => {
                c.str("Accepted");
                c.str("Lease");
                c.u64(*expires_at_ms);
                c.u64(*fence);
                c.u64(*acquisition_attempt);
            }
            Ok(MutationValue::State { state, attempt }) => {
                c.str("Accepted");
                c.str("State");
                outcome::encode_state(&mut c, *state);
                c.u64(*attempt);
            }
            Err(error) => {
                c.str("Rejected");
                c.str("Native");
                source_errors::encode_engine_error(&mut c, error)
                    .expect("checked durable mutation error");
            }
        }
        c.into_bytes()
    }
}

fn mutation_error(r: &mut Reader<'_>) -> Result<EngineError, RestoreError> {
    // Reject unsupported families before allocating nested error collections.
    if text(r, 64)? != "Native" {
        return Err(RestoreError::UnsupportedDomainValue);
    }
    source_errors::decode_engine_error(r)
}

fn decode_original_patch(r: &mut Reader<'_>) -> Result<crate::patch::Patch, RestoreError> {
    let expected = r.digest()?;
    let bytes = r.field()?;
    let mut nested = Reader::new(expected, "celetixo/patch/v1", bytes, r.limits)?;
    let patch = lifecycle::decode_patch(&mut nested)?;
    nested.end()?;
    r.entries = r
        .entries
        .checked_add(nested.entries)
        .ok_or(RestoreError::Limit)?;
    if r.entries > r.limits.collection_entries {
        return Err(RestoreError::Limit);
    }
    lifecycle::canonical(patch.id().0, expected, &patch.canonical_bytes(), bytes)?;
    Ok(patch)
}
fn decode_bindings(r: &mut Reader<'_>) -> Result<crate::evidence::EvidenceBindings, RestoreError> {
    Ok(crate::evidence::EvidenceBindings {
        transaction: TransactionId::new(text(r, 256)?).map_err(|_| RestoreError::Identity)?,
        attempt: r.number()?,
        base_snapshot: crate::ids::SnapshotId(r.digest()?),
        candidate_snapshot: crate::ids::SnapshotId(r.digest()?),
        patch: crate::ids::PatchId(r.digest()?),
        analysis: r.digest()?,
    })
}

pub fn restore_mutation_outcome(
    expected_scope: &Scope,
    expected_key: &MutationKey,
    request: &MutationRequest,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<MutationOutcomeRecord, RestoreError> {
    let mut r = Reader::new(expected, MutationOutcomeRecord::FORMAT, bytes, limits)?;
    scope(&mut r, expected_scope)?;
    if actor(&mut r)? != expected_key.actor
        || r.field()? != expected_key.key.as_str().as_bytes()
        || r.digest()? != request.digest(expected_scope, &expected_key.actor)
    {
        return Err(RestoreError::BindingMismatch);
    }
    let recorded_at_ms = r.number()?;
    let resolved_attempt = match r.number()? {
        0 => None,
        1 => Some(r.number()?),
        _ => return Err(RestoreError::Encoding),
    };
    let state_before = match r.number()? {
        0 => None,
        1 => Some(outcome::decode_state(&mut r)?),
        _ => return Err(RestoreError::Encoding),
    };
    let lease_events_before = r.number()?;
    let administration = if matches!(request, MutationRequest::Administrative(_)) {
        Some(crate::engine::AdministrativeContext {
            counter_before: r.number()?,
            target_active_before: match r.number()? {
                0 => None,
                1 => Some(false),
                2 => Some(true),
                _ => return Err(RestoreError::Encoding),
            },
        })
    } else {
        None
    };
    let source_execution = if matches!(request, MutationRequest::SourceExecutor(_)) {
        Some(source_mutation::context(&mut r)?)
    } else {
        None
    };
    let source_commit = if matches!(request, MutationRequest::SourceCommit(_)) {
        Some(Box::new(source_commit::decode_context(&mut r)?))
    } else {
        None
    };
    let outcome = match text(&mut r, 64)? {
        "Accepted" => Ok(match text(&mut r, 64)? {
            "SourceCommitted" => {
                MutationValue::SourceCommitted(Box::new(source_commit::decode_receipt(&mut r)?))
            }
            "SourceAttempt" => {
                MutationValue::SourceAttempt(Box::new(source_mutation::result(&mut r)?))
            }
            "Allocation" => MutationValue::Allocation {
                resource: administrative_mutation::resource(&mut r)?,
                id: r.number()?,
            },
            "Revocation" => MutationValue::Revocation {
                resource: administrative_mutation::resource(&mut r)?,
                id: r.number()?,
            },
            "Policy" => MutationValue::Policy {
                policy: r.digest()?,
                model_generation: r.number()?,
            },
            "Repository" => MutationValue::Repository {
                snapshot: SnapshotId(r.digest()?),
                model: Box::new(semantic::decode_model_fields(&mut r)?),
            },
            "Bindings" => MutationValue::Bindings {
                bindings: decode_bindings(&mut r)?,
            },
            "Evidence" => MutationValue::Evidence {
                evidence: crate::ids::EvidenceId(r.digest()?),
            },
            "Analysis" => MutationValue::Analysis {
                attempt: r.number()?,
                accesses: lifecycle::decode_access(&mut r)?,
            },
            "Lease" => MutationValue::Lease {
                expires_at_ms: r.number()?,
                fence: r.number()?,
                acquisition_attempt: r.number()?,
            },
            "State" => MutationValue::State {
                state: outcome::decode_state(&mut r)?,
                attempt: r.number()?,
            },
            _ => return Err(RestoreError::Encoding),
        }),
        "Rejected" => Err(mutation_error(&mut r)?),
        _ => return Err(RestoreError::Encoding),
    };
    r.end()?;
    let record = MutationOutcomeRecord::new(
        expected_scope.clone(),
        expected_key.clone(),
        MutationRecord {
            request: request.clone(),
            recorded_at_ms,
            resolved_attempt,
            state_before,
            lease_events_before,
            administration,
            source_execution,
            source_commit,
            outcome,
        },
    )?;
    lifecycle::canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

fn decode_original_snapshot(r: &mut Reader<'_>) -> Result<Snapshot, RestoreError> {
    let expected = r.digest()?;
    let bytes = r.field()?;
    let mut nested = Reader::new(expected, "celetixo/snapshot/v1", bytes, r.limits)?;
    nested.entries = r.entries;
    let snapshot = decode_snapshot_fields(&mut nested)?;
    nested.end()?;
    r.entries = nested.entries;
    lifecycle::canonical(
        snapshot.id().0,
        expected,
        &snapshot.canonical_bytes(),
        bytes,
    )?;
    Ok(snapshot)
}
fn decode_original_analysis(
    r: &mut Reader<'_>,
) -> Result<crate::repository::RepositoryAnalysis, RestoreError> {
    let expected = r.digest()?;
    let bytes = r.field()?;
    let mut nested = Reader::new(expected, "celetixo/repository-analysis/v1", bytes, r.limits)?;
    nested.entries = r.entries;
    let analysis = semantic::decode_original_analysis(&mut nested)?;
    nested.end()?;
    r.entries = nested.entries;
    lifecycle::canonical(
        analysis.digest(),
        expected,
        &analysis.canonical_bytes(),
        bytes,
    )?;
    Ok(analysis)
}
