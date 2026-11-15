//! Behind-head live arrival counting and two-stop down-arrow target projection.
//! Separate catch-up boundary state and overlay streams live in `timeline_reading`.

use std::sync::Arc;

use papo_sdk::{HistoryCursor, HistorySnapshot, Message, ReadPoint, Uuid};

use super::TimelineError;

/// Newly arrived foreign messages behind a timeline viewport and their two down-arrow targets.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TimelineArrivals {
    /// Count of unseen foreign messages newer than this window's exposed frontier.
    pub count: u64,
    /// First down-arrow stop: oldest unseen foreign arrival.
    pub oldest_unseen: Option<HistoryCursor>,
    /// Second down-arrow stop: newest live tail message on the channel.
    pub live_head: Option<HistoryCursor>,
}

impl std::fmt::Debug for TimelineArrivals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimelineArrivals")
            .field("count", &self.count)
            .field("has_oldest_unseen", &self.oldest_unseen.is_some())
            .field("has_live_head", &self.live_head.is_some())
            .finish()
    }
}

pub(super) fn is_live_complete(raw: &HistorySnapshot) -> bool {
    raw.empty_complete
        || raw
            .chunks
            .last()
            .is_some_and(|c| c.newest_complete && c.live_generation == Some(raw.generation))
}

pub(super) fn initial_live_tail(raw: &HistorySnapshot) -> Result<Option<ReadPoint>, TimelineError> {
    if !is_live_complete(raw) {
        return Ok(None);
    }
    raw.messages
        .last()
        .map(|msg| message_point(msg.as_ref()))
        .transpose()
        .map(Option::flatten)
}

pub(super) fn tail_before_live_arrivals(
    raw: &HistorySnapshot,
) -> Result<Option<ReadPoint>, TimelineError> {
    if !is_live_complete(raw) {
        return Ok(None);
    }
    if let Some(cursor) = &raw.live_arrival_start {
        let floor = cursor_point(cursor)?;
        return tail_before_point(raw, floor);
    }
    initial_live_tail(raw)
}

pub(super) fn tail_before_point(
    raw: &HistorySnapshot,
    floor: ReadPoint,
) -> Result<Option<ReadPoint>, TimelineError> {
    let idx = lower_bound_ge(&raw.messages, floor)?;
    if idx == 0 {
        return Ok(None);
    }
    message_point(raw.messages[idx - 1].as_ref())
}

pub(super) fn compute_arrivals(
    self_user: Uuid,
    raw: &HistorySnapshot,
    window_seen_to: Option<ReadPoint>,
) -> Result<TimelineArrivals, TimelineError> {
    if !is_live_complete(raw) || raw.messages.is_empty() {
        return Ok(TimelineArrivals::default());
    }
    let cutoff = window_seen_to
        .map(|point| resolve_in_snapshot(raw, point))
        .transpose()?;
    if let Some(cutoff) = cutoff
        && let Some(last) = raw.messages.last()
        && message_point(last)?.is_some_and(|last_pt| last_pt <= cutoff)
    {
        return Ok(TimelineArrivals::default());
    }
    let start_idx = match cutoff {
        Some(cutoff) => first_after(&raw.messages, cutoff)?,
        None => 0,
    };

    let mut count = 0u64;
    let mut oldest_unseen = None;
    for idx in start_idx..raw.messages.len() {
        let message = &raw.messages[idx];
        let Some(point) = message_point(message)? else {
            continue;
        };
        if cutoff.is_some_and(|c| point <= c) {
            continue;
        }
        let foreign = message
            .author_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            != Some(self_user);
        if !foreign {
            continue;
        }
        count += 1;
        if oldest_unseen.is_none() {
            oldest_unseen = message.cursor();
        }
    }
    let live_head = (count > 0)
        .then(|| raw.messages.last().and_then(|msg| msg.cursor()))
        .flatten();
    Ok(TimelineArrivals {
        count,
        oldest_unseen,
        live_head,
    })
}

fn parse_point(created: &str, id: &str) -> Result<ReadPoint, TimelineError> {
    let at =
        chrono::DateTime::parse_from_rfc3339(created).map_err(|_| TimelineError::InvalidCursor)?;
    if at.timestamp_subsec_nanos() % 1_000 != 0 {
        return Err(TimelineError::InvalidCursor);
    }
    let id = id.parse().map_err(|_| TimelineError::InvalidCursor)?;
    Ok(ReadPoint {
        created_at_us: at.timestamp_micros(),
        message_id: id,
    })
}

pub(super) fn cursor_point(cursor: &HistoryCursor) -> Result<ReadPoint, TimelineError> {
    parse_point(&cursor.created_at, &cursor.id)
}

pub(super) fn message_point(message: &Message) -> Result<Option<ReadPoint>, TimelineError> {
    let (Some(created), Some(id)) = (message.created_at.as_deref(), message.id.as_deref()) else {
        return Ok(None);
    };
    parse_point(created, id).map(Some)
}

pub(super) fn resolve_in_snapshot(
    raw: &HistorySnapshot,
    point: ReadPoint,
) -> Result<ReadPoint, TimelineError> {
    let idx = lower_bound_ge(&raw.messages, point)?;
    if let Some(msg) = raw.messages.get(idx)
        && let Some(resolved) = message_point(msg)?
        && resolved == point
    {
        return Ok(resolved);
    }
    if !raw.tombstones.contains(&point.message_id) && point.created_at_us % 1_000 == 0 {
        let low_key = ReadPoint {
            created_at_us: point.created_at_us.saturating_sub(1_000),
            message_id: Uuid::nil(),
        };
        let high_us = point.created_at_us.saturating_add(1_000);
        let mut cursor = lower_bound_ge(&raw.messages, low_key)?;
        while let Some(msg) = raw.messages.get(cursor) {
            if let Some(candidate) = message_point(msg)? {
                if candidate.created_at_us > high_us {
                    break;
                }
                if candidate.message_id == point.message_id {
                    return Ok(candidate);
                }
            }
            cursor += 1;
        }
    }
    Ok(point)
}

pub(super) fn find_message_cursor(
    raw: &HistorySnapshot,
    point: ReadPoint,
) -> Result<Option<HistoryCursor>, TimelineError> {
    let resolved = resolve_in_snapshot(raw, point)?;
    let idx = lower_bound_ge(&raw.messages, resolved)?;
    let Some(msg) = raw.messages.get(idx) else {
        return Ok(None);
    };
    if message_point(msg)? == Some(resolved) {
        return Ok(msg.cursor());
    }
    Ok(None)
}

fn partition_point_raw(
    messages: &imbl::Vector<Arc<Message>>,
    mut pred: impl FnMut(ReadPoint) -> bool,
) -> Result<usize, TimelineError> {
    let mut low = 0;
    let mut high = messages.len();
    while low < high {
        let mid = low + (high - low) / 2;
        let keep_left = match message_point(&messages[mid])? {
            Some(point) => pred(point),
            None => true,
        };
        if keep_left {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    Ok(low)
}

fn lower_bound_ge(
    messages: &imbl::Vector<Arc<Message>>,
    target: ReadPoint,
) -> Result<usize, TimelineError> {
    partition_point_raw(messages, |point| point < target)
}

fn first_after(
    messages: &imbl::Vector<Arc<Message>>,
    target: ReadPoint,
) -> Result<usize, TimelineError> {
    partition_point_raw(messages, |point| point <= target)
}
