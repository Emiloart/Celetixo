//! Checked lease history and binary-safe reason values for normalized SQL rows.
use super::lifecycle::{canonical, decode_elements, encode_elements};
use super::outcome::{context, validate_context};
use super::*;
use crate::engine::{LeaseEvent, LeaseEventKind};
use crate::ids::{PrincipalId, TransactionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionReasonRecord {
    scope: Scope,
    transaction: TransactionId,
    attempt: u64,
    reason: String,
    digest: Digest,
}
impl TransactionReasonRecord {
    pub const FORMAT: &str = "celetixo/transaction-reason-record/v1";
    pub fn new(
        scope: Scope,
        transaction: TransactionId,
        attempt: u64,
        reason: String,
    ) -> Result<Self, RestoreError> {
        if reason.trim().is_empty() {
            return Err(RestoreError::InvalidDomainValue);
        }
        let mut record = Self {
            scope,
            transaction,
            attempt,
            reason,
            digest: Digest::from_bytes([0; 32]),
        };
        record.digest = Digest::of(Self::FORMAT, &record.canonical_bytes());
        Ok(record)
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = context(&self.scope, &self.transaction, self.attempt);
        c.str(&self.reason);
        c.into_bytes()
    }
}
pub fn restore_transaction_reason(
    scope: &Scope,
    transaction: &TransactionId,
    attempt: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<TransactionReasonRecord, RestoreError> {
    let mut r = Reader::new(expected, TransactionReasonRecord::FORMAT, bytes, limits)?;
    validate_context(&mut r, scope, transaction, attempt)?;
    let reason = r.text()?;
    r.end()?;
    let record = TransactionReasonRecord::new(scope.clone(), transaction.clone(), attempt, reason)?;
    canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseEventRecord {
    scope: Scope,
    transaction: TransactionId,
    sequence: u64,
    event: LeaseEvent,
    digest: Digest,
}
impl LeaseEventRecord {
    pub const FORMAT: &str = "celetixo/lease-event-record/v1";
    pub fn new(
        scope: Scope,
        transaction: TransactionId,
        sequence: u64,
        event: LeaseEvent,
    ) -> Result<Self, RestoreError> {
        if event.fence == 0 || event.authorization_grant == 0 {
            return Err(RestoreError::InvalidDomainValue);
        }
        match &event.kind {
            LeaseEventKind::Acquired { expires_at } if *expires_at <= event.at => {
                return Err(RestoreError::InvalidDomainValue);
            }
            LeaseEventKind::Renewed {
                previous_expiry,
                expires_at,
            } if *previous_expiry <= event.at || expires_at < previous_expiry => {
                return Err(RestoreError::InvalidDomainValue);
            }
            LeaseEventKind::Released { reason } if reason.trim().is_empty() => {
                return Err(RestoreError::InvalidDomainValue);
            }
            _ => {}
        }
        let mut record = Self {
            scope,
            transaction,
            sequence,
            event,
            digest: Digest::from_bytes([0; 32]),
        };
        record.digest = Digest::of(Self::FORMAT, &record.canonical_bytes());
        Ok(record)
    }
    pub fn event(&self) -> &LeaseEvent {
        &self.event
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut c = context(&self.scope, &self.transaction, self.sequence);
        let e = &self.event;
        c.u64(e.fence);
        c.u64(e.authorization_grant);
        c.u64(e.acquisition_attempt);
        c.u64(e.at);
        c.str(e.owner.as_str());
        encode_elements(&mut c, &e.scopes);
        match &e.kind {
            LeaseEventKind::Acquired { expires_at } => {
                c.str("Acquired");
                c.u64(*expires_at)
            }
            LeaseEventKind::Renewed {
                previous_expiry,
                expires_at,
            } => {
                c.str("Renewed");
                c.u64(*previous_expiry);
                c.u64(*expires_at)
            }
            LeaseEventKind::Expired => c.str("Expired"),
            LeaseEventKind::Revoked => c.str("Revoked"),
            LeaseEventKind::Released { reason } => {
                c.str("Released");
                c.str(reason)
            }
        }
        c.into_bytes()
    }
}
pub fn restore_lease_event(
    scope: &Scope,
    transaction: &TransactionId,
    sequence: u64,
    expected: Digest,
    bytes: &[u8],
    limits: DomainDecodeLimits,
) -> Result<LeaseEventRecord, RestoreError> {
    let mut r = Reader::new(expected, LeaseEventRecord::FORMAT, bytes, limits)?;
    validate_context(&mut r, scope, transaction, sequence)?;
    let fence = r.number()?;
    let authorization_grant = r.number()?;
    let acquisition_attempt = r.number()?;
    let at = r.number()?;
    let owner = PrincipalId::new(r.text()?).map_err(|_| RestoreError::Identity)?;
    let scopes = decode_elements(&mut r)?;
    let kind = match r.text()?.as_str() {
        "Acquired" => LeaseEventKind::Acquired {
            expires_at: r.number()?,
        },
        "Renewed" => LeaseEventKind::Renewed {
            previous_expiry: r.number()?,
            expires_at: r.number()?,
        },
        "Expired" => LeaseEventKind::Expired,
        "Revoked" => LeaseEventKind::Revoked,
        "Released" => LeaseEventKind::Released { reason: r.text()? },
        _ => return Err(RestoreError::Encoding),
    };
    r.end()?;
    let record = LeaseEventRecord::new(
        scope.clone(),
        transaction.clone(),
        sequence,
        LeaseEvent {
            fence,
            authorization_grant,
            acquisition_attempt,
            at,
            owner,
            scopes,
            kind,
        },
    )?;
    canonical(record.digest(), expected, &record.canonical_bytes(), bytes)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{OrganizationId, RepositoryId, WorkspaceId};
    fn scope() -> Scope {
        Scope {
            organization: OrganizationId::new("org").unwrap(),
            workspace: WorkspaceId::new("ws").unwrap(),
            repository: RepositoryId::new("repo").unwrap(),
        }
    }
    fn tx() -> TransactionId {
        TransactionId::new("tx").unwrap()
    }
    #[test]
    fn binary_reason_keeps_exact_controls_and_checks_context_digest_and_limits() {
        let record =
            TransactionReasonRecord::new(scope(), tx(), u64::MAX, " reason\0λ\n".into()).unwrap();
        let bytes = record.canonical_bytes();
        assert_eq!(
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            include_str!("../../tests/fixtures/transaction-reason-record-v1.hex").trim()
        );
        assert_eq!(
            restore_transaction_reason(
                &scope(),
                &tx(),
                u64::MAX,
                record.digest(),
                &bytes,
                DomainDecodeLimits::default()
            )
            .unwrap(),
            record
        );
        assert!(
            restore_transaction_reason(
                &scope(),
                &tx(),
                0,
                record.digest(),
                &bytes,
                DomainDecodeLimits::default()
            )
            .is_err()
        );
        assert!(
            restore_transaction_reason(
                &scope(),
                &tx(),
                u64::MAX,
                Digest::from_bytes([0; 32]),
                &bytes,
                DomainDecodeLimits::default()
            )
            .is_err()
        );
        assert_eq!(
            restore_transaction_reason(
                &scope(),
                &tx(),
                u64::MAX,
                record.digest(),
                &bytes,
                DomainDecodeLimits {
                    encoded_bytes: bytes.len() - 1,
                    ..Default::default()
                }
            ),
            Err(RestoreError::Limit)
        );
        assert!(TransactionReasonRecord::new(scope(), tx(), 0, " \n".into()).is_err());
        for end in 0..bytes.len() {
            let truncated = &bytes[..end];
            assert!(
                restore_transaction_reason(
                    &scope(),
                    &tx(),
                    u64::MAX,
                    Digest::of(TransactionReasonRecord::FORMAT, truncated),
                    truncated,
                    DomainDecodeLimits::default()
                )
                .is_err()
            );
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(
            restore_transaction_reason(
                &scope(),
                &tx(),
                u64::MAX,
                Digest::of(TransactionReasonRecord::FORMAT, &trailing),
                &trailing,
                DomainDecodeLimits::default()
            )
            .is_err()
        );
    }
    #[test]
    fn all_lease_events_roundtrip_and_reject_forged_tags_invalid_values_and_budgets() {
        for kind in [
            LeaseEventKind::Acquired {
                expires_at: u64::MAX,
            },
            LeaseEventKind::Renewed {
                previous_expiry: 2,
                expires_at: u64::MAX,
            },
            LeaseEventKind::Expired,
            LeaseEventKind::Revoked,
            LeaseEventKind::Released {
                reason: "reason\0λ".into(),
            },
        ] {
            let event = LeaseEvent {
                fence: u64::MAX,
                authorization_grant: u64::MAX,
                acquisition_attempt: u64::MAX,
                at: 1,
                owner: PrincipalId::new("owner").unwrap(),
                scopes: BTreeSet::from([
                    ElementId::new("a").unwrap(),
                    ElementId::new("b").unwrap(),
                ]),
                kind,
            };
            let record = LeaseEventRecord::new(scope(), tx(), u64::MAX, event.clone()).unwrap();
            let bytes = record.canonical_bytes();
            if matches!(record.event().kind, LeaseEventKind::Released { .. }) {
                assert_eq!(
                    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                    include_str!("../../tests/fixtures/lease-event-record-v1.hex").trim()
                );
            }
            assert_eq!(
                restore_lease_event(
                    &scope(),
                    &tx(),
                    u64::MAX,
                    record.digest(),
                    &bytes,
                    DomainDecodeLimits::default()
                )
                .unwrap(),
                record
            );
            assert!(
                restore_lease_event(
                    &scope(),
                    &tx(),
                    0,
                    record.digest(),
                    &bytes,
                    DomainDecodeLimits::default()
                )
                .is_err()
            );
            assert_eq!(
                restore_lease_event(
                    &scope(),
                    &tx(),
                    u64::MAX,
                    record.digest(),
                    &bytes,
                    DomainDecodeLimits {
                        collection_entries: 1,
                        ..Default::default()
                    }
                ),
                Err(RestoreError::Limit)
            );
            let mut bad = event;
            bad.fence = 0;
            assert!(LeaseEventRecord::new(scope(), tx(), 0, bad).is_err());
            for end in 0..bytes.len() {
                let part = &bytes[..end];
                assert!(
                    restore_lease_event(
                        &scope(),
                        &tx(),
                        u64::MAX,
                        Digest::of(LeaseEventRecord::FORMAT, part),
                        part,
                        DomainDecodeLimits::default()
                    )
                    .is_err()
                );
            }
        }
        let event = LeaseEvent {
            fence: 1,
            authorization_grant: 1,
            acquisition_attempt: 0,
            at: 1,
            owner: PrincipalId::new("owner").unwrap(),
            scopes: BTreeSet::new(),
            kind: LeaseEventKind::Expired,
        };
        let mut bytes = LeaseEventRecord::new(scope(), tx(), 0, event.clone())
            .unwrap()
            .canonical_bytes();
        *bytes.last_mut().unwrap() = b'X';
        assert!(
            restore_lease_event(
                &scope(),
                &tx(),
                0,
                Digest::of(LeaseEventRecord::FORMAT, &bytes),
                &bytes,
                DomainDecodeLimits::default()
            )
            .is_err()
        );
        for kind in [
            LeaseEventKind::Acquired { expires_at: 1 },
            LeaseEventKind::Renewed {
                previous_expiry: 1,
                expires_at: 2,
            },
            LeaseEventKind::Renewed {
                previous_expiry: 4,
                expires_at: 3,
            },
            LeaseEventKind::Released { reason: " ".into() },
        ] {
            assert!(
                LeaseEventRecord::new(
                    scope(),
                    tx(),
                    0,
                    LeaseEvent {
                        kind,
                        ..event.clone()
                    }
                )
                .is_err()
            );
        }
    }
}
