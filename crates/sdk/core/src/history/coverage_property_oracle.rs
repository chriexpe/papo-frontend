//! Dual oracle tracking server ground truth, accepted client evidence, and gap consistency.
//! Randomized operation driving and proptest assertions live in `coverage_property_tests`.

use super::{
    HistoryOrder, HistorySnapshot,
    coverage_tests::{MAX_UNIVERSE_ID, key_for},
};
use crate::{ChronologicalKey, Message, Uuid};
use std::collections::{BTreeMap, BTreeSet};

// Discretized evidence oracle:
// pos 2*id = key k(id) (1..=16), pos 2*id-1 = gap strictly below k(id),
// pos 1 = (-inf, k1), pos 33 = (k16, +inf).
pub(super) const ORACLE_SLOTS: usize = (MAX_UNIVERSE_ID as usize) * 2 + 2;

#[derive(Clone)]
pub(super) struct Oracle {
    /// Server ground truth: live messages (`id` -> `Message`).
    pub server_live: BTreeMap<u128, Message>,
    /// Deleted message IDs; real server IDs are unique and never recreated after deletion.
    pub server_deleted: BTreeSet<u128>,
    /// Historical intervals proven by accepted REST pages or live arrivals before `live_tail_from`.
    pub historical: [bool; ORACLE_SLOTS],
    /// Position index >= which coverage depends on `live_gen`'s live tail (`empty_after` / `newest_complete`).
    pub live_tail_from: Option<usize>,
    pub live_gen: u64,
    pub current_gen: u64,
}

impl Default for Oracle {
    fn default() -> Self {
        Self {
            server_live: BTreeMap::new(),
            server_deleted: BTreeSet::new(),
            historical: [false; ORACLE_SLOTS],
            live_tail_from: None,
            live_gen: 0,
            current_gen: 1,
        }
    }
}

impl Oracle {
    pub fn slot_key(id: u128) -> usize {
        usize::try_from(id).unwrap() * 2
    }

    pub fn apply_historical(&mut self, from_pos: usize, to_pos_excl: usize) {
        for slot in &mut self.historical[from_pos..to_pos_excl] {
            *slot = true;
        }
    }

    pub fn set_live_tail(&mut self, from_pos: usize, gen_id: u64) {
        self.live_tail_from = Some(if self.live_gen == gen_id {
            self.live_tail_from.map_or(from_pos, |f| f.min(from_pos))
        } else {
            from_pos
        });
        self.live_gen = gen_id;
    }

    pub fn is_slot_covered(&self, slot: usize) -> bool {
        self.historical[slot]
            || (self.live_gen == self.current_gen
                && self
                    .live_tail_from
                    .is_some_and(|from| slot >= from && slot < ORACLE_SLOTS - 1))
    }

    pub fn is_range_complete(&self, after: Option<u128>, before: Option<u128>) -> bool {
        let start = after.map_or(1, |id| Self::slot_key(id) + 1);
        let end = before.map_or(ORACLE_SLOTS - 1, Self::slot_key);
        if start >= end {
            return true;
        }
        (start..end).all(|slot| self.is_slot_covered(slot))
    }

    pub fn clear_coverage(&mut self) {
        self.historical = [false; ORACLE_SLOTS];
        self.live_tail_from = None;
        self.live_gen = 0;
    }

    pub fn clear_live(&mut self, gen_id: u64) {
        self.current_gen = gen_id;
        if self.live_gen != gen_id {
            self.live_tail_from = None;
        }
    }
}

#[derive(Debug)]
pub(super) struct OracleViolation(pub String);

impl std::fmt::Display for OracleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub(super) fn check_snapshot_against_oracles(
    snapshot: &HistorySnapshot,
    oracle: &Oracle,
) -> Result<(), OracleViolation> {
    let loaded_ids: Vec<u128> = snapshot
        .messages
        .iter()
        .map(|m| m.id.as_deref().unwrap().parse::<Uuid>().unwrap().as_u128())
        .collect();
    check_ranges_and_soundness(snapshot, oracle, &loaded_ids)?;
    check_gaps_against_chunks(snapshot, &loaded_ids)
}

fn check_ranges_and_soundness(
    snapshot: &HistorySnapshot,
    oracle: &Oracle,
    loaded_ids: &[u128],
) -> Result<(), OracleViolation> {
    let bounds: Vec<Option<u128>> = std::iter::once(None)
        .chain((1..=MAX_UNIVERSE_ID).map(Some))
        .chain(std::iter::once(None))
        .collect();
    for (i, &after) in bounds[..bounds.len() - 1].iter().enumerate() {
        for &before in &bounds[i + 1..] {
            if let (Some(a), Some(b)) = (after, before)
                && a >= b
            {
                continue;
            }
            let hr = snapshot
                .range(after.map(key_for), before.map(key_for))
                .map_err(|e| OracleViolation(format!("range error: {e:?}")))?;
            let slice_ids: Vec<u128> = hr
                .indices
                .clone()
                .map(|idx| {
                    snapshot.messages[idx]
                        .id
                        .as_deref()
                        .unwrap()
                        .parse::<Uuid>()
                        .unwrap()
                        .as_u128()
                })
                .collect();
            let expected_slice: Vec<u128> = loaded_ids
                .iter()
                .copied()
                .filter(|&id| after.is_none_or(|a| id > a) && before.is_none_or(|b| id < b))
                .collect();
            if slice_ids != expected_slice {
                return Err(OracleViolation(format!(
                    "range slice mismatch for ({after:?}, {before:?}): got {slice_ids:?}, expected {expected_slice:?}"
                )));
            }
            if hr.complete {
                if !oracle.is_range_complete(after, before) {
                    return Err(OracleViolation(format!(
                        "EVIDENCE CONSISTENCY VIOLATION for ({after:?}, {before:?}): snapshot claimed complete without accepted oracle evidence"
                    )));
                }
                let server_slice: Vec<u128> = oracle
                    .server_live
                    .keys()
                    .copied()
                    .filter(|&id| after.is_none_or(|a| id > a) && before.is_none_or(|b| id < b))
                    .collect();
                if slice_ids != server_slice {
                    return Err(OracleViolation(format!(
                        "SOUNDNESS VIOLATION for ({after:?}, {before:?}): client claimed complete with {slice_ids:?} while server has {server_slice:?}"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn check_gaps_against_chunks(
    snapshot: &HistorySnapshot,
    loaded_ids: &[u128],
) -> Result<(), OracleViolation> {
    let gap_points: BTreeSet<(usize, ChronologicalKey)> = snapshot
        .gaps
        .iter()
        .map(|g| {
            let dir = match g.order {
                HistoryOrder::Desc => 0,
                HistoryOrder::Asc => 1,
            };
            (dir, ChronologicalKey::from_cursor(&g.cursor).unwrap())
        })
        .collect();
    check_chunk_edge_gaps(snapshot, &gap_points)?;
    check_loaded_pair_gaps(snapshot, loaded_ids, &gap_points)?;
    check_outer_tail_gaps(snapshot, loaded_ids, &gap_points)
}

fn check_chunk_edge_gaps(
    snapshot: &HistorySnapshot,
    gap_points: &BTreeSet<(usize, ChronologicalKey)>,
) -> Result<(), OracleViolation> {
    for (index, chunk) in snapshot.chunks.iter().enumerate() {
        let first = ChronologicalKey::from_cursor(&chunk.first).unwrap();
        let last = ChronologicalKey::from_cursor(&chunk.last).unwrap();
        let left_empty = index == 0
            && snapshot
                .empty_before
                .as_ref()
                .is_some_and(|b| b.generation.is_some() && first <= b.key);
        let left_bridged = left_empty
            || (index > 0
                && snapshot.chunks[index - 1].generation.is_some() != chunk.generation.is_some()
                && ChronologicalKey::from_cursor(&snapshot.chunks[index - 1].last).is_ok_and(
                    |prev_last| {
                        snapshot
                            .range(Some(prev_last), Some(first))
                            .is_ok_and(|r| r.complete)
                    },
                ));
        let right_empty = index + 1 == snapshot.chunks.len()
            && snapshot
                .empty_after
                .as_ref()
                .is_some_and(|a| a.generation == Some(snapshot.generation) && last >= a.key);
        let right_bridged = right_empty
            || (index + 1 < snapshot.chunks.len()
                && chunk.generation.is_some() != snapshot.chunks[index + 1].generation.is_some()
                && ChronologicalKey::from_cursor(&snapshot.chunks[index + 1].first).is_ok_and(
                    |next_first| {
                        snapshot
                            .range(Some(last), Some(next_first))
                            .is_ok_and(|r| r.complete)
                    },
                ));
        if (!left_bridged && gap_points.contains(&(0, first)) == chunk.oldest_complete)
            || (left_empty && gap_points.contains(&(0, first)))
        {
            return Err(OracleViolation(
                "chunk oldest_complete or empty_before disagrees with Desc gap".into(),
            ));
        }
        if (!right_bridged && gap_points.contains(&(1, last)) == chunk.newest_complete)
            || (right_empty && gap_points.contains(&(1, last)))
        {
            return Err(OracleViolation(
                "chunk newest_complete or empty_after disagrees with Asc gap".into(),
            ));
        }
    }
    Ok(())
}

fn check_loaded_pair_gaps(
    snapshot: &HistorySnapshot,
    loaded_ids: &[u128],
    gap_points: &BTreeSet<(usize, ChronologicalKey)>,
) -> Result<(), OracleViolation> {
    for pair in loaded_ids.windows(2) {
        let a = key_for(pair[0]);
        let b = key_for(pair[1]);
        let hr = snapshot.range(Some(a), Some(b)).unwrap();
        let has_desc_gap = gap_points
            .iter()
            .any(|&(dir, k)| dir == 0 && k > a && k <= b);
        let has_asc_gap = gap_points
            .iter()
            .any(|&(dir, k)| dir == 1 && k >= a && k < b);
        let has_gap_between = has_desc_gap || has_asc_gap;
        if hr.complete && has_gap_between {
            return Err(OracleViolation(format!(
                "gap consistency violation: complete range between {} and {} still has gap",
                pair[0], pair[1]
            )));
        }
        if !hr.complete
            && snapshot.chunks.iter().any(|c| c.generation.is_some())
            && !has_gap_between
        {
            // Only an unvalidated (`generation == None`) disk-restored interval whose seams
            // to adjacent chunks are proven complete may lack an explicit gap inside `(a, b)`.
            let covered_only_with_unvalidated = snapshot.chunks.iter().any(|c| {
                if c.generation.is_some() {
                    return false;
                }
                let first = ChronologicalKey::from_cursor(&c.first).unwrap();
                let last = ChronologicalKey::from_cursor(&c.last).unwrap();
                let left_reaches = first <= a
                    || snapshot
                        .range(Some(a), Some(first))
                        .is_ok_and(|r| r.complete);
                let right_reaches = last >= b
                    || snapshot
                        .range(Some(last), Some(b))
                        .is_ok_and(|r| r.complete);
                first <= b && last >= a && left_reaches && right_reaches
            });
            if !covered_only_with_unvalidated {
                return Err(OracleViolation(format!(
                    "gap consistency violation: incomplete range between {} and {} has no gap",
                    pair[0], pair[1]
                )));
            }
        }
    }
    Ok(())
}

fn check_outer_tail_gaps(
    snapshot: &HistorySnapshot,
    loaded_ids: &[u128],
    gap_points: &BTreeSet<(usize, ChronologicalKey)>,
) -> Result<(), OracleViolation> {
    if let (Some(&first_id), Some(&last_id)) = (loaded_ids.first(), loaded_ids.last()) {
        let first_key = key_for(first_id);
        let last_key = key_for(last_id);
        let head_complete = snapshot.range(None, Some(first_key)).unwrap().complete;
        let has_oldest_gap = gap_points
            .iter()
            .any(|&(dir, k)| dir == 0 && k <= first_key);
        if head_complete && has_oldest_gap {
            return Err(OracleViolation(
                "oldest range complete yet Desc gap published".into(),
            ));
        }
        let first_chunk = &snapshot.chunks[0];
        let first_empty = snapshot
            .empty_before
            .as_ref()
            .is_some_and(|b| b.generation.is_some() && first_key <= b.key);
        if !head_complete
            && ((!first_chunk.oldest_complete && !first_empty) || first_chunk.generation.is_some())
            && !has_oldest_gap
        {
            return Err(OracleViolation(
                "oldest range incomplete in verified or gapped chunk yet no Desc gap published"
                    .into(),
            ));
        }
        let tail_complete = snapshot.range(Some(last_key), None).unwrap().complete;
        let has_newest_gap = gap_points.iter().any(|&(dir, k)| dir == 1 && k >= last_key);
        if tail_complete && has_newest_gap {
            return Err(OracleViolation(
                "newest range complete yet Asc gap published".into(),
            ));
        }
        let last_chunk = snapshot.chunks.last().unwrap();
        let last_empty = snapshot
            .empty_after
            .as_ref()
            .is_some_and(|a| a.generation == Some(snapshot.generation) && last_key >= a.key);
        if !tail_complete
            && ((!last_chunk.newest_complete && !last_empty)
                || (last_chunk.generation.is_some()
                    && last_chunk.live_generation == Some(snapshot.generation)))
            && !has_newest_gap
        {
            return Err(OracleViolation(
                "newest range incomplete without matching Asc gap".into(),
            ));
        }
    }
    Ok(())
}
