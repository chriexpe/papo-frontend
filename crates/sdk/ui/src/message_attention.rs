//! Why a message asks this account's attention, as facts for a client's highlight: it mentions
//! you, it mentions everyone, it replies to one of your messages. Which facts win, and what
//! they look like, are the client's; whether a notification is due is `papo-sdk`'s policy.

use papo_sdk::{
    Message, Uuid,
    mentions::{contains_everyone_mention, contains_user_mention},
};

use crate::{message_actions::is_own, timeline_item::ReplyOriginal};

/// What in a message concerns this account. A message the account wrote concerns it in no way;
/// a webhook's or bridged author's never counts as the account's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attention {
    /// It mentions the account directly.
    pub mentions_you: bool,
    /// It mentions everyone.
    pub mentions_everyone: bool,
    /// It replies to a message of the account's that history holds. An original history does
    /// not hold proves nothing either way, so it never counts.
    pub replies_to_you: bool,
}

/// What in `message`, replying to `reply` when it replies, concerns `account_user`.
#[must_use]
pub fn attention(
    message: &Message,
    reply: Option<&ReplyOriginal>,
    account_user: Uuid,
) -> Attention {
    if is_own(message, account_user) {
        return Attention::default();
    }
    let content = message.content.as_deref().unwrap_or_default();
    let replies_to_you = matches!(reply, Some(ReplyOriginal::Held { message: original, .. })
        if is_own(original, account_user));
    Attention {
        mentions_you: contains_user_mention(content, account_user),
        mentions_everyone: contains_everyone_mention(content),
        replies_to_you,
    }
}

#[cfg(test)]
mod tests {
    //! Attention facts from synthetic messages.

    use super::*;
    use crate::message_content::MessageContent;
    use std::sync::Arc;

    const ME: u128 = 1;
    const OTHER: u128 = 2;

    fn message(author: u128, content: &str) -> Message {
        Message {
            author_id: Some(Uuid::from_u128(author).to_string()),
            content: Some(content.to_owned()),
            ..Message::default()
        }
    }

    fn held(author: u128) -> ReplyOriginal {
        ReplyOriginal::Held {
            id: Uuid::from_u128(9),
            message: Arc::new(message(author, "original")),
            content: Arc::new(MessageContent::parse("original")),
        }
    }

    fn me() -> Uuid {
        Uuid::from_u128(ME)
    }

    #[test]
    fn mentions_of_you_and_everyone_are_facts_of_their_own() {
        let direct = message(OTHER, &format!("hi @mention(<@{}>)", me()));
        assert_eq!(
            attention(&direct, None, me()),
            Attention {
                mentions_you: true,
                ..Attention::default()
            }
        );
        let everyone = message(OTHER, "@everyone deploy at five");
        assert!(attention(&everyone, None, me()).mentions_everyone);
    }

    #[test]
    fn only_a_held_original_of_yours_makes_a_reply_to_you() {
        let reply = message(OTHER, "agreed");
        assert!(attention(&reply, Some(&held(ME)), me()).replies_to_you);
        assert!(!attention(&reply, Some(&held(OTHER)), me()).replies_to_you);
        for unknown in [ReplyOriginal::Unknown, ReplyOriginal::Deleted] {
            assert!(!attention(&reply, Some(&unknown), me()).replies_to_you);
        }
    }

    #[test]
    fn a_webhook_carrying_your_id_is_not_you() {
        let mut relayed = message(ME, &format!("ping @mention(<@{}>)", me()));
        relayed.webhook_id = Some(Uuid::from_u128(9).to_string());
        assert!(attention(&relayed, None, me()).mentions_you);
        let ReplyOriginal::Held { id, content, .. } = held(ME) else {
            unreachable!()
        };
        let relayed_original = ReplyOriginal::Held {
            id,
            message: std::sync::Arc::new(relayed),
            content,
        };
        let reply = message(OTHER, "agreed");
        assert!(!attention(&reply, Some(&relayed_original), me()).replies_to_you);
    }

    #[test]
    fn your_own_message_asks_nothing_of_you() {
        let own = message(ME, &format!("@everyone and @mention(<@{}>)", me()));
        assert_eq!(attention(&own, Some(&held(ME)), me()), Attention::default());
    }
}
