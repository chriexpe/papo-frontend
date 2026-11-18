//! Behind-head live arrival counting and two-stop down-arrow target projection.
//! Separate catch-up boundary state and overlay streams live in `timeline_reading`.

use papo_sdk::{ChronologicalKey, HistoryCursor, HistorySnapshot, Uuid, is_foreign_message};

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

pub(super) fn is_live_complete(raw: &HistorySnapshot) -> Result<bool, TimelineError> {
    let after = raw
        .messages
        .last()
        .map(|msg| ChronologicalKey::from_message(msg.as_ref()))
        .transpose()?;
    Ok(raw.range(after, None)?.complete)
}

pub(super) fn initial_live_tail(
    raw: &HistorySnapshot,
) -> Result<Option<ChronologicalKey>, TimelineError> {
    if !is_live_complete(raw)? {
        return Ok(None);
    }
    Ok(raw
        .messages
        .last()
        .map(|msg| ChronologicalKey::from_message(msg.as_ref()))
        .transpose()?)
}

pub(super) fn tail_before_live_arrivals(
    raw: &HistorySnapshot,
) -> Result<Option<ChronologicalKey>, TimelineError> {
    if !is_live_complete(raw)? {
        return Ok(None);
    }
    if let Some(cursor) = &raw.live_arrival_start {
        let floor = ChronologicalKey::from_cursor(cursor)?;
        return tail_before_point(raw, floor);
    }
    initial_live_tail(raw)
}

pub(super) fn tail_before_point(
    raw: &HistorySnapshot,
    floor: ChronologicalKey,
) -> Result<Option<ChronologicalKey>, TimelineError> {
    let idx = raw.range(None, Some(floor))?.indices.end;
    if idx == 0 {
        return Ok(None);
    }
    Ok(Some(ChronologicalKey::from_message(
        raw.messages[idx - 1].as_ref(),
    )?))
}

pub(super) fn compute_arrivals(
    self_user: Uuid,
    raw: &HistorySnapshot,
    window_seen_to: Option<ChronologicalKey>,
) -> Result<TimelineArrivals, TimelineError> {
    if !is_live_complete(raw)? || raw.messages.is_empty() {
        return Ok(TimelineArrivals::default());
    }
    let cutoff = window_seen_to
        .map(|point| raw.resolve(point).map(|resolved| resolved.key))
        .transpose()?;
    let indices = raw.range(cutoff, None)?.indices;
    if indices.is_empty() {
        return Ok(TimelineArrivals::default());
    }

    let mut count = 0u64;
    let mut oldest_unseen = None;
    for idx in indices {
        let message = &raw.messages[idx];
        if !is_foreign_message(message, self_user) {
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
