//! Immutable semantic facts. This catalog cannot install authoritative model
//! heads, admit transactions, renew leases, record verification or commit source.
use crate::{StoreError, UnixConnection, snapshots, validate_schema};
use celetixo_engine::ids::{Digest, Scope, SnapshotId};
use celetixo_engine::persistence::{
    DomainDecodeLimits, EngineeringModelRecord, restore_model_record, restore_repository_analysis,
};
use celetixo_engine::repository::RepositoryAnalysis;
use postgres::{Client, GenericClient, IsolationLevel};

const ANALYSIS_FORMAT: &str = "celetixo/repository-analysis/v1";

pub struct SemanticCatalog {
    client: Client,
}

impl SemanticCatalog {
    pub fn connect(configuration: &UnixConnection) -> Result<Self, StoreError> {
        let mut client = configuration.connect()?;
        validate_schema(&mut client)?;
        Ok(Self { client })
    }

    /// Retain an immutable candidate record; existing model/head state is untouched.
    pub fn put_model(&mut self, record: &EngineeringModelRecord) -> Result<(), StoreError> {
        let mut tx = self.client.transaction().map_err(StoreError::database)?;
        let snapshot = snapshots::load(&mut tx, record.scope(), record.snapshot())?;
        let bytes = record.canonical_bytes();
        restore_model_record(
            &snapshot,
            record.model().generation,
            record.digest(),
            &bytes,
            DomainDecodeLimits::default(),
        )?;
        put_payload(
            &mut tx,
            record.scope(),
            record.digest(),
            EngineeringModelRecord::FORMAT,
            &bytes,
        )?;
        tx.commit().map_err(|_| StoreError::CommitIndeterminate)
    }

    pub fn read_model(
        &mut self,
        scope: &Scope,
        snapshot: SnapshotId,
        generation: u64,
        expected: Digest,
    ) -> Result<EngineeringModelRecord, StoreError> {
        let mut tx = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(StoreError::database)?;
        let snapshot = snapshots::load(&mut tx, scope, snapshot)?;
        let bytes = load_payload(&mut tx, scope, expected, EngineeringModelRecord::FORMAT)?;
        let record = restore_model_record(
            &snapshot,
            generation,
            expected,
            &bytes,
            DomainDecodeLimits::default(),
        )?;
        tx.commit().map_err(StoreError::database)?;
        Ok(record)
    }

    pub fn put_analysis(&mut self, analysis: &RepositoryAnalysis) -> Result<(), StoreError> {
        let mut tx = self.client.transaction().map_err(StoreError::database)?;
        let snapshot = snapshots::load(&mut tx, analysis.scope(), analysis.snapshot())?;
        let bytes = analysis.canonical_bytes();
        restore_repository_analysis(
            &snapshot,
            analysis.index_generation(),
            analysis.digest(),
            &bytes,
            DomainDecodeLimits::default(),
        )?;
        put_payload(
            &mut tx,
            analysis.scope(),
            analysis.digest(),
            ANALYSIS_FORMAT,
            &bytes,
        )?;
        tx.commit().map_err(|_| StoreError::CommitIndeterminate)
    }

    pub fn read_analysis(
        &mut self,
        scope: &Scope,
        snapshot: SnapshotId,
        index: Digest,
        expected: Digest,
    ) -> Result<RepositoryAnalysis, StoreError> {
        let mut tx = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(StoreError::database)?;
        let snapshot = snapshots::load(&mut tx, scope, snapshot)?;
        let bytes = load_payload(&mut tx, scope, expected, ANALYSIS_FORMAT)?;
        let analysis = restore_repository_analysis(
            &snapshot,
            index,
            expected,
            &bytes,
            DomainDecodeLimits::default(),
        )?;
        tx.commit().map_err(StoreError::database)?;
        Ok(analysis)
    }
}

pub(crate) fn put_payload(
    tx: &mut impl GenericClient,
    scope: &Scope,
    id: Digest,
    format: &str,
    bytes: &[u8],
) -> Result<(), StoreError> {
    let a = snapshots::scope_parameters(scope);
    tx.execute("INSERT INTO celetixo.domain_payloads (organization_id,workspace_id,repository_id,payload_digest,payload_format,payload_version,payload_bytes) VALUES ($1::text,$2::text,$3::text,$4::bytea,$5::text,1,$6::bytea) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2],&&id.as_bytes()[..],&format,&bytes]).map_err(StoreError::database)?;
    if load_payload(tx, scope, id, format)? != bytes {
        return Err(StoreError::Integrity);
    }
    Ok(())
}

fn load_payload(
    tx: &mut impl GenericClient,
    scope: &Scope,
    id: Digest,
    format: &str,
) -> Result<Vec<u8>, StoreError> {
    let a = snapshots::scope_parameters(scope);
    let row = tx.query_opt("SELECT payload_format::text,payload_version,payload_bytes FROM celetixo.domain_payloads WHERE organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text AND payload_digest=$4::bytea", &[&a[0],&a[1],&a[2],&&id.as_bytes()[..]]).map_err(StoreError::database)?.ok_or(StoreError::NotFound)?;
    if row.get::<_, String>(0) != format || row.get::<_, i32>(1) != 1 {
        return Err(StoreError::Integrity);
    }
    Ok(row.get(2))
}
