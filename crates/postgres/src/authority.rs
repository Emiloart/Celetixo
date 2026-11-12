//! Scoped PostgreSQL authority for the checked native Engine lifecycle.
//! Captured-source and policy state fail closed until their full recovery exists.
use crate::{StoreError, UnixConnection, migrations, semantic, snapshots, validate_schema};
use celetixo_engine::engine::Permission;
use celetixo_engine::engine::authority::{
    AuthorityAccess, AuthorityActor, AuthorityBackend, AuthorityFailure, AuthorityMode,
    AuthorityState, NativeRecordLimits, NativeRecords, RepositoryRecord, SessionGrantRecord,
};
use celetixo_engine::ids::{Digest, PrincipalId, Scope, SnapshotId};
use celetixo_engine::persistence::RestoreError;
use postgres::{Client, GenericClient, IsolationLevel, Row, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const FILTER: &str =
    "organization_id=$1::text AND workspace_id=$2::text AND repository_id=$3::text";
const PROFILE: &str = "native-reference-v1";

mod native;
mod source;

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRegistrationView {
    pub state_generation: u64,
    pub records: celetixo_engine::engine::authority::SourceRecords,
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
/// Native transactions, attempts, leases, reference evidence and exact prepared
/// commit outcomes survive reopen. Captured-source and policy state fail closed.
pub struct PostgresAuthority {
    runtime: Login,
    administration: Login,
    limits: NativeRecordLimits,
    checkpoint: Option<Arc<Checkpoint>>,
    source_profile: bool,
}

impl PostgresAuthority {
    /// Explicit trusted profile selection, including pre-registration grants.
    /// An already bound native scope cannot be reinterpreted as source state.
    pub fn connect_source(
        runtime: UnixConnection,
        administration: UnixConnection,
    ) -> Result<Self, AuthorityFailure> {
        let mut authority = Self::connect(runtime, administration)?;
        authority.source_profile = true;
        Ok(authority)
    }
    /// Trusted coherent metadata inspection. Source readers and capabilities
    /// still require independent runtime authentication and resolution.
    pub fn inspect_source(
        &self,
        scope: &Scope,
    ) -> Result<SourceRegistrationView, AuthorityFailure> {
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
        let a = snapshots::scope_parameters(scope);
        let profile = tx
            .query_one(
                &format!("SELECT authority_profile FROM celetixo.repositories WHERE {FILTER}"),
                &[&a[0], &a[1], &a[2]],
            )
            .map_err(database)?;
        if profile.get::<_, Option<String>>(0).as_deref() != Some(source::PROFILE) {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
        let records = source::load(&mut tx, scope, self.limits)?;
        tx.commit().map_err(database)?;
        Ok(SourceRegistrationView {
            state_generation: generation,
            records,
        })
    }
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
            source_profile: false,
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
    pub fn inspect_native(&self, scope: &Scope) -> Result<RegistrationView, AuthorityFailure> {
        self.inspect_registration(scope)
    }

    /// Compatibility name retained from the registration milestone. The view
    /// now includes all checked native lifecycle records, not only registration.
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
        profile_gate(&mut tx, scope, PROFILE, false)?;
        let records = load_native(&mut tx, scope, self.limits)?;
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
        if self.source_profile {
            return source::access(self, context, evaluate);
        }
        if matches!(
            context.actor(),
            AuthorityActor::SourceExecutor { .. } | AuthorityActor::SourceCleanup { .. }
        ) {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
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
        profile_gate(&mut tx, context.scope(), PROFILE, update)?;
        let before = load_native(&mut tx, context.scope(), self.limits)?;
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
        checked(after.clone(), self.limits)?;
        if update {
            publish_native(&mut tx, context, login, generation, &before, &after)?;
            // SQL constraints are insufficient: verify the exact checked facts
            // read back through the same coherent transaction before COMMIT.
            if load_native(&mut tx, context.scope(), self.limits)? != after {
                return Err(AuthorityFailure::Integrity);
            }
            self.checkpoint(AuthorityStage::BeforeCommit)?;
            login.check(&mut tx, Some(context.scope()))?;
            native::final_commit_gate(&mut tx, &before, &after)?;
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
fn profile_gate(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    expected: &str,
    bind: bool,
) -> Result<(), AuthorityFailure> {
    let a = snapshots::scope_parameters(scope);
    let row = tx
        .query_one(
            &format!("SELECT authority_profile FROM celetixo.repositories WHERE {FILTER}"),
            &[&a[0], &a[1], &a[2]],
        )
        .map_err(database)?;
    match row.get::<_, Option<String>>(0) {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => Err(AuthorityFailure::UnsupportedProfile),
        None if !bind => Ok(()),
        None => {
            let count=tx.execute(&format!("UPDATE celetixo.repositories SET authority_profile=$4 WHERE {FILTER} AND authority_profile IS NULL"),&[&a[0],&a[1],&a[2],&expected]).map_err(database)?;
            if count == 1 {
                Ok(())
            } else {
                Err(AuthorityFailure::Contention)
            }
        }
    }
}

fn checked(
    records: NativeRecords,
    limits: NativeRecordLimits,
) -> Result<AuthorityState, AuthorityFailure> {
    AuthorityState::from_native_records(records, limits).map_err(restore)
}

fn load_native(
    tx: &mut Transaction<'_>,
    scope: &Scope,
    limits: NativeRecordLimits,
) -> Result<NativeRecords, AuthorityFailure> {
    native::load(tx, scope, limits)
}

fn publish_native(
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
    publish_session_grants(tx, context, next, before, after)?;
    native::publish(tx, next, before, after)?;
    publish_envelope(tx, context, login, generation, next, after, false)
}
fn publish_session_grants(
    tx: &mut Transaction<'_>,
    context: &AuthorityAccess,
    next: u64,
    before: &NativeRecords,
    after: &NativeRecords,
) -> Result<(), AuthorityFailure> {
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
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn publish_envelope(
    tx: &mut Transaction<'_>,
    context: &AuthorityAccess,
    login: &Login,
    generation: u64,
    next: u64,
    after: &NativeRecords,
    source: bool,
) -> Result<(), AuthorityFailure> {
    let scope = context.scope();
    let a = snapshots::scope_parameters(scope);
    tx.execute("INSERT INTO celetixo.engine_counters (organization_id,workspace_id,repository_id,last_clock_ms,next_session_grant) VALUES ($1::text,$2::text,$3::text,$4::text::numeric,$5::text::numeric) ON CONFLICT (organization_id,workspace_id,repository_id) DO UPDATE SET last_clock_ms=EXCLUDED.last_clock_ms,next_session_grant=EXCLUDED.next_session_grant", &[&a[0],&a[1],&a[2],&after.clock_ms.to_string(),&after.session_counter.to_string()]).map_err(database)?;
    let changed=tx.execute(&format!("UPDATE celetixo.repositories SET state_generation=$5::text::numeric WHERE {FILTER} AND state_generation=$4::text::numeric"), &[&a[0],&a[1],&a[2],&generation.to_string(),&next.to_string()]).map_err(database)?;
    if changed != 1 {
        return Err(AuthorityFailure::Contention);
    }
    let (kind, principal, grant, permission) = match context.actor() {
        AuthorityActor::SourceExecutor { principal, grant } if source => (
            "SourceExecutor",
            Some(principal.as_str()),
            Some(grant.to_string()),
            None,
        ),
        AuthorityActor::SourceCleanup { principal, grant } if source => (
            "SourceCleanup",
            Some(principal.as_str()),
            Some(grant.to_string()),
            None,
        ),
        AuthorityActor::SourceExecutor { .. } | AuthorityActor::SourceCleanup { .. } => {
            return Err(AuthorityFailure::UnsupportedProfile);
        }
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
