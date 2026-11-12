//! Typed immutable source authority definitions. No live adapter is restored.
use super::*;
use crate::engine::SourceArtifactAdapter;
use crate::ids::{Canonical, SourceArtifactAdapterId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceArtifactRegistrationRecord {
    scope: Scope,
    registration: u64,
    adapter: SourceArtifactAdapter,
    retention: Digest,
    provenance: Digest,
}
impl SourceArtifactRegistrationRecord {
    pub const FORMAT: &str = "celetixo/source-artifact-registration/v1";

    pub fn new(
        scope: Scope,
        registration: u64,
        adapter: SourceArtifactAdapter,
        retention: Digest,
    ) -> Result<Self, RestoreError> {
        if registration == 0 {
            return Err(RestoreError::BindingMismatch);
        }
        let provenance = adapter.registration_provenance(&scope, registration, retention);
        Ok(Self {
            scope,
            registration,
            adapter,
            retention,
            provenance,
        })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn registration(&self) -> u64 {
        self.registration
    }
    pub fn adapter(&self) -> &SourceArtifactAdapter {
        &self.adapter
    }
    pub fn retention(&self) -> Digest {
        self.retention
    }
    pub fn provenance(&self) -> Digest {
        self.provenance
    }
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        c.str(self.scope.organization.as_str());
        c.str(self.scope.workspace.as_str());
        c.str(self.scope.repository.as_str());
        c.u64(self.registration);
        c.str(self.adapter.identity.as_str());
        c.digest(self.adapter.configuration);
        c.digest(self.retention);
        c.digest(self.provenance);
        c.into_bytes()
    }
}

pub fn restore_source_artifact_registration(
    scope: &Scope,
    registration: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<SourceArtifactRegistrationRecord, RestoreError> {
    let mut r = Reader::new(
        expected,
        SourceArtifactRegistrationRecord::FORMAT,
        bytes,
        limits,
    )?;
    if r.text()? != scope.organization.as_str()
        || r.text()? != scope.workspace.as_str()
        || r.text()? != scope.repository.as_str()
        || r.number()? != registration
    {
        return Err(RestoreError::BindingMismatch);
    }
    let adapter = SourceArtifactAdapter {
        identity: SourceArtifactAdapterId::new(r.text()?).map_err(|_| RestoreError::Identity)?,
        configuration: r.digest()?,
    };
    let retention = r.digest()?;
    let provenance = r.digest()?;
    r.end()?;
    let record =
        SourceArtifactRegistrationRecord::new(scope.clone(), registration, adapter, retention)?;
    if provenance != record.provenance() {
        return Err(RestoreError::BindingMismatch);
    }
    lifecycle::canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}
