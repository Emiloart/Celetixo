//! Structural original admin input, including invalid options and unresolved
//! selectors. Parsing never allocates a grant or resolves a runtime capability.
use super::*;
use crate::engine::{
    AdministrativeMutation as A, AdministrativeResource, Permission, SourceArtifactAdapter,
};
use crate::ids::{PrincipalId, SourceArtifactAdapterId};
use crate::source_execution::BackendIdentity;

pub(super) fn is_kind(kind: &str) -> bool {
    matches!(
        kind,
        "GrantSession"
            | "RevokeSession"
            | "GrantExecutor"
            | "RevokeExecutor"
            | "GrantCleanup"
            | "RevokeCleanup"
            | "RegisterArtifactStore"
            | "RevokeArtifactStore"
    )
}
fn principal(r: &mut Reader<'_>) -> Result<PrincipalId, RestoreError> {
    PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)
}
fn backend(r: &mut Reader<'_>) -> Result<BackendIdentity, RestoreError> {
    Ok(BackendIdentity {
        name: r.text()?,
        version: r.text()?,
        image: r.digest()?,
        profile: r.digest()?,
    })
}
pub(super) fn resource(r: &mut Reader<'_>) -> Result<AdministrativeResource, RestoreError> {
    Ok(match r.field()? {
        b"Session" => AdministrativeResource::Session,
        b"Executor" => AdministrativeResource::Executor,
        b"Cleanup" => AdministrativeResource::Cleanup,
        b"ArtifactStore" => AdministrativeResource::ArtifactStore,
        _ => return Err(RestoreError::Encoding),
    })
}
fn permission(r: &mut Reader<'_>) -> Result<Permission, RestoreError> {
    Ok(match r.field()? {
        b"Register" => Permission::Register,
        b"ManageModel" => Permission::ManageModel,
        b"Read" => Permission::Read,
        b"Propose" => Permission::Propose,
        b"Analyze" => Permission::Analyze,
        b"Reserve" => Permission::Reserve,
        b"Execute" => Permission::Execute,
        b"Verify" => Permission::Verify,
        b"Commit" => Permission::Commit,
        b"Cancel" => Permission::Cancel,
        _ => return Err(RestoreError::Encoding),
    })
}
pub(super) fn decode(r: &mut Reader<'_>, kind: &str) -> Result<A, RestoreError> {
    Ok(match kind {
        "GrantSession" => {
            let principal = principal(r)?;
            let mut permissions = BTreeSet::new();
            for _ in 0..r.count()? {
                if !permissions.insert(permission(r)?) {
                    return Err(RestoreError::NonCanonical);
                }
            }
            A::GrantSession {
                principal,
                permissions,
            }
        }
        "RevokeSession" => A::RevokeSession {
            grant: r.number()?,
            principal: principal(r)?,
        },
        "GrantExecutor" => {
            let principal = principal(r)?;
            let backend = backend(r)?;
            let mut tiers = BTreeSet::new();
            for _ in 0..r.count()? {
                if !tiers.insert(source_execution::tier(r)?) {
                    return Err(RestoreError::NonCanonical);
                }
            }
            A::GrantExecutor {
                principal,
                backend,
                tiers,
                resources: source_execution::resources(r)?,
                expires_at_ms: r.number()?,
            }
        }
        "RevokeExecutor" => A::RevokeExecutor {
            grant: r.number()?,
            principal: principal(r)?,
            backend: backend(r)?,
        },
        "GrantCleanup" => A::GrantCleanup {
            principal: principal(r)?,
            backend: r.digest()?,
            expires_at_ms: r.number()?,
        },
        "RevokeCleanup" => A::RevokeCleanup {
            grant: r.number()?,
            principal: principal(r)?,
            backend: r.digest()?,
        },
        "RegisterArtifactStore" => A::RegisterArtifactStore {
            adapter: SourceArtifactAdapter {
                identity: SourceArtifactAdapterId::new(r.text()?)
                    .map_err(|_| RestoreError::Identity)?,
                configuration: r.digest()?,
            },
            retention: r.digest()?,
        },
        "RevokeArtifactStore" => A::RevokeArtifactStore {
            registration: r.number()?,
        },
        _ => return Err(RestoreError::Encoding),
    })
}
