//! Cache-based ordered unread counts from history's coverage queries; an unknown range keeps
//! the count a lower bound, never an inferred exact total.

use super::{ReadState, ReadingError, UnreadCount};
use crate::{HistorySnapshot, MessageKey};

pub(super) fn update(
    state: &mut ReadState,
    snapshot: &HistorySnapshot,
    server_head: Option<MessageKey>,
) -> Result<(), ReadingError> {
    let coverage = snapshot.coverage()?;
    let point = state
        .point()
        .map(|point| coverage.resolve(point).map(|resolved| resolved.key))
        .transpose()?;
    let unread = coverage.range(point, None)?;
    let mut count = u64::try_from(unread.messages.len()).map_err(|_| ReadingError::InvalidPoint)?;
    let first = snapshot
        .messages
        .get(unread.messages.start)
        .map(|message| MessageKey::from_message(message))
        .transpose()?
        .map(|point| point.message_id);
    let cached_head = snapshot
        .messages
        .last()
        .map(|message| MessageKey::from_message(message))
        .transpose()?;
    // Metadata names a real additional message, but never proves the unknown middle complete.
    if server_head.is_some_and(|head| Some(head) > point && Some(head) > cached_head) {
        count += 1;
    }
    let exact =
        unread.complete && server_head.is_none_or(|head| Some(head) <= cached_head.max(point));
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
