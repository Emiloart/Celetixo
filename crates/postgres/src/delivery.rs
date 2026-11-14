//! Scoped event delivery state, separate from Engine transaction truth. The
//! authenticated database login must have an explicit administrator-owned
//! service binding. Actor IDs in messages never authorize these operations.
use crate::{
    StoreError, UnixConnection,
    authority::events::{FRAME_FORMAT, aggregate_name},
    migrations, validate_schema,
};
use celetixo_engine::ids::{Digest, Scope};
use celetixo_protocol::{
    MAX_WIRE_BYTES, ValidatedLifecycleEvent, celetixo::platform::v1::AggregateKind,
    decode_lifecycle_event,
};
use postgres::{Client, GenericClient, Transaction};
use std::time::Duration;
use thiserror::Error;

const FILTER: &str =
    "organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text";

#[derive(Debug, Error)]
pub enum DeliveryError {
    #[error(transparent)]
    Storage(#[from] StoreError),
    #[error("event workload identity is not authorized for this operation")]
    Unauthorized,
    #[error("invalid bounded event delivery configuration")]
    Configuration,
    #[error("malformed or unsupported event schema")]
    Schema,
    #[error("event scope does not match this authenticated workload")]
    Scope,
    #[error("event does not match its immutable authoritative publication")]
    Authority,
    #[error("event identity has already been used for different bytes")]
    Substitution,
    #[error("publisher claim expired or was fenced by another publisher")]
    LeaseLost,
    #[error("delivery database commit outcome is unknown; reconcile before acknowledging")]
    CommitIndeterminate,
}
type Result<T> = std::result::Result<T, DeliveryError>;
fn database(error: postgres::Error) -> DeliveryError {
    StoreError::database(error).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    Publisher,
    Consumer,
}
impl ServiceKind {
    fn name(self) -> &'static str {
        match self {
            Self::Publisher => "Publisher",
            Self::Consumer => "Consumer",
        }
    }
}

/// Trusted operator configuration. No credentials implement Debug or enter a
/// URL/environment/workflow payload. Every operation uses a fresh login.
pub struct EventDelivery {
    connection: UnixConnection,
    scope: Scope,
    service_id: String,
    kind: ServiceKind,
}

pub struct DeliveryClaim {
    event: ValidatedLifecycleEvent,
    bytes: Vec<u8>,
    generation: u64,
    attempts: u64,
    owner: String,
}
impl DeliveryClaim {
    pub fn event(&self) -> &ValidatedLifecycleEvent {
        &self.event
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn attempts(&self) -> u64 {
        self.attempts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxReceipt {
    pub event: Digest,
    pub replayed: bool,
    pub projection_advanced: bool,
    pub current_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuarantineReason {
    Schema,
    Scope,
    Authority,
    Substitution,
    RedeliveryLimit,
}
impl QuarantineReason {
    fn name(self) -> &'static str {
        match self {
            Self::Schema => "Schema",
            Self::Scope => "Scope",
            Self::Authority => "Authority",
            Self::Substitution => "Substitution",
            Self::RedeliveryLimit => "RedeliveryLimit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryStatus {
    pub pending: u64,
    pub claimed: u64,
    pub quarantined: u64,
    pub unframed: u64,
    pub oldest_pending_age_ms: u64,
}

fn number(text: String) -> Result<u64> {
    text.parse().map_err(|_| DeliveryError::Authority)
}
fn parameters(scope: &Scope) -> [&str; 3] {
    [
        scope.organization.as_str(),
        scope.workspace.as_str(),
        scope.repository.as_str(),
    ]
}
fn identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
}

// This database-wide advisory lock stabilizes derived inbox metadata during an
// Engine write/readback without granting consumers UPDATE on Engine tables.
// Hash collisions only serialize unrelated scopes; they cannot grant access.
pub(crate) fn effects_lock(
    tx: &mut impl GenericClient,
    scope: &Scope,
    exclusive: bool,
) -> Result<()> {
    let mut bytes = Vec::new();
    for field in parameters(scope) {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    let digest = Digest::of("celetixo/event-effects-lock/v1", &bytes);
    let key = i64::from_be_bytes(
        digest.as_bytes()[..8]
            .try_into()
            .map_err(|_| DeliveryError::Configuration)?,
    );
    let sql = if exclusive {
        "SELECT pg_advisory_xact_lock($1)"
    } else {
        "SELECT pg_advisory_xact_lock_shared($1)"
    };
    tx.query_one(sql, &[&key]).map_err(database)?;
    Ok(())
}

impl EventDelivery {
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn connect(
        connection: UnixConnection,
        scope: Scope,
        kind: ServiceKind,
        service_id: &str,
    ) -> Result<Self> {
        if !identity(service_id) {
            return Err(DeliveryError::Configuration);
        }
        let store = Self {
            connection,
            scope,
            service_id: service_id.into(),
            kind,
        };
        let mut client = store.open()?;
        store.authorize(&mut client, kind)?;
        Ok(store)
    }
    fn open(&self) -> Result<Client> {
        let mut client = self.connection.connect()?;
        client
            .query_one("SELECT pg_advisory_lock_shared($1)", &[&migrations::LOCK])
            .map_err(database)?;
        validate_schema(&mut client)?;
        Ok(client)
    }
    fn authorize(&self, tx: &mut impl GenericClient, kind: ServiceKind) -> Result<()> {
        if self.kind != kind {
            return Err(DeliveryError::Unauthorized);
        }
        let a = parameters(&self.scope);
        let allowed: bool = tx.query_one("SELECT celetixo.has_event_service_scope($1::text,$2::text,$3::text,$4,$5::text) AND celetixo.has_repository_scope($1::text,$2::text,$3::text)", &[&a[0],&a[1],&a[2],&kind.name(),&self.service_id]).map_err(database)?.get(0);
        let safe: bool = tx.query_one("SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls) FROM pg_roles WHERE rolname=session_user", &[]).map_err(database)?.get(0);
        // Event workloads may not acquire transaction truth/DDL authority through
        // direct, inherited or PUBLIC privileges. The server enforces grants too.
        let mutable_truth: bool = tx.query_one("SELECT EXISTS (SELECT 1 FROM pg_class AS t JOIN pg_namespace AS n ON n.oid=t.relnamespace WHERE n.nspname='celetixo' AND t.relkind='r' AND t.relname NOT IN ('outbox_delivery','inbox','event_projections','event_quarantine') AND (has_table_privilege(session_user,t.oid,'INSERT,UPDATE,DELETE,TRUNCATE') OR has_any_column_privilege(session_user,t.oid,'INSERT,UPDATE') OR pg_has_role(session_user,t.relowner,'MEMBER'))) OR has_schema_privilege(session_user,'celetixo','CREATE')", &[]).map_err(database)?.get(0);
        if !allowed || !safe || mutable_truth {
            return Err(DeliveryError::Unauthorized);
        }
        Ok(())
    }

    /// Claim under database time; network I/O begins only after this transaction
    /// commits. SKIP LOCKED lets independent publishers share a bounded backlog.
    pub fn claim(
        &self,
        limit: u32,
        lease: Duration,
        max_attempts: u32,
    ) -> Result<Vec<DeliveryClaim>> {
        let lease_ms =
            u64::try_from(lease.as_millis()).map_err(|_| DeliveryError::Configuration)?;
        if !(1..=128).contains(&limit)
            || !(100..=60_000).contains(&lease_ms)
            || !(1..=32).contains(&max_attempts)
        {
            return Err(DeliveryError::Configuration);
        }
        let mut client = self.open()?;
        let mut tx = client.transaction().map_err(database)?;
        self.authorize(&mut tx, ServiceKind::Publisher)?;
        let a = parameters(&self.scope);
        // A process dying on its final attempt cannot strand an expired claim.
        tx.execute(&format!("UPDATE celetixo.outbox_delivery SET quarantined_at=clock_timestamp(),claim_owner=NULL,claim_expires_at=NULL,last_error='publication attempts exhausted' WHERE {FILTER} AND published_at IS NULL AND quarantined_at IS NULL AND publication_attempts>=$4::bigint AND (claim_expires_at IS NULL OR claim_expires_at<=clock_timestamp())"), &[&a[0],&a[1],&a[2],&i64::from(max_attempts)]).map_err(database)?;
        let rows = tx.query(&format!("WITH pending AS (SELECT event_id FROM celetixo.outbox_delivery AS d WHERE {FILTER} AND published_at IS NULL AND quarantined_at IS NULL AND publication_attempts<$4::bigint AND (next_attempt_at IS NULL OR next_attempt_at<=clock_timestamp()) AND (claim_expires_at IS NULL OR claim_expires_at<=clock_timestamp()) AND EXISTS (SELECT 1 FROM celetixo.lifecycle_frames AS f WHERE f.organization_id=d.organization_id AND f.workspace_id=d.workspace_id AND f.repository_id=d.repository_id AND f.event_id=d.event_id) ORDER BY event_id LIMIT $5 FOR UPDATE SKIP LOCKED) UPDATE celetixo.outbox_delivery AS d SET claim_owner=$6::text,claim_expires_at=clock_timestamp()+$7::bigint*interval '1 millisecond',claim_generation=claim_generation+1,publication_attempts=publication_attempts+1 FROM pending WHERE d.organization_id=$1::text AND d.workspace_id=$2::text AND d.repository_id=$3::text AND d.event_id=pending.event_id RETURNING d.event_id::bytea,d.claim_generation::text,d.publication_attempts::text"), &[&a[0],&a[1],&a[2],&i64::from(max_attempts),&i64::from(limit),&self.service_id,&(lease_ms as i64)]).map_err(database)?;
        let mut claims = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Vec<u8> = row.get(0);
            let decoded = (|| {
                let bytes = authoritative_frame(&mut tx, &self.scope, &id)?;
                let event = decode_lifecycle_event(&bytes).map_err(|_| DeliveryError::Schema)?;
                self.check_scope(&event)?;
                if event.event().event_id != id || event.canonical_bytes() != bytes {
                    return Err(DeliveryError::Authority);
                }
                Ok((event, bytes))
            })();
            let (event, bytes) = match decoded {
                Ok(value) => value,
                Err(
                    DeliveryError::Schema
                    | DeliveryError::Scope
                    | DeliveryError::Authority
                    | DeliveryError::Substitution,
                ) => {
                    // Preserve the immutable fact and commit an explicit poison
                    // status. Rolling back this counter would retry forever and
                    // prevent healthy records in this batch from progressing.
                    tx.execute(&format!("UPDATE celetixo.outbox_delivery SET quarantined_at=clock_timestamp(),claim_owner=NULL,claim_expires_at=NULL,last_error='unsupported or inconsistent immutable event frame' WHERE {FILTER} AND event_id=$4::bytea AND claim_owner=$5::text AND claim_generation=$6::text::numeric"), &[&a[0],&a[1],&a[2],&id,&self.service_id,&number(row.get(1))?.to_string()]).map_err(database)?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            claims.push(DeliveryClaim {
                event,
                bytes,
                generation: number(row.get(1))?,
                attempts: number(row.get(2))?,
                owner: self.service_id.clone(),
            });
        }
        self.authorize(&mut tx, ServiceKind::Publisher)?;
        tx.commit()
            .map_err(|_| DeliveryError::CommitIndeterminate)?;
        Ok(claims)
    }

    /// Call only after a successful JetStream PubAck. Lost PubAck/SQL replies
    /// cause reconciliation/redelivery; they never repeat an Engine commit.
    pub fn confirm_published(
        &self,
        claim: &DeliveryClaim,
        stream: &str,
        broker_sequence: u64,
    ) -> Result<()> {
        if !identity(stream) || broker_sequence == 0 || claim.owner != self.service_id {
            return Err(DeliveryError::Configuration);
        }
        self.finish(claim, Some((stream, broker_sequence)), Duration::ZERO)
    }
    pub fn retry_later(&self, claim: &DeliveryClaim, delay: Duration) -> Result<()> {
        if delay.is_zero() || delay > Duration::from_secs(300) {
            return Err(DeliveryError::Configuration);
        }
        self.finish(claim, None, delay)
    }
    fn finish(
        &self,
        claim: &DeliveryClaim,
        ack: Option<(&str, u64)>,
        delay: Duration,
    ) -> Result<()> {
        self.check_scope(&claim.event)?;
        let mut client = self.open()?;
        let mut tx = client.transaction().map_err(database)?;
        self.authorize(&mut tx, ServiceKind::Publisher)?;
        let a = parameters(&self.scope);
        let stream = ack.map(|a| a.0);
        let sequence = ack.map(|a| a.1.to_string());
        let changed=tx.execute(&format!("UPDATE celetixo.outbox_delivery SET published_at=CASE WHEN $7::text IS NOT NULL THEN clock_timestamp() ELSE NULL END,broker_stream=$7::text,broker_sequence=$8::text::numeric,next_attempt_at=CASE WHEN $7::text IS NULL THEN clock_timestamp()+$9::bigint*interval '1 millisecond' ELSE NULL END,claim_owner=NULL,claim_expires_at=NULL,last_error=CASE WHEN $7::text IS NULL THEN 'broker acknowledgement unavailable' ELSE NULL END WHERE {FILTER} AND event_id=$4::bytea AND claim_owner=$5::text AND claim_generation=$6::text::numeric AND claim_expires_at>clock_timestamp() AND published_at IS NULL AND quarantined_at IS NULL"), &[&a[0],&a[1],&a[2],&claim.event.event().event_id,&self.service_id,&claim.generation.to_string(),&stream,&sequence,&(delay.as_millis() as i64)]).map_err(database)?;
        if changed != 1 {
            return Err(DeliveryError::LeaseLost);
        }
        self.authorize(&mut tx, ServiceKind::Publisher)?;
        tx.commit().map_err(|_| DeliveryError::CommitIndeterminate)
    }

    fn check_scope(&self, event: &ValidatedLifecycleEvent) -> Result<()> {
        let selected = event.event().scope.as_ref().ok_or(DeliveryError::Schema)?;
        let a = parameters(&self.scope);
        if selected.organization_id != a[0]
            || selected.workspace_id != a[1]
            || selected.repository_id != a[2]
        {
            return Err(DeliveryError::Scope);
        }
        Ok(())
    }

    /// Exactly-once database projection effect. All effects and the inbox receipt
    /// commit before a broker acknowledgement. Older facts remain replayable but
    /// cannot overwrite a newer aggregate projection. No external effect runs here.
    pub fn apply(&self, bytes: &[u8]) -> Result<InboxReceipt> {
        let event = decode_lifecycle_event(bytes).map_err(|_| DeliveryError::Schema)?;
        self.check_scope(&event)?;
        let frame = Digest::of(FRAME_FORMAT, bytes);
        let e = event.event();
        let mut client = self.open()?;
        let mut tx = client.transaction().map_err(database)?;
        self.authorize(&mut tx, ServiceKind::Consumer)?;
        let a = parameters(&self.scope);
        effects_lock(&mut tx, &self.scope, false)?;
        let authoritative = authoritative_frame(&mut tx, &self.scope, &e.event_id)?;
        if authoritative != bytes {
            return Err(DeliveryError::Substitution);
        }
        let revision = tx
            .query_opt(
                &format!("SELECT revision::text FROM celetixo.repository_heads WHERE {FILTER}"),
                &[&a[0], &a[1], &a[2]],
            )
            .map_err(database)?
            .map(|row| number(row.get(0)))
            .transpose()?
            .unwrap_or(0);
        if e.commit
            .as_ref()
            .is_some_and(|fact| fact.revision > revision)
        {
            return Err(DeliveryError::Authority);
        }
        // The immutable frame was created by the checked Engine path. Current
        // normalized state is also read before the derived metadata effect.
        let transaction = e
            .commit
            .as_ref()
            .map(|f| f.transaction_id.as_str())
            .or_else(|| e.source_attempt.as_ref().map(|f| f.transaction_id.as_str()))
            .or_else(|| e.transaction.as_ref().map(|_| e.aggregate_id.as_str()));
        if let Some(transaction) = transaction {
            let row=tx.query_opt(&format!("SELECT state,current_attempt::text FROM celetixo.transactions WHERE {FILTER} AND transaction_id=$4::text"), &[&a[0],&a[1],&a[2],&transaction]).map_err(database)?.ok_or(DeliveryError::Authority)?;
            let attempt = number(row.get(1))?;
            if e.commit.is_some() && row.get::<_, String>(0) != "Committed"
                || e.transaction
                    .as_ref()
                    .is_some_and(|fact| fact.attempt > attempt)
                || e.source_attempt
                    .as_ref()
                    .is_some_and(|fact| fact.transaction_attempt > attempt)
            {
                return Err(DeliveryError::Authority);
            }
        }
        let inserted=tx.execute("INSERT INTO celetixo.inbox (organization_id,workspace_id,repository_id,consumer_id,event_id,event_payload_digest,result_payload_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5::bytea,$6::bytea,$6::bytea) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2],&self.service_id,&e.event_id,&&frame.as_bytes()[..]]).map_err(database)?;
        let original=tx.query_one(&format!("SELECT event_payload_digest::bytea,result_payload_digest::bytea FROM celetixo.inbox WHERE {FILTER} AND consumer_id=$4::text AND event_id=$5::bytea"), &[&a[0],&a[1],&a[2],&self.service_id,&e.event_id]).map_err(database)?;
        if original.get::<_, Vec<u8>>(0) != frame.as_bytes()
            || original.get::<_, Vec<u8>>(1) != frame.as_bytes()
        {
            return Err(DeliveryError::Substitution);
        }
        let kind = aggregate_name(
            AggregateKind::try_from(e.aggregate_kind).map_err(|_| DeliveryError::Schema)?,
        );
        let advanced=tx.execute("INSERT INTO celetixo.event_projections (organization_id,workspace_id,repository_id,consumer_id,aggregate_kind,aggregate_id,aggregate_sequence,event_id,frame_digest) VALUES ($1::text,$2::text,$3::text,$4::text,$5,$6::text,$7::text::numeric,$8::bytea,$9::bytea) ON CONFLICT (organization_id,workspace_id,repository_id,consumer_id,aggregate_kind,aggregate_id) DO UPDATE SET aggregate_sequence=EXCLUDED.aggregate_sequence,event_id=EXCLUDED.event_id,frame_digest=EXCLUDED.frame_digest WHERE celetixo.event_projections.aggregate_sequence<EXCLUDED.aggregate_sequence", &[&a[0],&a[1],&a[2],&self.service_id,&kind,&e.aggregate_id,&e.aggregate_sequence.to_string(),&e.event_id,&&frame.as_bytes()[..]]).map_err(database)?;
        self.authorize(&mut tx, ServiceKind::Consumer)?;
        tx.commit()
            .map_err(|_| DeliveryError::CommitIndeterminate)?;
        Ok(InboxReceipt {
            event: Digest::from_bytes(
                e.event_id
                    .clone()
                    .try_into()
                    .map_err(|_| DeliveryError::Schema)?,
            ),
            replayed: inserted == 0,
            projection_advanced: advanced == 1,
            current_revision: revision,
        })
    }

    pub fn quarantine(&self, bytes: &[u8], reason: QuarantineReason) -> Result<Digest> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(DeliveryError::Schema);
        }
        let id = Digest::of("celetixo/quarantined-event/v1", bytes);
        let mut client = self.open()?;
        let mut tx = client.transaction().map_err(database)?;
        self.authorize(&mut tx, ServiceKind::Consumer)?;
        let a = parameters(&self.scope);
        effects_lock(&mut tx, &self.scope, false)?;
        tx.execute("INSERT INTO celetixo.event_quarantine (organization_id,workspace_id,repository_id,consumer_id,message_digest,reason,message_bytes) VALUES ($1::text,$2::text,$3::text,$4::text,$5::bytea,$6,$7) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2],&self.service_id,&&id.as_bytes()[..],&reason.name(),&bytes]).map_err(database)?;
        let original=tx.query_one(&format!("SELECT message_bytes FROM celetixo.event_quarantine WHERE {FILTER} AND consumer_id=$4::text AND message_digest=$5::bytea"), &[&a[0],&a[1],&a[2],&self.service_id,&&id.as_bytes()[..]]).map_err(database)?;
        if original.get::<_, Vec<u8>>(0) != bytes {
            return Err(DeliveryError::Substitution);
        }
        self.authorize(&mut tx, ServiceKind::Consumer)?;
        tx.commit()
            .map_err(|_| DeliveryError::CommitIndeterminate)?;
        Ok(id)
    }

    pub fn status(&self) -> Result<DeliveryStatus> {
        let mut client = self.open()?;
        self.authorize(&mut client, self.kind)?;
        let a = parameters(&self.scope);
        let row=client.query_one(&format!("SELECT count(*) FILTER (WHERE published_at IS NULL AND quarantined_at IS NULL)::text,count(*) FILTER (WHERE claim_expires_at>clock_timestamp())::text,count(*) FILTER (WHERE quarantined_at IS NOT NULL)::text,COALESCE(max(EXTRACT(EPOCH FROM clock_timestamp()-o.created_at)*1000) FILTER (WHERE published_at IS NULL),0)::bigint::text FROM celetixo.outbox_delivery AS d JOIN celetixo.outbox AS o USING(organization_id,workspace_id,repository_id,event_id) WHERE {FILTER}"), &[&a[0],&a[1],&a[2]]).map_err(database)?;
        let unframed=client.query_one(&format!("SELECT count(*)::text FROM celetixo.outbox AS o WHERE {FILTER} AND NOT EXISTS (SELECT 1 FROM celetixo.lifecycle_frames AS f WHERE f.organization_id=o.organization_id AND f.workspace_id=o.workspace_id AND f.repository_id=o.repository_id AND f.event_id=o.event_id)"), &[&a[0],&a[1],&a[2]]).map_err(database)?;
        Ok(DeliveryStatus {
            pending: number(row.get(0))?,
            claimed: number(row.get(1))?,
            quarantined: number(row.get(2))?,
            unframed: number(unframed.get(0))?,
            oldest_pending_age_ms: number(row.get(3))?,
        })
    }

    /// Reconciliation cursor is the durable inbox itself. This bounded scan
    /// recovers facts lost to stream retention, exhausted redelivery or a new
    /// consumer. It does not reset Engine state or trust broker arrival order.
    pub fn missing_frames(&self, limit: u32) -> Result<Vec<Vec<u8>>> {
        if !(1..=128).contains(&limit) {
            return Err(DeliveryError::Configuration);
        }
        let mut client = self.open()?;
        let mut tx = client.transaction().map_err(database)?;
        self.authorize(&mut tx, ServiceKind::Consumer)?;
        effects_lock(&mut tx, &self.scope, false)?;
        let a = parameters(&self.scope);
        let rows=tx.query("SELECT f.event_id::bytea FROM celetixo.lifecycle_frames AS f WHERE f.organization_id=$1::text AND f.workspace_id=$2::text AND f.repository_id=$3::text AND NOT EXISTS (SELECT 1 FROM celetixo.inbox AS i WHERE i.organization_id=f.organization_id AND i.workspace_id=f.workspace_id AND i.repository_id=f.repository_id AND i.consumer_id=$4::text AND i.event_id=f.event_id) ORDER BY f.publication_generation,f.event_id LIMIT $5", &[&a[0],&a[1],&a[2],&self.service_id,&i64::from(limit)]).map_err(database)?;
        let mut frames = Vec::with_capacity(rows.len());
        for row in rows {
            frames.push(authoritative_frame(
                &mut tx,
                &self.scope,
                &row.get::<_, Vec<u8>>(0),
            )?);
        }
        tx.commit()
            .map_err(|_| DeliveryError::CommitIndeterminate)?;
        Ok(frames)
    }
}

fn authoritative_frame(tx: &mut Transaction<'_>, scope: &Scope, id: &[u8]) -> Result<Vec<u8>> {
    let a = parameters(scope);
    let row=tx.query_opt("SELECT p.payload_format::text,p.payload_version,octet_length(p.payload_bytes),f.frame_digest::bytea FROM celetixo.lifecycle_frames AS f JOIN celetixo.domain_payloads AS p ON p.organization_id=f.organization_id AND p.workspace_id=f.workspace_id AND p.repository_id=f.repository_id AND p.payload_digest=f.frame_digest WHERE f.organization_id=$1::text AND f.workspace_id=$2::text AND f.repository_id=$3::text AND f.event_id=$4::bytea", &[&a[0],&a[1],&a[2],&id]).map_err(database)?.ok_or(DeliveryError::Authority)?;
    let length: i32 = row.get(2);
    if row.get::<_, String>(0) != FRAME_FORMAT
        || row.get::<_, i32>(1) != 1
        || length < 0
        || length as usize > MAX_WIRE_BYTES
    {
        return Err(DeliveryError::Authority);
    }
    let digest: Vec<u8> = row.get(3);
    let row=tx.query_one(&format!("SELECT payload_bytes FROM celetixo.domain_payloads WHERE {FILTER} AND payload_digest=$4::bytea"), &[&a[0],&a[1],&a[2],&digest]).map_err(database)?;
    let bytes: Vec<u8> = row.get(0);
    if Digest::of(FRAME_FORMAT, &bytes).as_bytes() != digest.as_slice() {
        return Err(DeliveryError::Authority);
    }
    Ok(bytes)
}
