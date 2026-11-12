//! Immutable, content-addressed patches with exact before-images.
use crate::ids::{Canonical, Digest, ElementId, PatchId, SnapshotId};
use crate::snapshot::State;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub expected: Option<i64>,
    pub value: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatchError {
    #[error("patch must contain at least one change")]
    Empty,
    #[error("patch contains a no-op for {0}")]
    NoOp(ElementId),
    #[error("patch before-image mismatch for {element}: expected {expected:?}, found {actual:?}")]
    BeforeImage {
        element: ElementId,
        expected: Option<i64>,
        actual: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    id: PatchId,
    base_snapshot: SnapshotId,
    changes: BTreeMap<ElementId, Change>,
}
impl Patch {
    pub fn new(
        base_snapshot: SnapshotId,
        changes: BTreeMap<ElementId, Change>,
    ) -> Result<Self, PatchError> {
        if changes.is_empty() {
            return Err(PatchError::Empty);
        }
        for (element, change) in &changes {
            if change.expected == change.value {
                return Err(PatchError::NoOp(element.clone()));
            }
        }
        Ok(Self {
            id: PatchId(Digest::of(
                "celetixo/patch/v1",
                &patch_bytes(base_snapshot, &changes),
            )),
            base_snapshot,
            changes,
        })
    }
    pub fn id(&self) -> PatchId {
        self.id
    }
    pub fn base_snapshot(&self) -> SnapshotId {
        self.base_snapshot
    }
    pub fn changes(&self) -> &BTreeMap<ElementId, Change> {
        &self.changes
    }
    /// Existing v1 identity preimage, not a source mutation or authorization API.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        patch_bytes(self.base_snapshot, &self.changes)
    }
    /// Applies atomically to a copy. A mismatch never partially mutates the caller's state.
    pub fn apply(&self, state: &State) -> Result<State, PatchError> {
        for (element, change) in &self.changes {
            let actual = state.get(element).copied();
            if actual != change.expected {
                return Err(PatchError::BeforeImage {
                    element: element.clone(),
                    expected: change.expected,
                    actual,
                });
            }
        }
        let mut candidate = state.clone();
        for (element, change) in &self.changes {
            match change.value {
                Some(value) => {
                    candidate.insert(element.clone(), value);
                }
                None => {
                    candidate.remove(element);
                }
            }
        }
        Ok(candidate)
    }
}

fn patch_bytes(base_snapshot: SnapshotId, changes: &BTreeMap<ElementId, Change>) -> Vec<u8> {
    let mut out = Canonical::default();
    out.digest(base_snapshot.0);
    out.u64(changes.len() as u64);
    for (element, change) in changes {
        out.str(element.as_str());
        for value in [change.expected, change.value] {
            match value {
                None => out.str("absent"),
                Some(value) => {
                    out.str("present");
                    out.i64(value);
                }
            }
        }
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::Digest;
    fn e(s: &str) -> ElementId {
        ElementId::new(s).unwrap()
    }
    fn base(s: &str) -> SnapshotId {
        SnapshotId(Digest::of("test/snapshot", s.as_bytes()))
    }
    #[test]
    fn digest_binds_base_before_images_keys_and_values() {
        let changes = [(
            e("a"),
            Change {
                expected: Some(1),
                value: Some(2),
            },
        )]
        .into();
        let patch = Patch::new(base("one"), changes).unwrap();
        assert_ne!(
            patch.id(),
            Patch::new(base("two"), patch.changes().clone())
                .unwrap()
                .id()
        );
        assert_ne!(
            patch.id(),
            Patch::new(
                base("one"),
                [(
                    e("a"),
                    Change {
                        expected: Some(0),
                        value: Some(2)
                    }
                )]
                .into()
            )
            .unwrap()
            .id()
        );
        assert_ne!(
            patch.id(),
            Patch::new(
                base("one"),
                [(
                    e("b"),
                    Change {
                        expected: Some(1),
                        value: Some(2)
                    }
                )]
                .into()
            )
            .unwrap()
            .id()
        );
    }
    #[test]
    fn add_delete_update_and_wrong_before_images() {
        let original: State = [(e("delete"), 4), (e("update"), 1)].into();
        let patch = Patch::new(
            base("one"),
            [
                (
                    e("add"),
                    Change {
                        expected: None,
                        value: Some(9),
                    },
                ),
                (
                    e("delete"),
                    Change {
                        expected: Some(4),
                        value: None,
                    },
                ),
                (
                    e("update"),
                    Change {
                        expected: Some(1),
                        value: Some(2),
                    },
                ),
            ]
            .into(),
        )
        .unwrap();
        assert_eq!(
            patch.apply(&original).unwrap(),
            [(e("add"), 9), (e("update"), 2)].into()
        );
        let changed = [(e("delete"), 4), (e("update"), 8)].into();
        assert!(matches!(
            patch.apply(&changed),
            Err(PatchError::BeforeImage { .. })
        ));
        assert_eq!(original[&e("update")], 1);
    }
    #[test]
    fn empty_and_noop_patches_are_rejected() {
        assert_eq!(
            Patch::new(base("one"), BTreeMap::new()),
            Err(PatchError::Empty)
        );
        for value in [None, Some(7)] {
            assert!(matches!(
                Patch::new(
                    base("one"),
                    [(
                        e("a"),
                        Change {
                            expected: value,
                            value
                        }
                    )]
                    .into()
                ),
                Err(PatchError::NoOp(_))
            ));
        }
    }
}
