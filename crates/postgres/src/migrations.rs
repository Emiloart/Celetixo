use crate::StoreError;
use postgres::{Client, GenericClient};
use sha2::{Digest as _, Sha256};

struct Migration {
    name: &'static str,
    sql: &'static str,
}
const MIGRATIONS: &[Migration] = &[
    Migration {
        name: "0001_authority",
        sql: include_str!("../migrations/0001_authority.sql"),
    },
    Migration {
        name: "0002_lifecycle_reconstruction",
        sql: include_str!("../migrations/0002_lifecycle_reconstruction.sql"),
    },
    Migration {
        name: "0003_native_lifecycle",
        sql: include_str!("../migrations/0003_native_lifecycle.sql"),
    },
];
const LOCK: i64 = 0x43454c455449584f;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReceipt {
    pub schema_version: i64,
    pub migration_sha256: [u8; 32],
    pub newly_applied: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStage {
    AfterDdl,
    BeforeCommit,
}

/// A migration is a trusted operator action under the schema-owner login. All
/// DDL and its checksum ledger publish in one explicit PostgreSQL transaction.
pub fn migrate(client: &mut Client) -> Result<MigrationReceipt, StoreError> {
    migrate_with_checkpoint(client, |_| Ok(()))
}

/// Trusted operator failure-injection boundary. It cannot grant database
/// privileges or skip validation; returning an error rolls back the migration.
pub fn migrate_with_checkpoint(
    client: &mut Client,
    mut checkpoint: impl FnMut(MigrationStage) -> Result<(), StoreError>,
) -> Result<MigrationReceipt, StoreError> {
    let mut transaction = client.transaction().map_err(StoreError::database)?;
    transaction
        .query_one("SELECT pg_advisory_xact_lock($1)", &[&LOCK])
        .map_err(StoreError::database)?;
    transaction.batch_execute("CREATE SCHEMA IF NOT EXISTS celetixo; REVOKE ALL ON SCHEMA celetixo FROM PUBLIC; CREATE TABLE IF NOT EXISTS celetixo.schema_migrations (version BIGINT PRIMARY KEY CHECK (version > 0), migration_name TEXT NOT NULL, migration_sha256 BYTEA NOT NULL CHECK (octet_length(migration_sha256)=32), applied_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()); REVOKE ALL ON celetixo.schema_migrations FROM PUBLIC;").map_err(StoreError::database)?;
    let rows = transaction.query("SELECT version, migration_name, migration_sha256 FROM celetixo.schema_migrations ORDER BY version", &[]).map_err(StoreError::database)?;
    validate_history(&rows)?;
    let newly_applied = rows.len() < MIGRATIONS.len();
    for (offset, migration) in MIGRATIONS.iter().enumerate().skip(rows.len()) {
        let version = i64::try_from(offset + 1).map_err(|_| StoreError::SchemaMismatch)?;
        let digest = checksum(migration);
        transaction
            .batch_execute(migration.sql)
            .map_err(StoreError::database)?;
        checkpoint(MigrationStage::AfterDdl)?;
        transaction.execute("INSERT INTO celetixo.schema_migrations (version, migration_name, migration_sha256) VALUES ($1,$2,$3)", &[&version,&migration.name,&&digest[..]]).map_err(StoreError::database)?;
    }
    checkpoint(MigrationStage::BeforeCommit)?;
    transaction
        .commit()
        .map_err(|_| StoreError::CommitIndeterminate)?;
    Ok(MigrationReceipt {
        schema_version: i64::try_from(MIGRATIONS.len()).map_err(|_| StoreError::SchemaMismatch)?,
        migration_sha256: checksum(&MIGRATIONS[MIGRATIONS.len() - 1]),
        newly_applied,
    })
}

pub fn validate_schema(client: &mut impl GenericClient) -> Result<(), StoreError> {
    let rows = client.query("SELECT version, migration_name, migration_sha256 FROM celetixo.schema_migrations ORDER BY version", &[]).map_err(StoreError::database)?;
    validate_history(&rows)?;
    if rows.len() != MIGRATIONS.len() {
        return Err(StoreError::SchemaMismatch);
    }
    Ok(())
}

fn checksum(migration: &Migration) -> [u8; 32] {
    Sha256::digest(migration.sql.as_bytes()).into()
}

fn validate_history(rows: &[postgres::Row]) -> Result<(), StoreError> {
    if rows.len() > MIGRATIONS.len() {
        return Err(StoreError::SchemaMismatch);
    }
    for (offset, row) in rows.iter().enumerate() {
        let expected = &MIGRATIONS[offset];
        if row.get::<_, i64>(0)
            != i64::try_from(offset + 1).map_err(|_| StoreError::SchemaMismatch)?
            || row.get::<_, String>(1) != expected.name
            || row.get::<_, Vec<u8>>(2) != checksum(expected)
        {
            return Err(StoreError::SchemaMismatch);
        }
    }
    Ok(())
}
