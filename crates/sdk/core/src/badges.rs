//! Account-owned eligible mention totals over held history; reading, history and notification
//! policy retain authority over read evidence, coverage, admission and trigger eligibility.

mod access;
mod counts;
mod recount;
pub(crate) use access::restrict;
#[cfg(test)]
mod tests;

use crate::{Generation, Session, UnreadCount, Uuid};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::watch;

/// One channel's independent unread signal and eligible unread trigger total.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelBadges {
    /// `None` means reading evidence is insufficient; notification settings never clear this.
    pub has_unread: Option<bool>,
    /// Eligible unread triggers, not ordinary messages admitted by All-mode delivery.
    /// `AtLeast` also covers unresolved read boundaries and historical trigger evidence.
    pub mention_count: UnreadCount,
    /// Whether this channel belongs to this account's admitted direct-conversation list.
    pub direct_message: bool,
}
impl ChannelBadges {
    pub(super) const fn unknown(direct_message: bool) -> Self {
        Self {
            has_unread: None,
            mention_count: UnreadCount::AtLeast(0),
            direct_message,
        }
    }
}

/// Immutable account-scoped badge image. Denied channels contribute no cached-history values.
#[derive(Clone, Debug)]
pub struct BadgesSnapshot {
    /// Session epoch that admitted these inputs.
    pub generation: Generation,
    /// Meaningful publication sequence; zero denotes the not-yet-evaluated initial image.
    pub version: u64,
    /// The resource owner has admitted both complete channel and DM lists this generation.
    /// Otherwise even an empty image leaves aggregate counts uncertain.
    pub channels_complete: bool,
    /// All known non-denied channels, including uncertain cold/quarantined contributions.
    pub channels: BTreeMap<Uuid, Arc<ChannelBadges>>,
}

pub(crate) struct Badges {
    pub image: Arc<crate::observable::StateCell<Arc<BadgesSnapshot>>>,
    started: AtomicBool,
}
impl Badges {
    pub fn new(generation: Generation) -> Self {
        Self {
            image: crate::observable::StateCell::new(Arc::new(BadgesSnapshot {
                generation,
                version: 0,
                channels_complete: false,
                channels: BTreeMap::new(),
            })),
            started: AtomicBool::new(false),
        }
    }
}
impl Session {
    /// Subscribes atomically to coalesced badge images without hydrating history or inbox rows.
    /// Closes on account removal, replacement, sign-out or shutdown. Unchanged channels retain
    /// their `Arc`; unchanged totals do not publish another image.
    #[must_use]
    pub fn subscribe_badges(&self) -> watch::Receiver<Arc<BadgesSnapshot>> {
        let receiver = self.inner.badges.image.subscribe();
        recount::start(self);
        receiver
    }
}
