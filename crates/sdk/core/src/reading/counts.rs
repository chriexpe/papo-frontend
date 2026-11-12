//! Cache-based ordered unread counts; unknown prefixes/gaps remain lower bounds, never inferred completeness.

use super::{ReadPoint, ReadState, ReadingError, UnreadCount};
use crate::HistorySnapshot;

pub(super) fn update(
    state: &mut ReadState,
    snapshot: &HistorySnapshot,
    server_head: Option<ReadPoint>,
) -> Result<(), ReadingError> {
    let point = state.point();
    let (mut low, mut high) = (0, snapshot.messages.len());
    while low < high {
        let middle = low + (high - low) / 2;
        if Some(ReadPoint::message(&snapshot.messages[middle])?) <= point {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let mut count =
        u64::try_from(snapshot.messages.len() - low).map_err(|_| ReadingError::InvalidPoint)?;
    let first = snapshot
        .messages
        .get(low)
        .map(|message| ReadPoint::message(message))
        .transpose()?
        .map(|point| point.message_id);
    let cached_head = snapshot
        .messages
        .last()
        .map(|message| ReadPoint::message(message))
        .transpose()?;
    // Metadata names a real additional message, but never proves the unknown middle complete.
    if server_head.is_some_and(|head| Some(head) > point && Some(head) > cached_head) {
        count += 1;
    }
    let exact = complete_after(snapshot, point)?
        && server_head.is_none_or(|head| Some(head) <= cached_head.max(point));
    state.unread_count = if exact {
        UnreadCount::Exact(count)
    } else {
        UnreadCount::AtLeast(count)
    };
    state.first_unread = first;
    Ok(())
}

pub(super) fn unresolved(state: &mut ReadState) {
    // An unknown remote mark may be newer than every cached row; no positive lower
    // bound or first-unread claim is justified until its ordering key is resolved.
    state.unread_count = UnreadCount::AtLeast(0);
    state.first_unread = None;
}

fn complete_after(
    snapshot: &HistorySnapshot,
    point: Option<ReadPoint>,
) -> Result<bool, ReadingError> {
    if snapshot.empty_complete {
        return Ok(true);
    }
    let Some(last) = snapshot.chunks.last() else {
        return Ok(false);
    };
    if !last.newest_complete || last.live_generation != Some(snapshot.generation) {
        return Ok(false);
    }
    // The run containing an already-deleted anchor still proves its successor ordering.
    // A cursor older than cached history is explicitly unknown, even if all retained rows fit.
    let first = ReadPoint::parse(Some(&last.first.created_at), Some(&last.first.id))?;
    let end = ReadPoint::parse(Some(&last.last.created_at), Some(&last.last.id))?;
    Ok(match point {
        Some(point) => point >= first && point <= end,
        None => snapshot.chunks.len() == 1 && last.oldest_complete,
    })
}
