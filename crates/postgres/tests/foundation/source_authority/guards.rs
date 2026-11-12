use super::*;
use lifecycle::Harness;

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture and local Linux artifact store"]
fn source_budgets_corrupt_state_and_legacy_registration_fail_without_publication() {
    let h = Harness::new("source-guards");
    let (_, admission, _) = h.ready("source-attempt");
    let before = h.authority.inspect_source(h.scope()).unwrap();
    for limits in [
        NativeRecordLimits {
            entries: 1,
            ..Default::default()
        },
        NativeRecordLimits {
            value_bytes: 8,
            ..Default::default()
        },
        NativeRecordLimits {
            total_value_bytes: 8,
            ..Default::default()
        },
    ] {
        assert_eq!(
            authority()
                .with_limits(limits)
                .unwrap()
                .inspect_source(h.scope()),
            Err(AuthorityFailure::Integrity)
        );
        assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), before);
    }
    let foreign = scope("source-unmapped");
    assert_eq!(
        h.authority.inspect_source(&foreign),
        Err(AuthorityFailure::ScopeDenied)
    );
    assert_eq!(
        h.engine.current(&h.owner, &foreign),
        Err(EngineError::Authority(AuthorityFailure::ScopeDenied))
    );
    let a = [
        h.scope().organization.as_str(),
        h.scope().workspace.as_str(),
        h.scope().repository.as_str(),
    ];
    let mut owner = owner();
    // A database-valid state label without matching immutable event/evidence
    // facts is corruption, rather than permission to reconstruct those facts.
    assert_eq!(owner.execute("UPDATE celetixo.source_attempts SET state='EvidenceSealed' WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap(), 1);
    assert_eq!(
        h.authority.inspect_source(h.scope()),
        Err(AuthorityFailure::Integrity)
    );
    assert_eq!(
        h.engine.validate_source_attempt(&h.executor, &admission),
        Err(EngineError::Authority(AuthorityFailure::Integrity))
    );
    assert_eq!(owner.query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap().get::<_,String>(0),before.state_generation.to_string());
    owner.execute("UPDATE celetixo.source_attempts SET state='Verifying' WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap();
    assert_eq!(h.authority.inspect_source(h.scope()).unwrap(), before);
    // A legacy scaffold row has no configured adapter definition. Retain it in
    // this isolated fixture, but never infer a usable source storage reader.
    owner.execute("INSERT INTO celetixo.artifact_store_registrations (organization_id,workspace_id,repository_id,registration_id,generation,adapter_identity,retention_digest,provenance_digest) VALUES ($1::text,$2::text,$3::text,2,$4::text::numeric,'legacy-reader',$5::bytea,$6::bytea)", &[&a[0],&a[1],&a[2],&before.state_generation.to_string(),&&d("retention").as_bytes()[..],&&d("legacy").as_bytes()[..]]).unwrap();
    assert_eq!(
        h.authority.inspect_source(h.scope()),
        Err(AuthorityFailure::UnsupportedProfile)
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn source_unpublished_metadata_and_profile_reinterpretation_are_rejected() {
    let selected = scope("source-unpublished");
    let mut owner = owner();
    authorize_source(&mut owner, &selected);
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    owner
        .execute(
            "INSERT INTO celetixo.organizations (organization_id) VALUES ($1::text)",
            &[&a[0]],
        )
        .unwrap();
    owner.execute("INSERT INTO celetixo.workspaces (organization_id,workspace_id) VALUES ($1::text,$2::text)",&[&a[0],&a[1]]).unwrap();
    owner.execute("INSERT INTO celetixo.repositories (organization_id,workspace_id,repository_id,authority_profile) VALUES ($1::text,$2::text,$3::text,'source-execution-v1')",&[&a[0],&a[1],&a[2]]).unwrap();
    owner.execute("INSERT INTO celetixo.engine_counters (organization_id,workspace_id,repository_id,next_executor_grant) VALUES ($1::text,$2::text,$3::text,1)",&[&a[0],&a[1],&a[2]]).unwrap();
    assert_eq!(
        authority().inspect_source(&selected),
        Err(AuthorityFailure::Integrity)
    );
    assert_eq!(
        sql_authority().inspect_native(&selected),
        Err(AuthorityFailure::UnsupportedProfile)
    );
    let error=owner.execute("UPDATE celetixo.repositories SET authority_profile='native-reference-v1' WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&a[0],&a[1],&a[2]]).unwrap_err();
    assert_eq!(error.code().unwrap().code(), "23514");
    assert_eq!(owner.query_one("SELECT count(*) FROM celetixo.authority_publications WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&a[0],&a[1],&a[2]]).unwrap().get::<_,i64>(0),0);
}
