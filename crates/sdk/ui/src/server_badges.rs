//! Account/session badge aggregates; core owns channel evidence, policy and lifecycle authority.

use std::{collections::BTreeMap, pin::Pin, sync::Arc};

use futures_util::Stream;
use papo_sdk::{AccountId, ServerId, UnreadCount};

mod projection;
#[cfg(test)]
mod tests;

/// A badge total over one explicitly scoped set of channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadgeAggregate {
    /// Eligible unread trigger total; incomplete evidence preserves a lower bound.
    pub mention_count: UnreadCount,
    /// `None` means unread presence is unknown, not false.
    pub has_unread: Option<bool>,
}

impl Default for BadgeAggregate {
    fn default() -> Self {
        Self {
            mention_count: UnreadCount::AtLeast(0),
            has_unread: None,
        }
    }
}

/// One account's complete channel aggregate and its direct-message subset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountBadges {
    /// All admitted channels belonging to this account only.
    pub all: BadgeAggregate,
    /// Only this account's server channels, without its direct conversations: what a server
    /// tile shows beside a separate Direct entry, so a DM is counted once.
    pub channels: BadgeAggregate,
    /// Only this account's direct conversations, never another account's DMs.
    pub dms: BadgeAggregate,
}

/// Coherent rail badge projection over the visible registry and currently open sessions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerBadgesSnapshot {
    /// Visible accounts keyed by full server/user identity; closed sessions are unknown.
    pub accounts: BTreeMap<AccountId, Arc<AccountBadges>>,
    /// Each server's active account only; absent selection or closed session is unknown.
    pub servers: BTreeMap<ServerId, Arc<AccountBadges>>,
}

impl ServerBadgesSnapshot {
    /// `accounts`' direct conversations as one: what a cross-server Direct entry shows for
    /// the accounts its list holds. An account without badges (its session closed) is an
    /// unknown part; unknown parts keep it a lower bound, as in one account's aggregate.
    #[must_use]
    pub fn direct_messages<'a>(
        &self,
        accounts: impl IntoIterator<Item = &'a AccountId>,
    ) -> BadgeAggregate {
        let unknown = BadgeAggregate::default();
        let parts: Vec<&BadgeAggregate> = accounts
            .into_iter()
            .map(|account| {
                self.accounts
                    .get(account)
                    .map_or(&unknown, |badges| &badges.dms)
            })
            .collect();
        projection::combine(parts.into_iter())
    }
}

/// A changed aggregate image. Equal values do not publish or replace their item `Arc`s.
#[derive(Clone, Debug)]
pub struct ServerBadgesUpdate {
    /// Monotonic aggregate publication version, independent of registry commits.
    pub version: u64,
    /// Full current image; unchanged entries retain their previous `Arc`.
    pub snapshot: ServerBadgesSnapshot,
}

/// Pull-driven aggregate changes, without a runtime, background task or unbounded queue.
pub type ServerBadgesStream = Pin<Box<dyn Stream<Item = ServerBadgesUpdate> + Send>>;

/// Atomic aggregate baseline and subsequent changed-only images.
pub struct ServerBadgesSubscription {
    /// Aggregate version represented by the baseline.
    pub version: u64,
    /// Initial account and active-account server values.
    pub snapshot: ServerBadgesSnapshot,
    /// Subsequent changed images; client shutdown clears stale values before ending.
    pub stream: ServerBadgesStream,
}

pub(crate) use projection::subscribe;
