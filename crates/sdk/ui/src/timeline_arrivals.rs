//! Behind-head live arrival counting and two-stop down-arrow target projection.
//! Separate catch-up boundary state and overlay streams live in `timeline_reading`.

use papo_sdk::{CatchUpState, HistoryCursor, HistorySnapshot, ReadPoint, ReadState, Uuid};

use super::TimelineError;

/// Newly arrived foreign messages behind a timeline viewport and their two down-arrow targets.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TimelineArrivals {
    /// Count of unseen foreign messages newer than the channel/window seen frontier.
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

fn is_live_complete(raw: &HistorySnapshot) -> bool {
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
        .and_then(|msg| msg.cursor())
        .as_ref()
        .map(read_point_from_cursor)
        .transpose()
}

pub(super) fn compute_arrivals(
    self_user: Uuid,
    raw: &HistorySnapshot,
    read_state: Option<&ReadState>,
    catch_state: Option<&CatchUpState>,
    initial_tail: Option<ReadPoint>,
) -> Result<TimelineArrivals, TimelineError> {
    if !is_live_complete(raw) || raw.messages.is_empty() {
        return Ok(TimelineArrivals::default());
    }
    let read_pt = read_state.and_then(|s| s.local_point.max(s.uploaded_point));
    let seen_pt = catch_state.and_then(|s| s.seen_to);
    let cutoff = read_pt
        .max(seen_pt)
        .max(initial_tail)
        .map(|point| resolve_in_snapshot(raw, point));

    let mut count = 0u64;
    let mut oldest_unseen = None;
    for message in &raw.messages {
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

fn message_point(message: &papo_sdk::Message) -> Result<Option<ReadPoint>, TimelineError> {
    let (Some(created), Some(id)) = (message.created_at.as_deref(), message.id.as_deref()) else {
        return Ok(None);
    };
    let at =
        chrono::DateTime::parse_from_rfc3339(created).map_err(|_| TimelineError::InvalidCursor)?;
    if at.timestamp_subsec_nanos() % 1_000 != 0 {
        return Err(TimelineError::InvalidCursor);
    }
    let id = id.parse().map_err(|_| TimelineError::InvalidCursor)?;
    Ok(Some(ReadPoint {
        created_at_us: at.timestamp_micros(),
        message_id: id,
    }))
}

fn resolve_in_snapshot(raw: &HistorySnapshot, point: ReadPoint) -> ReadPoint {
    let target = point.message_id.to_string();
    for msg in &raw.messages {
        if msg.id.as_deref() == Some(target.as_str())
            && let Ok(Some(resolved)) = message_point(msg)
        {
            return resolved;
        }
    }
    point
}

pub(super) fn read_point_from_cursor(cursor: &HistoryCursor) -> Result<ReadPoint, TimelineError> {
    let key = crate::timeline_window::key(cursor)?;
    Ok(ReadPoint {
        created_at_us: key.0,
        message_id: key.1,
    })
}
