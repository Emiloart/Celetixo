use super::*;

pub(super) fn grants(
    tx: &mut Transaction<'_>,
    context: &AuthorityAccess,
    generation: u64,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    if (before.executors != after.executors
        || before.cleanup != after.cleanup
        || before.artifact_stores != after.artifact_stores)
        && !matches!(context.actor(), AuthorityActor::EmbeddingAdministrator)
    {
        return Err(AuthorityFailure::Protocol);
    }
    for (id, g) in &after.executors {
        let previous = before.executors.get(id);
        if previous == Some(g) {
            continue;
        }
        if let Some(old) = previous {
            let mut expected = old.clone();
            expected.active = g.active;
            if expected != *g || (!old.active && g.active) {
                return Err(AuthorityFailure::Protocol);
            }
            revoke(tx, scope, "executor", *id, g.active, generation)?;
        } else {
            if *id <= before.executor_counter {
                return Err(AuthorityFailure::Protocol);
            }
            grant(
                tx,
                scope,
                "executor",
                *id,
                &g.principal,
                g.active,
                g.expires_at_ms,
                generation,
            )?;
            let r = g.resource_ceiling;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_executor_definitions (organization_id,workspace_id,repository_id,grant_id,expires_at_ms,backend_name,backend_version,backend_image,backend_profile,wall_time_ms,cpu_time_ms,memory_bytes,disk_bytes,process_count,file_descriptors,stdout_bytes,stderr_bytes,artifact_bytes) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text::numeric,$6,$7,$8::bytea,$9::bytea,$10::text::numeric,$11::text::numeric,$12::text::numeric,$13::text::numeric,$14::text::numeric,$15::text::numeric,$16::text::numeric,$17::text::numeric,$18::text::numeric)",
                &[
                    &id.to_string(),
                    &g.expires_at_ms.to_string(),
                    &g.backend.name,
                    &g.backend.version,
                    &&g.backend.image.as_bytes()[..],
                    &&g.backend.profile.as_bytes()[..],
                    &r.wall_time_ms.to_string(),
                    &r.cpu_time_ms.to_string(),
                    &r.memory_bytes.to_string(),
                    &r.disk_bytes.to_string(),
                    &r.process_count.to_string(),
                    &r.file_descriptors.to_string(),
                    &r.stdout_bytes.to_string(),
                    &r.stderr_bytes.to_string(),
                    &r.artifact_bytes.to_string(),
                ],
            )?;
            for tier in &g.tiers {
                let tier = match tier {
                    ExecutionTier::T1 => "T1",
                    ExecutionTier::T2 => "T2",
                    ExecutionTier::T3 => "T3",
                    ExecutionTier::T0 => return Err(AuthorityFailure::Protocol),
                };
                execute(
                    tx,
                    scope,
                    "INSERT INTO celetixo.source_executor_tiers (organization_id,workspace_id,repository_id,grant_id,execution_tier) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5)",
                    &[&id.to_string(), &tier],
                )?;
            }
        }
    }
    for (id, g) in &after.cleanup {
        let previous = before.cleanup.get(id);
        if previous == Some(g) {
            continue;
        }
        if let Some(old) = previous {
            let mut expected = old.clone();
            expected.active = g.active;
            if expected != *g || (!old.active && g.active) {
                return Err(AuthorityFailure::Protocol);
            }
            revoke(tx, scope, "cleanup", *id, g.active, generation)?;
        } else {
            if *id <= before.cleanup_counter {
                return Err(AuthorityFailure::Protocol);
            }
            grant(
                tx,
                scope,
                "cleanup",
                *id,
                &g.principal,
                g.active,
                g.expires_at_ms,
                generation,
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_cleanup_definitions (organization_id,workspace_id,repository_id,grant_id,expires_at_ms,backend_profile) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text::numeric,$6::bytea)",
                &[
                    &id.to_string(),
                    &g.expires_at_ms.to_string(),
                    &&g.backend.as_bytes()[..],
                ],
            )?;
        }
    }
    for (id, g) in &after.artifact_stores {
        let previous = before.artifact_stores.get(id);
        if previous == Some(g) {
            continue;
        }
        if let Some(old) = previous {
            let mut expected = old.clone();
            expected.active = g.active;
            if expected != *g || (!old.active && g.active) {
                return Err(AuthorityFailure::Protocol);
            }
            one(execute(
                tx,
                scope,
                &format!(
                    "UPDATE celetixo.artifact_store_registrations SET active=$5,generation=$6::text::numeric WHERE {FILTER} AND registration_id=$4::text::numeric"
                ),
                &[&id.to_string(), &g.active, &generation.to_string()],
            )?)?;
        } else {
            if *id <= before.artifact_counter {
                return Err(AuthorityFailure::Protocol);
            }
            let adapter = g
                .adapter
                .as_ref()
                .ok_or(AuthorityFailure::UnsupportedProfile)?;
            let definition = SourceArtifactRegistrationRecord::new(
                scope.clone(),
                *id,
                adapter.clone(),
                g.retention,
            )
            .map_err(restore)?;
            if definition.provenance() != g.provenance {
                return Err(AuthorityFailure::Integrity);
            }
            put(
                tx,
                scope,
                definition.digest(),
                SourceArtifactRegistrationRecord::FORMAT,
                &definition.canonical_bytes(),
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.artifact_store_registrations (organization_id,workspace_id,repository_id,registration_id,generation,adapter_identity,retention_digest,provenance_digest,active,definition_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text::numeric,$6::text,$7::bytea,$8::bytea,$9,$10::bytea)",
                &[
                    &id.to_string(),
                    &generation.to_string(),
                    &adapter.identity.as_str(),
                    &&g.retention.as_bytes()[..],
                    &&g.provenance.as_bytes()[..],
                    &g.active,
                    &&definition.digest().as_bytes()[..],
                ],
            )?;
        }
    }
    if before
        .executors
        .keys()
        .any(|id| !after.executors.contains_key(id))
        || before
            .cleanup
            .keys()
            .any(|id| !after.cleanup.contains_key(id))
        || before
            .artifact_stores
            .keys()
            .any(|id| !after.artifact_stores.contains_key(id))
    {
        return Err(AuthorityFailure::Protocol);
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn grant(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    kind: &str,
    id: u64,
    principal: &PrincipalId,
    active: bool,
    expiry: u64,
    generation: u64,
) -> SqlResult<()> {
    let a = snapshots::scope_parameters(scope);
    tx.execute("INSERT INTO celetixo.principals (organization_id,principal_id) VALUES ($1::text,$2::text) ON CONFLICT DO NOTHING",&[&a[0],&principal.as_str()]).map_err(database)?;
    execute(
        tx,
        scope,
        "INSERT INTO celetixo.grants (organization_id,workspace_id,repository_id,grant_kind,grant_id,principal_id,generation,active,expires_at_ms) VALUES ($1::text,$2::text,$3::text,$4,$5::text::numeric,$6::text,$7::text::numeric,$8,$9::text::numeric)",
        &[
            &kind,
            &id.to_string(),
            &principal.as_str(),
            &generation.to_string(),
            &active,
            &expiry.to_string(),
        ],
    )?;
    Ok(())
}
fn revoke(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    kind: &str,
    id: u64,
    active: bool,
    generation: u64,
) -> SqlResult<()> {
    one(execute(
        tx,
        scope,
        &format!(
            "UPDATE celetixo.grants SET active=$6,generation=$7::text::numeric WHERE {FILTER} AND grant_kind=$4 AND grant_id=$5::text::numeric"
        ),
        &[&kind, &id.to_string(), &active, &generation.to_string()],
    )?)
}

pub(super) fn catalogs(
    tx: &mut Transaction<'_>,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    for (id, a) in &after.analyses {
        if before.analyses.contains_key(id) {
            continue;
        }
        put(
            tx,
            scope,
            *id,
            "celetixo/repository-analysis/v1",
            &a.canonical_bytes(),
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.source_analyses (organization_id,workspace_id,repository_id,analysis_digest,snapshot_digest,index_generation,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::bytea,$6::bytea,$4::bytea)",
            &[
                &&id.as_bytes()[..],
                &&a.snapshot().0.as_bytes()[..],
                &&a.index_generation().as_bytes()[..],
            ],
        )?;
    }
    for (id, p) in &after.policies {
        if before.policies.contains_key(id) {
            continue;
        }
        let patch = PatchPolicyRecord::new(p.input().patch_policy.clone()).map_err(restore)?;
        put(
            tx,
            scope,
            patch.digest(),
            PatchPolicyRecord::FORMAT,
            &patch.canonical_bytes(),
        )?;
        put(
            tx,
            scope,
            *id,
            SourceExecutionPolicyRecord::FORMAT,
            &p.canonical_bytes(),
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.policies (organization_id,workspace_id,repository_id,policy_kind,generation,policy_digest,payload_digest) VALUES ($1::text,$2::text,$3::text,'source_execution',$4::text::numeric,$5::bytea,$5::bytea)",
            &[&p.input().generation.to_string(), &&id.as_bytes()[..]],
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.source_policy_catalog (organization_id,workspace_id,repository_id,policy_digest,policy_generation,analysis_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text::numeric,$6::bytea)",
            &[
                &&id.as_bytes()[..],
                &p.input().generation.to_string(),
                &&p.input().analysis_artifact.as_bytes()[..],
            ],
        )?;
        dependency(tx, scope, *id, "PatchPolicy", patch.digest())?;
        for definition in p.checks().values() {
            let check = CheckSpecRecord::new(definition.input().check.clone());
            put(
                tx,
                scope,
                check.digest(),
                CheckSpecRecord::FORMAT,
                &check.canonical_bytes(),
            )?;
            put(
                tx,
                scope,
                definition.digest(),
                SourceCheckDefinitionRecord::FORMAT,
                &definition.canonical_bytes(),
            )?;
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_check_definitions (organization_id,workspace_id,repository_id,definition_digest,check_spec_digest,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::bytea,$4::bytea) ON CONFLICT DO NOTHING",
                &[
                    &&definition.digest().as_bytes()[..],
                    &&check.digest().as_bytes()[..],
                ],
            )?;
            dependency(tx, scope, *id, "CheckDefinition", definition.digest())?;
            dependency(tx, scope, *id, "CheckSpec", check.digest())?;
        }
    }
    for ((transaction, attempt), c) in &after.attempt_contexts {
        let old = before
            .attempt_contexts
            .get(&(transaction.clone(), *attempt));
        if old.is_none() {
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_attempt_contexts (organization_id,workspace_id,repository_id,transaction_id,attempt_number,analysis_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6::bytea)",
                &[
                    &transaction.as_str(),
                    &attempt.to_string(),
                    &&c.analysis.as_bytes()[..],
                ],
            )?;
        }
        if let Some(policy) = c.policy
            && old.and_then(|c| c.policy).is_none()
        {
            execute(
                tx,
                scope,
                "INSERT INTO celetixo.source_attempt_policies (organization_id,workspace_id,repository_id,transaction_id,attempt_number,policy_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::text::numeric,$6::bytea)",
                &[
                    &transaction.as_str(),
                    &attempt.to_string(),
                    &&policy.as_bytes()[..],
                ],
            )?;
        }
    }
    if before.policy_head != after.policy_head {
        let id = after.policy_head.ok_or(AuthorityFailure::Protocol)?;
        let p = after.policies.get(&id).ok_or(AuthorityFailure::Integrity)?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.policy_heads (organization_id,workspace_id,repository_id,policy_kind,generation,policy_digest) VALUES ($1::text,$2::text,$3::text,'source_execution',$4::text::numeric,$5::bytea) ON CONFLICT (organization_id,workspace_id,repository_id,policy_kind) DO UPDATE SET generation=EXCLUDED.generation,policy_digest=EXCLUDED.policy_digest",
            &[&p.input().generation.to_string(), &&id.as_bytes()[..]],
        )?;
    }
    Ok(())
}
fn dependency(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    policy: Digest,
    kind: &str,
    payload: Digest,
) -> SqlResult<()> {
    execute(
        tx,
        scope,
        "INSERT INTO celetixo.source_policy_dependencies (organization_id,workspace_id,repository_id,policy_digest,dependency_kind,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5,$6::bytea) ON CONFLICT DO NOTHING",
        &[&&policy.as_bytes()[..], &kind, &&payload.as_bytes()[..]],
    )?;
    Ok(())
}
pub(super) fn heads(
    tx: &mut Transaction<'_>,
    generation: u64,
    before: &SourceRecords,
    after: &SourceRecords,
) -> SqlResult<()> {
    let scope = &after.base.scope;
    let Some(repo) = &after.base.repository else {
        return Ok(());
    };
    if let Some(old) = &before.base.repository
        && old.initial != repo.initial
    {
        return Err(AuthorityFailure::UnsupportedProfile);
    }
    if before.base.repository.as_ref() == Some(repo) && before.analysis_head == after.analysis_head
    {
        return Ok(());
    }
    let snapshot = repo
        .registered
        .get(&repo.current)
        .ok_or(AuthorityFailure::Integrity)?;
    let analysis = after
        .analysis_head
        .and_then(|id| after.analyses.get(&id))
        .ok_or(AuthorityFailure::Integrity)?;
    let model = EngineeringModelRecord::new(snapshot, repo.model.clone())
        .map_err(|_| AuthorityFailure::Integrity)?;
    put(
        tx,
        scope,
        model.digest(),
        EngineeringModelRecord::FORMAT,
        &model.canonical_bytes(),
    )?;
    if before.base.repository.is_none() {
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.repository_heads (organization_id,workspace_id,repository_id,revision,snapshot_digest,head_generation) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::bytea,$6::text::numeric)",
            &[
                &snapshot.revision().0.to_string(),
                &&repo.current.0.as_bytes()[..],
                &generation.to_string(),
            ],
        )?;
        execute(
            tx,
            scope,
            "INSERT INTO celetixo.engine_model_heads (organization_id,workspace_id,repository_id,initial_snapshot_digest,snapshot_digest,model_generation,model_payload_digest,model_kind,runtime_profile,analysis_payload_digest,index_generation) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::bytea,$6::text::numeric,$7::bytea,'Source',$8,$9::bytea,$10::bytea)",
            &[
                &&repo.initial.0.as_bytes()[..],
                &&repo.current.0.as_bytes()[..],
                &repo.model.generation.to_string(),
                &&model.digest().as_bytes()[..],
                &PROFILE,
                &&analysis.digest().as_bytes()[..],
                &&analysis.index_generation().as_bytes()[..],
            ],
        )?;
    } else {
        let old = before
            .base
            .repository
            .as_ref()
            .ok_or(AuthorityFailure::Integrity)?;
        let old_snapshot = old
            .registered
            .get(&old.current)
            .ok_or(AuthorityFailure::Integrity)?;
        let old_model = EngineeringModelRecord::new(old_snapshot, old.model.clone())
            .map_err(|_| AuthorityFailure::Integrity)?;
        if old.current != repo.current {
            let change = after.history.last().ok_or(AuthorityFailure::Protocol)?;
            if after.history.len() != before.history.len() + 1
                || change.receipt.publication_snapshot != repo.current
                || change.receipt.revision != snapshot.revision()
                || old_snapshot.revision().0.checked_add(1) != Some(snapshot.revision().0)
            {
                return Err(AuthorityFailure::Protocol);
            }
            one(execute(
                tx,
                scope,
                &format!(
                    "UPDATE celetixo.repository_heads SET revision=$6::text::numeric,snapshot_digest=$7::bytea,head_generation=$8::text::numeric WHERE {FILTER} AND revision=$4::text::numeric AND snapshot_digest=$5::bytea"
                ),
                &[
                    &old_snapshot.revision().0.to_string(),
                    &&old.current.0.as_bytes()[..],
                    &snapshot.revision().0.to_string(),
                    &&repo.current.0.as_bytes()[..],
                    &generation.to_string(),
                ],
            )?)?;
        }
        one(execute(
            tx,
            scope,
            &format!(
                "UPDATE celetixo.engine_model_heads SET model_generation=$4::text::numeric,model_payload_digest=$5::bytea,analysis_payload_digest=$6::bytea,index_generation=$7::bytea,snapshot_digest=$9::bytea WHERE {FILTER} AND runtime_profile=$8 AND snapshot_digest=$10::bytea AND model_generation=$11::text::numeric AND model_payload_digest=$12::bytea AND analysis_payload_digest=$13::bytea"
            ),
            &[
                &repo.model.generation.to_string(),
                &&model.digest().as_bytes()[..],
                &&analysis.digest().as_bytes()[..],
                &&analysis.index_generation().as_bytes()[..],
                &PROFILE,
                &&repo.current.0.as_bytes()[..],
                &&old.current.0.as_bytes()[..],
                &old.model.generation.to_string(),
                &&old_model.digest().as_bytes()[..],
                &before.analysis_head.map(|id| id.as_bytes().to_vec()),
            ],
        )?)?;
    }
    Ok(())
}
