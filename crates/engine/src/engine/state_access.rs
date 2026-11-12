//! Explicit state evaluation and publication. Domain rejection is an outcome,
//! not an instruction to roll back: it may retain an idempotent rejection or a
//! revalidation/reconciliation transition. Storage failure is a separate result.
use super::*;

impl InMemoryStore {
    pub(super) fn read<R>(
        &self,
        operation: impl FnOnce(&Database) -> Result<R, EngineError>,
    ) -> Result<R, EngineError> {
        let guard = self.lock()?;
        operation(&guard)
    }

    pub(super) fn update<R>(
        &self,
        operation: impl FnOnce(&mut Database) -> Result<R, EngineError>,
    ) -> Result<R, EngineError> {
        let mut guard = self.lock()?;
        evaluate_and_publish(&mut guard, operation, |_| Ok(()))
    }
}

/// Evaluate against a private stage, then explicitly confirm publication before
/// exposing its domain outcome. The in-memory adapter supplies no persistence;
/// this callback is the internal publication seam, not a public backend API.
/// A future SQL adapter must load checked scoped records and perform all final
/// checks and persistence within its explicit storage transaction.
fn evaluate_and_publish<R>(
    current: &mut Database,
    operation: impl FnOnce(&mut Database) -> Result<R, EngineError>,
    publish: impl FnOnce(&Database) -> Result<(), EngineError>,
) -> Result<R, EngineError> {
    let mut staged = current.clone();
    let outcome = operation(&mut staged);
    publish(&staged)?;
    *current = staged;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn rejected_outcomes_can_retain_state_and_publication_precedes_acknowledgement() {
        let mut current = Database::default();
        for domain_result in [Ok(7), Err(EngineError::ResponseLost)] {
            let mut published = false;
            let outcome = evaluate_and_publish(
                &mut current,
                |state| {
                    state.now += 1;
                    // Models durable retained outcomes such as a source receipt
                    // whose delivery response was lost. Err must not undo it.
                    domain_result.clone()
                },
                |state| {
                    assert!(state.now > 0);
                    published = true;
                    Ok(())
                },
            );
            assert!(published);
            assert_eq!(outcome, domain_result);
        }
        assert_eq!(current.now, 2);
    }

    #[test]
    fn publication_failure_withholds_success_and_rejection_without_exposing_staged_state() {
        for domain_result in [Ok(7), Err(EngineError::IncompleteAnalysis)] {
            let mut current = Database {
                now: 10,
                ..Default::default()
            };
            let outcome = evaluate_and_publish(
                &mut current,
                |state| {
                    state.now = 20;
                    domain_result
                },
                |state| {
                    assert_eq!(state.now, 20);
                    Err(EngineError::StoreUnavailable)
                },
            );
            assert_eq!(outcome, Err(EngineError::StoreUnavailable));
            assert_eq!(current.now, 10);
        }
    }

    #[test]
    fn unwinding_never_publishes_or_implicitly_commits_on_drop() {
        for failure_in_publication in [false, true] {
            let mut current = Database {
                now: 10,
                ..Default::default()
            };
            let result = catch_unwind(AssertUnwindSafe(|| {
                evaluate_and_publish(
                    &mut current,
                    |state| -> Result<(), EngineError> {
                        state.now = 20;
                        assert!(failure_in_publication, "controlled evaluation panic");
                        Ok(())
                    },
                    |_| -> Result<(), EngineError> {
                        panic!("controlled publication panic");
                    },
                )
            }));
            assert!(result.is_err());
            assert_eq!(current.now, 10);
        }
    }

    #[test]
    fn panicking_update_leaves_original_state_and_poisoned_store_fails_closed() {
        let (store, admin) = InMemoryStore::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            store.update(|state| -> Result<(), EngineError> {
                state.now = 20;
                panic!("controlled state update panic");
            })
        }));
        assert!(result.is_err());
        assert_eq!(
            store.read(|state| Ok(state.now)),
            Err(EngineError::StoreUnavailable)
        );
        assert_eq!(admin.advance_time(1), Err(EngineError::StoreUnavailable));
        // Only this private inspection bypasses poisoning to establish that the
        // previous stage survived. Production has no poison-recovery fallback.
        let poisoned = store.inner.lock().err().expect("store must be poisoned");
        assert_eq!(poisoned.into_inner().now, 0);
    }
}
