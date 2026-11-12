//! PostgreSQL catalogs and checked native lifecycle/commit authority.
//! Complete captured-source recovery and network service deployment are pending.
//! Rust evaluates domain transitions; there is no memory fallback or SQL
//! semantic decision service.
mod authority;
mod migrations;
mod semantic;
mod snapshots;

pub use authority::{AuthorityStage, PostgresAuthority, RegistrationView};
pub use migrations::{
    MigrationReceipt, MigrationStage, migrate, migrate_with_checkpoint, validate_schema,
};
pub use semantic::SemanticCatalog;
pub use snapshots::SnapshotCatalog;

use postgres::{Client, NoTls};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

/// Explicit trusted operator connection configuration. Secrets are neither
/// printable via Debug nor placed in a connection URL, command or environment.
/// The initial adapter supports authenticated Unix sockets only. Network/TLS
/// service deployment remains a separate, fail-closed integration.
pub struct UnixConnection {
    directory: PathBuf,
    database: String,
    user: String,
    password: Vec<u8>,
}

impl UnixConnection {
    pub fn new(
        directory: impl AsRef<Path>,
        database: &str,
        user: &str,
        password: &[u8],
    ) -> Result<Self, StoreError> {
        let directory = directory.as_ref();
        if !directory.is_absolute()
            || database.is_empty()
            || user.is_empty()
            || database.len() > 256
            || user.len() > 256
            || database.chars().chain(user.chars()).any(char::is_control)
            || password.is_empty()
            || password.len() > 4096
            || password.contains(&0)
        {
            return Err(StoreError::Configuration);
        }
        Ok(Self {
            directory: directory.into(),
            database: database.into(),
            user: user.into(),
            password: password.to_vec(),
        })
    }

    pub fn connect(&self) -> Result<Client, StoreError> {
        #[cfg(not(unix))]
        {
            return Err(StoreError::Configuration);
        }
        #[cfg(unix)]
        {
            let mut config = postgres::Config::new();
            config.host_path(&self.directory).dbname(&self.database).user(&self.user)
                .password(&self.password).application_name("celetixo-postgres-foundation")
                .connect_timeout(Duration::from_secs(5))
                .options("-c statement_timeout=10000 -c lock_timeout=2000 -c idle_in_transaction_session_timeout=10000 -c synchronous_commit=on -c search_path=pg_catalog");
            let mut client = config.connect(NoTls).map_err(StoreError::database)?;
            let version: i32 = client
                .query_one("SELECT current_setting('server_version_num')::integer", &[])
                .map_err(StoreError::database)?
                .get(0);
            if version / 10_000 != 17 {
                return Err(StoreError::UnsupportedServer);
            }
            let settings = client.query_one("SELECT current_setting('fsync'), current_setting('full_page_writes'), current_setting('synchronous_commit'), pg_is_in_recovery()", &[]).map_err(StoreError::database)?;
            if settings.get::<_, String>(0) != "on"
                || settings.get::<_, String>(1) != "on"
                || settings.get::<_, String>(2) != "on"
                || settings.get::<_, bool>(3)
            {
                return Err(StoreError::Durability);
            }
            Ok(client)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StoreError {
    #[error("invalid or unsupported PostgreSQL connection configuration")]
    Configuration,
    #[error("the reviewed PostgreSQL 17 server profile is required")]
    UnsupportedServer,
    #[error("required PostgreSQL primary/durability settings are unavailable")]
    Durability,
    #[error("PostgreSQL operation failed (SQLSTATE {code})")]
    Database { code: String },
    #[error("PostgreSQL migration history differs from the compiled schema")]
    SchemaMismatch,
    #[error("migration interrupted at an explicit operator checkpoint")]
    MigrationInterrupted,
    #[error("durable record is missing or not authorized in this scope")]
    NotFound,
    #[error("immutable durable record content or binding differs")]
    Integrity,
    #[error("repository already has a different initial snapshot")]
    AlreadyRegistered,
    #[error("database commit outcome is indeterminate; reconcile durable state before retrying")]
    CommitIndeterminate,
    #[error(transparent)]
    Snapshot(#[from] celetixo_engine::persistence::RestoreError),
}

impl StoreError {
    // Database details can contain source rows, SQL or credentials. Keep only
    // the stable classification at the public diagnostic boundary.
    fn database(error: postgres::Error) -> Self {
        Self::Database {
            code: error.code().map_or("connection", |c| c.code()).into(),
        }
    }
}
