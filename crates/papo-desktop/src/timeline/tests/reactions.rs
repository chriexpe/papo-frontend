//! Reaction chips, headless: one per Unicode emoji under the message in
//! the SDK's order, spoken with their count and "including you", custom
//! emoji left out, and none at all on a message without reactions.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::Message;
use papo_sdk::operations::interactions::{MessageReactionSummary, MessageUserReaction};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;

use super::{message, runtime};
use crate::design::MessageLayout;

fn total(emoji_id: Option<&str>, unicode: Option<&str>, count: i64) -> MessageReactionSummary {
    MessageReactionSummary {
        emoji_id: emoji_id.map(str::to_owned),
        unicode: unicode.map(str::to_owned),
        count: Some(count),
    }
}

/// Message 1 with these totals, 👍 among them yours.
fn reacted() -> Vector<Arc<TimelineItem>> {
    let item = message(1, "ana", 60, "queijo minas");
    let TimelineItem::Message {
        id,
        message,
        content,
        ..
    } = item.as_ref()
    else {
        unreachable!()
    };
    let message = Message {
        reactions: Some(vec![
            total(None, Some("🧀"), 4),
            total(None, Some("👍"), 3),
            total(Some("00000000-0000-4000-8000-000000000009"), None, 2),
        ]),
        user_reactions: Some(vec![MessageUserReaction {
            id: None,
            emoji_id: None,
            unicode: Some("👍".into()),
        }]),
        ..Message::clone(message)
    };
    let item = TimelineItem::Message {
        id: *id,
        message: Arc::new(message),
        content: Arc::<MessageContent>::clone(content),
        reply: None,
        pending: None,
    };
    [Arc::new(item)].into_iter().collect()
}

#[test]
fn chips_follow_the_backend_and_say_yours() {
    let rt = runtime(reacted(), MessageLayout::default());
    let labels: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|element| element.path.contains("/chips/") && element.role == "button")
        .filter_map(|element| element.label)
        .collect();
    // Custom emoji wait for the catalog; with what the account may do not
    // known yet, nothing offers to add a reaction.
    assert_eq!(labels, ["🧀, 4", "👍, 3, including you"]);
}

#[test]
fn a_message_without_reactions_has_no_chips() {
    let rt = runtime(
        [message(1, "ana", 60, "nada")].into_iter().collect(),
        MessageLayout::default(),
    );
    assert!(
        !rt.elements()
            .iter()
            .any(|element| element.path.ends_with("/chips"))
    );
}
