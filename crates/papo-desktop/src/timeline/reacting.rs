//! What a timeline's rows read to draw their reactions and hover bars,
//! beyond the message: the account's outgoing queue (a chip waits on it),
//! its quick reactions, what it may do in the channel, and the hover bar
//! Add reaction stretched. The shell fills it once per view.

use std::sync::Arc;

use papo_sdk::{OutgoingSnapshot, Uuid};
use papo_sdk_ui::ChannelCapabilities;
use papo_sdk_ui::operations::ReactionEmoji;

use crate::reaction_emoji::{ReactionEmojiLook, ShownReaction};

/// What a row's reactions and hover bar read beyond the message.
#[derive(Clone)]
pub(crate) struct Reacting {
    /// The account's outgoing queue: a chip whose change is in it waits.
    pub(crate) outgoing: Arc<OutgoingSnapshot>,
    /// The account's quick reactions, most used first.
    pub(crate) quick: Arc<[ReactionEmoji]>,
    /// What the account's custom emoji are called and draw as; none before an account shows.
    pub(crate) emoji: Option<ReactionEmojiLook>,
    /// What the account may do in the channel; none while unknown.
    pub(crate) capabilities: Option<ChannelCapabilities>,
    /// The message whose bar Add reaction stretched, and the emoji ← and →
    /// are on.
    pub(crate) stretched: Option<(Uuid, usize)>,
}

impl Reacting {
    /// `emoji` as a row draws it; none when it can't be shown (a custom emoji without its
    /// catalog).
    pub(crate) fn shown(&self, emoji: &ReactionEmoji) -> Option<ShownReaction> {
        match (&self.emoji, emoji) {
            (Some(look), _) => look.shown(emoji),
            (None, ReactionEmoji::Unicode(glyph)) => Some(ShownReaction::unicode(glyph)),
            (None, ReactionEmoji::Custom(_)) => None,
        }
    }

    /// Nothing known yet: no quick reactions, an empty queue.
    pub(crate) fn unknown() -> Reacting {
        Reacting {
            outgoing: Arc::new(OutgoingSnapshot {
                account: None,
                version: 0,
                operations: Vec::new(),
                upload_progress: None,
                fault: None,
            }),
            quick: Arc::from([]),
            emoji: None,
            capabilities: None,
            stretched: None,
        }
    }
}
