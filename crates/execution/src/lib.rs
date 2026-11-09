//! Subordinate execution components. Engine admission and evidence authority remain
//! in celetixo-engine. Copying source into a private directory is not a sandbox.
pub mod artifacts;
pub mod backend;
pub mod coordinator;
pub mod materialize;
pub mod oci;
mod oci_adapter;
#[cfg(target_os = "linux")]
pub mod workspaces;
