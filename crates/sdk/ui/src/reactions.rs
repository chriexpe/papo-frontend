//! A message's reaction chips, this account's quick reactions, and who reacted. From history's
//! backend totals, this account's confirmed membership and the outgoing queue; nothing here
//! counts or guesses. Sending a reaction is `ReactionService`; names, words and layout are the
//! client's.

#[cfg(test)]
mod tests;

use std::collections::HashMap;

use papo_sdk::{
    Message, OperationKind, OperationState, OutgoingSnapshot, Uuid,
    operations::interactions::{MessageReactionGroup, ReactorPage},
};

use crate::{ChannelCapabilities, operations::ReactionEmoji};

/// The emoji that fill the quick reactions this account has not used enough to rank.
pub const DEFAULT_QUICK_REACTIONS: [&str; 8] = ["😂", "👍", "❤️", "🔥", "😭", "👀", "🙏", "😮"];

/// One emoji's reactions on a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionChip {
    /// The emoji.
    pub emoji: ReactionEmoji,
    /// How many reacted with it: the backend's total, never adjusted locally.
    pub count: u64,
    /// This account's reaction is among them, as last confirmed (a page, or this account's own
    /// confirmed change). Another device's change shows once a page reports it: a live total
    /// names no user.
    pub mine: bool,
    /// A change of this account's to it is queued or being sent: the chip waits for it.
    pub waiting: bool,
}

impl ReactionChip {
    /// Whether this account may toggle the chip now: not while a change waits, and with the
    /// channel's permission to add (or, for its own, to remove) a reaction. Unknown
    /// capabilities fail closed.
    #[must_use]
    pub fn can_toggle(&self, capabilities: Option<ChannelCapabilities>) -> bool {
        let Some(capabilities) = capabilities else {
            return false;
        };
        let allowed = if self.mine {
            capabilities.remove_reaction
        } else {
            capabilities.add_reaction
        };
        allowed && !self.waiting
    }
}

/// The chips under `message`, in the backend's order (by each emoji's first reaction; a new
/// emoji comes last). `outgoing` marks the ones this account's queued changes wait on.
#[must_use]
pub fn reaction_chips(message: &Message, outgoing: &OutgoingSnapshot) -> Vec<ReactionChip> {
    let id = message
        .id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok());
    let mine: Vec<ReactionEmoji> = message
        .user_reactions
        .iter()
        .flatten()
        .filter_map(|reaction| emoji(reaction.emoji_id.as_deref(), reaction.unicode.as_deref()))
        .collect();
    message
        .reactions
        .iter()
        .flatten()
        .filter_map(|total| {
            let emoji = emoji(total.emoji_id.as_deref(), total.unicode.as_deref())?;
            let count = total.count.and_then(|count| u64::try_from(count).ok())?;
            (count > 0).then(|| ReactionChip {
                mine: mine.contains(&emoji),
                waiting: id.is_some_and(|id| waiting(outgoing, id, &emoji)),
                emoji,
                count,
            })
        })
        .collect()
}

/// This account's `count` quick reactions: its most used Unicode emoji first (`usage` is the
/// expression library's `reaction_usage`, most used first), then the defaults it has not
/// used. Custom emoji are left out until the SDK has an emoji catalog to draw them from.
#[must_use]
pub fn quick_reactions(usage: &[(String, u64)], count: usize) -> Vec<String> {
    let mut quick: Vec<String> = Vec::with_capacity(count);
    let used = usage
        .iter()
        .map(|(key, _)| key.as_str())
        .filter(|key| !key.is_empty() && Uuid::parse_str(key).is_err());
    for key in used.chain(DEFAULT_QUICK_REACTIONS) {
        if quick.len() == count {
            break;
        }
        if !quick.iter().any(|held| held == key) {
            quick.push(key.to_owned());
        }
    }
    quick
}

/// Someone who reacted to the message, with every emoji of theirs the loaded pages hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reactor {
    /// Their user ID; their name and avatar are the client's to resolve.
    pub user: Uuid,
    /// Their emoji, in the order they reacted.
    pub emoji: Vec<ReactionEmoji>,
    /// This account.
    pub you: bool,
}

/// Who reacted to one message, folded from `reaction_users` pages asked oldest first
/// (`HistoryOrder::Asc`): this account first (from its confirmed membership), then everyone
/// else in the order they reacted. A page holds 100 reactions of every emoji together, so one
/// emoji's people may need several pages: see [`Reactors::wants_more`].
#[derive(Clone, Debug)]
pub struct Reactors {
    account_user: Uuid,
    /// This account's confirmed emoji, in chip order.
    yours: Vec<ReactionEmoji>,
    /// Everyone else, in the order of their first reaction.
    others: Vec<Reactor>,
    index: HashMap<Uuid, usize>,
    exhausted: bool,
}

impl Reactors {
    /// An empty list for a message with these `chips`, before any page.
    #[must_use]
    pub fn new(account_user: Uuid, chips: &[ReactionChip]) -> Self {
        Self {
            account_user,
            yours: chips
                .iter()
                .filter(|chip| chip.mine)
                .map(|chip| chip.emoji.clone())
                .collect(),
            others: Vec::new(),
            index: HashMap::new(),
            exhausted: false,
        }
    }

    /// Adds the next page. This account's own rows in it are skipped: its membership comes
    /// from the chips, which a confirmed change reaches before any page does.
    pub fn push(&mut self, page: &ReactorPage) {
        for (user, emoji) in in_order(&page.groups) {
            if user == self.account_user {
                continue;
            }
            let at = *self.index.entry(user).or_insert_with(|| {
                self.others.push(Reactor {
                    user,
                    emoji: Vec::new(),
                    you: false,
                });
                self.others.len() - 1
            });
            if !self.others[at].emoji.contains(&emoji) {
                self.others[at].emoji.push(emoji);
            }
        }
        self.exhausted = page.next.is_none();
    }

    /// The people to list: everyone, or those who reacted with `filter`; this account first.
    #[must_use]
    pub fn rows(&self, filter: Option<&ReactionEmoji>) -> Vec<Reactor> {
        let you = (!self.yours.is_empty() && filter.is_none_or(|emoji| self.yours.contains(emoji)))
            .then(|| Reactor {
                user: self.account_user,
                emoji: self.yours.clone(),
                you: true,
            });
        you.into_iter()
            .chain(
                self.others
                    .iter()
                    .filter(|reactor| filter.is_none_or(|emoji| reactor.emoji.contains(emoji)))
                    .cloned(),
            )
            .collect()
    }

    /// Whether another page is needed to show everyone `filter` counts on `chips` (every
    /// chip, for all): pages remain and the loaded ones list fewer people than the total.
    #[must_use]
    pub fn wants_more(&self, filter: Option<&ReactionEmoji>, chips: &[ReactionChip]) -> bool {
        if self.exhausted {
            return false;
        }
        let listed = |emoji: &ReactionEmoji| {
            let others = self
                .others
                .iter()
                .filter(|reactor| reactor.emoji.contains(emoji))
                .count();
            u64::try_from(others).unwrap_or(u64::MAX) + u64::from(self.yours.contains(emoji))
        };
        chips
            .iter()
            .filter(|chip| filter.is_none_or(|emoji| *emoji == chip.emoji))
            .any(|chip| listed(&chip.emoji) < chip.count)
    }

    /// Whether every page is loaded.
    #[must_use]
    pub fn exhausted(&self) -> bool {
        self.exhausted
    }
}

/// The page's reactions in the order they were made, as `(user, emoji)`. A group lists only
/// its own users, so the order across emoji comes from each reaction's `(created_at, id)`.
/// Entries without a user or a readable time are skipped (the SDK already validated them).
fn in_order(groups: &[MessageReactionGroup]) -> Vec<(Uuid, ReactionEmoji)> {
    let mut reactions = Vec::new();
    for group in groups {
        let Some(emoji) = emoji(group.emoji_id.as_deref(), group.unicode.as_deref()) else {
            continue;
        };
        for user in group.users.iter().flatten() {
            let who = user
                .user_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok());
            let id = user.id.as_deref().and_then(|id| Uuid::parse_str(id).ok());
            let at = user
                .created_at
                .as_deref()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok());
            if let (Some(who), Some(id), Some(at)) = (who, id, at) {
                reactions.push(((at, id), who, emoji.clone()));
            }
        }
    }
    reactions.sort_by_key(|(at, _, _)| *at);
    reactions
        .into_iter()
        .map(|(_, who, emoji)| (who, emoji))
        .collect()
}

/// The emoji a wire reaction names: exactly one of a custom emoji ID or a Unicode sequence.
fn emoji(emoji_id: Option<&str>, unicode: Option<&str>) -> Option<ReactionEmoji> {
    match (emoji_id, unicode) {
        (Some(id), None) => Uuid::parse_str(id).ok().map(ReactionEmoji::Custom),
        (None, Some(unicode)) if !unicode.is_empty() => {
            Some(ReactionEmoji::Unicode(unicode.to_owned()))
        }
        _ => None,
    }
}

/// A reaction change of this account's to `message` with `emoji` is queued or being sent.
fn waiting(outgoing: &OutgoingSnapshot, message: Uuid, wanted: &ReactionEmoji) -> bool {
    outgoing.operations.iter().any(|operation| {
        matches!(
            operation.state,
            OperationState::Queued | OperationState::Sending
        ) && match operation.kind.as_ref() {
            OperationKind::Reaction {
                target: papo_sdk::MessageTarget::Remote(target),
                emoji_id,
                unicode,
                ..
            } => *target == message && same(wanted, *emoji_id, unicode.as_deref()),
            _ => false,
        }
    })
}

fn same(wanted: &ReactionEmoji, emoji_id: Option<Uuid>, unicode: Option<&str>) -> bool {
    match wanted {
        ReactionEmoji::Custom(id) => emoji_id == Some(*id) && unicode.is_none(),
        ReactionEmoji::Unicode(wanted) => emoji_id.is_none() && unicode == Some(wanted.as_str()),
    }
}
