use super::*;

pub(super) fn publish(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    attempts(tx, before, after)?;
    operations(tx, before, after)?;
    rejections(tx, before, after)?;
    evidence(tx, before, after)?;
    let scope = &after.base.scope;
    for (key, o) in &after.unprepared_commits {
        if before.unprepared_commits.contains_key(key) {
            continue;
        }
        let outcome = CommitOutcomeRecord::new(
            scope.clone(),
            o.transaction.clone(),
            o.attempt,
            o.outcome.clone(),
        )
        .map_err(restore)?;
        put(
            tx,
            scope,
            outcome.digest(),
            CommitOutcomeRecord::FORMAT,
            &outcome.canonical_bytes(),
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.source_unprepared_commits (organization_id,workspace_id,repository_id,idempotency_key,transaction_id,attempt_number,outcome_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text,$6::text::numeric,$7::bytea)",
            &[
                &key.as_str(),
                &o.transaction.as_str(),
                &o.attempt.to_string(),
                &&outcome.digest().as_bytes()[..],
            ],
        )?;
    }
    Ok(())
}
fn attempts(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    for (id, a) in &after.attempts {
        let previous = before.attempts.get(id);
        if previous == Some(a) {
            continue;
        }
        let last = a.events.last().ok_or(AuthorityFailure::Integrity)?;
        let state = SourceEventRecord::new(
            SourceEventKind::Attempt,
            scope.clone(),
            id.clone(),
            last.clone(),
        )
        .map_err(restore)?;
        if state.event().to != a.state {
            return Err(AuthorityFailure::Integrity);
        }
        let executor = a.executor_grant.map(|id| id.to_string());
        if previous.is_none() {
            let request =
                SourceRequestRecord::new(a.patch.clone(), a.options.clone()).map_err(restore)?;
            let b = a.admission.binding();
            let base = after
                .base
                .repository
                .as_ref()
                .and_then(|r| r.registered.get(&b.base_snapshot))
                .ok_or(AuthorityFailure::Integrity)?;
            let application = a
                .patch
                .apply(base)
                .map_err(|_| AuthorityFailure::Integrity)?;
            put(
                tx,
                scope,
                a.admission.id().0,
                SourceAdmissionRecord::FORMAT,
                &a.admission.canonical_bytes(),
            )?;
            put(
                tx,
                scope,
                a.patch.id().0,
                "celetixo/source-patch/v1",
                &a.patch.canonical_bytes(),
            )?;
            put(
                tx,
                scope,
                application.report().digest(),
                "celetixo/source-application-report/v1",
                &application.report().canonical_bytes(),
            )?;
            put(
                tx,
                scope,
                a.admission.plan().id().0,
                VerificationPlanRecord::FORMAT,
                &a.admission.plan().canonical_bytes(),
            )?;
            put(
                tx,
                scope,
                request.digest(),
                SourceRequestRecord::FORMAT,
                &request.canonical_bytes(),
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_attempts (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id,transaction_attempt,admission_digest,admission_payload_digest,patch_digest,patch_payload_digest,application_payload_digest,owner_grant_id,executor_grant_id,state) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text,$6::text::numeric,$7::bytea,$7::bytea,$8::bytea,$8::bytea,$9::bytea,$10::text::numeric,$11::text::numeric,$12)",
                &[
                    &id.as_str(),
                    &b.transaction.as_str(),
                    &b.transaction_attempt.to_string(),
                    &&a.admission.id().0.as_bytes()[..],
                    &&a.patch.id().0.as_bytes()[..],
                    &&application.report().digest().as_bytes()[..],
                    &a.owner_grant.to_string(),
                    &executor,
                    &state.to_label(),
                ],
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_admission_contexts (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id,request_payload_digest,admission_fence,lease_event_count) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text,$6::bytea,$7::text::numeric,$8::text::numeric)",
                &[
                    &id.as_str(),
                    &b.transaction.as_str(),
                    &&request.digest().as_bytes()[..],
                    &a.admission_fence.to_string(),
                    &a.admission_lease_event_count.to_string(),
                ],
            )?;
        } else {
            one(execute(
                tx,
                scope,
                &format!(
                    "UPDATE celetixo.source_attempts SET state=$5,executor_grant_id=$6::text::numeric WHERE {FILTER} AND source_attempt_id=$4::text"
                ),
                &[&id.as_str(), &state.to_label(), &executor],
            )?)?;
        }
        for event in a.events.iter().skip(previous.map_or(0, |p| p.events.len())) {
            let event = SourceEventRecord::new(
                SourceEventKind::Attempt,
                scope.clone(),
                id.clone(),
                event.clone(),
            )
            .map_err(restore)?;
            put(
                tx,
                scope,
                event.digest(),
                event.kind().format(),
                &event.canonical_bytes(),
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_attempt_events (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,at_ms,from_state,to_state,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6::text::numeric,$7,$8,$9::bytea)",
                &[
                    &id.as_str(),
                    &event.event().sequence.to_string(),
                    &event.event().at_ms.to_string(),
                    &event.from_label(),
                    &event.to_label(),
                    &&event.digest().as_bytes()[..],
                ],
            )?;
        }
        if a.state.is_terminal() && previous.is_none_or(|p| !p.state.is_terminal()) {
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_attempt_outcomes (organization_id,workspace_id,repository_id,source_attempt_id,terminal_state,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5,$6::bytea)",
                &[
                    &id.as_str(),
                    &state.to_label(),
                    &&state.digest().as_bytes()[..],
                ],
            )?;
        }
    }
    Ok(())
}
fn operations(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    for (key, o) in &after.operations {
        if before.operations.contains_key(key) {
            continue;
        }
        let b = o.request.patch().binding();
        let outcome = SourceOutcomeRecord::new(
            SourceOutcomeContext {
                scope: scope.clone(),
                transaction: b.transaction.clone(),
                target_attempt: o.target_attempt,
                key: key.clone(),
                request: o.request.request_digest(),
            },
            o.outcome.clone(),
        )
        .map_err(restore)?;
        put(
            tx,
            scope,
            o.request.digest(),
            SourceRequestRecord::FORMAT,
            &o.request.canonical_bytes(),
        )?;
        put(
            tx,
            scope,
            outcome.digest(),
            SourceOutcomeRecord::FORMAT,
            &outcome.canonical_bytes(),
        )?;
        let kind = if o.outcome.is_ok() {
            "Accepted"
        } else {
            "Rejected"
        };
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.operations (organization_id,workspace_id,repository_id,operation_kind,idempotency_key,request_digest,transaction_id,attempt_number,outcome_kind,outcome_payload_digest,request_payload_digest) VALUES ($1::text,$2::text,$3::text,'SourceAdmission',$4::text,$5::bytea,$6::text,$7::text::numeric,$8,$9::bytea,$10::bytea)",
            &[
                &key.as_str(),
                &&o.request.request_digest().as_bytes()[..],
                &b.transaction.as_str(),
                &o.target_attempt.to_string(),
                &kind,
                &&outcome.digest().as_bytes()[..],
                &&o.request.digest().as_bytes()[..],
            ],
        )?;
        let fence = o.fence.map(|v| v.to_string());
        let analysis = o.observed_analysis.map(|v| v.as_bytes().to_vec());
        let policy = o.observed_policy.map(|v| v.as_bytes().to_vec());
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.source_operation_contexts (organization_id,workspace_id,repository_id,idempotency_key,transaction_id,owner_grant_id,recorded_at_ms,fence_generation,lease_event_count,observed_analysis,observed_policy,observed_model_generation) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text,$6::text::numeric,$7::text::numeric,$8::text::numeric,$9::text::numeric,$10::bytea,$11::bytea,$12::text::numeric)",
            &[
                &key.as_str(),
                &b.transaction.as_str(),
                &o.owner_grant.to_string(),
                &o.recorded_at_ms.to_string(),
                &fence,
                &o.lease_event_count.to_string(),
                &analysis,
                &policy,
                &o.observed_model_generation.to_string(),
            ],
        )?;
    }
    Ok(())
}
fn rejections(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    for (id, r) in &after.rejections {
        if before.rejections.contains_key(id) {
            continue;
        }
        let mut matching = after.operations.iter().filter(|(_, o)| {
            o.request.patch().binding().source_attempt == *id
                && o.request.request_digest() == r.request
                && o.outcome == Err(r.error.clone())
                && o.recorded_at_ms == r.rejected_at_ms
        });
        let (key, o) = matching.next().ok_or(AuthorityFailure::Integrity)?;
        if matching.next().is_some() {
            return Err(AuthorityFailure::Integrity);
        }
        let outcome = SourceOutcomeRecord::new(
            SourceOutcomeContext {
                scope: scope.clone(),
                transaction: r.transaction.clone(),
                target_attempt: o.target_attempt,
                key: key.clone(),
                request: r.request,
            },
            o.outcome.clone(),
        )
        .map_err(restore)?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.source_rejections (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id,transaction_attempt,request_digest,rejection_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text,$6::text::numeric,$7::bytea,$8::bytea)",
            &[
                &id.as_str(),
                &r.transaction.as_str(),
                &r.transaction_attempt.to_string(),
                &&r.request.as_bytes()[..],
                &&outcome.digest().as_bytes()[..],
            ],
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.source_rejection_contexts (organization_id,workspace_id,repository_id,source_attempt_id,transaction_id,resolved_attempt,operation_kind,idempotency_key) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text,$6::text::numeric,'SourceAdmission',$7::text)",
            &[
                &id.as_str(),
                &r.transaction.as_str(),
                &o.target_attempt.to_string(),
                &key.as_str(),
            ],
        )?;
        for event in &r.events {
            let event = SourceEventRecord::new(
                SourceEventKind::Rejection,
                scope.clone(),
                id.clone(),
                event.clone(),
            )
            .map_err(restore)?;
            put(
                tx,
                scope,
                event.digest(),
                event.kind().format(),
                &event.canonical_bytes(),
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_rejection_events (organization_id,workspace_id,repository_id,source_attempt_id,event_sequence,at_ms,from_state,to_state,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6::text::numeric,$7,$8,$9::bytea)",
                &[
                    &id.as_str(),
                    &event.event().sequence.to_string(),
                    &event.event().at_ms.to_string(),
                    &event.from_label(),
                    &event.to_label(),
                    &&event.digest().as_bytes()[..],
                ],
            )?;
        }
    }
    Ok(())
}
fn evidence(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    for (id, e) in &after.evidence {
        if before.evidence.contains_key(id) {
            continue;
        }
        let attempt = after
            .attempts
            .values()
            .find(|a| a.admission.id() == e.receipt.admission)
            .ok_or(AuthorityFailure::Integrity)?;
        let b = attempt.admission.binding();
        let executor = attempt.executor_grant.ok_or(AuthorityFailure::Integrity)?;
        put(
            tx,
            scope,
            id.0,
            celetixo_engine::source_evidence::SOURCE_EVIDENCE_FORMAT,
            &e.evidence.canonical_bytes(),
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.evidence (organization_id,workspace_id,repository_id,evidence_digest,evidence_kind,transaction_id,attempt_number,evidence_payload_digest,verifier_grant_kind,verifier_grant_id) VALUES ($1::text,$2::text,$3::text,$4::bytea,'SourceV2',$5::text,$6::text::numeric,$4::bytea,'executor',$7::text::numeric)",
            &[
                &&id.0.as_bytes()[..],
                &b.transaction.as_str(),
                &b.transaction_attempt.to_string(),
                &executor.to_string(),
            ],
        )?;
        let receipt = SourceEvidenceReceiptRecord::new(
            scope.clone(),
            e.store_registration,
            e.receipt.clone(),
        )
        .map_err(restore)?;
        put(
            tx,
            scope,
            receipt.digest(),
            SourceEvidenceReceiptRecord::FORMAT,
            &receipt.canonical_bytes(),
        )?;
        let mut objects = vec![("Envelope", None, e.receipt.envelope.clone())];
        objects.extend(
            e.evidence
                .provenance()
                .values()
                .cloned()
                .map(|r| ("Provenance", None, r)),
        );
        for (check, record) in e.evidence.checks() {
            objects.extend(
                record
                    .input()
                    .artifacts
                    .values()
                    .cloned()
                    .map(|r| ("Check", Some(check.as_str()), r)),
            );
        }
        for (sequence, (role, check, reference)) in objects.into_iter().enumerate() {
            let reference =
                SourceArtifactReferenceRecord::new(scope.clone(), e.store_registration, reference)
                    .map_err(restore)?;
            let object = reference.reference();
            put(
                tx,
                scope,
                reference.digest(),
                SourceArtifactReferenceRecord::FORMAT,
                &reference.canonical_bytes(),
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.artifact_contents (organization_id,workspace_id,repository_id,object_digest,byte_length) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text::numeric) ON CONFLICT DO NOTHING",
                &[&&object.content.as_bytes()[..], &object.bytes.to_string()],
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.artifact_locations (organization_id,workspace_id,repository_id,registration_id,object_digest,locator_kind) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::bytea,'ContentAddressed') ON CONFLICT DO NOTHING",
                &[
                    &e.store_registration.to_string(),
                    &&object.content.as_bytes()[..],
                ],
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.artifact_references (organization_id,workspace_id,repository_id,registration_id,reference_digest,object_digest,reference_name,media_type,retention_digest,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::bytea,$6::bytea,$7,$8,$9::bytea,$5::bytea) ON CONFLICT DO NOTHING",
                &[
                    &e.store_registration.to_string(),
                    &&reference.digest().as_bytes()[..],
                    &&object.content.as_bytes()[..],
                    &object.name,
                    &object.media_type,
                    &&object.retention_policy.as_bytes()[..],
                ],
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_evidence_objects (organization_id,workspace_id,repository_id,evidence_digest,object_sequence,object_role,check_id,reference_name,registration_id,reference_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text::numeric,$6,$7::text,$8,$9::text::numeric,$10::bytea)",
                &[
                    &&id.0.as_bytes()[..],
                    &(sequence as u64).to_string(),
                    &role,
                    &check,
                    &object.name,
                    &e.store_registration.to_string(),
                    &&reference.digest().as_bytes()[..],
                ],
            )?;
        }
        let assessment = match e.receipt.assessment {
            celetixo_engine::source_evidence::SourceEvidenceAssessment::Passed => "Passed",
            celetixo_engine::source_evidence::SourceEvidenceAssessment::Failed { .. } => "Failed",
            celetixo_engine::source_evidence::SourceEvidenceAssessment::Incomplete { .. } => {
                "Incomplete"
            }
        };
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.evidence_receipts (organization_id,workspace_id,repository_id,evidence_digest,source_attempt_id,registration_id,assessment,envelope_object_digest,receipt_payload_digest,recorded_at_ms,envelope_reference_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text,$6::text::numeric,$7,$8::bytea,$9::bytea,$10::text::numeric,$11::bytea)",
            &[
                &&id.0.as_bytes()[..],
                &b.source_attempt.as_str(),
                &e.store_registration.to_string(),
                &assessment,
                &&e.receipt.envelope.content.as_bytes()[..],
                &&receipt.digest().as_bytes()[..],
                &e.receipt.recorded_at_ms.to_string(),
                &&receipt.envelope().digest().as_bytes()[..],
            ],
        )?;
    }
    Ok(())
}
