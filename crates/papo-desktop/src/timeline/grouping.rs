//! Author grouping (`docs/design.md` §6): whether a message continues the
//! group of the row before it, so it hides the repeated header and sits at
//! the within-group distance. Grouping is presentation only: every message
//! stays its own row, and only its neighbour decides how it looks.

use papo_sdk::{Message, MessageKey};
use papo_sdk_ui::TimelineItem;

/// Consecutive messages further apart than this start a new group.
const GROUP_WINDOW_US: i64 = 5 * 60 * 1_000_000;

/// Who wrote a message, as grouping compares authors: the real author.
/// A bridged message's author is its source and external ID, never the
/// bridge account or a display name.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Author<'a> {
    Member(&'a str),
    External { source: &'a str, id: &'a str },
}

/// The message's author, when the payload names one.
pub(crate) fn author(message: &Message) -> Option<Author<'_>> {
    if let Some(external) = &message.external_author {
        return match (external.source.as_deref(), external.id.as_deref()) {
            (Some(source), Some(id)) => Some(Author::External { source, id }),
            // A bridged message without its source or ID matches no one.
            _ => None,
        };
    }
    message.author_id.as_deref().map(Author::Member)
}

/// Whether `message` continues the group of `previous`, the item just
/// before it. Only a message continues a message: a date separator (a new
/// day), the unread marker, a gap or a loading row starts a new group.
pub(crate) fn continues(previous: Option<&TimelineItem>, message: &Message) -> bool {
    let Some(TimelineItem::Message {
        message: previous, ..
    }) = previous
    else {
        return false;
    };
    let same_author = match (author(previous), author(message)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    let (Ok(before), Ok(after)) = (
        MessageKey::from_message(previous),
        MessageKey::from_message(message),
    ) else {
        return false;
    };
    let apart_us = after.created_at_us - before.created_at_us;
    same_author && (0..=GROUP_WINDOW_US).contains(&apart_us)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use papo_sdk::{Message, Uuid};
    use papo_sdk_ui::TimelineItem;
    use papo_sdk_ui::message_content::MessageContent;

    use super::continues;

    fn message(n: u128, author: &str, at: &str) -> Message {
        Message {
            id: Some(Uuid::from_u128(n).to_string()),
            author_id: Some(author.to_owned()),
            content: Some(String::new()),
            created_at: Some(at.to_owned()),
            ..Message::default()
        }
    }

    fn item(message: Message) -> TimelineItem {
        let id = Uuid::parse_str(message.id.as_deref().unwrap()).unwrap();
        TimelineItem::Message {
            id,
            message: Arc::new(message),
            content: Arc::new(MessageContent::parse("")),
        }
    }

    fn bridged(n: u128, source: &str, id: &str, at: &str) -> Message {
        let payload = serde_json::json!({
            "id": Uuid::from_u128(n).to_string(),
            "author_id": "bridge",
            "content": "",
            "created_at": at,
            "external_author": { "source": source, "id": id, "name": "Ana" },
        });
        serde_json::from_value(payload).unwrap()
    }

    #[test]
    fn the_same_author_within_five_minutes_continues() {
        let first = item(message(1, "ana", "2026-11-20T14:00:00Z"));
        assert!(continues(
            Some(&first),
            &message(2, "ana", "2026-11-20T14:05:00Z")
        ));
        assert!(!continues(
            Some(&first),
            &message(2, "ana", "2026-11-20T14:05:01Z")
        ));
        assert!(!continues(
            Some(&first),
            &message(2, "bia", "2026-11-20T14:01:00Z")
        ));
    }

    #[test]
    fn only_a_message_is_continued() {
        let next = message(2, "ana", "2026-11-20T14:01:00Z");
        assert!(!continues(None, &next));
        let separator = TimelineItem::DateSeparator {
            day: 0,
            before: Uuid::from_u128(2),
        };
        assert!(!continues(Some(&separator), &next));
        let unread = TimelineItem::UnreadMarker {
            before: Uuid::from_u128(2),
        };
        assert!(!continues(Some(&unread), &next));
    }

    #[test]
    fn bridged_authors_match_on_source_and_external_id() {
        let at = "2026-11-20T14:00:00Z";
        let first = item(bridged(1, "matrix", "@ana", at));
        assert!(continues(Some(&first), &bridged(2, "matrix", "@ana", at)));
        // Same bridge account and display name, another person.
        assert!(!continues(Some(&first), &bridged(2, "matrix", "@bia", at)));
        assert!(!continues(Some(&first), &bridged(2, "irc", "@ana", at)));
        // The bridge account itself is not the bridged author.
        assert!(!continues(Some(&first), &message(2, "bridge", at)));
    }
}
