//! The rows' stable keys, from the SDK items' identities: a message by its
//! ID, a date separator by the message after it, a gap by its cursor, a
//! loading row by its edge. Never an index.

use papo_sdk::HistoryOrder;
use papo_sdk_ui::{TimelineDirection, TimelineItem};

/// The row key of `item`, unique within its timeline.
pub(crate) fn row_key(item: &TimelineItem) -> String {
    match item {
        TimelineItem::Message { id, .. } => message_key(*id),
        TimelineItem::LocalEcho { id, .. } => echo_key(*id),
        TimelineItem::DateSeparator { before, .. } => format!("d{before}"),
        TimelineItem::UnreadMarker { .. } => "unread".to_owned(),
        TimelineItem::Gap(gap) => {
            let edge = if gap.order == HistoryOrder::Asc {
                'n'
            } else {
                'o'
            };
            format!("g{edge}{}", gap.cursor.id)
        }
        TimelineItem::Loading(TimelineDirection::Older) => "loading-older".to_owned(),
        TimelineItem::Loading(TimelineDirection::Newer) => "loading-newer".to_owned(),
        // A kind this client does not draw yet.
        _ => "other".to_owned(),
    }
}

/// The row key of message `id`.
pub(crate) fn message_key(id: papo_sdk::Uuid) -> String {
    format!("m{id}")
}

/// The row key of the local echo with local identity `id`.
pub(crate) fn echo_key(id: papo_sdk::Uuid) -> String {
    format!("e{id}")
}

/// The key, inside message `id`'s row, of the message itself: the row less
/// the distance above it, which takes the keyboard. Its own (not one key
/// for every message) so focus remembered by key comes back to it.
pub(crate) fn message_part_key(id: papo_sdk::Uuid) -> String {
    format!("message-{id}")
}

/// The key path `Tx::scroll_key_to` lands message `id` by: the message
/// itself, which the anchor's offset is measured to.
pub(crate) fn anchor_key(id: papo_sdk::Uuid) -> String {
    format!("{}/{}", message_key(id), message_part_key(id))
}

/// The element key of message `id`'s author name, which their card opens
/// beside.
pub(crate) fn author_key(id: papo_sdk::Uuid) -> String {
    format!("author-{id}")
}

/// The element key of message `id`'s author avatar, which their card opens
/// beside too.
pub(crate) fn avatar_key(id: papo_sdk::Uuid) -> String {
    format!("avatar-{id}")
}
