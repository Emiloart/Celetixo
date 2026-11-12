use super::*;

pub(super) fn catalogs(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    for row in loader.rows(tx,"source_analyses","analysis_digest::bytea,snapshot_digest::bytea,index_generation::bytea,payload_digest::bytea","analysis_digest")? {
        let id=digest(&row,0)?;
        if digest(&row,3)?!=id {return Err(AuthorityFailure::Integrity);}
        let snapshot=loader.snapshot(tx,SnapshotId(digest(&row,1)?))?;
        let bytes=loader.payload(tx,id,"celetixo/repository-analysis/v1")?;
        let value=restore_repository_analysis(&snapshot,digest(&row,2)?,id,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        loader.budget.entries(value.elements().len()+value.dependencies().len()+value.dependencies().values().map(BTreeSet::len).sum::<usize>()+value.checks().len()+value.checks().values().map(BTreeSet::len).sum::<usize>()+value.gaps().len())?;
        if records.analyses.insert(id,value).is_some() {return Err(AuthorityFailure::Integrity);}
    }
    let heads = loader.rows(
        tx,
        "engine_model_heads",
        "analysis_payload_digest::bytea,index_generation::bytea",
        "repository_id",
    )?;
    if let Some(row) = heads.first() {
        let id = digest(row, 0)?;
        let index = digest(row, 1)?;
        if records
            .analyses
            .get(&id)
            .is_none_or(|a| a.index_generation() != index)
        {
            return Err(AuthorityFailure::Integrity);
        }
        records.analysis_head = Some(id);
    }
    let mut definitions = BTreeMap::new();
    for row in loader.rows(
        tx,
        "source_check_definitions",
        "definition_digest::bytea,check_spec_digest::bytea,payload_digest::bytea",
        "definition_digest",
    )? {
        let id = digest(&row, 0)?;
        let check = digest(&row, 1)?;
        if digest(&row, 2)? != id {
            return Err(AuthorityFailure::Integrity);
        }
        let bytes = loader.payload(tx, check, CheckSpecRecord::FORMAT)?;
        let check =
            restore_check_spec(check, &bytes, loader.budget.decode_limits()).map_err(restore)?;
        loader.budget.entries(check.check().command().len())?;
        let bytes = loader.payload(tx, id, SourceCheckDefinitionRecord::FORMAT)?;
        let definition = restore_source_check_definition(
            check.check(),
            id,
            &bytes,
            loader.budget.decode_limits(),
        )
        .map_err(restore)?;
        let size =
            SourceCheckDefinitionRecord::recovery_size(definition.definition()).map_err(restore)?;
        loader.budget.entries(size.entries)?;
        // Full typed nested definitions remain bounded by canonical payload and
        // the decoder. Charge owned copies before retaining the catalog.
        loader.budget.value(size.bytes)?;
        definitions.insert(id, definition.definition().clone());
    }
    let mut dependencies: BTreeMap<Digest, BTreeMap<String, BTreeSet<Digest>>> = BTreeMap::new();
    for row in loader.rows(
        tx,
        "source_policy_dependencies",
        "policy_digest::bytea,dependency_kind,payload_digest::bytea",
        "policy_digest,dependency_kind,payload_digest",
    )? {
        dependencies
            .entry(digest(&row, 0)?)
            .or_default()
            .entry(row.get(1))
            .or_default()
            .insert(digest(&row, 2)?);
    }
    let mut registry = BTreeMap::new();
    for row in loader.rows(
        tx,
        "policies",
        "policy_kind,generation::text,policy_digest::bytea,payload_digest::bytea",
        "policy_kind,generation",
    )? {
        if row.get::<_, String>(0) != "source_execution" || digest(&row, 2)? != digest(&row, 3)? {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
        registry.insert(digest(&row, 2)?, number(&row, 1)?);
    }
    let mut used_definitions = BTreeSet::new();
    for row in loader.rows(
        tx,
        "source_policy_catalog",
        "policy_digest::bytea,policy_generation::text,analysis_digest::bytea,policy_kind",
        "policy_digest",
    )? {
        let id = digest(&row, 0)?;
        let generation = number(&row, 1)?;
        if row.get::<_, String>(3) != "source_execution" || registry.remove(&id) != Some(generation)
        {
            return Err(AuthorityFailure::Integrity);
        }
        let analysis = records
            .analyses
            .get(&digest(&row, 2)?)
            .ok_or(AuthorityFailure::Integrity)?;
        let snapshot = loader.snapshot(tx, analysis.snapshot())?;
        let mut deps = dependencies
            .remove(&id)
            .ok_or(AuthorityFailure::Integrity)?;
        let patch = deps
            .remove("PatchPolicy")
            .ok_or(AuthorityFailure::Integrity)?;
        if patch.len() != 1 {
            return Err(AuthorityFailure::Integrity);
        }
        let patch = *patch.first().ok_or(AuthorityFailure::Integrity)?;
        let bytes = loader.payload(tx, patch, PatchPolicyRecord::FORMAT)?;
        let patch =
            restore_patch_policy(patch, &bytes, loader.budget.decode_limits()).map_err(restore)?;
        let definition_ids = deps
            .remove("CheckDefinition")
            .ok_or(AuthorityFailure::Integrity)?;
        let check_ids = deps
            .remove("CheckSpec")
            .ok_or(AuthorityFailure::Integrity)?;
        if !deps.is_empty() {
            return Err(AuthorityFailure::Integrity);
        }
        let mut selected = BTreeMap::new();
        let mut expected_checks = BTreeSet::new();
        for definition in &definition_ids {
            let value = definitions
                .get(definition)
                .ok_or(AuthorityFailure::Integrity)?;
            let size = SourceCheckDefinitionRecord::recovery_size(value).map_err(restore)?;
            loader.budget.entries(size.entries)?;
            loader.budget.value(size.bytes)?;
            expected_checks.insert(value.input().check.digest());
            used_definitions.insert(*definition);
            selected.insert(*definition, value.clone());
        }
        if check_ids != expected_checks {
            return Err(AuthorityFailure::Integrity);
        }
        let bytes = loader.payload(tx, id, SourceExecutionPolicyRecord::FORMAT)?;
        let policy = restore_source_execution_policy(
            &snapshot,
            analysis,
            generation,
            patch.policy(),
            &selected,
            id,
            &bytes,
            loader.budget.decode_limits(),
        )
        .map_err(restore)?;
        if policy
            .policy()
            .checks()
            .values()
            .map(|d| d.digest())
            .collect::<BTreeSet<_>>()
            != definition_ids
        {
            return Err(AuthorityFailure::Integrity);
        }
        records.policies.insert(id, policy.policy().clone());
    }
    if !registry.is_empty()
        || !dependencies.is_empty()
        || used_definitions != definitions.keys().copied().collect()
    {
        return Err(AuthorityFailure::Integrity);
    }
    for row in loader.rows(
        tx,
        "policy_heads",
        "policy_kind,generation::text,policy_digest::bytea",
        "policy_kind",
    )? {
        let id = digest(&row, 2)?;
        let generation = number(&row, 1)?;
        if row.get::<_, String>(0) != "source_execution"
            || records.policy_head.is_some()
            || records
                .policies
                .get(&id)
                .is_none_or(|p| p.input().generation != generation)
        {
            return Err(AuthorityFailure::Integrity);
        }
        records.policy_head = Some(id);
    }
    for row in loader.rows(
        tx,
        "source_attempt_contexts",
        "transaction_id::text,attempt_number::text,analysis_digest::bytea",
        "transaction_id,attempt_number",
    )? {
        records.attempt_contexts.insert(
            (transaction(&row, 0)?, number(&row, 1)?),
            SourceAnalysisContext {
                analysis: digest(&row, 2)?,
                policy: None,
            },
        );
    }
    for row in loader.rows(
        tx,
        "source_attempt_policies",
        "transaction_id::text,attempt_number::text,policy_digest::bytea",
        "transaction_id,attempt_number",
    )? {
        let context = records
            .attempt_contexts
            .get_mut(&(transaction(&row, 0)?, number(&row, 1)?))
            .ok_or(AuthorityFailure::Integrity)?;
        context.policy = Some(digest(&row, 2)?);
    }
    Ok(())
}

pub(super) fn grants(
    tx: &mut Transaction<'_>,
    loader: &mut Loader<'_>,
    records: &mut SourceRecords,
) -> SqlResult<()> {
    let counters = loader.rows(
        tx,
        "engine_counters",
        "next_executor_grant::text,next_cleanup_grant::text,next_artifact_store::text",
        "repository_id",
    )?;
    if let Some(row) = counters.first() {
        records.executor_counter = number(row, 0)?;
        records.cleanup_counter = number(row, 1)?;
        records.artifact_counter = number(row, 2)?;
    }
    let current = loader.rows(
        tx,
        "repositories",
        "state_generation::text",
        "repository_id",
    )?;
    let current = number(current.first().ok_or(AuthorityFailure::Integrity)?, 0)?;
    let mut source_grants = BTreeMap::new();
    for row in loader.rows(tx,"grants","grant_id::text,principal_id::text,active,grant_kind,expires_at_ms::text,payload_digest::bytea,generation::text","grant_kind,grant_id")? {
        let kind=row.get::<_,String>(3);
        if kind=="session" {continue;}
        if !matches!(kind.as_str(),"executor"|"cleanup") || optional_digest(&row,5)?.is_some() {return Err(AuthorityFailure::UnsupportedProfile);}
        generation(number(&row,6)?,current)?;
        let expiry=optional_number(&row,4)?.ok_or(AuthorityFailure::Integrity)?;
        source_grants.insert((kind,number(&row,0)?),(principal(&row,1)?,row.get::<_,bool>(2),expiry));
    }
    let mut tiers: BTreeMap<u64, BTreeSet<ExecutionTier>> = BTreeMap::new();
    for row in loader.rows(
        tx,
        "source_executor_tiers",
        "grant_id::text,execution_tier",
        "grant_id,execution_tier",
    )? {
        let tier = match row.get::<_, String>(1).as_str() {
            "T1" => ExecutionTier::T1,
            "T2" => ExecutionTier::T2,
            "T3" => ExecutionTier::T3,
            _ => return Err(AuthorityFailure::Integrity),
        };
        tiers.entry(number(&row, 0)?).or_default().insert(tier);
    }
    for row in loader.rows(tx,"source_executor_definitions","grant_id::text,expires_at_ms::text,backend_name,backend_version,backend_image::bytea,backend_profile::bytea,wall_time_ms::text,cpu_time_ms::text,memory_bytes::text,disk_bytes::text,process_count::text,file_descriptors::text,stdout_bytes::text,stderr_bytes::text,artifact_bytes::text,grant_kind","grant_id")? {
        let id=number(&row,0)?;let (principal,active,expiry)=source_grants.remove(&("executor".into(),id)).ok_or(AuthorityFailure::Integrity)?;
        if row.get::<_,String>(15)!="executor" || expiry!=number(&row,1)? {return Err(AuthorityFailure::Integrity);}
        records.executors.insert(id,SourceExecutorRecord {principal,active,expires_at_ms:expiry,
            backend:BackendIdentity {name:row.get(2),version:row.get(3),image:digest(&row,4)?,profile:digest(&row,5)?},
            tiers:tiers.remove(&id).ok_or(AuthorityFailure::Integrity)?,
            resource_ceiling:ResourceLimits {wall_time_ms:number(&row,6)?,cpu_time_ms:number(&row,7)?,memory_bytes:number(&row,8)?,disk_bytes:number(&row,9)?,process_count:number(&row,10)?,file_descriptors:number(&row,11)?,stdout_bytes:number(&row,12)?,stderr_bytes:number(&row,13)?,artifact_bytes:number(&row,14)?}});
    }
    for row in loader.rows(
        tx,
        "source_cleanup_definitions",
        "grant_id::text,expires_at_ms::text,backend_profile::bytea,grant_kind",
        "grant_id",
    )? {
        let id = number(&row, 0)?;
        let (principal, active, expiry) = source_grants
            .remove(&("cleanup".into(), id))
            .ok_or(AuthorityFailure::Integrity)?;
        if row.get::<_, String>(3) != "cleanup" || expiry != number(&row, 1)? {
            return Err(AuthorityFailure::Integrity);
        }
        records.cleanup.insert(
            id,
            SourceCleanupRecord {
                principal,
                active,
                expires_at_ms: expiry,
                backend: digest(&row, 2)?,
            },
        );
    }
    if !source_grants.is_empty() || !tiers.is_empty() {
        return Err(AuthorityFailure::Integrity);
    }
    for row in loader.rows(tx,"artifact_store_registrations","registration_id::text,generation::text,adapter_identity::text,retention_digest::bytea,provenance_digest::bytea,active,definition_payload_digest::bytea","registration_id")? {
        let id=number(&row,0)?;generation(number(&row,1)?,current)?;
        let payload=optional_digest(&row,6)?.ok_or(AuthorityFailure::UnsupportedProfile)?;
        let bytes=loader.payload(tx,payload,SourceArtifactRegistrationRecord::FORMAT)?;
        let definition=restore_source_artifact_registration(loader.scope,id,payload,&bytes,loader.budget.decode_limits()).map_err(restore)?;
        if definition.adapter().identity.as_str()!=row.get::<_,String>(2) || definition.retention()!=digest(&row,3)? || definition.provenance()!=digest(&row,4)? {return Err(AuthorityFailure::Integrity);}
        records.artifact_stores.insert(id,SourceArtifactStoreRecord {adapter:Some(definition.adapter().clone()),retention:definition.retention(),provenance:definition.provenance(),active:row.get(5)});
    }
    Ok(())
}
