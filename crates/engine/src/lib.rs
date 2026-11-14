//! Deterministic semantic transaction mechanics, independent of providers and clients.
pub mod access;
#[path = "input_admission.rs"]
pub mod credentials;
pub mod engine;
pub mod evidence;
pub mod ids;
pub mod model;
pub mod patch;
pub mod persistence;
pub mod repository;
pub mod snapshot;
pub mod source_evidence;
pub mod source_execution;
pub mod source_patch;
pub mod source_publication;
pub mod transaction;
