//! A frozen real v8 commit, exported from the published adapter, exercises the
//! upgrade path without regenerating old semantics from the new implementation.
use super::*;

#[derive(Deserialize)]
struct PublishedFixture {
    schema: String,
    source_commit: String,
    binary_sha256: String,
    source_test: String,
    scope: [String; 3],
    insertion_order: Vec<String>,
    tables: BTreeMap<String, Vec<serde_json::Value>>,
}

#[test]
#[ignore = "requires the authenticated PostgreSQL fixture"]
fn published_v8_commit_survives_atomic_event_upgrade_and_checked_legacy_backfill() {
    let fixture: PublishedFixture =
        serde_json::from_str(include_str!("../../../fixtures/published-v8-native.json")).unwrap();
    assert_eq!(fixture.schema, "celetixo/published-v8-native-fixture/v1");
    assert_eq!(
        fixture.source_commit,
        "9bdd1af451a864ba03de516bb0cd10b5fd53e3ab"
    );
    assert_eq!(fixture.binary_sha256.len(), 64);
    assert_eq!(
        fixture.source_test,
        "native_authority::native_lifecycle_reopens_and_commits_receipt_operation_terminal_and_outbox_atomically"
    );
    let whitelist: BTreeSet<&str> = [
        "organizations",
        "principals",
        "workspaces",
        "repositories",
        "domain_payloads",
        "engine_counters",
        "reservation_fences",
        "grants",
        "outbox",
        "snapshots",
        "authority_publications",
        "engine_model_heads",
        "grant_permissions",
        "outbox_delivery",
        "repository_heads",
        "transactions",
        "attempts",
        "access_provenance",
        "committed_changes",
        "evidence",
        "operations",
        "reservations",
        "transaction_outcomes",
        "attempt_checks",
        "reference_evidence_admissions",
        "reservation_events",
        "reference_evidence_signers",
    ]
    .into();
    assert_eq!(
        fixture
            .tables
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        whitelist
    );
    assert_eq!(
        fixture
            .insertion_order
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        whitelist
    );
    let (_admin, mut sql, database, _) = fresh_prior_database("event_upgrade", 8);
    let scope = Scope {
        organization: OrganizationId::new(&fixture.scope[0]).unwrap(),
        workspace: WorkspaceId::new(&fixture.scope[1]).unwrap(),
        repository: RepositoryId::new(&fixture.scope[2]).unwrap(),
    };
    let a = [
        scope.organization.as_str(),
        scope.workspace.as_str(),
        scope.repository.as_str(),
    ];
    {
        let mut tx = sql.transaction().unwrap();
        tx.batch_execute("SET CONSTRAINTS ALL DEFERRED").unwrap();
        for table in &fixture.insertion_order {
            tx.execute(&format!("INSERT INTO celetixo.{table} SELECT * FROM jsonb_populate_recordset(NULL::celetixo.{table},$1::text::jsonb)"), &[&serde_json::to_string(&fixture.tables[table]).unwrap()]).unwrap();
        }
        tx.commit().unwrap();
    }
    let legacy_generation:String=sql.query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap().get(0);
    let ledger_before=sql.query("SELECT version,migration_name,migration_sha256 FROM celetixo.schema_migrations ORDER BY version",&[]).unwrap().into_iter().map(|row|(row.get::<_,i64>(0),row.get::<_,String>(1),row.get::<_,Vec<u8>>(2))).collect::<Vec<_>>();
    for stage in [MigrationStage::AfterDdl, MigrationStage::BeforeCommit] {
        assert_eq!(
            migrate_with_checkpoint(&mut sql, |point| if point == stage {
                Err(StoreError::MigrationInterrupted)
            } else {
                Ok(())
            }),
            Err(StoreError::MigrationInterrupted)
        );
        assert!(
            sql.query_one("SELECT to_regclass('celetixo.lifecycle_frames')::text", &[])
                .unwrap()
                .get::<_, Option<String>>(0)
                .is_none()
        );
        assert_eq!(
            sql.query_one("SELECT max(version) FROM celetixo.schema_migrations", &[])
                .unwrap()
                .get::<_, i64>(0),
            8
        );
    }
    assert_eq!(migrate(&mut sql).unwrap().schema_version, 9);
    let ledger_after=sql.query("SELECT version,migration_name,migration_sha256 FROM celetixo.schema_migrations WHERE version<=8 ORDER BY version",&[]).unwrap().into_iter().map(|row|(row.get::<_,i64>(0),row.get::<_,String>(1),row.get::<_,Vec<u8>>(2))).collect::<Vec<_>>();
    assert_eq!(ledger_before, ledger_after);
    // Every original row survives byte-for-byte JSON projection. Only delivery
    // state acquires new nullable columns; no original field is removed.
    for (table, expected) in &fixture.tables {
        let columns = expected[0].as_object().unwrap().keys().collect::<Vec<_>>();
        assert!(
            columns
                .iter()
                .all(|name| name.bytes().all(|ch| ch.is_ascii_lowercase() || ch == b'_'))
        );
        let expression = columns
            .iter()
            .map(|key| format!("'{key}',to_jsonb(f)->'{key}'"))
            .collect::<Vec<_>>()
            .join(",");
        let condition = if table == "organizations" || table == "principals" {
            "organization_id=$1::text"
        } else if table == "workspaces" {
            "organization_id=$1::text AND workspace_id=$2::text"
        } else {
            "organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text"
        };
        let parameters: Vec<&(dyn postgres::types::ToSql + Sync)> = match table.as_str() {
            "organizations" | "principals" => vec![&a[0]],
            "workspaces" => vec![&a[0], &a[1]],
            _ => vec![&a[0], &a[1], &a[2]],
        };
        let rows=sql.query(&format!("SELECT jsonb_build_object({expression})::text FROM celetixo.{table} f WHERE {condition}"),&parameters).unwrap();
        let mut actual = rows
            .iter()
            .map(|row| {
                serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(0))
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        actual.sort();
        let mut expected = expected
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>();
        expected.sort();
        assert_eq!(actual, expected, "{table}");
    }
    assert_eq!(sql.query_one("SELECT state_generation::text FROM celetixo.event_stream_origins WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap().get::<_,String>(0),legacy_generation);
    authorize_registration(&mut sql, &scope);
    let authority = Arc::new(
        PostgresAuthority::connect(
            connection("runtime", Some(&database)),
            connection("runtime_other", Some(&database)),
        )
        .unwrap(),
    );
    let before = authority.inspect_native(&scope).unwrap();
    assert_eq!(before.records.repository.as_ref().unwrap().history.len(), 1);
    assert_eq!(
        before.records.transactions[&id("a")].state,
        TransactionState::Committed
    );
    let (engine, admin) = Engine::with_authority_backend(authority.clone());
    admin
        .grant(
            PrincipalId::new("event-upgrade-observer").unwrap(),
            scope.clone(),
            [Permission::Read].into(),
        )
        .unwrap();
    let after = authority.inspect_native(&scope).unwrap();
    assert_eq!(after.records.repository, before.records.repository);
    assert_eq!(after.records.transactions, before.records.transactions);
    assert_eq!(after.records.operations, before.records.operations);
    assert_eq!(after.records.evidence, before.records.evidence);
    assert!(
        engine
            .current(
                &admin
                    .authenticate_existing_session(&scope, 3, |_, _| Ok(()))
                    .unwrap(),
                &scope
            )
            .is_ok()
    );
    let frame_count=sql.query_one("SELECT count(*) FROM celetixo.lifecycle_frames WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap().get::<_,i64>(0);
    assert_eq!(frame_count, 3); // original commit, current transaction, new authority publication
    assert_eq!(sql.query_one("SELECT count(*) FROM celetixo.outbox WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND aggregate_kind='Repository'", &[&a[0],&a[1],&a[2]]).unwrap().get::<_,i64>(0),1);
}
