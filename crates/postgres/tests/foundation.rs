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
    MigrationStage, SemanticCatalog, SnapshotCatalog, StoreError, UnixConnection, migrate,
    migrate_with_checkpoint, validate_schema,
};
use postgres::Client;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

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

/// Build the exact supported, published v1 schema in a fresh disposable database.
/// This never rewrites the migration ledger of an existing fixture/database.
fn fresh_prior_database(label: &str, version: u8) -> (Client, Client, String, Snapshot) {
    assert!(matches!(version, 1 | 2));
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
        if version == 2 {
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
    assert_eq!(receipt.schema_version, 3);
    assert!(receipt.newly_applied);
    assert!(!migrate(&mut client).unwrap().newly_applied);
    validate_schema(&mut client).unwrap();
    for mutation in [
        "DELETE FROM celetixo.schema_migrations WHERE version=1",
        "UPDATE celetixo.schema_migrations SET migration_name='wrong' WHERE version=1",
        "INSERT INTO celetixo.schema_migrations (version,migration_name,migration_sha256) VALUES (4,'future',decode(repeat('00',32),'hex'))",
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
    assert_eq!(migrate(&mut client).unwrap().schema_version, 3);
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
    assert_eq!(migrate(&mut client).unwrap().schema_version, 3);
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
        "UPDATE celetixo.schema_migrations SET version=4 WHERE version=3",
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
