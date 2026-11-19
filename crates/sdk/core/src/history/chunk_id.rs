//! Persistent identity of a proven run, allocated from a counter scoped to one history set.
//! Coverage (boundaries, anchors, joins) stays in `coverage` and `chunk_join`; the layout
//! guard that rejects repeated identities before rows are built stays in `persisted_layout`.
//!
//! An identity never derives from a message, so moving a run's boundaries (retention, a later
//! page, a live arrival) never changes it and two disjoint runs can never share it. It says
//! nothing about connectivity: only anchors and boundaries, which are message cursors, prove
//! that two runs meet.

use serde::{Deserialize, Serialize};

use super::HistoryError;

/// Domain tag of a chunk row key, so a stored key is never mistaken for a message UUID.
const ROW_KEY_TAG: [u8; 8] = *b"papochnk";

/// One run's identity. Ordered by allocation: a smaller value was issued earlier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct ChunkId(u64);

impl ChunkId {
    /// The 16-byte cache row key: the domain tag followed by the big-endian counter value.
    pub fn row_key(self) -> [u8; 16] {
        let mut key = [0; 16];
        key[..8].copy_from_slice(&ROW_KEY_TAG);
        key[8..].copy_from_slice(&self.0.to_be_bytes());
        key
    }
}

/// The allocator of one history set. `high_water` is the last value issued (0 before the
/// first); it is persisted with the layout metadata in the same commit as the rows that use
/// it, so a value seen on disk is never issued again, including after reload. A value issued
/// in memory but never committed may be issued again after a reset; no row ever held it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ChunkIds {
    high_water: u64,
}

impl ChunkIds {
    /// Resumes allocation after the persisted high-water mark.
    pub fn resume(high_water: u64) -> Self {
        Self { high_water }
    }

    /// The last value issued; the value to persist.
    pub fn high_water(self) -> u64 {
        self.high_water
    }

    /// Fails when no further value can be issued. A model mutation issues at most one value,
    /// so checking this before it changes anything means it never fails halfway.
    pub fn ensure_available(self) -> Result<(), HistoryError> {
        self.high_water
            .checked_add(1)
            .map(drop)
            .ok_or(HistoryError::Layout)
    }

    /// Issues a value never issued before in this set.
    pub fn allocate(&mut self) -> Result<ChunkId, HistoryError> {
        self.high_water = self.high_water.checked_add(1).ok_or(HistoryError::Layout)?;
        Ok(ChunkId(self.high_water))
    }

    /// Whether `id` was issued by this allocator: a stored identity above the high-water mark
    /// would be issued again later.
    pub fn issued(self, id: ChunkId) -> bool {
        (1..=self.high_water).contains(&id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_is_monotonic_resumes_after_the_high_water_mark_and_is_checked() {
        let mut ids = ChunkIds::default();
        let first = ids.allocate().unwrap();
        let second = ids.allocate().unwrap();
        assert!(first < second);
        assert_eq!(ids.high_water(), 2);
        assert!(ids.issued(first) && ids.issued(second));

        let mut resumed = ChunkIds::resume(ids.high_water());
        let third = resumed.allocate().unwrap();
        assert!(third > second);
        assert!(!ids.issued(third));

        let mut exhausted = ChunkIds::resume(u64::MAX);
        assert!(matches!(exhausted.allocate(), Err(HistoryError::Layout)));
        assert_eq!(exhausted.high_water(), u64::MAX);
    }

    #[test]
    fn row_keys_are_distinct_per_value_and_tagged() {
        let mut ids = ChunkIds::default();
        let first = ids.allocate().unwrap().row_key();
        let second = ids.allocate().unwrap().row_key();
        assert_ne!(first, second);
        assert_eq!(&first[..8], b"papochnk");
    }
}
