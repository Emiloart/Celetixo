use crate::{StoreError, UnixConnection, validate_schema};
use celetixo_engine::ids::{Digest, Scope, SnapshotId};
use celetixo_engine::persistence::{SnapshotDecodeLimits, restore_snapshot};
use celetixo_engine::snapshot::Snapshot;
use postgres::{Client, GenericClient, IsolationLevel};

const FORMAT: &str = "celetixo/snapshot/v1";

/// Durable immutable snapshot catalog, not an Engine transaction service. A
/// catalog connection is a trusted service identity governed by database RLS.
pub struct SnapshotCatalog {
    client: Client,
}

impl SnapshotCatalog {
    pub fn connect(configuration: &UnixConnection) -> Result<Self, StoreError> {
        let mut client = configuration.connect()?;
        validate_schema(&mut client)?;
        Ok(Self { client })
    }

    /// Establish initial catalog state. It cannot advance an existing head.
    /// This does not register semantic facts or grant transaction authority.
    pub fn register_initial(&mut self, snapshot: &Snapshot) -> Result<(), StoreError> {
        validate_snapshot(snapshot)?;
        let scope = snapshot.scope();
        let args = scope_parameters(scope);
        let mut tx = self.client.transaction().map_err(StoreError::database)?;
        tx.execute("INSERT INTO celetixo.organizations (organization_id) VALUES ($1::text) ON CONFLICT DO NOTHING", &[&args[0]]).map_err(StoreError::database)?;
        tx.execute("INSERT INTO celetixo.workspaces (organization_id,workspace_id) VALUES ($1::text,$2::text) ON CONFLICT DO NOTHING", &[&args[0],&args[1]]).map_err(StoreError::database)?;
        tx.execute("INSERT INTO celetixo.repositories (organization_id,workspace_id,repository_id) VALUES ($1::text,$2::text,$3::text) ON CONFLICT DO NOTHING", &[&args[0],&args[1],&args[2]]).map_err(StoreError::database)?;
        tx.query_one("SELECT state_generation::text FROM celetixo.repositories WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text FOR UPDATE", &[&args[0],&args[1],&args[2]]).map_err(StoreError::database)?;
        persist(&mut tx, snapshot)?;
        let revision = snapshot.revision().0.to_string();
        tx.execute("INSERT INTO celetixo.repository_heads (organization_id,workspace_id,repository_id,revision,snapshot_digest) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::bytea) ON CONFLICT DO NOTHING", &[&args[0],&args[1],&args[2],&revision,&&snapshot.id().0.as_bytes()[..]]).map_err(StoreError::database)?;
        let row = tx.query_one("SELECT revision::text,snapshot_digest::bytea FROM celetixo.repository_heads WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text", &[&args[0],&args[1],&args[2]]).map_err(StoreError::database)?;
        if row.get::<_, String>(0) != revision
            || row.get::<_, Vec<u8>>(1) != snapshot.id().0.as_bytes()
        {
            return Err(StoreError::AlreadyRegistered);
        }
        tx.commit().map_err(|_| StoreError::CommitIndeterminate)
    }

    /// Store a private candidate without changing authoritative repository head.
    pub fn put_candidate(&mut self, snapshot: &Snapshot) -> Result<(), StoreError> {
        validate_snapshot(snapshot)?;
        let mut tx = self.client.transaction().map_err(StoreError::database)?;
        persist(&mut tx, snapshot)?;
        tx.commit().map_err(|_| StoreError::CommitIndeterminate)
    }

    pub fn read(&mut self, scope: &Scope, expected: SnapshotId) -> Result<Snapshot, StoreError> {
        let mut tx = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(StoreError::database)?;
        let snapshot = load(&mut tx, scope, expected)?;
        tx.commit().map_err(StoreError::database)?;
        Ok(snapshot)
    }
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<(), StoreError> {
    restore_snapshot(
        snapshot.id(),
        &snapshot.canonical_bytes(),
        SnapshotDecodeLimits::default(),
    )?;
    Ok(())
}

pub(crate) fn scope_parameters(scope: &Scope) -> [&str; 3] {
    [
        scope.organization.as_str(),
        scope.workspace.as_str(),
        scope.repository.as_str(),
    ]
}

fn persist(tx: &mut impl GenericClient, snapshot: &Snapshot) -> Result<(), StoreError> {
    let a = scope_parameters(snapshot.scope());
    let bytes = snapshot.canonical_bytes();
    let snapshot_id = snapshot.id();
    let id = &snapshot_id.0.as_bytes()[..];
    tx.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text,1,$6) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2],&id,&FORMAT,&bytes]).map_err(StoreError::database)?;
    tx.execute("INSERT INTO celetixo.snapshots (organization_id,workspace_id,repository_id,snapshot_digest,revision,manifest_digest,payload_digest) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text::numeric,$6::bytea,$4::bytea) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2],&id,&snapshot.revision().0.to_string(),&&snapshot.manifest().digest().as_bytes()[..]]).map_err(StoreError::database)?;
    if load(tx, snapshot.scope(), snapshot.id())? != *snapshot {
        return Err(StoreError::Integrity);
    }
    Ok(())
}

pub(crate) fn load(
    tx: &mut impl GenericClient,
    scope: &Scope,
    expected: SnapshotId,
) -> Result<Snapshot, StoreError> {
    let a = scope_parameters(scope);
    let row = tx.query_opt("SELECT s.revision::text,s.manifest_digest::bytea,s.payload_digest::bytea,p.payload_format::text,p.payload_version,p.payload_bytes FROM celetixo.snapshots s JOIN celetixo.domain_payloads p USING (organization_id,workspace_id,repository_id,payload_digest) WHERE s.organization_id=$1::text AND s.workspace_id=$2::text AND s.repository_id=$3::text AND s.snapshot_digest=$4::bytea", &[&a[0],&a[1],&a[2],&&expected.0.as_bytes()[..]]).map_err(StoreError::database)?.ok_or(StoreError::NotFound)?;
    if row.get::<_, String>(3) != FORMAT
        || row.get::<_, i32>(4) != 1
        || row.get::<_, Vec<u8>>(2) != expected.0.as_bytes()
    {
        return Err(StoreError::Integrity);
    }
    let snapshot = restore_snapshot(
        expected,
        &row.get::<_, Vec<u8>>(5),
        SnapshotDecodeLimits::default(),
    )?;
    let manifest = Digest::from_bytes(
        row.get::<_, Vec<u8>>(1)
            .try_into()
            .map_err(|_| StoreError::Integrity)?,
    );
    if snapshot.scope() != scope
        || snapshot.revision().0.to_string() != row.get::<_, String>(0)
        || snapshot.manifest().digest() != manifest
    {
        return Err(StoreError::Integrity);
    }
    Ok(snapshot)
}
