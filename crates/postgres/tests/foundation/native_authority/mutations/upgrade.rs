//! A real published v5 row and its original payloads survive later additive upgrades.
use super::*;
use celetixo_engine::persistence::MutationOutcomeRecord;

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn v5_original_mutation_upgrade_preserves_rows_and_rolls_back_errors_and_process_death() {
    let (_admin, mut client, database, snapshot) = fresh_prior_database("executor_upgrade", 5);
    let scope = snapshot.scope();
    let actor = MutationActor::Session {
        principal: PrincipalId::new("original-owner").unwrap(),
        grant: 1,
    };
    let key = MutationKey {
        actor,
        key: key("original-rejection"),
    };
    let request = MutationRequest::Reserve {
        transaction: id("missing-original-target"),
        ttl_ms: 100,
    };
    let record = MutationRecord {
        request: request.clone(),
        recorded_at_ms: 23,
        resolved_attempt: None,
        state_before: None,
        lease_events_before: 0,
        administration: None,
        source_execution: None,
        source_commit: None,
        outcome: Err(EngineError::NotFound),
    };
    let outcome = MutationOutcomeRecord::new(scope.clone(), key.clone(), record).unwrap();
    let a = [
        scope.organization.as_str(),
        scope.workspace.as_str(),
        scope.repository.as_str(),
    ];
    client.execute("INSERT INTO celetixo.principals (organization_id,principal_id) VALUES ($1::text,'original-owner')",&[&a[0]]).unwrap();
    client.execute("INSERT INTO celetixo.grants (organization_id,workspace_id,repository_id,grant_kind,grant_id,principal_id,generation) VALUES ($1::text,$2::text,$3::text,'session',1,'original-owner',1)",&[&a[0],&a[1],&a[2]]).unwrap();
    let input = request.canonical_bytes(scope, &key.actor);
    let output = outcome.canonical_bytes();
    for (id, format, bytes) in [
        (
            request.digest(scope, &key.actor),
            MutationRequest::FORMAT,
            &input,
        ),
        (outcome.digest(), MutationOutcomeRecord::FORMAT, &output),
    ] {
        client.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text,1,$6::bytea)",&[&a[0],&a[1],&a[2],&&id.as_bytes()[..],&format,&bytes]).unwrap();
    }
    client.execute("INSERT INTO celetixo.engine_mutations (organization_id,workspace_id,repository_id,actor_kind,grant_kind,actor_principal_id,actor_grant_id,actor_digest,idempotency_key,request_kind,request_digest,request_payload_digest,transaction_id,recorded_at_ms,lease_events_before,outcome_kind,outcome_payload_digest) VALUES ($1::text,$2::text,$3::text,'Session','session','original-owner',1,$4::bytea,'original-rejection','Reserve',$5::bytea,$5::bytea,'missing-original-target',23,0,'Rejected',$6::bytea)",&[&a[0],&a[1],&a[2],&&key.actor.digest().as_bytes()[..],&&request.digest(scope,&key.actor).as_bytes()[..],&&outcome.digest().as_bytes()[..]]).unwrap();
    let before: String = client
        .query_one(
            "SELECT to_jsonb(m)::text FROM celetixo.engine_mutations m",
            &[],
        )
        .unwrap()
        .get(0);
    for target in [MigrationStage::AfterDdl, MigrationStage::BeforeCommit] {
        assert_eq!(
            migrate_with_checkpoint(&mut client, |stage| if stage == target {
                Err(StoreError::MigrationInterrupted)
            } else {
                Ok(())
            }),
            Err(StoreError::MigrationInterrupted)
        );
        let row=client.query_one("SELECT (SELECT count(*) FROM celetixo.schema_migrations), EXISTS(SELECT 1 FROM pg_attribute WHERE attrelid='celetixo.engine_mutations'::regclass AND attname='raw_source_attempt_id' AND NOT attisdropped)",&[]).unwrap();
        assert_eq!(row.get::<_, i64>(0), 5);
        assert!(!row.get::<_, bool>(1));
        assert_eq!(
            client
                .query_one(
                    "SELECT to_jsonb(m)::text FROM celetixo.engine_mutations m",
                    &[]
                )
                .unwrap()
                .get::<_, String>(0),
            before
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
            .query_one("SELECT max(version) FROM celetixo.schema_migrations", &[])
            .unwrap()
            .get::<_, i64>(0),
        5
    );
    assert_eq!(migrate(&mut client).unwrap().schema_version, 7);
    validate_schema(&mut client).unwrap();
    assert!(!migrate(&mut client).unwrap().newly_applied);
    let after:String=client.query_one("SELECT (to_jsonb(m)-ARRAY['raw_source_attempt_id','raw_source_admission_digest','resolved_source_attempt_id','source_admission_digest','source_state_before','source_executor_before','source_events_before','source_events_after','source_before_event_sequence','source_effect_event_sequence','result_source_state','result_source_event_sequence','result_source_executor_grant_id','source_commit_actor_digest'])::text FROM celetixo.engine_mutations m",&[]).unwrap().get(0);
    assert_eq!(after, before);
    for (id, expected) in [
        (request.digest(scope, &key.actor), input),
        (outcome.digest(), output),
    ] {
        let bytes: Vec<u8> = client
            .query_one(
                "SELECT payload_bytes FROM celetixo.domain_payloads WHERE payload_digest=$1::bytea",
                &[&&id.as_bytes()[..]],
            )
            .unwrap()
            .get(0);
        assert_eq!(bytes, expected);
    }
    // The complete fixture database and original rows are retained for review.
}
