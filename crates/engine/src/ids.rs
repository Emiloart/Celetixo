//! Scoped opaque identities and domain-separated content digests.
use sha2::{Digest as _, Sha256};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("identity must contain 1..=256 non-whitespace, non-control characters")]
pub struct InvalidId;

macro_rules! opaque_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidId> {
                let value = value.into();
                if value.is_empty() || value.len() > 256 || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    return Err(InvalidId);
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str { &self.0 }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
    )+};
}
opaque_id!(
    OrganizationId,
    WorkspaceId,
    RepositoryId,
    PrincipalId,
    TransactionId,
    SourceAttemptId,
    ElementId,
    InvariantId,
    CheckId,
    IdempotencyKey
);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Scope {
    pub organization: OrganizationId,
    pub workspace: WorkspaceId,
    pub repository: RepositoryId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// Decode an existing content address; consumers still verify its payload.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn of(domain: &str, bytes: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update((domain.len() as u64).to_be_bytes());
        hash.update(domain.as_bytes());
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
        Self(hash.finalize().into())
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

macro_rules! digest_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub Digest);
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
    )+};
}
digest_id!(
    SnapshotId,
    PatchId,
    EvidenceId,
    SourcePatchId,
    SourcePlanId,
    SourceAdmissionId,
    SourceEvidenceId
);

/// Length-delimited canonical encoding. Maps/sets are encoded in sorted order by callers.
#[derive(Default)]
pub(crate) struct Canonical(Vec<u8>);
impl Canonical {
    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        self.0
            .extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        self.0.extend_from_slice(bytes);
    }
    pub(crate) fn str(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }
    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes(&value.to_be_bytes());
    }
    pub(crate) fn i64(&mut self, value: i64) {
        self.bytes(&value.to_be_bytes());
    }
    pub(crate) fn digest(&mut self, value: Digest) {
        self.bytes(value.as_bytes());
    }
    pub(crate) fn finish(self, domain: &str) -> Digest {
        Digest::of(domain, &self.0)
    }
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifiers_reject_empty_or_ambiguous_values() {
        assert!(TransactionId::new("").is_err());
        assert!(TransactionId::new("a b").is_err());
        assert!(TransactionId::new("a\nb").is_err());
        assert_eq!(
            TransactionId::new("attempt:1").unwrap().as_str(),
            "attempt:1"
        );
    }
    #[test]
    fn digests_bind_domain_and_unambiguous_bytes() {
        assert_ne!(
            Digest::of("snapshot/v1", b"a"),
            Digest::of("patch/v1", b"a")
        );
        assert_ne!(Digest::of("a", b"bc"), Digest::of("ab", b"c"));
        assert_eq!(Digest::of("a", b"bc").to_string().len(), 64);
    }
}
