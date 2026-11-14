use super::*;
use crate::persistence::{RecoverySize, RestoreError};

impl SourceRevalidationReceipt {
    pub const FORMAT: &str = "celetixo/source-revalidation-receipt/v1";
    pub fn digest(&self) -> Digest {
        Digest::of(Self::FORMAT, &self.canonical_bytes())
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = Canonical::default();
        self.encode_fields(&mut c);
        c.into_bytes()
    }
    pub(crate) fn encode_fields(&self, c: &mut Canonical) {
        for value in [
            self.scope.organization.as_str(),
            self.scope.workspace.as_str(),
            self.scope.repository.as_str(),
            self.transaction.as_str(),
        ] {
            c.str(value);
        }
        c.u64(self.previous_attempt);
        c.u64(self.attempt);
        c.digest(self.previous_base.0);
        c.digest(self.base.0);
        c.u64(self.revision.0);
        c.u64(self.model_generation);
        c.digest(self.analysis);
        c.digest(self.index_generation);
        c.digest(self.policy);
        c.u64(self.policy_generation);
        c.u64(self.previous_fence);
        c.u64(self.fence);
        c.u64(self.expires_at_ms);
        self.accesses.encode(c);
        c.digest(self.impact);
        c.u64(self.invalidated_source_attempts.len() as u64);
        for id in &self.invalidated_source_attempts {
            c.str(id.as_str());
        }
        c.u64(self.intervening_revisions.len() as u64);
        for revision in &self.intervening_revisions {
            c.u64(revision.0);
        }
        c.digest(self.request);
        c.u64(self.revalidated_at_ms);
    }
    /// Structural binding only. Historical effects and original decisions are
    /// checked by complete authority restoration; this metadata grants nothing.
    pub fn validate_request(
        &self,
        scope: &Scope,
        principal: &PrincipalId,
        request: &SourceRevalidationRequest,
    ) -> Result<(), RestoreError> {
        if request.scope != *scope
            || request.principal != *principal
            || self.scope != *scope
            || self.transaction != request.transaction
            || self.previous_attempt != request.previous_attempt
            || self.previous_attempt.checked_add(1) != Some(self.attempt)
            || self.previous_base != request.previous_base
            || self.base != request.head
            || self.revision != request.revision
            || self.model_generation != request.model_generation
            || self.model_generation == 0
            || self.analysis != request.analysis
            || self.index_generation != request.index_generation
            || self.policy != request.policy
            || self.policy_generation != request.policy_generation
            || self.previous_fence != request.fence
            || self.previous_fence == 0
            || self.fence <= self.previous_fence
            || self.expires_at_ms <= self.revalidated_at_ms
            || expiration(self.revalidated_at_ms, request.ttl_ms)
                .map_or(true, |expiry| self.expires_at_ms > expiry)
            || self.request != request.digest(scope, principal)
            || self
                .intervening_revisions
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .intervening_revisions
                .last()
                .is_some_and(|r| *r > self.revision)
        {
            return Err(RestoreError::BindingMismatch);
        }
        Ok(())
    }
    pub(crate) fn recovery_size(&self) -> Result<RecoverySize, RestoreError> {
        let mut size = RecoverySize {
            entries: 1,
            bytes: 4096,
        };
        for set in [
            &self.accesses.reads,
            &self.accesses.writes,
            &self.accesses.invariant_scopes,
        ] {
            for id in set {
                size.entries = size.entries.checked_add(1).ok_or(RestoreError::Limit)?;
                size.bytes = size
                    .bytes
                    .checked_add(id.as_str().len() + 8)
                    .ok_or(RestoreError::Limit)?;
            }
        }
        for crate::access::Predicate::Prefix(prefix) in &self.accesses.predicates {
            size.entries = size.entries.checked_add(1).ok_or(RestoreError::Limit)?;
            size.bytes = size
                .bytes
                .checked_add(prefix.len() + 32)
                .ok_or(RestoreError::Limit)?;
        }
        for id in &self.invalidated_source_attempts {
            size.entries = size.entries.checked_add(1).ok_or(RestoreError::Limit)?;
            size.bytes = size
                .bytes
                .checked_add(id.as_str().len() + 8)
                .ok_or(RestoreError::Limit)?;
        }
        size.entries = size
            .entries
            .checked_add(self.intervening_revisions.len())
            .ok_or(RestoreError::Limit)?;
        size.bytes = size
            .bytes
            .checked_add(
                self.intervening_revisions
                    .len()
                    .checked_mul(16)
                    .ok_or(RestoreError::Limit)?,
            )
            .ok_or(RestoreError::Limit)?;
        Ok(size)
    }
}
