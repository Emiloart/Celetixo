//! Common subordinate-executor contracts. Implementations cannot decide whether
//! evidence is admissible, publish a source revision, or widen Engine authority.
use crate::materialize::PrivateWorkspace;
use celetixo_engine::engine::{Engine, SourceAdmission, SourceExecutorCapability};
use celetixo_engine::ids::Digest;
use celetixo_engine::source_evidence::SourceCheckRecordInput;
use celetixo_engine::source_execution::{BackendIdentity, SourceCheckDefinition};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardDecision {
    Continue,
    Cancelled,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionCheckpoint {
    BeforeClaimPublication,
    AfterClaimPublication,
    BeforeRecordPublication,
    BeforeControlValidation,
    AfterRecordPublication,
}

/// Trusted callbacks may narrow execution and drive the reference Engine clock.
/// A backend must still validate Engine authority itself before side effects,
/// before command release and while supervising the complete process tree.
pub trait ExecutionGuard {
    fn revalidate(&mut self) -> Result<GuardDecision, String>;
    fn checkpoint(&mut self, _checkpoint: ExecutionCheckpoint) -> Result<(), String> {
        Ok(())
    }
}

pub(crate) mod sealed {
    pub trait Backend {}
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("requested execution facility is unsupported: {0}")]
    Unsupported(&'static str),
    #[error("execution bindings differ from Engine admission")]
    Binding,
    #[error("execution authority was rejected: {0}")]
    Authority(String),
    #[error("execution {0} was already claimed; reconcile instead of relaunching")]
    AlreadyClaimed(Digest),
    #[error("execution capacity is busy or requires owned reconciliation")]
    Busy,
    #[error("backend execution failed: {0}")]
    Runtime(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug)]
pub struct IsolationObservation {
    pub media_type: String,
    /// Actual versioned backend observations, retained as bytes rather than a
    /// digest that points only to a controller's private working directory.
    pub bytes: Vec<u8>,
}

/// Raw facts from one admitted check, with no aggregate pass/admission flag.
/// The coordinator validates the exact bindings and publishes artifacts. The
/// Engine independently reads those artifacts and decides their assessment.
#[derive(Debug)]
pub struct BackendCheckResult {
    pub execution: Digest,
    pub record: SourceCheckRecordInput,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub isolation: Option<IsolationObservation>,
}

/// Only reviewed implementations in this crate can implement this interface.
/// It exposes Engine-neutral evidence facts, not a particular container, VM,
/// process runtime or remote-execution record representation.
pub trait VerificationBackend: sealed::Backend {
    fn identity(&self) -> &BackendIdentity;
    fn execution_identity(
        &self,
        admission: &SourceAdmission,
        check: &SourceCheckDefinition,
    ) -> Digest;
    fn execute(
        &self,
        engine: &Engine,
        executor: &SourceExecutorCapability,
        admission: &SourceAdmission,
        workspace: &mut PrivateWorkspace,
        check: &SourceCheckDefinition,
        guard: &mut dyn ExecutionGuard,
    ) -> Result<BackendCheckResult, BackendError>;
}
