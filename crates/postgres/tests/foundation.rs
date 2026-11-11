//! Actual PostgreSQL tests are opt-in; a configured workflow is not durability proof.
#![cfg(unix)]
use celetixo_engine::ids::{
    Digest, OrganizationId, RepositoryId, Revision, Scope, SnapshotId, WorkspaceId,
};
use celetixo_engine::snapshot::{InclusionPolicy, Snapshot, SnapshotManifest};
use celetixo_postgres::{
    MigrationStage, SnapshotCatalog, StoreError, UnixConnection, migrate, migrate_with_checkpoint,
    validate_schema,
};
use postgres::Client;
use serde::Deserialize;
use std::collections::BTreeMap;
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
fn owner() -> Client {
    let mut c = connection("migration", None).connect().unwrap();
    migrate(&mut c).unwrap();
    c
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
        "UPDATE celetixo.schema_migrations SET version=2",
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
fn snapshot_catalog_survives_actual_database_process_restart() {
    let config = fixture();
    let selected = scope("restart");
    {
        let mut admin = owner();
        authorize(&mut admin, "runtime", &selected);
        let mut catalog = SnapshotCatalog::connect(&connection("runtime", None)).unwrap();
        catalog
            .register_initial(&snapshot(selected.clone(), 123))
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
