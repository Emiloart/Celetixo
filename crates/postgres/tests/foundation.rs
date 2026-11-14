//! Actual PostgreSQL tests are opt-in; a configured workflow is not durability proof.
#![cfg(unix)]
use celetixo_engine::access::Completeness;
use celetixo_engine::ids::{CheckId, ElementId};
use celetixo_engine::ids::{
    Digest, OrganizationId, RepositoryId, Revision, Scope, SnapshotId, WorkspaceId,
};
use celetixo_engine::model::EngineeringModel;
use celetixo_engine::persistence::EngineeringModelRecord;
use celetixo_engine::persistence::NativeIntentRecord;
use celetixo_engine::repository::{AnalysisGap, RepositoryAnalysis, RepositoryAnalysisInput};
use celetixo_engine::snapshot::{InclusionPolicy, Snapshot, SnapshotManifest};
use celetixo_postgres::{
    AuthorityStage, MigrationStage, PostgresAuthority, SemanticCatalog, SnapshotCatalog,
    StoreError, UnixConnection, migrate, migrate_with_checkpoint, validate_schema,
};
use postgres::Client;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Barrier};

#[derive(Deserialize)]
struct Role {
    user: String,
    password_file: PathBuf,
}
#[derive(Deserialize)]
struct Configuration {
    host: PathBuf,
    database: String,
    roles: BTreeMap<String, Role>,
}
fn fixture() -> Configuration {
    let path = std::env::var_os("CELETIXO_PG_TEST_CONFIG")
        .expect("reviewed PostgreSQL fixture configuration is required");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn connection(role: &str, database: Option<&str>) -> UnixConnection {
    let config = fixture();
    let role = &config.roles[role];
    let password = std::fs::read_to_string(&role.password_file).unwrap();
    UnixConnection::new(
        &config.host,
        database.unwrap_or(&config.database),
        &role.user,
        password.trim().as_bytes(),
    )
    .unwrap()
}
fn scope(label: &str) -> Scope {
    Scope {
        organization: OrganizationId::new(format!("pg-{label}-{}", std::process::id())).unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    }
}
fn snapshot(scope: Scope, revision: u64) -> Snapshot {
    Snapshot::new(
        scope,
        Revision(revision),
        SnapshotManifest {
            base_commit: "fixture".into(),
            canonicalization_version: 1,
            inclusion_policy: InclusionPolicy::complete(),
            provenance: "public-pg-foundation/v1".into(),
            inputs: vec![],
            environment: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap()
}

fn semantic_model(snapshot: &Snapshot, generation: u64) -> EngineeringModelRecord {
    let a = ElementId::new("a").unwrap();
    let b = ElementId::new("b").unwrap();
    EngineeringModelRecord::new(
        snapshot,
        EngineeringModel {
            generation,
            analyzer: "finite-reference".into(),
            version: "v1".into(),
            configuration: Digest::of("pg-semantic-test/v1", b"config"),
            dependencies: BTreeMap::from([
                (a.clone(), BTreeSet::from([b.clone()])),
                (b, BTreeSet::from([a])),
            ]),
            completeness: Completeness::Incomplete {
                reasons: vec!["unresolved reference".into()],
            },
        },
    )
    .unwrap()
}

fn semantic_analysis(snapshot: &Snapshot, index: Digest) -> RepositoryAnalysis {
    let a = ElementId::new("a").unwrap();
    let b = ElementId::new("b").unwrap();
    RepositoryAnalysis::new(
        snapshot,
        RepositoryAnalysisInput {
            index_generation: index,
            analyzer_version: "structural-test/v1".into(),
            configuration: Digest::of("pg-semantic-test/v1", b"config"),
            elements: BTreeSet::from([a.clone(), b.clone()]),
            dependencies: BTreeMap::from([
                (a.clone(), BTreeSet::from([b.clone()])),
                (b, BTreeSet::from([a.clone()])),
            ]),
            checks: BTreeMap::from([(a, BTreeSet::from([CheckId::new("unit").unwrap()]))]),
            gaps: BTreeSet::from([AnalysisGap {
                stage: "resolve".into(),
                path: None,
                detail: "unresolved reference".into(),
            }]),
        },
    )
    .unwrap()
}
fn owner() -> Client {
    let mut c = connection("migration", None).connect().unwrap();
    migrate(&mut c).unwrap();
    c
}

/// Build an exact supported published schema in a fresh fixture database.
/// This never rewrites the migration ledger of an existing fixture/database.
fn fresh_prior_database(label: &str, version: u8) -> (Client, Client, String, Snapshot) {
    assert!(matches!(version, 1..=6));
    assert!(label.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
    let database = format!("celetixo_v{version}_{label}_{}", std::process::id());
    let config = fixture();
    let migration_owner = &config.roles["migration"].user;
    assert!(
        migration_owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    );
    let mut admin = connection("admin", None).connect().unwrap();
    admin
        .batch_execute(&format!(
            "CREATE DATABASE {database} OWNER {migration_owner}"
        ))
        .unwrap();
    let mut client = connection("migration", Some(&database)).connect().unwrap();
    let sql = include_str!("../migrations/0001_authority.sql");
    let checksum: [u8; 32] = Sha256::digest(sql.as_bytes()).into();
    let hex: String = checksum.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(
        hex,
        "aacc58cc8cf70224b8343074b68aafbebb57350cf5d963e4ddc0de296c091647"
    );
    {
        let mut tx = client.transaction().unwrap();
        tx.batch_execute("CREATE SCHEMA celetixo; REVOKE ALL ON SCHEMA celetixo FROM PUBLIC; CREATE TABLE celetixo.schema_migrations (version BIGINT PRIMARY KEY CHECK (version>0), migration_name TEXT NOT NULL, migration_sha256 BYTEA NOT NULL CHECK (octet_length(migration_sha256)=32), applied_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()); REVOKE ALL ON celetixo.schema_migrations FROM PUBLIC;").unwrap();
        tx.batch_execute(sql).unwrap();
        tx.execute("INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (1,'0001_authority',$1::bytea)",&[&&checksum[..]]).unwrap();
        if version >= 2 {
            let sql = include_str!("../migrations/0002_lifecycle_reconstruction.sql");
            let checksum: [u8; 32] = Sha256::digest(sql.as_bytes()).into();
            let hex: String = checksum.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(
                hex,
                "99789c46e55a38cd2f54e7433f58c7c5d12351a6a724e0dd9482b38d646a5d8e"
            );
            tx.batch_execute(sql).unwrap();
            tx.execute("INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (2,'0002_lifecycle_reconstruction',$1::bytea)", &[&&checksum[..]]).unwrap();
        }
        if version >= 3 {
            let sql = include_str!("../migrations/0003_native_lifecycle.sql");
            let checksum: [u8; 32] = Sha256::digest(sql.as_bytes()).into();
            let hex: String = checksum.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(
                hex,
                "607423e4399f40bcc95161983e64e1ca7922880e49277216eebde62033d99640"
            );
            tx.batch_execute(sql).unwrap();
            tx.execute("INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (3,'0003_native_lifecycle',$1::bytea)", &[&&checksum[..]]).unwrap();
        }
        if version >= 4 {
            let sql = include_str!("../migrations/0004_source_lifecycle.sql");
            let checksum: [u8; 32] = Sha256::digest(sql.as_bytes()).into();
            let hex: String = checksum.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(
                hex,
                "27f8a7f8824ca3b87e978810dcf1af9dd6bdda3a0a5029491644e72789a1ed27"
            );
            tx.batch_execute(sql).unwrap();
            tx.execute("INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (4,'0004_source_lifecycle',$1::bytea)", &[&&checksum[..]]).unwrap();
        }
        if version >= 5 {
            let sql = include_str!("../migrations/0005_mutation_recovery.sql");
            let checksum: [u8; 32] = Sha256::digest(sql.as_bytes()).into();
            let hex: String = checksum.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(
                hex,
                "cb36137693cf89d37b9ba13bad09cd28e667a69005348eae414fceb7e22cb60e"
            );
            tx.batch_execute(sql).unwrap();
            tx.execute("INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (5,'0005_mutation_recovery',$1::bytea)", &[&&checksum[..]]).unwrap();
        }
        if version >= 6 {
            let sql = include_str!("../migrations/0006_source_executor_mutations.sql");
            let checksum: [u8; 32] = Sha256::digest(sql.as_bytes()).into();
            let hex: String = checksum.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(
                hex,
                "18128aba1e1bed29e48f949ca3a8aa3b567f7af4a775217bc2b5880d2e5db561"
            );
            tx.batch_execute(sql).unwrap();
            tx.execute("INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (6,'0006_source_executor_mutations',$1::bytea)", &[&&checksum[..]]).unwrap();
        }
        tx.commit().unwrap();
    }
    let selected = scope(label);
    authorize(&mut client, "runtime", &selected);
    let snapshot = snapshot(selected.clone(), 7);
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    client
        .execute(
            "INSERT INTO celetixo.organizations (organization_id) VALUES ($1::text)",
            &[&a[0]],
        )
        .unwrap();
    client.execute("INSERT INTO celetixo.workspaces (organization_id,workspace_id) VALUES ($1::text,$2::text)",&[&a[0],&a[1]]).unwrap();
    client.execute("INSERT INTO celetixo.repositories (organization_id,workspace_id,repository_id) VALUES ($1::text,$2::text,$3::text)",&[&a[0],&a[1],&a[2]]).unwrap();
    client.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,'celetixo/snapshot/v1',1,$5::bytea)",&[&a[0],&a[1],&a[2],&&snapshot.id().0.as_bytes()[..],&snapshot.canonical_bytes()]).unwrap();
    client.execute("INSERT INTO celetixo.snapshots (organization_id,workspace_id,repository_id,snapshot_digest,revision,manifest_digest,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,7,$5::bytea,$4::bytea)",&[&a[0],&a[1],&a[2],&&snapshot.id().0.as_bytes()[..],&&snapshot.manifest().digest().as_bytes()[..]]).unwrap();
    client.execute("INSERT INTO celetixo.repository_heads (organization_id,workspace_id,repository_id,revision,snapshot_digest) VALUES ($1::text,$2::text,$3::text,7,$4::bytea)",&[&a[0],&a[1],&a[2],&&snapshot.id().0.as_bytes()[..]]).unwrap();
    (admin, client, database, snapshot)
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn v1_upgrade_is_atomic_preserves_data_and_rejects_incomplete_or_future_history() {
    let (mut admin, mut client, database, expected) = fresh_prior_database("upgrade", 1);
    assert_eq!(
        validate_schema(&mut client),
        Err(StoreError::SchemaMismatch)
    );
    assert!(matches!(
        SnapshotCatalog::connect(&connection("runtime", Some(&database))),
        Err(StoreError::SchemaMismatch)
    ));
    let mut checkpoints = 0;
    assert_eq!(
        migrate_with_checkpoint(&mut client, |stage| {
            if stage == MigrationStage::AfterDdl {
                checkpoints += 1;
                return Err(StoreError::MigrationInterrupted);
            }
            Ok(())
        }),
        Err(StoreError::MigrationInterrupted)
    );
    assert_eq!(checkpoints, 1);
    let present: Option<String> = client
        .query_one(
            "SELECT to_regclass('celetixo.engine_model_heads')::text",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(present.is_none());
    let count: i64 = client
        .query_one("SELECT count(*) FROM celetixo.schema_migrations", &[])
        .unwrap()
        .get(0);
    assert_eq!(count, 1);
    let receipt = migrate(&mut client).unwrap();
    assert_eq!(receipt.schema_version, 7);
    assert!(receipt.newly_applied);
    assert!(!migrate(&mut client).unwrap().newly_applied);
    validate_schema(&mut client).unwrap();
    for mutation in [
        "DELETE FROM celetixo.schema_migrations WHERE version=1",
        "UPDATE celetixo.schema_migrations SET migration_name='wrong' WHERE version=1",
        "INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (8,'future',decode(repeat('00',32),'hex'))",
    ] {
        let mut tx = client.transaction().unwrap();
        tx.batch_execute(mutation).unwrap();
        assert_eq!(validate_schema(&mut tx), Err(StoreError::SchemaMismatch));
        tx.rollback().unwrap();
    }
    {
        let mut catalog =
            SnapshotCatalog::connect(&connection("runtime", Some(&database))).unwrap();
        assert_eq!(
            catalog.read(expected.scope(), expected.id()).unwrap(),
            expected
        );
    }
    drop(client);
    admin
        .batch_execute(&format!("DROP DATABASE {database}"))
        .unwrap();
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn process_death_during_v1_upgrade_retains_v1_until_recovered_publication() {
    let (mut admin, mut client, database, expected) = fresh_prior_database("upgrade_crash", 1);
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "migration_crash_child"])
        .env("CELETIXO_PG_CRASH_DATABASE", &database)
        .output()
        .unwrap();
    assert_eq!(child.status.code(), Some(75));
    let present: Option<String> = client
        .query_one("SELECT to_regclass('celetixo.engine_counters')::text", &[])
        .unwrap()
        .get(0);
    assert!(present.is_none());
    let versions: Vec<i64> = client
        .query(
            "SELECT version FROM celetixo.schema_migrations ORDER BY version",
            &[],
        )
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(versions, vec![1]);
    assert_eq!(migrate(&mut client).unwrap().schema_version, 7);
    {
        let mut catalog =
            SnapshotCatalog::connect(&connection("runtime", Some(&database))).unwrap();
        assert_eq!(
            catalog.read(expected.scope(), expected.id()).unwrap(),
            expected
        );
    }
    drop(client);
    admin
        .batch_execute(&format!("DROP DATABASE {database}"))
        .unwrap();
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn v2_upgrade_recovers_errors_and_process_death_with_scoped_immutable_publications() {
    let (mut admin, mut client, database, expected) = fresh_prior_database("upgrade_native", 2);
    for checkpoint in [MigrationStage::AfterDdl, MigrationStage::BeforeCommit] {
        assert_eq!(
            migrate_with_checkpoint(&mut client, |stage| if stage == checkpoint {
                Err(StoreError::MigrationInterrupted)
            } else {
                Ok(())
            }),
            Err(StoreError::MigrationInterrupted)
        );
        let count: i64 = client
            .query_one("SELECT count(*) FROM celetixo.schema_migrations", &[])
            .unwrap()
            .get(0);
        assert_eq!(count, 2);
        let present: Option<String> = client
            .query_one(
                "SELECT to_regclass('celetixo.authority_publications')::text",
                &[],
            )
            .unwrap()
            .get(0);
        assert!(present.is_none());
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
        2
    );
    assert_eq!(migrate(&mut client).unwrap().schema_version, 7);
    validate_schema(&mut client).unwrap();
    assert!(!migrate(&mut client).unwrap().newly_applied);
    {
        let mut catalog =
            SnapshotCatalog::connect(&connection("runtime", Some(&database))).unwrap();
        assert_eq!(
            catalog.read(expected.scope(), expected.id()).unwrap(),
            expected
        );
    }
    let selected = expected.scope();
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    let config = fixture();
    for role in ["runtime", "runtime_other"] {
        let name = &config.roles[role].user;
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        client.batch_execute(&format!("GRANT USAGE ON SCHEMA celetixo TO {name}; GRANT SELECT,INSERT,UPDATE,DELETE ON celetixo.authority_publications,celetixo.reference_evidence_admissions,celetixo.reference_evidence_signers TO {name}; GRANT EXECUTE ON FUNCTION celetixo.has_repository_scope(text,text,text) TO {name}")).unwrap();
    }
    for table in [
        "authority_publications",
        "reference_evidence_admissions",
        "reference_evidence_signers",
    ] {
        let row = client.query_one("SELECT relrowsecurity,relforcerowsecurity FROM pg_class WHERE oid=$1::text::regclass", &[&format!("celetixo.{table}")]).unwrap();
        assert!(row.get::<_, bool>(0));
        assert!(row.get::<_, bool>(1));
    }
    {
        let mut service = connection("runtime", Some(&database)).connect().unwrap();
        let insert = "INSERT INTO celetixo.authority_publications (organization_id,workspace_id,repository_id,state_generation,observed_clock_ms,authenticated_database_role,authenticated_database_role_oid,actor_kind) SELECT $1::text,$2::text,$3::text,1,0,rolname,oid,'EmbeddingAdministrator' FROM pg_roles WHERE rolname=session_user";
        service.execute(insert, &[&a[0], &a[1], &a[2]]).unwrap();
        assert!(service.execute("UPDATE celetixo.authority_publications SET observed_clock_ms=1 WHERE organization_id=$1::text", &[&a[0]]).is_err());
        assert!(
            service
                .execute(
                    "DELETE FROM celetixo.authority_publications WHERE organization_id=$1::text",
                    &[&a[0]]
                )
                .is_err()
        );
        let mut foreign = connection("runtime_other", Some(&database))
            .connect()
            .unwrap();
        assert_eq!(
            foreign
                .query_one("SELECT count(*) FROM celetixo.authority_publications", &[])
                .unwrap()
                .get::<_, i64>(0),
            0
        );
        assert!(foreign.execute(insert, &[&a[0], &a[1], &a[2]]).is_err());
        assert_eq!(
            service
                .query_one("SELECT count(*) FROM celetixo.authority_publications", &[])
                .unwrap()
                .get::<_, i64>(0),
            1
        );
    }
    drop(client);
    admin
        .batch_execute(&format!("DROP DATABASE {database}"))
        .unwrap();
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn reconstruction_metadata_enforces_scope_current_head_and_zero_based_attempts() {
    use celetixo_engine::access::AccessSet;
    use celetixo_engine::evidence::CheckSpec;
    use celetixo_engine::transaction::TransactionIntent;
    let mut admin = owner();
    let selected = scope("reconstruction");
    let foreign = scope("reconstruction_foreign");
    authorize(&mut admin, "runtime", &selected);
    authorize(&mut admin, "runtime_other", &foreign);
    let base = snapshot(selected.clone(), 0);
    let candidate = snapshot(selected.clone(), 1);
    let mut catalog = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
    catalog.register_initial(&base).unwrap();
    catalog.put_candidate(&candidate).unwrap();
    let model = semantic_model(&base, 1);
    SemanticCatalog::connect(&connection("runtime", None))
        .unwrap()
        .put_model(&model)
        .unwrap();
    let config = fixture();
    let runtime = &config.roles["runtime"].user;
    let other = &config.roles["runtime_other"].user;
    for role in [runtime, other] {
        assert!(role.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        admin.batch_execute(&format!("GRANT SELECT,INSERT,UPDATE ON celetixo.engine_counters,celetixo.engine_model_heads TO {role}")).unwrap();
    }
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    let mut service = connection("runtime", None).connect().unwrap();
    service.execute("INSERT INTO celetixo.engine_counters (organization_id,workspace_id,repository_id,next_session_grant) VALUES ($1::text,$2::text,$3::text,'18446744073709551615')",&[&a[0],&a[1],&a[2]]).unwrap();
    service.execute("INSERT INTO celetixo.engine_model_heads (organization_id,workspace_id,repository_id,initial_snapshot_digest,snapshot_digest,model_generation,model_payload_digest,model_kind) VALUES ($1::text,$2::text,$3::text,$4::bytea,$4::bytea,1,$5::bytea,'Reference')",&[&a[0],&a[1],&a[2],&&base.id().0.as_bytes()[..],&&model.digest().as_bytes()[..]]).unwrap();
    let mut foreign_service = connection("runtime_other", None).connect().unwrap();
    for table in ["engine_counters", "engine_model_heads"] {
        let count: i64 = foreign_service
            .query_one(
                &format!("SELECT count(*) FROM celetixo.{table} WHERE organization_id=$1::text"),
                &[&a[0]],
            )
            .unwrap()
            .get(0);
        assert_eq!(count, 0);
    }
    assert!(foreign_service.execute("INSERT INTO celetixo.engine_counters (organization_id,workspace_id,repository_id) VALUES ($1::text,$2::text,$3::text)",&[&a[0],&a[1],&a[2]]).is_err());
    assert!(service.execute("UPDATE celetixo.engine_counters SET next_session_grant=-1 WHERE organization_id=$1::text",&[&a[0]]).is_err());
    assert!(service.execute("UPDATE celetixo.engine_model_heads SET model_kind='Source' WHERE organization_id=$1::text",&[&a[0]]).is_err());
    {
        let mut tx = service.transaction().unwrap();
        tx.execute("UPDATE celetixo.engine_model_heads SET snapshot_digest=$4::bytea WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&a[0],&a[1],&a[2],&&candidate.id().0.as_bytes()[..]]).unwrap();
        assert!(tx.commit().is_err());
    }
    // DDL/FK fixtures below create metadata directly under the trusted owner.
    // They do not run an Engine transition or establish semantic admissibility.
    let intent = NativeIntentRecord::new(
        &base,
        TransactionIntent {
            objective: "zero-based metadata fixture".into(),
            accesses: AccessSet::default(),
            invariants: vec![],
            required_checks: vec![
                CheckSpec::new(
                    CheckId::new("test").unwrap(),
                    vec!["not-executed".into()],
                    "fixture".into(),
                    model.digest(),
                    model.digest(),
                )
                .unwrap(),
            ],
            depends_on: BTreeSet::new(),
        },
    )
    .unwrap();
    let mut tx = admin.transaction().unwrap();
    tx.execute(
        "INSERT INTO celetixo.principals (organization_id,principal_id) VALUES ($1::text,'owner')",
        &[&a[0]],
    )
    .unwrap();
    tx.execute("INSERT INTO celetixo.grants (organization_id,workspace_id,repository_id,grant_kind,grant_id,principal_id,generation) VALUES ($1::text,$2::text,$3::text,'session',1,'owner',1)",&[&a[0],&a[1],&a[2]]).unwrap();
    tx.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,'celetixo/native-intent-record/v1',1,$5::bytea)",&[&a[0],&a[1],&a[2],&&intent.digest().as_bytes()[..],&intent.canonical_bytes()]).unwrap();
    tx.execute("INSERT INTO celetixo.transactions (organization_id,workspace_id,repository_id,transaction_id,owner_principal_id,owner_grant_id,state,current_attempt,original_snapshot_digest,intent_payload_digest) VALUES ($1::text,$2::text,$3::text,'zero','owner',1,'Proposed',0,$4::bytea,$5::bytea)",&[&a[0],&a[1],&a[2],&&base.id().0.as_bytes()[..],&&intent.digest().as_bytes()[..]]).unwrap();
    tx.execute("INSERT INTO celetixo.attempts (organization_id,workspace_id,repository_id,transaction_id,attempt_number,base_snapshot_digest,model_payload_digest) VALUES ($1::text,$2::text,$3::text,'zero',0,$4::bytea,$5::bytea)",&[&a[0],&a[1],&a[2],&&base.id().0.as_bytes()[..],&&model.digest().as_bytes()[..]]).unwrap();
    tx.commit().unwrap();
    let ordinal:String=admin.query_one("SELECT current_attempt::text FROM celetixo.transactions WHERE organization_id=$1::text",&[&a[0]]).unwrap().get(0);
    assert_eq!(ordinal, "0");
}
fn authorize(owner: &mut Client, role: &str, scope: &Scope) {
    let config = fixture();
    let name = &config.roles[role].user;
    // Role names originate in the private operator fixture, never a request.
    assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    let attributes=owner.query_one("SELECT rolsuper,rolcreaterole,rolcreatedb,rolbypassrls,pg_has_role(oid,current_user,'MEMBER') FROM pg_roles WHERE rolname=$1", &[name]).unwrap();
    for n in 0..5 {
        assert!(
            !attributes.get::<_, bool>(n),
            "runtime has excessive role privilege {n}"
        );
    }
    owner.execute("INSERT INTO celetixo.db_role_scopes (database_role,database_role_oid,organization_id,workspace_id,repository_id) SELECT rolname,oid,$2::text,$3::text,$4::text FROM pg_roles WHERE rolname=$1 ON CONFLICT DO NOTHING", &[name,&scope.organization.as_str(),&scope.workspace.as_str(),&scope.repository.as_str()]).unwrap();
    owner.batch_execute(&format!("GRANT USAGE ON SCHEMA celetixo TO {name}; GRANT EXECUTE ON FUNCTION celetixo.has_repository_scope(text,text,text), celetixo.has_workspace_scope(text,text), celetixo.has_organization_scope(text) TO {name}; GRANT SELECT ON celetixo.schema_migrations TO {name}; GRANT UPDATE(state_generation) ON celetixo.repositories TO {name}; GRANT SELECT,INSERT ON celetixo.organizations,celetixo.workspaces,celetixo.repositories,celetixo.domain_payloads,celetixo.snapshots,celetixo.repository_heads TO {name};")).unwrap();
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn snapshot_reopen_scope_isolation_immutable_rows_and_exact_u64_revision() {
    let mut admin = owner();
    let a = scope("a");
    let b = scope("b");
    authorize(&mut admin, "runtime", &a);
    authorize(&mut admin, "runtime_other", &b);
    let initial = snapshot(a.clone(), u64::MAX);
    let foreign = snapshot(b.clone(), 7);
    {
        let mut first = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
        first.register_initial(&initial).unwrap();
        first.register_initial(&initial).unwrap();
        let candidate = snapshot(a.clone(), 9);
        first.put_candidate(&candidate).unwrap();
        assert_eq!(
            first.register_initial(&candidate),
            Err(StoreError::AlreadyRegistered)
        );
        let mut other = SnapshotCatalog::connect(&connection("runtime_other", None)).unwrap();
        other.register_initial(&foreign).unwrap();
        assert_eq!(first.read(&b, foreign.id()), Err(StoreError::NotFound));
        assert!(first.register_initial(&foreign).is_err());
    }
    let mut reopened = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
    assert_eq!(reopened.read(&a, initial.id()).unwrap(), initial);
    // A client-controlled tenant selector cannot replace authenticated session_user.
    let mut runtime = connection("runtime", None).connect().unwrap();
    runtime
        .execute(
            "SELECT set_config('celetixo.organization',$1,false)",
            &[&b.organization.as_str()],
        )
        .unwrap();
    let count: i64 = runtime
        .query_one(
            "SELECT count(*) FROM celetixo.snapshots WHERE organization_id=$1::text",
            &[&b.organization.as_str()],
        )
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    assert!(
        runtime
            .query("SELECT * FROM celetixo.db_role_scopes", &[])
            .is_err()
    );
    assert!(
        runtime
            .batch_execute("SET SESSION AUTHORIZATION celetixo_migration")
            .is_err()
    );
    assert!(
        runtime
            .batch_execute("SET ROLE celetixo_migration")
            .is_err()
    );
    let changed = admin
        .execute(
            "UPDATE celetixo.snapshots SET revision=0 WHERE organization_id=$1::text",
            &[&a.organization.as_str()],
        )
        .unwrap_err();
    assert_eq!(changed.code().unwrap().code(), "23514");
    assert!(
        admin
            .query_one(
                "SELECT '18446744073709551616'::numeric::celetixo.uint64",
                &[]
            )
            .is_err()
    );
    assert!(
        admin
            .query_one("SELECT '-1'::numeric::celetixo.uint64", &[])
            .is_err()
    );
    let attrs=admin.query("SELECT relname,relrowsecurity,relforcerowsecurity FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='celetixo' AND relkind='r' AND relname NOT IN ('schema_migrations','db_role_scopes')", &[]).unwrap();
    assert!(attrs.len() > 30);
    for row in attrs {
        assert!(
            row.get::<_, bool>(1) && row.get::<_, bool>(2),
            "missing RLS on {}",
            row.get::<_, String>(0)
        );
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn migration_checksum_version_skew_and_concurrent_migrators_fail_closed() {
    let mut client = owner();
    assert!(!migrate(&mut client).unwrap().newly_applied);
    let handles: Vec<_> = (0..3)
        .map(|_| {
            std::thread::spawn(|| migrate(&mut connection("migration", None).connect().unwrap()))
        })
        .collect();
    for handle in handles {
        assert!(!handle.join().unwrap().unwrap().newly_applied);
    }
    for sql in [
        "UPDATE celetixo.schema_migrations SET migration_sha256=decode(repeat('00',32),'hex')",
        "UPDATE celetixo.schema_migrations SET version=7 WHERE version=6",
    ] {
        let mut tx = client.transaction().unwrap();
        tx.batch_execute(sql).unwrap();
        assert_eq!(validate_schema(&mut tx), Err(StoreError::SchemaMismatch));
        tx.rollback().unwrap();
    }
    validate_schema(&mut client).unwrap();
    assert!(
        connection("runtime", None)
            .connect()
            .unwrap()
            .batch_execute("CREATE SCHEMA forged")
            .is_err()
    );
}

#[test]
#[ignore = "subprocess crash helper; invoked only by the parent test"]
fn migration_crash_child() {
    let Ok(database) = std::env::var("CELETIXO_PG_CRASH_DATABASE") else {
        panic!("parent must bind disposable database");
    };
    let mut client = connection("migration", Some(&database)).connect().unwrap();
    migrate_with_checkpoint(&mut client, |stage| {
        if stage == MigrationStage::AfterDdl {
            std::process::exit(75);
        }
        Ok(())
    })
    .unwrap();
    panic!("expected interruption before SQL commit");
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn process_death_rolls_back_ddl_and_ledger_before_successful_recovery() {
    let database = format!("celetixo_migration_crash_{}", std::process::id());
    let config = fixture();
    let owner = &config.roles["migration"].user;
    assert!(owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    let mut admin = connection("admin", None).connect().unwrap();
    admin
        .batch_execute(&format!("CREATE DATABASE {database} OWNER {owner}"))
        .unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "migration_crash_child"])
        .env("CELETIXO_PG_CRASH_DATABASE", &database)
        .output()
        .unwrap();
    assert_eq!(child.status.code(), Some(75));
    {
        let mut client = connection("migration", Some(&database)).connect().unwrap();
        let object: Option<String> = client
            .query_one(
                "SELECT to_regclass('celetixo.schema_migrations')::text",
                &[],
            )
            .unwrap()
            .get(0);
        assert!(object.is_none());
        let receipt = migrate(&mut client).unwrap();
        assert!(receipt.newly_applied);
        validate_schema(&mut client).unwrap();
    }
    admin
        .batch_execute(&format!("DROP DATABASE {database}"))
        .unwrap();
}

#[test]
fn database_connection_configuration_does_not_accept_implicit_or_network_transport() {
    assert!(UnixConnection::new("localhost", "db", "user", b"password").is_err());
    assert!(UnixConnection::new("/private/socket", "", "user", b"password").is_err());
    assert!(UnixConnection::new("/private/socket", "db", "user", b"").is_err());
}

#[test]
#[ignore = "requires permission to restart the dedicated reviewed PostgreSQL fixture"]
fn immutable_catalogs_survive_actual_database_process_restart() {
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    let config = fixture();
    let selected = scope("restart");
    {
        let mut admin = owner();
        authorize(&mut admin, "runtime", &selected);
        let mut catalog = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
        catalog
            .register_initial(&snapshot(selected.clone(), 123))
            .unwrap();
        let base = snapshot(selected.clone(), 123);
        let mut semantic = SemanticCatalog::connect(&connection("runtime", None)).unwrap();
        semantic
            .put_model(&semantic_model(&base, u64::MAX))
            .unwrap();
        semantic
            .put_analysis(&semantic_analysis(
                &base,
                Digest::of("pg-semantic-test/v1", b"index"),
            ))
            .unwrap();
    }
    let native_scope = scope("registration-restart");
    let native_before = {
        let mut administrator_connection = owner();
        authorize_registration(&mut administrator_connection, &native_scope);
        let backend = Arc::new(sql_authority());
        let (engine, administrator) = Engine::with_authority_backend(backend.clone());
        let session = administrator
            .grant(
                PrincipalId::new("restart-worker").unwrap(),
                native_scope.clone(),
                Permission::ALL.into_iter().collect(),
            )
            .unwrap();
        engine
            .register(&session, snapshot(native_scope.clone(), 0), native_model(1))
            .unwrap();
        backend.inspect_registration(&native_scope).unwrap()
    };
    let (full_native_scope, full_native_before) = native_authority::prepare_database_restart();
    #[cfg(target_os = "linux")]
    let (source_scope, source_before, source_artifact_path) =
        source_authority::prepare_database_restart();
    let root = config.host.parent().unwrap();
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/goal04/postgres_fixture.py");
    let result = Command::new("python3")
        .arg(script)
        .args(["restart", "--root"])
        .arg(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "reviewed fixture restart failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        sql_authority().inspect_registration(&native_scope).unwrap(),
        native_before
    );
    native_authority::assert_database_restart(&full_native_scope, &full_native_before);
    #[cfg(target_os = "linux")]
    source_authority::assert_database_restart(&source_scope, &source_before, &source_artifact_path);
    let mut reopened = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
    let expected = snapshot(selected.clone(), 123);
    assert_eq!(reopened.read(&selected, expected.id()).unwrap(), expected);
    let mut semantic = SemanticCatalog::connect(&connection("runtime", None)).unwrap();
    let model = semantic_model(&expected, u64::MAX);
    let analysis = semantic_analysis(&expected, Digest::of("pg-semantic-test/v1", b"index"));
    assert_eq!(
        semantic
            .read_model(&selected, expected.id(), u64::MAX, model.digest())
            .unwrap(),
        model
    );
    assert_eq!(
        semantic
            .read_analysis(
                &selected,
                expected.id(),
                analysis.index_generation(),
                analysis.digest()
            )
            .unwrap(),
        analysis
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn semantic_records_reopen_with_scope_snapshot_and_generation_binding_without_installing_authority()
{
    let mut admin = owner();
    let selected = scope("semantic-a");
    let foreign = scope("semantic-b");
    authorize(&mut admin, "runtime", &selected);
    authorize(&mut admin, "runtime_other", &foreign);
    let base = snapshot(selected.clone(), 0);
    let candidate = snapshot(selected.clone(), 1);
    let other = snapshot(foreign.clone(), 0);
    let model = semantic_model(&base, u64::MAX);
    let analysis = semantic_analysis(&base, Digest::of("pg-semantic-test/v1", b"index"));
    {
        let mut snapshots = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
        snapshots.register_initial(&base).unwrap();
        let mut catalog = SemanticCatalog::connect(&connection("runtime", None)).unwrap();
        // A payload cannot silently introduce an unregistered snapshot.
        assert_eq!(
            catalog.put_model(&semantic_model(&candidate, 1)),
            Err(StoreError::NotFound)
        );
        assert_eq!(
            catalog.put_analysis(&semantic_analysis(
                &candidate,
                Digest::of("pg-semantic-test/v1", b"candidate-index")
            )),
            Err(StoreError::NotFound)
        );
        snapshots.put_candidate(&candidate).unwrap();
        for _ in 0..2 {
            catalog.put_model(&model).unwrap();
            catalog.put_analysis(&analysis).unwrap();
        }
        catalog.put_model(&semantic_model(&candidate, 1)).unwrap();
        catalog
            .put_analysis(&semantic_analysis(
                &candidate,
                Digest::of("pg-semantic-test/v1", b"candidate-index"),
            ))
            .unwrap();
        let mut other_snapshots =
            SnapshotCatalog::connect(&connection("runtime_other", None)).unwrap();
        other_snapshots.register_initial(&other).unwrap();
        let mut other_catalog =
            SemanticCatalog::connect(&connection("runtime_other", None)).unwrap();
        assert_eq!(
            other_catalog.read_model(&selected, base.id(), u64::MAX, model.digest()),
            Err(StoreError::NotFound)
        );
        assert_eq!(
            other_catalog.read_analysis(
                &selected,
                base.id(),
                analysis.index_generation(),
                analysis.digest()
            ),
            Err(StoreError::NotFound)
        );
        assert_eq!(other_catalog.put_model(&model), Err(StoreError::NotFound));
        assert_eq!(
            other_catalog.put_analysis(&analysis),
            Err(StoreError::NotFound)
        );
    }
    let mut reopened = SemanticCatalog::connect(&connection("runtime", None)).unwrap();
    assert_eq!(
        reopened
            .read_model(&selected, base.id(), u64::MAX, model.digest())
            .unwrap(),
        model
    );
    assert_eq!(
        reopened
            .read_analysis(
                &selected,
                base.id(),
                analysis.index_generation(),
                analysis.digest()
            )
            .unwrap(),
        analysis
    );
    assert!(
        reopened
            .read_model(&selected, candidate.id(), u64::MAX, model.digest())
            .is_err()
    );
    assert!(
        reopened
            .read_model(&selected, base.id(), 1, model.digest())
            .is_err()
    );
    assert!(
        reopened
            .read_analysis(
                &selected,
                candidate.id(),
                analysis.index_generation(),
                analysis.digest()
            )
            .is_err()
    );
    assert!(
        reopened
            .read_analysis(
                &selected,
                base.id(),
                Digest::of("pg-semantic-test/v1", b"stale"),
                analysis.digest()
            )
            .is_err()
    );
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    let head = admin.query_one("SELECT revision::text,snapshot_digest::bytea FROM celetixo.repository_heads WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap();
    assert_eq!(head.get::<_, String>(0), "0");
    assert_eq!(head.get::<_, Vec<u8>>(1), base.id().0.as_bytes());
    let count: i64 = admin
        .query_one(
            "SELECT count(*) FROM celetixo.transactions WHERE organization_id=$1::text",
            &[&a[0]],
        )
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    let generation: String = admin.query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap().get(0);
    assert_eq!(generation, "0");
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn semantic_catalog_rejects_forged_bytes_formats_versions_contexts_and_generations() {
    let mut admin = owner();
    let selected = scope("semantic-integrity");
    authorize(&mut admin, "runtime", &selected);
    let base = snapshot(selected.clone(), 0);
    SnapshotCatalog::connect(&connection("runtime", None))
        .unwrap()
        .register_initial(&base)
        .unwrap();
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    let mut catalog = SemanticCatalog::connect(&connection("runtime", None)).unwrap();
    for kind in 0..2 {
        for variant in 0..6 {
            let source = if variant == 3 {
                snapshot(scope("semantic-forged"), variant)
            } else if variant == 4 {
                snapshot(selected.clone(), variant)
            } else {
                base.clone()
            };
            let model = semantic_model(&source, variant + 1);
            let index = Digest::of("pg-semantic-test/v1", &[variant as u8]);
            let analysis = semantic_analysis(&source, index);
            let bytes = if kind == 0 {
                model.canonical_bytes()
            } else {
                analysis.canonical_bytes()
            };
            let digest = if kind == 0 {
                model.digest()
            } else {
                analysis.digest()
            };
            let expected = if variant == 0 {
                Digest::of("pg-semantic-forged/v1", &bytes)
            } else {
                digest
            };
            let format = if variant == 1 {
                "unsupported/semantic/v9"
            } else if kind == 0 {
                EngineeringModelRecord::FORMAT
            } else {
                "celetixo/repository-analysis/v1"
            };
            let version: i32 = if variant == 2 { 2 } else { 1 };
            admin.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text,$6,$7::bytea)", &[&a[0],&a[1],&a[2],&&expected.as_bytes()[..],&format,&version,&bytes]).unwrap();
            let result = if kind == 0 {
                catalog
                    .read_model(
                        &selected,
                        base.id(),
                        if variant == 5 { u64::MAX } else { variant + 1 },
                        expected,
                    )
                    .map(|_| ())
            } else {
                catalog
                    .read_analysis(
                        &selected,
                        base.id(),
                        if variant == 5 {
                            Digest::of("pg-semantic-test/v1", b"stale")
                        } else {
                            index
                        },
                        expected,
                    )
                    .map(|_| ())
            };
            assert!(
                result.is_err(),
                "forged semantic record kind {kind}, variant {variant}"
            );
            if variant == 1 || variant == 2 {
                let result = if kind == 0 {
                    catalog.put_model(&model)
                } else {
                    catalog.put_analysis(&analysis)
                };
                assert_eq!(
                    result,
                    Err(StoreError::Integrity),
                    "conflicting immutable bytes cannot acknowledge put"
                );
                let stored = admin.query_one("SELECT payload_format::text,payload_version,payload_bytes FROM celetixo.domain_payloads WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND payload_digest=$4::bytea", &[&a[0],&a[1],&a[2],&&expected.as_bytes()[..]]).unwrap();
                assert_eq!(stored.get::<_, String>(0), format);
                assert_eq!(stored.get::<_, i32>(1), version);
                assert_eq!(stored.get::<_, Vec<u8>>(2), bytes);
            }
        }
    }
}

#[test]
#[ignore = "requires reviewed PostgreSQL and Git; captures/indexes trusted fixture source without executing it"]
fn actual_repository_analysis_reopens_and_retains_engine_rejection_of_incomplete_impact() {
    use celetixo_engine::access::AccessSet;
    use celetixo_engine::engine::{Engine, EngineError, InMemoryStore, Permission};
    use celetixo_engine::evidence::CheckSpec;
    use celetixo_engine::ids::{PrincipalId, TransactionId};
    use celetixo_engine::transaction::{TransactionIntent, TransactionState};
    use celetixo_repository::capture::{CaptureRequest, capture_git_tree};
    use celetixo_repository::impact::ImpactAnalyzer;
    use celetixo_repository::index::{IndexConfig, Indexer};

    let fixture = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "lib.ts",
            "import { missing } from './absent';\nexport function consume() { return missing(); }\n",
        ),
        (
            "main.py",
            "def leaf():\n    return 1\ndef consume():\n    return leaf()\n",
        ),
        (
            "helper.js",
            "export const leaf = () => 1;\nexport const consume = () => leaf();\n",
        ),
        (
            "main.go",
            "package fixture\nfunc Leaf() int { return 1 }\nfunc Consume() int { return Leaf() }\n",
        ),
        (
            "lib.rs",
            "fn leaf() -> i32 { 1 }\nfn consume() -> i32 { leaf() }\n",
        ),
    ] {
        std::fs::write(fixture.path().join(path), source).unwrap();
    }
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(fixture.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_DATE", "2026-11-11T00:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-11-11T00:00:00+00:00")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Emiloart.",
                "-c",
                "user.email=ilodubahemeka@gmail.com",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "trusted Git fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "--quiet"]);
    git(&["add", "."]);
    git(&[
        "commit",
        "--quiet",
        "--no-gpg-sign",
        "-m",
        "multilanguage semantic persistence fixture",
    ]);
    let selected = scope("actual-repository");
    let captured = capture_git_tree(
        &CaptureRequest::new(fixture.path(), selected.clone(), Revision(0)),
        &git(&["rev-parse", "HEAD"]),
    )
    .unwrap();
    assert!(captured.report.complete());
    let index = Indexer::new(IndexConfig::default())
        .unwrap()
        .build(&captured.snapshot)
        .unwrap();
    assert_eq!(index.files.len(), 5);
    assert!(index.files.values().all(|file| file.language.is_some()));
    let original = ImpactAnalyzer::new(&captured, &index)
        .unwrap()
        .engine_contract(&captured)
        .unwrap();
    assert!(!original.is_complete());
    assert!(
        original
            .dependencies()
            .values()
            .any(|dependencies| !dependencies.is_empty())
    );

    let mut owner = owner();
    authorize(&mut owner, "runtime", &selected);
    SnapshotCatalog::connect(&connection("runtime", None))
        .unwrap()
        .register_initial(&captured.snapshot)
        .unwrap();
    {
        let mut catalog = SemanticCatalog::connect(&connection("runtime", None)).unwrap();
        catalog.put_analysis(&original).unwrap();
    }
    let mut catalog = SemanticCatalog::connect(&connection("runtime", None)).unwrap();
    let restored = catalog
        .read_analysis(
            &selected,
            captured.snapshot.id(),
            index.generation,
            original.digest(),
        )
        .unwrap();
    assert_eq!(restored, original);
    assert_eq!(restored.gaps(), original.gaps());

    // This fresh in-memory fixture owns administration. The catalog itself has
    // neither installed authority nor authenticated an end-user request.
    let (store, admin) = InMemoryStore::new();
    let engine = Engine::new(store);
    let session = admin
        .grant(
            PrincipalId::new("fixture-inspector").unwrap(),
            selected.clone(),
            Permission::ALL.into(),
        )
        .unwrap();
    engine
        .register_repository(&session, captured.snapshot.clone(), restored)
        .unwrap();
    let id = TransactionId::new("incomplete-after-reopen").unwrap();
    engine
        .propose(
            &session,
            &selected,
            id.clone(),
            captured.snapshot.id(),
            TransactionIntent {
                objective: "incomplete persisted analysis cannot become admissible".into(),
                accesses: AccessSet {
                    writes: BTreeSet::from([index.files["lib.ts"].file.clone()]),
                    ..Default::default()
                },
                invariants: vec![],
                depends_on: BTreeSet::new(),
                required_checks: vec![
                    CheckSpec::new(
                        CheckId::new("fixture-check").unwrap(),
                        vec!["fixture-check-never-executed".into()],
                        "fixture".into(),
                        Digest::of("pg-fixture/v1", b"env"),
                        Digest::of("pg-fixture/v1", b"config"),
                    )
                    .unwrap(),
                ],
            },
        )
        .unwrap();
    assert_eq!(
        engine.analyze(&session, &selected, &id),
        Err(EngineError::IncompleteAnalysis)
    );
    assert_eq!(
        engine.transaction(&session, &selected, &id).unwrap().state,
        TransactionState::Proposed
    );
    assert_eq!(
        engine.current(&session, &selected).unwrap(),
        captured.snapshot
    );
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn durable_payloads_cannot_forge_content_scope_format_or_manifest() {
    let mut admin = owner();
    let selected = scope("integrity");
    authorize(&mut admin, "runtime", &selected);
    let mut catalog = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
    catalog
        .register_initial(&snapshot(selected.clone(), 0))
        .unwrap();
    for variant in 0..4 {
        let original = snapshot(
            if variant == 1 {
                scope("foreign-payload")
            } else {
                selected.clone()
            },
            variant + 1,
        );
        let expected = if variant == 0 {
            SnapshotId(Digest::of("forged-snapshot", &original.canonical_bytes()))
        } else {
            original.id()
        };
        let format = if variant == 2 {
            "unsupported/snapshot/v9"
        } else {
            "celetixo/snapshot/v1"
        };
        let manifest = if variant == 3 {
            Digest::of("wrong-manifest", b"fixture")
        } else {
            original.manifest().digest()
        };
        let args = [
            selected.organization.as_str(),
            selected.workspace.as_str(),
            selected.repository.as_str(),
        ];
        admin.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text,1,$6)", &[&args[0],&args[1],&args[2],&&expected.0.as_bytes()[..],&format,&original.canonical_bytes()]).unwrap();
        admin.execute("INSERT INTO celetixo.snapshots (organization_id,workspace_id,repository_id,snapshot_digest,revision,manifest_digest,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text::numeric,$6::bytea,$4::bytea)", &[&args[0],&args[1],&args[2],&&expected.0.as_bytes()[..],&original.revision().0.to_string(),&&manifest.as_bytes()[..]]).unwrap();
        assert!(
            catalog.read(&selected, expected).is_err(),
            "forged durable payload variant {variant}"
        );
    }
}

// Restricted application-administration is deliberately a separate login from
// both runtime and migration owner. The reviewed list includes SELECT for
// unsupported-state detection, never blanket access to the role map/ledger.
fn authorize_registration(owner: &mut Client, selected: &Scope) {
    for role in ["runtime", "runtime_other"] {
        authorize(owner, role, selected);
        let config = fixture();
        let name = &config.roles[role].user;
        let reads = "principals,grants,grant_permissions,engine_counters,engine_model_heads,reservation_fences,policies,policy_heads,artifact_store_registrations,transactions,attempts,access_provenance,transaction_outcomes,committed_changes,reservations,reservation_claims,reservation_events,operations,source_attempts,source_attempt_events,source_attempt_outcomes,source_rejections,artifact_objects,evidence,evidence_receipts,attempt_checks,reference_evidence_admissions,reference_evidence_signers,authority_publications,outbox,outbox_delivery,inbox";
        let reads = format!(
            "{reads},source_analyses,source_policy_catalog,source_check_definitions,source_policy_dependencies,source_attempt_contexts,source_attempt_policies,source_admission_contexts,source_operation_contexts,source_rejection_contexts,source_rejection_events,source_unprepared_commits,source_executor_definitions,source_executor_tiers,source_cleanup_definitions,artifact_contents,artifact_locations,artifact_references,source_evidence_objects,engine_mutations,source_commit_mutations,source_committed_changes"
        );
        let reads = reads
            .split(',')
            .map(|t| format!("celetixo.{t}"))
            .collect::<Vec<_>>()
            .join(",");
        owner.batch_execute(&format!("GRANT SELECT ON {reads} TO {name}; GRANT INSERT,UPDATE ON celetixo.engine_counters,celetixo.engine_model_heads TO {name}; GRANT INSERT ON celetixo.authority_publications TO {name};")).unwrap();
        owner
            .batch_execute(&format!(
                "GRANT UPDATE(authority_profile) ON celetixo.repositories TO {name};"
            ))
            .unwrap();
        owner.batch_execute(&format!("GRANT INSERT ON celetixo.engine_mutations,celetixo.access_provenance,celetixo.transaction_outcomes,celetixo.committed_changes,celetixo.reservation_events,celetixo.operations,celetixo.evidence,celetixo.reference_evidence_admissions,celetixo.outbox,celetixo.outbox_delivery TO {name}; GRANT INSERT,UPDATE ON celetixo.transactions,celetixo.attempts,celetixo.reservations,celetixo.reservation_fences,celetixo.attempt_checks,celetixo.reference_evidence_signers TO {name}; GRANT INSERT,DELETE ON celetixo.reservation_claims TO {name}; GRANT UPDATE ON celetixo.repository_heads TO {name};")).unwrap();
        if role == "runtime_other" {
            owner.batch_execute(&format!("GRANT INSERT ON celetixo.principals,celetixo.grants,celetixo.grant_permissions TO {name}; GRANT UPDATE(active,generation) ON celetixo.grants TO {name};")).unwrap();
        }
    }
}

fn sql_authority() -> PostgresAuthority {
    PostgresAuthority::connect(
        connection("runtime", None),
        connection("runtime_other", None),
    )
    .unwrap()
}
fn native_model(generation: u64) -> EngineeringModel {
    EngineeringModel {
        generation,
        analyzer: "finite-reference".into(),
        version: "sql-registration/v1".into(),
        configuration: Digest::of("sql-registration-test/v1", b"model"),
        dependencies: BTreeMap::new(),
        completeness: Completeness::Complete,
    }
}
fn authority_error(
    failure: celetixo_engine::engine::authority::AuthorityFailure,
) -> celetixo_engine::engine::EngineError {
    celetixo_engine::engine::EngineError::Authority(failure)
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_registration_reopens_exact_state_preserves_uncertainty_and_requires_fresh_sessions() {
    use celetixo_engine::engine::{Engine, EngineError, Permission};
    use celetixo_engine::ids::PrincipalId;
    let mut owner = owner();
    let selected = scope("native-registration");
    authorize_registration(&mut owner, &selected);
    let backend = Arc::new(sql_authority());
    let (engine, administrator) = Engine::with_authority_backend(backend.clone());
    let worker = administrator
        .grant(
            PrincipalId::new("worker").unwrap(),
            selected.clone(),
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    let base = snapshot(selected.clone(), 0);
    engine
        .register(&worker, base.clone(), native_model(1))
        .unwrap();
    let mut updated = native_model(2);
    updated.completeness = Completeness::Incomplete {
        reasons: vec!["unknown\0dependency λ".into()],
    };
    engine
        .update_model(&worker, &selected, updated.clone())
        .unwrap();
    let before = backend.inspect_registration(&selected).unwrap();
    assert_eq!(before.state_generation, 3);
    assert!(before.records.clock_ms > 0);
    assert_eq!(before.records.repository.as_ref().unwrap().model, updated);
    for _ in 0..3 {
        assert_eq!(engine.current(&worker, &selected).unwrap(), base);
    }
    assert_eq!(
        backend.inspect_registration(&selected).unwrap(),
        before,
        "observations cannot publish clocks or audit rows"
    );
    // Private snapshot candidates never become proposal bases on reconstruction.
    let candidate = snapshot(selected.clone(), 1);
    SnapshotCatalog::connect(&connection("runtime", None))
        .unwrap()
        .put_candidate(&candidate)
        .unwrap();
    assert_eq!(backend.inspect_registration(&selected).unwrap(), before);
    drop(administrator);
    drop(engine);
    let reopened = Arc::new(sql_authority());
    let (engine, administrator) = Engine::with_authority_backend(reopened.clone());
    assert_eq!(
        engine.current(&worker, &selected),
        Err(EngineError::Unauthorized)
    );
    let grant = *before.records.grants.keys().next().unwrap();
    assert!(matches!(
        administrator
            .authenticate_existing_session(&selected, grant, |_, _| Err(EngineError::Unauthorized)),
        Err(EngineError::Unauthorized)
    ));
    let session = administrator
        .authenticate_existing_session(&selected, grant, |principal, scope| {
            assert_eq!(principal, worker.principal());
            assert_eq!(scope, &selected);
            Ok(())
        })
        .unwrap();
    assert_eq!(engine.current(&session, &selected).unwrap(), base);
    // A domain rejection still completes the update/publication protocol.
    assert_eq!(
        engine.register(&session, base.clone(), native_model(1)),
        Err(EngineError::AlreadyExists)
    );
    assert_eq!(
        reopened
            .inspect_registration(&selected)
            .unwrap()
            .state_generation,
        4
    );
    administrator.revoke(&session).unwrap();
    assert_eq!(
        engine.current(&session, &selected),
        Err(EngineError::Unauthorized)
    );
    let view = reopened.inspect_registration(&selected).unwrap();
    assert!(!view.records.grants[&grant].active);
    assert_eq!(view.state_generation, 5);
    let runtime = connection("runtime", None).connect().unwrap();
    drop(runtime);
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    let rows=owner.query("SELECT actor_kind,authenticated_database_role::text,authenticated_database_role_oid,requested_principal_id::text FROM celetixo.authority_publications WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text ORDER BY state_generation",&[&a[0],&a[1],&a[2]]).unwrap();
    let config = fixture();
    assert_eq!(rows.len(), 5);
    for (row, kind, role) in [
        (&rows[0], "EmbeddingAdministrator", "runtime_other"),
        (&rows[1], "Session", "runtime"),
        (&rows[4], "EmbeddingAdministrator", "runtime_other"),
    ] {
        assert_eq!(row.get::<_, String>(0), kind);
        assert_eq!(row.get::<_, String>(1), config.roles[role].user);
        let oid: u32 = owner
            .query_one(
                "SELECT oid FROM pg_roles WHERE rolname=$1",
                &[&config.roles[role].user],
            )
            .unwrap()
            .get(0);
        assert_eq!(row.get::<_, u32>(2), oid);
    }
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_registration_rolls_back_failures_and_withholds_lost_commit_acknowledgement() {
    use celetixo_engine::engine::authority::AuthorityFailure;
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    let mut owner = owner();
    let selected = scope("native-faults");
    authorize_registration(&mut owner, &selected);
    let backend = Arc::new(sql_authority());
    let (engine, administrator) = Engine::with_authority_backend(backend.clone());
    let worker = administrator
        .grant(
            PrincipalId::new("worker").unwrap(),
            selected.clone(),
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    let before = backend.inspect_registration(&selected).unwrap();
    let fault = Arc::new(sql_authority().with_checkpoint(Arc::new(|stage| {
        if stage == AuthorityStage::BeforeCommit {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (failed, auth) = Engine::with_authority_backend(fault);
    let grant = *before.records.grants.keys().next().unwrap();
    let session = auth
        .authenticate_existing_session(&selected, grant, |_, _| Ok(()))
        .unwrap();
    let base = snapshot(selected.clone(), 0);
    assert_eq!(
        failed.register(&session, base.clone(), native_model(1)),
        Err(authority_error(AuthorityFailure::Unavailable))
    );
    assert_eq!(backend.inspect_registration(&selected).unwrap(), before);
    // Failure after confirmed server commit simulates a withheld response. No
    // callback retry or ordinary domain response is permitted by this hook.
    let fault = Arc::new(sql_authority().with_checkpoint(Arc::new(|stage| {
        if stage == AuthorityStage::AfterCommit {
            Err(AuthorityFailure::Unavailable)
        } else {
            Ok(())
        }
    })));
    let (uncertain, auth) = Engine::with_authority_backend(fault);
    let session = auth
        .authenticate_existing_session(&selected, grant, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        uncertain.register(&session, base.clone(), native_model(1)),
        Err(authority_error(AuthorityFailure::CommitIndeterminate))
    );
    let observed = backend.inspect_registration(&selected).unwrap();
    assert_eq!(observed.state_generation, before.state_generation + 1);
    assert_eq!(
        observed.records.repository.as_ref().unwrap().current,
        base.id()
    );
    assert_eq!(engine.current(&worker, &selected).unwrap(), base);
    assert_eq!(observed.records.grants, before.records.grants);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_native_preserves_lifecycle_and_rejects_budget_clock_and_scope_without_publication() {
    use celetixo_engine::engine::authority::{AuthorityFailure, NativeRecordLimits};
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::evidence::CheckSpec;
    use celetixo_engine::ids::{PrincipalId, TransactionId};
    use celetixo_engine::transaction::TransactionIntent;
    let mut owner = owner();
    let selected = scope("native-rejects");
    authorize_registration(&mut owner, &selected);
    let backend = Arc::new(sql_authority());
    let (engine, administrator) = Engine::with_authority_backend(backend.clone());
    let worker = administrator
        .grant(
            PrincipalId::new("worker").unwrap(),
            selected.clone(),
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    let base = snapshot(selected.clone(), 0);
    engine
        .register(&worker, base.clone(), native_model(1))
        .unwrap();
    let check = CheckSpec::new(
        CheckId::new("check").unwrap(),
        vec!["reference-not-executed".into()],
        "reference/v1".into(),
        Digest::of("sql-test/v1", b"environment"),
        Digest::of("sql-test/v1", b"config"),
    )
    .unwrap();
    assert_eq!(
        engine
            .propose(
                &worker,
                &selected,
                TransactionId::new("supported").unwrap(),
                base.id(),
                TransactionIntent {
                    objective: "native lifecycle survives recovery".into(),
                    accesses: Default::default(),
                    invariants: vec![],
                    required_checks: vec![check],
                    depends_on: BTreeSet::new()
                }
            )
            .map(|view| view.state),
        Ok(celetixo_engine::transaction::TransactionState::Proposed)
    );
    let before = backend.inspect_registration(&selected).unwrap();
    assert_eq!(
        before.records.transactions[&TransactionId::new("supported").unwrap()].state,
        celetixo_engine::transaction::TransactionState::Proposed
    );
    let foreign = scope("unmapped");
    assert_eq!(
        backend.inspect_registration(&foreign),
        Err(AuthorityFailure::ScopeDenied)
    );
    let small = sql_authority()
        .with_limits(NativeRecordLimits {
            value_bytes: 1,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        small.inspect_registration(&selected),
        Err(AuthorityFailure::Integrity)
    );
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    owner.execute("UPDATE celetixo.engine_counters SET last_clock_ms='18446744073709551615' WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text",&[&a[0],&a[1],&a[2]]).unwrap();
    // Counter-only tampering is detached from the append-only publication and
    // fails checked recovery. Inject a matching NEW publication to exercise a
    // coherent future clock without rewriting any historical audit record.
    assert_eq!(
        backend.inspect_registration(&selected),
        Err(AuthorityFailure::Integrity)
    );
    owner.execute("INSERT INTO celetixo.authority_publications (organization_id,workspace_id,repository_id,state_generation,observed_clock_ms,authenticated_database_role,authenticated_database_role_oid,actor_kind,snapshot_digest) SELECT organization_id,workspace_id,repository_id,state_generation+1,'18446744073709551615',session_user::name,(SELECT oid FROM pg_roles WHERE rolname=session_user),'EmbeddingAdministrator',snapshot_digest FROM celetixo.authority_publications WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text ORDER BY state_generation DESC LIMIT 1", &[&a[0],&a[1],&a[2]]).unwrap();
    owner.execute("UPDATE celetixo.repositories SET state_generation=state_generation+1 WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&a[0],&a[1],&a[2]]).unwrap();
    let future = backend.inspect_registration(&selected).unwrap();
    assert_eq!(
        engine.current(&worker, &selected),
        Err(authority_error(AuthorityFailure::ClockRegression))
    );
    assert_eq!(
        engine.update_model(&worker, &selected, native_model(2)),
        Err(authority_error(AuthorityFailure::ClockRegression))
    );
    assert_eq!(backend.inspect_registration(&selected).unwrap(), future);
    // Preserve the injected future-clock fixture for operator review; no ledger
    // or clock is rewritten to disguise the explicit rejection.
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_registration_serializes_independent_instances_and_bootstrap_creators() {
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    let mut owner = owner();
    let selected = scope("native-races");
    authorize_registration(&mut owner, &selected);
    let barrier = Arc::new(Barrier::new(2));
    let children = (0..2)
        .map(|i| {
            let selected = selected.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let (engine, administrator) =
                    Engine::with_authority_backend(Arc::new(sql_authority()));
                barrier.wait();
                let worker = administrator
                    .grant(
                        PrincipalId::new(format!("worker-{i}")).unwrap(),
                        selected.clone(),
                        Permission::ALL.into_iter().collect(),
                    )
                    .unwrap();
                (engine, worker)
            })
        })
        .collect::<Vec<_>>();
    let handles = children
        .into_iter()
        .map(|child| child.join().unwrap())
        .collect::<Vec<_>>();
    let backend = sql_authority();
    let view = backend.inspect_registration(&selected).unwrap();
    assert_eq!(view.state_generation, 2);
    assert_eq!(view.records.grants.len(), 2);
    assert_eq!(view.records.session_counter, 2);
    let base = snapshot(selected.clone(), 0);
    let barrier = Arc::new(Barrier::new(2));
    let children = handles
        .into_iter()
        .map(|(engine, worker)| {
            let base = base.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                engine.register(&worker, base, native_model(1))
            })
        })
        .collect::<Vec<_>>();
    let results = children
        .into_iter()
        .map(|child| child.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(celetixo_engine::engine::EngineError::AlreadyExists))
            .count(),
        1
    );
    let view = backend.inspect_registration(&selected).unwrap();
    assert_eq!(view.state_generation, 4);
    assert_eq!(view.records.repository.unwrap().current, base.id());
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_registration_roles_scope_revocation_and_live_schema_fencing_fail_closed() {
    use celetixo_engine::engine::authority::AuthorityFailure;
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    let mut owner = owner();
    let selected = scope("native-roles");
    authorize_registration(&mut owner, &selected);
    assert!(matches!(
        PostgresAuthority::connect(
            connection("migration", None),
            connection("runtime_other", None)
        ),
        Err(AuthorityFailure::ScopeDenied)
    ));
    assert!(matches!(
        PostgresAuthority::connect(connection("runtime", None), connection("runtime", None)),
        Err(AuthorityFailure::ScopeDenied)
    ));
    assert!(matches!(
        PostgresAuthority::connect(
            connection("runtime", None),
            connection("runtime_other", Some("another_database"))
        ),
        Err(AuthorityFailure::UnsupportedProfile)
    ));
    let backend = Arc::new(sql_authority());
    let (engine, administrator) = Engine::with_authority_backend(backend.clone());
    let worker = administrator
        .grant(
            PrincipalId::new("worker").unwrap(),
            selected.clone(),
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    let base = snapshot(selected.clone(), 0);
    engine
        .register(&worker, base.clone(), native_model(1))
        .unwrap();
    let config = fixture();
    let runtime = &config.roles["runtime"].user;
    let admin_name = &config.roles["runtime_other"].user;
    let a = [
        selected.organization.as_str(),
        selected.workspace.as_str(),
        selected.repository.as_str(),
    ];
    let mut service = connection("runtime", None).connect().unwrap();
    assert!(
        service
            .batch_execute("UPDATE celetixo.db_role_scopes SET authorized_at=clock_timestamp()")
            .is_err()
    );
    assert!(
        service
            .batch_execute("DELETE FROM celetixo.schema_migrations")
            .is_err()
    );
    assert!(
        service
            .execute(
                "UPDATE celetixo.grants SET active=false WHERE organization_id=$1::text",
                &[&a[0]]
            )
            .is_err()
    );
    owner.execute("UPDATE celetixo.db_role_scopes SET database_role_oid=0 WHERE database_role=$1::name AND organization_id=$2::text AND workspace_id=$3::text AND repository_id=$4::text",&[runtime,&a[0],&a[1],&a[2]]).unwrap();
    assert_eq!(
        engine.current(&worker, &selected),
        Err(authority_error(AuthorityFailure::ScopeDenied))
    );
    owner.execute("UPDATE celetixo.db_role_scopes SET database_role_oid=(SELECT oid FROM pg_roles WHERE rolname=$1) WHERE database_role=$1::name AND organization_id=$2::text AND workspace_id=$3::text AND repository_id=$4::text",&[runtime,&a[0],&a[1],&a[2]]).unwrap();
    assert_eq!(engine.current(&worker, &selected).unwrap(), base);
    owner
        .batch_execute(&format!(
            "GRANT UPDATE(active) ON celetixo.grants TO {runtime}"
        ))
        .unwrap();
    let elevated = engine.current(&worker, &selected);
    owner
        .batch_execute(&format!(
            "REVOKE UPDATE(active) ON celetixo.grants FROM {runtime}"
        ))
        .unwrap();
    assert_eq!(
        elevated,
        Err(authority_error(AuthorityFailure::ScopeDenied))
    );
    let mut role_administrator = connection("admin", None).connect().unwrap();
    role_administrator
        .batch_execute(&format!("GRANT {admin_name} TO {runtime}"))
        .unwrap();
    let inherited = engine.current(&worker, &selected);
    role_administrator
        .batch_execute(&format!("REVOKE {admin_name} FROM {runtime}"))
        .unwrap();
    assert_eq!(
        inherited,
        Err(authority_error(AuthorityFailure::ScopeDenied))
    );
    // An existing adapter coordinates each observation with a live migrator.
    let mut tx = owner.transaction().unwrap();
    tx.query_one(
        "SELECT pg_advisory_xact_lock($1)",
        &[&0x43454c455449584fi64],
    )
    .unwrap();
    let observer = backend.clone();
    let scope_copy = selected.clone();
    let child = std::thread::spawn(move || observer.inspect_registration(&scope_copy));
    // A real exclusive migration lock blocks this reader before its snapshot.
    let mut monitor = connection("migration", None).connect().unwrap();
    let start = std::time::Instant::now();
    loop {
        let waiting:bool=monitor.query_one("SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid WHERE l.locktype='advisory' AND NOT l.granted AND a.usename=$1)",&[admin_name]).unwrap().get(0);
        if waiting {
            break;
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "reader did not reach migration lock"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    tx.rollback().unwrap();
    assert!(child.join().unwrap().is_ok());
    // Role mapping revocation is independently checked by the runtime login.
    owner.execute("DELETE FROM celetixo.db_role_scopes WHERE database_role=$1::name AND organization_id=$2::text AND workspace_id=$3::text AND repository_id=$4::text",&[runtime,&a[0],&a[1],&a[2]]).unwrap();
    assert_eq!(
        engine.current(&worker, &selected),
        Err(authority_error(AuthorityFailure::ScopeDenied))
    );
}

fn sql_registration_crash_child() {
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    let selected = Scope {
        organization: OrganizationId::new(
            std::env::var("CELETIXO_PG_NATIVE_ORGANIZATION").unwrap(),
        )
        .unwrap(),
        workspace: WorkspaceId::new("workspace").unwrap(),
        repository: RepositoryId::new("repository").unwrap(),
    };
    let ready = PathBuf::from(std::env::var_os("CELETIXO_PG_NATIVE_READY").unwrap());
    assert!(ready.is_absolute());
    let checkpoint = Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            std::fs::write(&ready, b"uncommitted authority rows staged").unwrap();
            // Parent kills this process at a named boundary. A hard upper bound
            // also lets the database idle-transaction deadline fail closed.
            std::thread::sleep(std::time::Duration::from_secs(20));
            panic!("parent did not terminate the crash child");
        }
        Ok(())
    });
    let (engine, administrator) =
        Engine::with_authority_backend(Arc::new(sql_authority().with_checkpoint(checkpoint)));
    let _ = engine;
    administrator
        .grant(
            PrincipalId::new("crash-worker").unwrap(),
            selected,
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    panic!("crash child cannot acknowledge publication");
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_registration_process_death_rolls_back_before_a_fresh_independent_writer() {
    use celetixo_engine::engine::authority::AuthorityFailure;
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    if std::env::var_os("CELETIXO_PG_NATIVE_CHILD").is_some() {
        sql_registration_crash_child();
        return;
    }
    let mut owner = owner();
    let selected = scope("native-process-death");
    authorize_registration(&mut owner, &selected);
    let dir = tempfile::tempdir().unwrap();
    let ready = dir.path().join("ready");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "sql_registration_process_death_rolls_back_before_a_fresh_independent_writer",
            "--ignored",
        ])
        .env("CELETIXO_PG_NATIVE_CHILD", "1")
        .env(
            "CELETIXO_PG_NATIVE_ORGANIZATION",
            selected.organization.as_str(),
        )
        .env("CELETIXO_PG_NATIVE_READY", &ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    while !ready.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("crash child exited before publication boundary: {status}");
        }
        if start.elapsed() > std::time::Duration::from_secs(5) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("crash child failed to reach publication boundary");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let backend = Arc::new(sql_authority());
    assert_eq!(
        backend.inspect_registration(&selected),
        Err(AuthorityFailure::ScopeDenied)
    );
    let (_, administrator) = Engine::with_authority_backend(backend.clone());
    administrator
        .grant(
            PrincipalId::new("replacement").unwrap(),
            selected.clone(),
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    let view = backend.inspect_registration(&selected).unwrap();
    assert_eq!(view.state_generation, 1);
    assert_eq!(view.records.session_counter, 1);
    assert_eq!(view.records.grants.len(), 1);
    assert_eq!(
        view.records
            .grants
            .values()
            .next()
            .unwrap()
            .principal
            .as_str(),
        "replacement"
    );
    // The new operation also proves the dead process released its schema and
    // repository locks. Neither a partial grant nor publication survived.
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_registration_waiting_reader_observes_coherent_state_or_explicit_contention() {
    use celetixo_engine::engine::authority::AuthorityFailure;
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    let mut owner = owner();
    let selected = scope("native-coherent-reader");
    authorize_registration(&mut owner, &selected);
    let backend = Arc::new(sql_authority());
    let (_, administrator) = Engine::with_authority_backend(backend.clone());
    administrator
        .grant(
            PrincipalId::new("first").unwrap(),
            selected.clone(),
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    let release = Arc::new(Barrier::new(2));
    let gate = release.clone();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let writer = Arc::new(sql_authority().with_checkpoint(Arc::new(move |stage| {
        if stage == AuthorityStage::BeforeCommit {
            send.send(()).unwrap();
            gate.wait();
        }
        Ok(())
    })));
    let scope_copy = selected.clone();
    let child = std::thread::spawn(move || {
        let (_, administrator) = Engine::with_authority_backend(writer);
        administrator.grant(
            PrincipalId::new("second").unwrap(),
            scope_copy,
            Permission::ALL.into_iter().collect(),
        )
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let observer = backend.clone();
    let scope_copy = selected.clone();
    let reader = std::thread::spawn(move || observer.inspect_registration(&scope_copy));
    let start = std::time::Instant::now();
    let config = fixture();
    let login = &config.roles["runtime_other"].user;
    loop {
        let waiting:bool=owner.query_one("SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid WHERE NOT l.granted AND a.usename=$1)",&[login]).unwrap().get(0);
        if waiting {
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(1) {
            release.wait();
            child.join().unwrap().unwrap();
            panic!("reader did not wait on repository publication");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    release.wait();
    child.join().unwrap().unwrap();
    match reader.join().unwrap() {
        Ok(view) => {
            assert_eq!(view.state_generation, 2);
            assert_eq!(view.records.grants.len(), 2);
        }
        Err(error) => assert_eq!(error, AuthorityFailure::Contention),
    }
    let view = backend.inspect_registration(&selected).unwrap();
    assert_eq!(view.state_generation, 2);
    assert_eq!(view.records.grants.len(), 2);
}

#[test]
#[ignore = "requires the reviewed authenticated PostgreSQL fixture"]
fn sql_registration_checks_the_new_schema_after_waiting_for_a_migrator() {
    use celetixo_engine::engine::authority::AuthorityFailure;
    use celetixo_engine::engine::{Engine, Permission};
    use celetixo_engine::ids::PrincipalId;
    let database = format!("celetixo_native_schema_fence_{}", std::process::id());
    let config = fixture();
    let migration = &config.roles["migration"].user;
    assert!(
        migration
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    );
    let mut cluster = connection("admin", None).connect().unwrap();
    cluster
        .batch_execute(&format!("CREATE DATABASE {database} OWNER {migration}"))
        .unwrap();
    let mut owner = connection("migration", Some(&database)).connect().unwrap();
    migrate(&mut owner).unwrap();
    let selected = scope("native-schema-fence");
    authorize_registration(&mut owner, &selected);
    let backend = Arc::new(
        PostgresAuthority::connect(
            connection("runtime", Some(&database)),
            connection("runtime_other", Some(&database)),
        )
        .unwrap(),
    );
    let (_, administrator) = Engine::with_authority_backend(backend.clone());
    administrator
        .grant(
            PrincipalId::new("worker").unwrap(),
            selected.clone(),
            Permission::ALL.into_iter().collect(),
        )
        .unwrap();
    let before = backend.inspect_registration(&selected).unwrap();
    let mut migration = owner.transaction().unwrap();
    migration
        .query_one(
            "SELECT pg_advisory_xact_lock($1)",
            &[&0x43454c455449584fi64],
        )
        .unwrap();
    let observer = backend.clone();
    let scope_copy = selected.clone();
    let reader = std::thread::spawn(move || observer.inspect_registration(&scope_copy));
    let mut monitor = connection("migration", Some(&database)).connect().unwrap();
    let start = std::time::Instant::now();
    loop {
        let waiting:bool=monitor.query_one("SELECT EXISTS(SELECT 1 FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid WHERE l.locktype='advisory' AND NOT l.granted AND a.datname=$1)",&[&database]).unwrap().get(0);
        if waiting {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Simulate a newer writer's schema ledger in a dedicated disposable database.
    // Retain it afterward; never repair a published database's checksum history.
    migration.execute("INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (8,'unsupported-future-fixture',$1::bytea)",&[&&[0u8;32][..]]).unwrap();
    migration.commit().unwrap();
    assert_eq!(
        reader.join().unwrap(),
        Err(AuthorityFailure::SchemaMismatch)
    );
    assert_eq!(
        administrator
            .grant(
                PrincipalId::new("blocked").unwrap(),
                selected.clone(),
                BTreeSet::new()
            )
            .err(),
        Some(authority_error(AuthorityFailure::SchemaMismatch))
    );
    let generation:String=owner.query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text",&[&selected.organization.as_str()]).unwrap().get(0);
    assert_eq!(generation, before.state_generation.to_string());
}

#[path = "foundation/native_authority.rs"]
mod native_authority;
#[path = "foundation/source_authority.rs"]
mod source_authority;
