//! Shared reservation transitions evaluated only inside an Engine state callback.
//! Pure helpers keep authorization and lease rules at one Engine boundary.
use super::*;

impl Engine {
    pub(super) fn reserve_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        ttl: u64,
    ) -> Result<u64, EngineError> {
        self.authorize(db, session, scope, Permission::Reserve)?;
        let key = (scope.clone(), id.clone());
        let mut tx = db
            .transactions
            .get(&key)
            .ok_or(EngineError::NotFound)?
            .clone();
        if tx.owner != session.principal {
            return Err(EngineError::Unauthorized);
        }
        tx.transition(TransactionState::Reserved)?;
        let expires_at = expiration(db.now, ttl)?;
        let writes = tx
            .attempt()
            .effective
            .as_ref()
            .ok_or(EngineError::IncompleteAnalysis)?
            .writes
            .clone();
        for ((other_scope, other_id), other) in &db.transactions {
            if other_scope == scope
                && other_id != id
                && let Some(lease) = &other.lease
                && lease_active(db, lease)
                && !lease.writes.is_disjoint(&writes)
            {
                return Err(EngineError::ReservationConflict);
            }
        }
        let fence = db
            .reservation_fences
            .get(scope)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(EngineError::CounterExhausted)?;
        let lease = Lease {
            fence,
            owner: session.token,
            acquired_at: db.now,
            acquired_attempt: tx.attempt().number,
            expires_at,
            writes,
        };
        tx.lease_history.push(lease.event(
            db.now,
            session.principal.clone(),
            LeaseEventKind::Acquired { expires_at },
        ));
        tx.lease = Some(lease);
        db.reservation_fences.insert(scope.clone(), fence);
        db.transactions.insert(key, tx);
        Ok(expires_at)
    }

    pub(super) fn renew_in(
        &self,
        db: &mut Database,
        session: &Session,
        scope: &Scope,
        id: &TransactionId,
        ttl: u64,
    ) -> Result<u64, EngineError> {
        self.authorize(db, session, scope, Permission::Reserve)?;
        let key = (scope.clone(), id.clone());
        let tx = db.transactions.get(&key).ok_or(EngineError::NotFound)?;
        require_lease(db, tx, session)?;
        let expires = expiration(db.now, ttl)?;
        let now = db.now;
        let tx = db.transactions.get_mut(&key).ok_or(EngineError::NotFound)?;
        let lease = tx.lease.as_mut().ok_or(EngineError::InvalidLease)?;
        // Renewal never accidentally shortens a still-valid lease.
        let previous_expiry = lease.expires_at;
        lease.expires_at = expires.max(lease.expires_at);
        tx.lease_history.push(lease.event(
            now,
            session.principal.clone(),
            LeaseEventKind::Renewed {
                previous_expiry,
                expires_at: lease.expires_at,
            },
        ));
        Ok(lease.expires_at)
    }
}
