//! First SQL authority profile: scoped grants and native repository registration.
//! Lifecycle rows reject rather than being discarded during partial recovery.
use crate::{StoreError, UnixConnection, migrations, semantic, snapshots, validate_schema};
use celetixo_engine::engine::Permission;
use celetixo_engine::engine::authority::{
    AuthorityAccess, AuthorityActor, AuthorityBackend, AuthorityFailure, AuthorityMode,
    AuthorityState, NativeRecordLimits, NativeRecords, RepositoryRecord, SessionGrantRecord,
};
use celetixo_engine::ids::{Digest, PrincipalId, Scope, SnapshotId};
use celetixo_engine::persistence::{
    DomainDecodeLimits, EngineeringModelRecord, RestoreError, restore_model_record,
};
use postgres::{Client, GenericClient, IsolationLevel, Row, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const FILTER: &str =
    "organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text";
const PROFILE: &str = "native-reference-v1";

/// Trusted fault-injection/observation points. Hooks cannot approve a transition,
/// replace state, bypass validation or turn a missing commit reply into success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityStage {
    SchemaLocked,
    StateLoaded,
    BeforeCommit,
    AfterCommit,
}
type Checkpoint = dyn Fn(AuthorityStage) -> Result<(), AuthorityFailure> + Send + Sync;

/// Storage diagnostics for trusted administration, never an authenticated Session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationView {
    pub state_generation: u64,
    pub records: NativeRecords,
}

struct Login {
    configuration: UnixConnection,
    name: String,
    oid: u32,
    peer_oid: Option<u32>,
    administrative: bool,
}

/// Trusted embedding adapter with independently authenticated, restricted runtime
/// and administration logins. Fresh connections hold no authoritative cache.
/// This initial profile publishes grants and registration only. Transactions,
/// reservations, evidence, policies and captured-source state fail closed.
pub struct PostgresAuthority {
    runtime: Login,
    administration: Login,
    limits: NativeRecordLimits,
    checkpoint: Option<Arc<Checkpoint>>,
}

impl PostgresAuthority {
    pub fn connect(
        runtime: UnixConnection,
        administration: UnixConnection,
    ) -> Result<Self, AuthorityFailure> {
        if runtime.directory != administration.directory
            || runtime.database != administration.database
        {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
        let mut runtime = Login::open(runtime, false)?;
        let mut administration = Login::open(administration, true)?;
        if runtime.oid == administration.oid || runtime.name == administration.name {
            return Err(AuthorityFailure::ScopeDenied);
        }
        for (login, other) in [(&runtime, &administration), (&administration, &runtime)] {
            let mut client = login.connect()?;
            let row = client
                .query_one(
                    "SELECT pg_catalog.pg_has_role(session_user,$1::oid,'MEMBER')",
                    &[&other.oid],
                )
                .map_err(database)?;
            if row.get::<_, bool>(0) {
                return Err(AuthorityFailure::ScopeDenied);
            }
        }
        runtime.peer_oid = Some(administration.oid);
        administration.peer_oid = Some(runtime.oid);
        Ok(Self {
            runtime,
            administration,
            limits: NativeRecordLimits::default(),
            checkpoint: None,
        })
    }

    /// Smaller budgets may be selected by the trusted embedding. Raising these
    /// beyond the reviewed default is unsupported, not a scalability feature.
    pub fn with_limits(mut self, limits: NativeRecordLimits) -> Result<Self, AuthorityFailure> {
        let maximum = NativeRecordLimits::default();
        if limits.entries == 0
            || limits.value_bytes == 0
            || limits.total_value_bytes == 0
            || limits.entries > maximum.entries
            || limits.value_bytes > maximum.value_bytes
            || limits.total_value_bytes > maximum.total_value_bytes
        {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
        self.limits = limits;
        Ok(self)
    }

    pub fn with_checkpoint(mut self, checkpoint: Arc<Checkpoint>) -> Self {
        self.checkpoint = Some(checkpoint);
        self
    }

    fn checkpoint(&self, stage: AuthorityStage) -> Result<(), AuthorityFailure> {
        match &self.checkpoint {
            Some(checkpoint) => checkpoint(stage),
            None => Ok(()),
        }
    }

    /// An administrative coherent observation, including exact grants and the
    /// publication generation. It does not infer an uncertain operation's result
    /// or retry it. Bootstrap requests do not yet have durable idempotency keys.
    pub fn inspect_registration(
        &self,
        scope: &Scope,
    ) -> Result<RegistrationView, AuthorityFailure> {
        let mut client = self.administration.connect()?;
        schema_lock(&mut client)?;
        let mut tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()
            .map_err(database)?;
        validate_schema(&mut tx).map_err(storage)?;
        self.administration.check(&mut tx, Some(scope))?;
        let generation = lock_repository(&mut tx, scope, false)?;
        let records = load_registration(&mut tx, scope, self.limits)?;
        checked(records.clone(), self.limits)?;
        tx.commit().map_err(database)?;
        Ok(RegistrationView {
            state_generation: generation,
            records,
        })
    }
}

impl AuthorityBackend for PostgresAuthority {
    fn access(
        &self,
        context: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure> {
        let administrative = matches!(
            context.mode(),
            AuthorityMode::AdministrativeRead | AuthorityMode::AdministrativeUpdate
        );
        let update = matches!(
            context.mode(),
            AuthorityMode::Update | AuthorityMode::AdministrativeUpdate
        );
        if administrative != matches!(context.actor(), AuthorityActor::EmbeddingAdministrator) {
            return Err(AuthorityFailure::Protocol);
        }
        let login = if administrative {
            &self.administration
        } else {
            &self.runtime
        };
        let mut client = login.connect()?;
        // Acquire before BEGIN: a repeatable-read snapshot created while waiting
        // on an exclusive migrator could otherwise see its earlier ledger.
        // This dedicated connection is dropped on every return/unwind, releasing
        // the session lock even when COMMIT's acknowledgement is lost.
        schema_lock(&mut client)?;
        self.checkpoint(AuthorityStage::SchemaLocked)?;
        let mut tx = client
            .build_transaction()
            .isolation_level(if update {
                IsolationLevel::ReadCommitted
            } else {
                IsolationLevel::RepeatableRead
            })
            .start()
            .map_err(database)?;
        validate_schema(&mut tx).map_err(storage)?;
        login.check(&mut tx, Some(context.scope()))?;
        if administrative && update {
            bootstrap(&mut tx, context.scope())?;
        }
        // FOR SHARE on reads intentionally uses a read/write SQL transaction;
        // it never writes. The immutable callback result is checked below.
        let generation = lock_repository(&mut tx, context.scope(), update)?;
        let before = load_registration(&mut tx, context.scope(), self.limits)?;
        let mut state = checked(before.clone(), self.limits)?;
        let now = database_clock(&mut tx)?;
        if update {
            state.advance_authoritative_clock(now)
        } else {
            state.observe_authoritative_clock(now)
        }
        .map_err(|_| AuthorityFailure::ClockRegression)?;
        self.checkpoint(AuthorityStage::StateLoaded)?;
        let observed = state.native_records(self.limits).map_err(restore)?;
        evaluate(&mut state)?;
        let after = state.native_records(self.limits).map_err(restore)?;
        registration_profile(&after)?;
        checked(after.clone(), self.limits)?;
        if update {
            publish_registration(&mut tx, context, login, generation, &before, &after)?;
            // SQL constraints are insufficient: verify the exact checked facts
            // read back through the same coherent transaction before COMMIT.
            if load_registration(&mut tx, context.scope(), self.limits)? != after {
                return Err(AuthorityFailure::Integrity);
            }
            self.checkpoint(AuthorityStage::BeforeCommit)?;
            tx.commit()
                .map_err(|_| AuthorityFailure::CommitIndeterminate)?;
            self.checkpoint(AuthorityStage::AfterCommit)
                .map_err(|_| AuthorityFailure::CommitIndeterminate)
        } else {
            if after != observed {
                return Err(AuthorityFailure::Protocol);
            }
            tx.commit().map_err(database)
        }
    }
}

impl Login {
    fn open(configuration: UnixConnection, administrative: bool) -> Result<Self, AuthorityFailure> {
        let mut client = configuration.connect().map_err(storage)?;
        schema_lock(&mut client)?;
        validate_schema(&mut client).map_err(storage)?;
        let (name, oid) = restricted_identity(&mut client)?;
        Ok(Self {
            configuration,
            name,
            oid,
            peer_oid: None,
            administrative,
        })
    }

    fn connect(&self) -> Result<Client, AuthorityFailure> {
        self.configuration.connect().map_err(storage)
    }

    fn check(
        &self,
        client: &mut impl GenericClient,
        scope: Option<&Scope>,
    ) -> Result<(), AuthorityFailure> {
        let (name, oid) = restricted_identity(client)?;
        if name != self.name || oid != self.oid {
            return Err(AuthorityFailure::ScopeDenied);
        }
        if let Some(peer) = self.peer_oid {
            let row = client
                .query_one(
                    "SELECT pg_catalog.pg_has_role(session_user,$1::oid,'MEMBER')",
                    &[&peer],
                )
                .map_err(database)?;
            if row.get::<_, bool>(0) {
                return Err(AuthorityFailure::ScopeDenied);
            }
        }
        // A deployment privilege change must not turn a runtime service into an
        // authorization administrator, or expose either login to scope/ledger
        // mutation. Column-level privileges count too.
        for table in if self.administrative {
            &["db_role_scopes", "schema_migrations"][..]
        } else {
            &[
                "db_role_scopes",
                "schema_migrations",
                "principals",
                "grants",
                "grant_permissions",
            ][..]
        } {
            let relation = format!("celetixo.{table}");
            let row=client.query_one("SELECT pg_catalog.has_table_privilege(session_user,$1,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES') OR pg_catalog.has_any_column_privilege(session_user,$1,'INSERT,UPDATE,REFERENCES')", &[&relation]).map_err(database)?;
            if row.get::<_, bool>(0) {
                return Err(AuthorityFailure::ScopeDenied);
            }
            if *table == "db_role_scopes" {
                let row=client.query_one("SELECT pg_catalog.has_table_privilege(session_user,$1,'SELECT') OR pg_catalog.has_any_column_privilege(session_user,$1,'SELECT')", &[&relation]).map_err(database)?;
                if row.get::<_, bool>(0) {
                    return Err(AuthorityFailure::ScopeDenied);
                }
            }
        }
        if let Some(scope) = scope {
            let a = snapshots::scope_parameters(scope);
            let row = client
                .query_one(
                    "SELECT celetixo.has_repository_scope($1::text,$2::text,$3::text)",
                    &[&a[0], &a[1], &a[2]],
                )
                .map_err(database)?;
            if !row.get::<_, bool>(0) {
                return Err(AuthorityFailure::ScopeDenied);
            }
        }
        Ok(())
    }
}

fn restricted_identity(client: &mut impl GenericClient) -> Result<(String, u32), AuthorityFailure> {
    // Check all reachable roles, including memberships without INHERIT; those
    // must not allow SET ROLE to an owner or a privileged service identity.
    let row = client.query_one(
        "SELECT session_user::text,r.oid,current_user=session_user,
         NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles p
           WHERE pg_catalog.pg_has_role(session_user,p.oid,'MEMBER')
             AND (p.rolsuper OR p.rolbypassrls OR p.rolcreaterole OR p.rolcreatedb OR p.rolreplication
               OR p.rolname IN ('pg_read_all_data','pg_write_all_data','pg_read_server_files','pg_write_server_files','pg_execute_server_program','pg_signal_backend','pg_database_owner')
               OR p.oid IN (SELECT datdba FROM pg_catalog.pg_database WHERE datname=current_database())
               OR p.oid IN (SELECT nspowner FROM pg_catalog.pg_namespace WHERE nspname='celetixo')
               OR p.oid IN (SELECT c.relowner FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='celetixo')
               OR p.oid IN (SELECT f.proowner FROM pg_catalog.pg_proc f JOIN pg_catalog.pg_namespace n ON n.oid=f.pronamespace WHERE n.nspname='celetixo')))
         FROM pg_catalog.pg_roles r WHERE r.rolname=session_user", &[],
    ).map_err(database)?;
    if !row.get::<_, bool>(2) || !row.get::<_, bool>(3) {
        return Err(AuthorityFailure::ScopeDenied);
    }
    Ok((row.get(0), row.get(1)))
}

fn schema_lock(client: &mut Client) -> Result<(), AuthorityFailure> {
    client
        .query_one(
            "SELECT pg_catalog.pg_advisory_lock_shared($1)",
            &[&migrations::LOCK],
        )
        .map_err(database)?;
    Ok(())
}

fn bootstrap(tx: &mut Transaction<'_>, scope: &Scope) -> Result<(), AuthorityFailure> {
    let a = snapshots::scope_parameters(scope);
    tx.execute("INSERT INTO celetixo.organizations (organization_id) VALUES ($1::text) ON CONFLICT DO NOTHING", &[&a[0]]).map_err(database)?;
    tx.execute("INSERT INTO celetixo.workspaces (organization_id,workspace_id) VALUES ($1::text,$2::text) ON CONFLICT DO NOTHING", &[&a[0],&a[1]]).map_err(database)?;
    tx.execute("INSERT INTO celetixo.repositories (organization_id,workspace_id,repository_id) VALUES ($1::text,$2::text,$3::text) ON CONFLICT DO NOTHING", &[&a[0],&a[1],&a[2]]).map_err(database)?;
    Ok(())
}

fn lock_repository(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    update: bool,
) -> Result<u64, AuthorityFailure> {
    let a = snapshots::scope_parameters(scope);
    let lock = if update { "UPDATE" } else { "SHARE" };
    let row = tx
        .query_opt(
            &format!(
                "SELECT state_generation::text FROM celetixo.repositories WHERE {FILTER} FOR {lock}"
            ),
            &[&a[0], &a[1], &a[2]],
        )
        .map_err(database)?
        .ok_or(AuthorityFailure::ScopeDenied)?;
    number(&row, 0)
}

fn database_clock(tx: &mut Transaction<'_>) -> Result<u64, AuthorityFailure> {
    let row = tx
        .query_one(
            "SELECT floor(extract(epoch FROM clock_timestamp())*1000)::numeric(20,0)::text",
            &[],
        )
        .map_err(database)?;
    number(&row, 0)
}

fn checked(
    records: NativeRecords,
    limits: NativeRecordLimits,
) -> Result<AuthorityState, AuthorityFailure> {
    registration_profile(&records)?;
    AuthorityState::from_native_records(records, limits).map_err(restore)
}

fn registration_profile(records: &NativeRecords) -> Result<(), AuthorityFailure> {
    if !records.transactions.is_empty()
        || !records.evidence.is_empty()
        || !records.operations.is_empty()
        || records.reservation_fence != 0
        || records.repository.as_ref().is_some_and(|r| {
            !r.history.is_empty() || r.initial != r.current || r.registered.len() != 1
        })
    {
        return Err(AuthorityFailure::UnsupportedProfile);
    }
    Ok(())
}

fn load_registration(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    limits: NativeRecordLimits,
) -> Result<NativeRecords, AuthorityFailure> {
    let a = snapshots::scope_parameters(scope);
    // Do not infer an empty supported state by filtering out unsupported facts.
    // Each EXISTS uses the scoped key prefix and returns a single boolean.
    for table in [
        "policies",
        "policy_heads",
        "artifact_store_registrations",
        "transactions",
        "attempts",
        "access_provenance",
        "transaction_outcomes",
        "committed_changes",
        "reservations",
        "reservation_claims",
        "reservation_events",
        "operations",
        "source_attempts",
        "source_attempt_events",
        "source_attempt_outcomes",
        "source_rejections",
        "artifact_objects",
        "evidence",
        "evidence_receipts",
        "attempt_checks",
        "reference_evidence_admissions",
        "reference_evidence_signers",
        "outbox",
        "outbox_delivery",
        "inbox",
    ] {
        let row = tx
            .query_one(
                &format!("SELECT EXISTS (SELECT 1 FROM celetixo.{table} WHERE {FILTER})"),
                &[&a[0], &a[1], &a[2]],
            )
            .map_err(database)?;
        if row.get::<_, bool>(0) {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
    }
    let mut records = NativeRecords::empty(scope.clone());
    let counters = tx.query_opt(&format!("SELECT last_clock_ms::text,next_session_grant::text,next_executor_grant::text,next_cleanup_grant::text,next_artifact_store::text FROM celetixo.engine_counters WHERE {FILTER}"), &[&a[0],&a[1],&a[2]]).map_err(database)?;
    if let Some(row) = &counters {
        records.clock_ms = number(row, 0)?;
        records.session_counter = number(row, 1)?;
        if (2..5).any(|i| row.get::<_, String>(i) != "0") {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
    }
    if let Some(row) = tx
        .query_opt(
            &format!("SELECT generation::text FROM celetixo.reservation_fences WHERE {FILTER}"),
            &[&a[0], &a[1], &a[2]],
        )
        .map_err(database)?
        && number(&row, 0)? != 0
    {
        return Err(AuthorityFailure::UnsupportedProfile);
    }
    let bound = i64::try_from(limits.entries).map_err(|_| AuthorityFailure::Integrity)?;
    let rows = tx.query(&format!("SELECT grant_id::text,principal_id::text,active,grant_kind,expires_at_ms::text,payload_digest::bytea FROM celetixo.grants WHERE {FILTER} ORDER BY grant_kind,grant_id LIMIT $4::bigint+1"), &[&a[0],&a[1],&a[2],&bound]).map_err(database)?;
    if rows.len() > limits.entries {
        return Err(AuthorityFailure::Integrity);
    }
    for row in rows {
        if row.get::<_, String>(3) != "session"
            || row.get::<_, Option<String>>(4).is_some()
            || row.get::<_, Option<Vec<u8>>>(5).is_some()
        {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
        records.grants.insert(
            number(&row, 0)?,
            SessionGrantRecord {
                principal: PrincipalId::new(row.get::<_, String>(1))
                    .map_err(|_| AuthorityFailure::Integrity)?,
                permissions: BTreeSet::new(),
                active: row.get(2),
            },
        );
    }
    let rows = tx.query(&format!("SELECT grant_id::text,permission,grant_kind FROM celetixo.grant_permissions WHERE {FILTER} ORDER BY grant_kind,grant_id,permission LIMIT $4::bigint+1"), &[&a[0],&a[1],&a[2],&bound]).map_err(database)?;
    if rows
        .len()
        .checked_add(records.grants.len())
        .is_none_or(|n| n > limits.entries)
    {
        return Err(AuthorityFailure::Integrity);
    }
    for row in rows {
        if row.get::<_, String>(2) != "session" {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
        records
            .grants
            .get_mut(&number(&row, 0)?)
            .ok_or(AuthorityFailure::Integrity)?
            .permissions
            .insert(permission(&row.get::<_, String>(1))?);
    }
    let head = tx.query_opt(&format!("SELECT snapshot_digest::bytea,revision::text FROM celetixo.repository_heads WHERE {FILTER}"), &[&a[0],&a[1],&a[2]]).map_err(database)?;
    let model = tx.query_opt(&format!("SELECT initial_snapshot_digest::bytea,snapshot_digest::bytea,model_generation::text,model_payload_digest::bytea,model_kind,runtime_profile,analysis_payload_digest::bytea,index_generation::bytea FROM celetixo.engine_model_heads WHERE {FILTER}"), &[&a[0],&a[1],&a[2]]).map_err(database)?;
    match (head, model) {
        (None, None) => {}
        (Some(head), Some(model)) => {
            let id = SnapshotId(digest(&head, 0)?);
            if number(&head, 1)? != 0
                || digest(&model, 0)? != id.0
                || digest(&model, 1)? != id.0
                || model.get::<_, String>(4) != "Reference"
                || model.get::<_, Option<String>>(5).as_deref() != Some(PROFILE)
                || model.get::<_, Option<Vec<u8>>>(6).is_some()
                || model.get::<_, Option<Vec<u8>>>(7).is_some()
            {
                return Err(AuthorityFailure::UnsupportedProfile);
            }
            // Check lengths before fetching either canonical value. The bound
            // covers their combined allocation as well as each individual field.
            let snapshot_bytes = payload_size(tx, scope, id.0, limits)?;
            let model_id = digest(&model, 3)?;
            let model_bytes = payload_size(tx, scope, model_id, limits)?;
            if snapshot_bytes
                .checked_add(model_bytes)
                .is_none_or(|n| n > limits.total_value_bytes)
            {
                return Err(AuthorityFailure::Integrity);
            }
            let snapshot = snapshots::load(tx, scope, id).map_err(storage)?;
            let row=tx.query_one(&format!("SELECT payload_format::text,payload_version,payload_bytes FROM celetixo.domain_payloads WHERE {FILTER} AND payload_digest=$4::bytea"), &[&a[0],&a[1],&a[2],&&model_id.as_bytes()[..]]).map_err(database)?;
            if row.get::<_, String>(0) != EngineeringModelRecord::FORMAT
                || row.get::<_, i32>(1) != 1
            {
                return Err(AuthorityFailure::Integrity);
            }
            let model = restore_model_record(
                &snapshot,
                number(&model, 2)?,
                model_id,
                &row.get::<_, Vec<u8>>(2),
                DomainDecodeLimits {
                    encoded_bytes: limits.value_bytes,
                    field_bytes: limits.value_bytes,
                    collection_entries: limits.entries,
                },
            )
            .map_err(restore)?;
            records.repository = Some(RepositoryRecord {
                initial: id,
                current: id,
                registered: BTreeMap::from([(id, snapshot)]),
                model: model.model().clone(),
                history: vec![],
            });
        }
        _ => return Err(AuthorityFailure::UnsupportedProfile),
    }
    if counters.is_none() && (!records.grants.is_empty() || records.repository.is_some()) {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(records)
}

fn payload_size(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    id: Digest,
    limits: NativeRecordLimits,
) -> Result<usize, AuthorityFailure> {
    let a = snapshots::scope_parameters(scope);
    let row=tx.query_opt(&format!("SELECT octet_length(payload_bytes) FROM celetixo.domain_payloads WHERE {FILTER} AND payload_digest=$4::bytea"), &[&a[0],&a[1],&a[2],&&id.as_bytes()[..]]).map_err(database)?.ok_or(AuthorityFailure::Integrity)?;
    let n = usize::try_from(row.get::<_, i32>(0)).map_err(|_| AuthorityFailure::Integrity)?;
    if n > limits.value_bytes || n > limits.total_value_bytes {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(n)
}

fn publish_registration(
    tx: &mut Transaction<'_>,
    context: &AuthorityAccess,
    login: &Login,
    generation: u64,
    before: &NativeRecords,
    after: &NativeRecords,
) -> Result<(), AuthorityFailure> {
    let next = generation
        .checked_add(1)
        .ok_or(AuthorityFailure::CounterExhausted)?;
    let scope = context.scope();
    let a = snapshots::scope_parameters(scope);
    if before.grants != after.grants
        && !matches!(context.actor(), AuthorityActor::EmbeddingAdministrator)
    {
        return Err(AuthorityFailure::Protocol);
    }
    for (id, grant) in &after.grants {
        if before.grants.get(id) == Some(grant) {
            continue;
        }
        if let Some(previous) = before.grants.get(id) {
            // Revocation is the only update; never rebind or reactivate a grant.
            if previous.principal != grant.principal
                || previous.permissions != grant.permissions
                || !previous.active
                || grant.active
            {
                return Err(AuthorityFailure::Protocol);
            }
            let count=tx.execute(&format!("UPDATE celetixo.grants SET active=$5,generation=$6::text::numeric WHERE {FILTER} AND grant_kind='session' AND grant_id=$4::text::numeric"), &[&a[0],&a[1],&a[2],&id.to_string(),&grant.active,&next.to_string()]).map_err(database)?;
            if count != 1 {
                return Err(AuthorityFailure::Integrity);
            }
        } else {
            tx.execute("INSERT INTO celetixo.principals (organization_id,principal_id) VALUES ($1::text,$2::text) ON CONFLICT DO NOTHING", &[&a[0],&grant.principal.as_str()]).map_err(database)?;
            tx.execute("INSERT INTO celetixo.grants (organization_id,workspace_id,repository_id,grant_kind,grant_id,principal_id,generation,active) VALUES ($1::text,$2::text,$3::text,'session',$4::text::numeric,$5::text,$6::text::numeric,$7)", &[&a[0],&a[1],&a[2],&id.to_string(),&grant.principal.as_str(),&next.to_string(),&grant.active]).map_err(database)?;
            for permission in &grant.permissions {
                tx.execute("INSERT INTO celetixo.grant_permissions (organization_id,workspace_id,repository_id,grant_kind,grant_id,permission) VALUES ($1::text,$2::text,$3::text,'session',$4::text::numeric,$5)", &[&a[0],&a[1],&a[2],&id.to_string(),&permission_name(*permission)]).map_err(database)?;
            }
        }
    }
    if before
        .grants
        .keys()
        .any(|id| !after.grants.contains_key(id))
    {
        return Err(AuthorityFailure::Protocol);
    }
    if let Some(repo) = &after.repository {
        let snapshot = &repo.registered[&repo.current];
        snapshots::persist(tx, snapshot).map_err(storage)?;
        let model = EngineeringModelRecord::new(snapshot, repo.model.clone())
            .map_err(|_| AuthorityFailure::Integrity)?;
        semantic::put_payload(
            tx,
            scope,
            model.digest(),
            EngineeringModelRecord::FORMAT,
            &model.canonical_bytes(),
        )
        .map_err(storage)?;
        if let Some(old) = &before.repository {
            if old.initial != repo.initial
                || old.current != repo.current
                || old.registered != repo.registered
                || old.history != repo.history
            {
                return Err(AuthorityFailure::UnsupportedProfile);
            }
            if old.model != repo.model {
                let previous = EngineeringModelRecord::new(snapshot, old.model.clone())
                    .map_err(|_| AuthorityFailure::Integrity)?;
                let count=tx.execute(&format!("UPDATE celetixo.engine_model_heads SET model_generation=$5::text::numeric,model_payload_digest=$6::bytea WHERE {FILTER} AND snapshot_digest=$4::bytea AND model_generation=$7::text::numeric AND model_payload_digest=$8::bytea AND runtime_profile=$9"), &[&a[0],&a[1],&a[2],&&repo.current.0.as_bytes()[..],&repo.model.generation.to_string(),&&model.digest().as_bytes()[..],&old.model.generation.to_string(),&&previous.digest().as_bytes()[..],&PROFILE]).map_err(database)?;
                if count != 1 {
                    return Err(AuthorityFailure::Contention);
                }
            }
        } else {
            tx.execute("INSERT INTO celetixo.repository_heads (organization_id,workspace_id,repository_id,revision,snapshot_digest,head_generation) VALUES ($1::text,$2::text,$3::text,0,$4::bytea,$5::text::numeric)", &[&a[0],&a[1],&a[2],&&repo.current.0.as_bytes()[..],&next.to_string()]).map_err(database)?;
            tx.execute("INSERT INTO celetixo.engine_model_heads (organization_id,workspace_id,repository_id,initial_snapshot_digest,snapshot_digest,model_generation,model_payload_digest,model_kind,runtime_profile) VALUES ($1::text,$2::text,$3::text,$4::bytea,$4::bytea,$5::text::numeric,$6::bytea,'Reference',$7)", &[&a[0],&a[1],&a[2],&&repo.current.0.as_bytes()[..],&repo.model.generation.to_string(),&&model.digest().as_bytes()[..],&PROFILE]).map_err(database)?;
        }
    } else if before.repository.is_some() {
        return Err(AuthorityFailure::Protocol);
    }
    tx.execute("INSERT INTO celetixo.engine_counters (organization_id,workspace_id,repository_id,last_clock_ms,next_session_grant) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text::numeric) ON CONFLICT (organization_id,workspace_id,repository_id) DO UPDATE SET last_clock_ms=EXCLUDED.last_clock_ms,next_session_grant=EXCLUDED.next_session_grant", &[&a[0],&a[1],&a[2],&after.clock_ms.to_string(),&after.session_counter.to_string()]).map_err(database)?;
    let changed=tx.execute(&format!("UPDATE celetixo.repositories SET state_generation=$5::text::numeric WHERE {FILTER} AND state_generation=$4::text::numeric"), &[&a[0],&a[1],&a[2],&generation.to_string(),&next.to_string()]).map_err(database)?;
    if changed != 1 {
        return Err(AuthorityFailure::Contention);
    }
    let (kind, principal, grant, permission) = match context.actor() {
        AuthorityActor::EmbeddingAdministrator => ("EmbeddingAdministrator", None, None, None),
        AuthorityActor::Session {
            principal,
            grant,
            permission,
        } => (
            "Session",
            Some(principal.as_str()),
            Some(grant.to_string()),
            Some(permission_name(*permission)),
        ),
    };
    let snapshot = after
        .repository
        .as_ref()
        .map(|r| r.current.0.as_bytes().to_vec());
    tx.execute("INSERT INTO celetixo.authority_publications (organization_id,workspace_id,repository_id,state_generation,observed_clock_ms,authenticated_database_role,authenticated_database_role_oid,actor_kind,requested_principal_id,requested_grant_id,requested_permission,snapshot_digest) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text::numeric,$6::text::name,$7,$8,$9::text,$10::text::numeric,$11,$12::bytea)", &[&a[0],&a[1],&a[2],&next.to_string(),&after.clock_ms.to_string(),&login.name,&login.oid,&kind,&principal,&grant,&permission,&snapshot]).map_err(database)?;
    Ok(())
}

fn number(row: &Row, column: usize) -> Result<u64, AuthorityFailure> {
    let text = row.get::<_, String>(column);
    let value = text
        .parse::<u64>()
        .map_err(|_| AuthorityFailure::Integrity)?;
    if value.to_string() != text {
        return Err(AuthorityFailure::Integrity);
    }
    Ok(value)
}
fn digest(row: &Row, column: usize) -> Result<Digest, AuthorityFailure> {
    Ok(Digest::from_bytes(
        row.get::<_, Vec<u8>>(column)
            .try_into()
            .map_err(|_| AuthorityFailure::Integrity)?,
    ))
}
fn permission(text: &str) -> Result<Permission, AuthorityFailure> {
    Permission::ALL
        .into_iter()
        .find(|p| permission_name(*p) == text)
        .ok_or(AuthorityFailure::Integrity)
}
fn permission_name(value: Permission) -> &'static str {
    match value {
        Permission::Register => "Register",
        Permission::ManageModel => "ManageModel",
        Permission::Read => "Read",
        Permission::Propose => "Propose",
        Permission::Analyze => "Analyze",
        Permission::Reserve => "Reserve",
        Permission::Execute => "Execute",
        Permission::Verify => "Verify",
        Permission::Commit => "Commit",
        Permission::Cancel => "Cancel",
    }
}
fn restore(error: RestoreError) -> AuthorityFailure {
    match error {
        RestoreError::UnsupportedDomainValue => AuthorityFailure::UnsupportedProfile,
        _ => AuthorityFailure::Integrity,
    }
}
fn storage(error: StoreError) -> AuthorityFailure {
    match error {
        StoreError::SchemaMismatch => AuthorityFailure::SchemaMismatch,
        StoreError::Integrity | StoreError::Snapshot(_) => AuthorityFailure::Integrity,
        StoreError::NotFound => AuthorityFailure::ScopeDenied,
        StoreError::CommitIndeterminate => AuthorityFailure::CommitIndeterminate,
        StoreError::Database { code } => classification(&code),
        _ => AuthorityFailure::Unavailable,
    }
}
fn database(error: postgres::Error) -> AuthorityFailure {
    classification(error.code().map_or("connection", |code| code.code()))
}
fn classification(code: &str) -> AuthorityFailure {
    match code {
        "40001" | "40P01" | "55P03" | "57014" => AuthorityFailure::Contention,
        "42501" => AuthorityFailure::ScopeDenied,
        "42P01" | "42703" | "42883" => AuthorityFailure::SchemaMismatch,
        _ if code.starts_with("23") => AuthorityFailure::Integrity,
        _ => AuthorityFailure::Unavailable,
    }
}
