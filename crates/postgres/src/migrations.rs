use crate::StoreError;
use postgres::{Client, GenericClient};
use sha2::{Digest as _, Sha256};

const MIGRATION: &str = include_str!("../migrations/0001_authority.sql");
const NAME: &str = "0001_authority";
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
    let digest: [u8; 32] = Sha256::digest(MIGRATION.as_bytes()).into();
    let rows = transaction.query("SELECT version, migration_name, migration_sha256 FROM celetixo.schema_migrations ORDER BY version", &[]).map_err(StoreError::database)?;
    let newly_applied = rows.is_empty();
    if newly_applied {
        transaction
            .batch_execute(MIGRATION)
            .map_err(StoreError::database)?;
        checkpoint(MigrationStage::AfterDdl)?;
        transaction.execute("INSERT INTO celetixo.schema_migrations (version, migration_name, migration_sha256) VALUES (1,$1,$2)", &[&NAME, &&digest[..]]).map_err(StoreError::database)?;
    } else if rows.len() != 1
        || rows[0].get::<_, i64>(0) != 1
        || rows[0].get::<_, String>(1) != NAME
        || rows[0].get::<_, Vec<u8>>(2) != digest
    {
        return Err(StoreError::SchemaMismatch);
    }
    checkpoint(MigrationStage::BeforeCommit)?;
    transaction
        .commit()
        .map_err(|_| StoreError::CommitIndeterminate)?;
    Ok(MigrationReceipt {
        schema_version: 1,
        migration_sha256: digest,
        newly_applied,
    })
}

pub fn validate_schema(client: &mut impl GenericClient) -> Result<(), StoreError> {
    let rows = client.query("SELECT version, migration_name, migration_sha256 FROM celetixo.schema_migrations ORDER BY version", &[]).map_err(StoreError::database)?;
    if rows.len() != 1
        || rows[0].get::<_, i64>(0) != 1
        || rows[0].get::<_, String>(1) != NAME
        || rows[0].get::<_, Vec<u8>>(2).as_slice()
            != Sha256::digest(MIGRATION.as_bytes()).as_slice()
    {
        return Err(StoreError::SchemaMismatch);
    }
    Ok(())
}
