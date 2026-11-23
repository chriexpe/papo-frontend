//! This account's confirmed reaction membership on held messages. Totals stay the backend's
//! (`mutations` applies its counts); sending and confirming a reaction belong to outgoing.
//!
//! A reaction event carries no user, so the only evidence of which emoji are this account's is
//! a fetched page's `user_reactions` and this account's own confirmed requests. A confirmed
//! change is settled here and re-applied over any page fetched before it, the way
//! `pending_reactions` keeps newer totals over an older page.

use papo_api::wire::{Message, MessageUserReaction};
use serde::{Deserialize, Serialize};

use super::{
    HistoryError,
    model::{Entry, Model},
};

/// One confirmed change of this account's membership for one emoji.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnReaction {
    /// Custom emoji identity, exactly as sent; exclusive with `unicode` (the outgoing queue
    /// validates that before sending).
    pub emoji_id: Option<String>,
    /// Unicode sequence, exactly as sent; exclusive with `emoji_id`.
    pub unicode: Option<String>,
    /// Whether this account now has the reaction.
    pub add: bool,
    /// The model revision it was settled at: a page asked for before it cannot hold it.
    /// Set by `Model::own_reaction`.
    #[serde(default)]
    pub revision: u64,
}

impl OwnReaction {
    fn same_emoji(&self, emoji_id: Option<&str>, unicode: Option<&str>) -> bool {
        self.emoji_id.as_deref() == emoji_id && self.unicode.as_deref() == unicode
    }

    /// Applies this change to a message's `user_reactions`. An added reaction has no backend
    /// reaction ID (`id` stays absent) until a page reports it.
    pub fn apply(&self, message: &mut Message) {
        let mine = message.user_reactions.get_or_insert_with(Vec::new);
        let held = mine.iter().any(|reaction| {
            self.same_emoji(reaction.emoji_id.as_deref(), reaction.unicode.as_deref())
        });
        if self.add && !held {
            mine.push(MessageUserReaction {
                id: None,
                emoji_id: self.emoji_id.clone(),
                unicode: self.unicode.clone(),
            });
        } else if !self.add {
            mine.retain(|reaction| {
                !self.same_emoji(reaction.emoji_id.as_deref(), reaction.unicode.as_deref())
            });
        }
    }
}

impl Model {
    /// Settles a confirmed change of this account's reaction on `message`. False when there is
    /// nothing to settle: the message is deleted, or a removal names a message this history
    /// does not hold (a removal may be confirmed by a not-found, which proves nothing about the
    /// message, so it never creates an entry). Settling again reaches the same state.
    pub fn own_reaction(
        &mut self,
        message: [u8; 16],
        mut change: OwnReaction,
    ) -> Result<bool, HistoryError> {
        if !change.add && !self.entries.contains_key(&message) {
            return Ok(false);
        }
        let revision = self.next_revision()?;
        let entry = self
            .entries
            .entry(message)
            .or_insert_with(|| Entry::empty(revision));
        if entry.deleted {
            return Ok(false);
        }
        if let Some(held) = &mut entry.message {
            change.apply(held);
        }
        entry.own_reactions.retain(|existing| {
            !existing.same_emoji(change.emoji_id.as_deref(), change.unicode.as_deref())
        });
        change.revision = revision;
        entry.own_reactions.push(change);
        entry.revision = revision;
        self.dirty.insert(message);
        Ok(true)
    }
}
