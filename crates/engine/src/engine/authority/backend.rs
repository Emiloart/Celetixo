//! Synchronous trusted authority adapter contract. SQL adapters own transactional
//! loading/publication; the callback still runs the Engine's Rust decisions.
use super::*;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityMode {
    Read,
    Update,
    AdministrativeRead,
    AdministrativeUpdate,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityActor {
    /// Attribution only. The callback checks the private Session identity and
    /// current scoped grant before any sensitive domain operation.
    Session {
        principal: PrincipalId,
        grant: u64,
        permission: Permission,
    },
    SourceExecutor {
        principal: PrincipalId,
        grant: u64,
    },
    SourceCleanup {
        principal: PrincipalId,
        grant: u64,
    },
    /// Held by the trusted embedding. A SQL adapter must use its separately
    /// authenticated administrative connection and record that service identity.
    EmbeddingAdministrator,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityAccess {
    scope: Scope,
    mode: AuthorityMode,
    actor: AuthorityActor,
}
impl AuthorityAccess {
    pub(in crate::engine) fn new(scope: Scope, mode: AuthorityMode, actor: AuthorityActor) -> Self {
        Self { scope, mode, actor }
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn mode(&self) -> AuthorityMode {
        self.mode
    }
    pub fn actor(&self) -> &AuthorityActor {
        &self.actor
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthorityFailure {
    #[error("authoritative storage is unavailable")]
    Unavailable,
    #[error("storage schema differs from this Engine adapter")]
    SchemaMismatch,
    #[error("durable authority records failed checked reconstruction")]
    Integrity,
    #[error("storage does not authorize the requested repository scope")]
    ScopeDenied,
    #[error("durable state or operation is unsupported by this adapter profile")]
    UnsupportedProfile,
    #[error("authoritative clock moved backward; operator reconciliation is required")]
    ClockRegression,
    #[error("authority storage transaction encountered bounded contention")]
    Contention,
    #[error("authority state generation is exhausted")]
    CounterExhausted,
    #[error("storage commit outcome is indeterminate; reconcile the exact operation identity")]
    CommitIndeterminate,
    #[error("authority backend violated its evaluation/publication contract")]
    Protocol,
}

/// Trusted storage implementation, never a worker/provider extension. Invoke
/// the callback exactly once on checked state in the requested scope. Callback
/// Ok means evaluation completed, not domain success: a rejection may retain
/// an outcome, revalidation or reconciliation. Publish all intended changes in
/// one explicit authority transaction and confirm commit before returning Ok.
/// On indeterminate commit withhold the domain response and require durable
/// operation lookup. Never commit on Drop, reuse stale cache as authority, or
/// fall back to process memory. Administrative writes use a separate trusted role.
pub trait AuthorityBackend: Send + Sync {
    fn access(
        &self,
        context: &AuthorityAccess,
        evaluate: &mut dyn FnMut(&mut AuthorityState) -> Result<(), AuthorityFailure>,
    ) -> Result<(), AuthorityFailure>;
}
