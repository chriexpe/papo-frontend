//! Cache-based ordered unread counts; unknown prefixes/gaps remain lower bounds, never inferred completeness.

use super::{ReadPoint, ReadState, ReadingError, UnreadCount};
use crate::{ChronologicalKey, HistorySnapshot};

pub(super) fn update(
    state: &mut ReadState,
    snapshot: &HistorySnapshot,
    server_head: Option<ReadPoint>,
) -> Result<(), ReadingError> {
    let point = state
        .point()
        .map(|point| Ok::<_, ReadingError>(snapshot.resolve(point)?.key))
        .transpose()?;
    let range = snapshot.range(point, None)?;
    let mut count = u64::try_from(range.indices.len()).map_err(|_| ReadingError::InvalidPoint)?;
    let first = range
        .indices
        .clone()
        .next()
        .map(|idx| ChronologicalKey::from_message(&snapshot.messages[idx]).map(|k| k.message_id))
        .transpose()?;
    let cached_head = snapshot
        .messages
        .last()
        .map(|message| ChronologicalKey::from_message(message))
        .transpose()?;
    // Metadata names a real additional message, but never proves the unknown middle complete.
    if server_head.is_some_and(|head| Some(head) > point && Some(head) > cached_head) {
        count += 1;
    }
    let exact =
        range.complete && server_head.is_none_or(|head| Some(head) <= cached_head.max(point));
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
