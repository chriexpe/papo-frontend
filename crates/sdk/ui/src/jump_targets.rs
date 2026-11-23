//! What a jump-anywhere field (Ctrl+K, `docs/design.md` §12) finds across
//! the accounts it is given: their servers' text channels, their direct
//! conversations, the people on their servers with no conversation yet, and
//! the servers themselves, in one ranked list. A leading `#` finds only
//! channels, `@` only people and conversations, `*` only servers. A name
//! equal to the text ranks first, then names starting with it, then names
//! with a word starting with it, then names containing it; within each,
//! channels, conversations, people, servers, then A–Z. Reads only what the
//! SDK holds; nothing is fetched. What an empty field shows (recent jumps,
//! unread conversations) is the client's, which knows its own history.

use std::collections::HashSet;

use papo_sdk::{AccountId, ChannelKind, ResourceSnapshot, Uuid};

use crate::display_name;

/// At most this many targets.
pub const MAX_JUMP_TARGETS: usize = 50;

/// Somewhere a jump can go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JumpKind {
    /// A server's text channel.
    Channel {
        /// The channel.
        channel: Uuid,
    },
    /// A direct conversation.
    Direct {
        /// The conversation.
        channel: Uuid,
        /// The other person in it.
        peer: Uuid,
    },
    /// A person with no conversation yet: a jump opens one.
    Person {
        /// The person.
        user: Uuid,
    },
    /// The server itself.
    Server,
}

/// One target, on one account's server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpTarget {
    /// The account the target belongs to.
    pub account: AccountId,
    /// Where it goes.
    pub kind: JumpKind,
    /// Its name: a channel's, a person's nickname else username, the server's.
    pub name: String,
    /// A person's username, shown beside the name.
    pub username: Option<String>,
}

/// Which kinds a query looks for, from its leading character.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Only {
    All,
    Channels,
    People,
    Servers,
}

/// The targets `query` finds across `accounts` (each with its resource
/// snapshot), ranked as the module says; none for an empty query.
#[must_use]
pub fn jump_targets(accounts: &[(AccountId, &ResourceSnapshot)], query: &str) -> Vec<JumpTarget> {
    let query = query.trim();
    let (only, text) = match query.chars().next() {
        Some('#') => (Only::Channels, &query[1..]),
        Some('@') => (Only::People, &query[1..]),
        Some('*') => (Only::Servers, &query[1..]),
        _ => (Only::All, query),
    };
    let needle = text.trim().to_lowercase();
    if needle.is_empty() && only == Only::All {
        return Vec::new();
    }
    let mut ranked = Vec::new();
    for (account, snapshot) in accounts {
        for target in candidates(*account, snapshot, only) {
            let names: Vec<String> = [Some(target.name.as_str()), target.username.as_deref()]
                .into_iter()
                .flatten()
                .map(str::to_lowercase)
                .collect();
            if let Some(rank) = names.iter().filter_map(|name| rank(name, &needle)).min() {
                ranked.push((
                    rank,
                    kind_order(&target.kind),
                    target.name.to_lowercase(),
                    target,
                ));
            }
        }
    }
    let order = |a: &Ranked, b: &Ranked| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2));
    // Only the best are kept: put them first, then sort just those.
    if ranked.len() > MAX_JUMP_TARGETS {
        ranked.select_nth_unstable_by(MAX_JUMP_TARGETS, order);
        ranked.truncate(MAX_JUMP_TARGETS);
    }
    ranked.sort_by(order);
    ranked.into_iter().map(|(_, _, _, target)| target).collect()
}

/// A target with what it ranks by: how well it matches, its kind, its name.
type Ranked = (u8, u8, String, JumpTarget);

/// How well `name` matches `needle` (both lowercase): 0 equal, 1 starting
/// with it, 2 a word starting with it, 3 containing it; none otherwise. An
/// empty needle (a prefix alone) matches everything as a start.
fn rank(name: &str, needle: &str) -> Option<u8> {
    if name == needle {
        Some(0)
    } else if name.starts_with(needle) {
        Some(1)
    } else if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(needle))
    {
        Some(2)
    } else if name.contains(needle) {
        Some(3)
    } else {
        None
    }
}

/// Channels, conversations, people, servers.
fn kind_order(kind: &JumpKind) -> u8 {
    match kind {
        JumpKind::Channel { .. } => 0,
        JumpKind::Direct { .. } => 1,
        JumpKind::Person { .. } => 2,
        JumpKind::Server => 3,
    }
}

/// Every target of `account` that `only` looks for.
fn candidates(account: AccountId, snapshot: &ResourceSnapshot, only: Only) -> Vec<JumpTarget> {
    let mut targets = Vec::new();
    let target = |kind, name: &str, username: Option<String>| JumpTarget {
        account,
        kind,
        name: name.to_owned(),
        username,
    };
    if matches!(only, Only::All | Only::Channels) {
        for channel in snapshot.channels.iter() {
            if channel.kind == ChannelKind::Text
                && let Some(name) = channel.name.as_deref()
            {
                let kind = JumpKind::Channel {
                    channel: channel.id,
                };
                targets.push(target(kind, name, None));
            }
        }
    }
    if matches!(only, Only::All | Only::People) {
        let person = |id: Uuid| {
            let user = snapshot.users.get(id)?;
            let name = display_name(user.nickname.as_deref(), user.username.as_deref())?;
            Some((name.to_owned(), user.username.clone()))
        };
        let mut with_conversation = HashSet::new();
        for dm in snapshot.dms.iter() {
            if snapshot.blocks.contains(dm.peer_id) {
                continue;
            }
            if let Some((name, username)) = person(dm.peer_id) {
                let kind = JumpKind::Direct {
                    channel: dm.id,
                    peer: dm.peer_id,
                };
                targets.push(target(kind, &name, username));
                with_conversation.insert(dm.peer_id);
            }
        }
        for id in snapshot.members.iter() {
            let banned = snapshot
                .users
                .get(id)
                .is_some_and(|user| user.banned == Some(true));
            let skipped = id == account.user()
                || banned
                || snapshot.blocks.contains(id)
                || with_conversation.contains(&id);
            if skipped {
                continue;
            }
            if let Some((name, username)) = person(id) {
                targets.push(target(JumpKind::Person { user: id }, &name, username));
            }
        }
    }
    if matches!(only, Only::All | Only::Servers)
        && let Some(name) = snapshot
            .server
            .as_ref()
            .and_then(|server| server.name.as_deref())
    {
        targets.push(target(JumpKind::Server, name, None));
    }
    targets
}

#[cfg(test)]
mod tests;
