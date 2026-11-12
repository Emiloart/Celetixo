use super::*;

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn v3_source_upgrade_preserves_legacy_artifact_associations_and_rolls_back_faults() {
    let (_admin, mut client, database, snapshot) = fresh_prior_database("source_upgrade", 3);
    let scope = snapshot.scope();
    let a = [
        scope.organization.as_str(),
        scope.workspace.as_str(),
        scope.repository.as_str(),
    ];
    let locator = Digest::of("legacy-locator/v1", b"fixture-object");
    let object = Digest::of("legacy-object/v1", b"same bytes");
    client.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,'legacy-locator/v1',1,$5::bytea)",&[&a[0],&a[1],&a[2],&&locator.as_bytes()[..],&b"fixture-object".as_slice()]).unwrap();
    client.execute("INSERT INTO celetixo.artifact_store_registrations (organization_id,workspace_id,repository_id,registration_id,generation,adapter_identity,retention_digest,provenance_digest) VALUES ($1::text,$2::text,$3::text,1,1,'legacy-reader',$4::bytea,$5::bytea)",&[&a[0],&a[1],&a[2],&&d("retention").as_bytes()[..],&&d("legacy-provenance").as_bytes()[..]]).unwrap();
    client.execute("INSERT INTO celetixo.artifact_objects (organization_id,workspace_id,repository_id,object_digest,byte_length,media_type,registration_id,locator_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,10,'text/plain',1,$5::bytea)",&[&a[0],&a[1],&a[2],&&object.as_bytes()[..],&&locator.as_bytes()[..]]).unwrap();
    for target in [MigrationStage::AfterDdl, MigrationStage::BeforeCommit] {
        assert_eq!(
            migrate_with_checkpoint(&mut client, |stage| if stage == target {
                Err(StoreError::MigrationInterrupted)
            } else {
                Ok(())
            }),
            Err(StoreError::MigrationInterrupted)
        );
        assert_eq!(
            client
                .query_one("SELECT count(*) FROM celetixo.schema_migrations", &[])
                .unwrap()
                .get::<_, i64>(0),
            3
        );
        assert!(
            client
                .query_one("SELECT to_regclass('celetixo.source_analyses')::text", &[])
                .unwrap()
                .get::<_, Option<String>>(0)
                .is_none()
        );
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "migration_crash_child"])
        .env("CELETIXO_PG_CRASH_DATABASE", &database)
        .output()
        .unwrap();
    assert_eq!(child.status.code(), Some(75));
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM celetixo.schema_migrations", &[])
            .unwrap()
            .get::<_, i64>(0),
        3
    );
    assert_eq!(migrate(&mut client).unwrap().schema_version, 4);
    validate_schema(&mut client).unwrap();
    assert!(!migrate(&mut client).unwrap().newly_applied);
    let row=client.query_one("SELECT original.object_digest=content.object_digest,original.byte_length=content.byte_length,original.registration_id=location.registration_id,original.locator_payload_digest=location.locator_payload_digest,location.locator_kind,store.definition_payload_digest IS NULL FROM celetixo.artifact_objects AS original JOIN celetixo.artifact_contents AS content USING (organization_id,workspace_id,repository_id,object_digest) JOIN celetixo.artifact_locations AS location USING (organization_id,workspace_id,repository_id,object_digest,registration_id) JOIN celetixo.artifact_store_registrations AS store USING (organization_id,workspace_id,repository_id,registration_id)",&[]).unwrap();
    for column in [0, 1, 2, 3, 5] {
        assert!(row.get::<_, bool>(column));
    }
    assert_eq!(row.get::<_, String>(4), "LegacyPayload");
    let mut catalog = SnapshotCatalog::connect(&connection("runtime", Some(&database))).unwrap();
    assert_eq!(catalog.read(scope, snapshot.id()).unwrap(), snapshot);
    // No fabricated logical name, configured selector or evidence receipt was
    // inferred from the old object row. All fixture data is retained for review.
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM celetixo.artifact_references", &[])
            .unwrap()
            .get::<_, i64>(0),
        0
    );
}
