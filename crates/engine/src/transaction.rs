//! Intent and lifecycle contracts. Only the Engine mutates stored transactions.
use crate::access::{AccessSet, Invariant};
use crate::evidence::CheckSpec;
use crate::ids::{PrincipalId, Scope, SnapshotId, TransactionId};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    Proposed,
    Analyzed,
    Reserved,
    Executing,
    Validating,
    RevalidationRequired,
    Committed,
    Aborted,
}

impl TransactionState {
    pub fn allows(self, next: Self) -> bool {
        use TransactionState::*;
        matches!(
            (self, next),
            (Proposed, Analyzed)
                | (Analyzed, Reserved)
                | (Reserved, Executing)
                | (Executing, Validating)
                | (Validating, Committed)
                | (Validating, RevalidationRequired)
                | (RevalidationRequired, Validating)
        ) || (next == Aborted && !matches!(self, Committed | Aborted))
    }
}

/// Native Engine admission contract, not a replacement for the cross-service TaskIntent DSL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionIntent {
    pub objective: String,
    pub accesses: AccessSet,
    pub invariants: Vec<Invariant>,
    pub required_checks: Vec<CheckSpec>,
    pub depends_on: BTreeSet<TransactionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionView {
    pub id: TransactionId,
    pub scope: Scope,
    pub owner: PrincipalId,
    pub state: TransactionState,
    pub attempt: u64,
    pub original_snapshot: SnapshotId,
    pub attempt_snapshot: SnapshotId,
    pub attempts: usize,
    pub last_reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::TransactionState::*;
    #[test]
    fn terminals_cannot_be_revived_and_paths_are_explicit() {
        for state in [
            Proposed,
            Analyzed,
            Reserved,
            Executing,
            Validating,
            RevalidationRequired,
            Committed,
            Aborted,
        ] {
            assert!(!Committed.allows(state));
            assert!(!Aborted.allows(state));
        }
        assert!(Proposed.allows(Analyzed));
        assert!(!Proposed.allows(Committed));
        assert!(Validating.allows(RevalidationRequired));
        assert!(RevalidationRequired.allows(Validating));
    }
}
