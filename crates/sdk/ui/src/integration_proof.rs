//! Deterministic, test-only observable list changes for the native integration proofs.
//! This module owns no network, account, UI widget or production service.

/// A stable-identity row carried across both native binding proofs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofRow {
    /// Stable identity, independent of its presentation text.
    pub id: String,
    /// Text changed independently of the row identity.
    pub title: String,
}

/// A minimal vector-diff vocabulary; production timeline diffs arrive in S4.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VectorDiff {
    /// Insert one item at an index.
    Insert {
        /// Zero-based insertion index.
        index: usize,
        /// Inserted item.
        value: ProofRow,
    },
    /// Replace one existing item without shifting neighboring items.
    Set {
        /// Zero-based index of the replaced item.
        index: usize,
        /// Replacement item with the same identity.
        value: ProofRow,
    },
    /// Append one item.
    PushBack {
        /// Item appended at the end.
        value: ProofRow,
    },
    /// Replace the complete window, including its version boundary.
    Reset {
        /// Complete replacement snapshot.
        values: Vec<ProofRow>,
    },
}

fn row(id: &str, title: &str) -> ProofRow {
    ProofRow {
        id: id.to_owned(),
        title: title.to_owned(),
    }
}

/// Batches used to verify ordered callbacks and main-thread list mutation.
///
/// The first batch is a snapshot, the second is incremental, and the third
/// simulates a bounded subscriber falling behind and receiving a reset.
#[must_use]
pub fn batches() -> Vec<Vec<VectorDiff>> {
    vec![
        vec![VectorDiff::Reset {
            values: vec![row("a", "Alpha"), row("c", "Charlie")],
        }],
        vec![
            VectorDiff::PushBack {
                value: row("d", "Delta"),
            },
            VectorDiff::Insert {
                index: 1,
                value: row("b", "Bravo"),
            },
            VectorDiff::Set {
                index: 0,
                value: row("a", "Alpha edited"),
            },
        ],
        vec![VectorDiff::Reset {
            values: vec![row("z", "After reset")],
        }],
    ]
}

#[cfg(test)]
mod tests {
    use super::{VectorDiff, batches};

    #[test]
    fn fixture_covers_snapshot_incremental_updates_and_reset() {
        let batches = batches();
        assert_eq!(batches.len(), 3);
        assert!(matches!(batches[0].as_slice(), [VectorDiff::Reset { .. }]));
        assert!(matches!(batches[1][0], VectorDiff::PushBack { .. }));
        assert!(matches!(batches[1][1], VectorDiff::Insert { .. }));
        assert!(matches!(batches[1][2], VectorDiff::Set { .. }));
        assert!(matches!(batches[2].as_slice(), [VectorDiff::Reset { .. }]));
    }
}
