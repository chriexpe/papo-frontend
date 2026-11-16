//! Public rail badge values and subscriptions; SDK reading owns counts and notification eligibility.

use std::{collections::BTreeMap, pin::Pin};

use futures_util::Stream;
use papo_sdk::{AccountId, ServerId, UnreadCount};

/// Coverage-aware totals for a set of channels, without presentation or delivery policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadgeTotals {
    /// `Some(true)` when any unread is known, `Some(false)` only when all counts are exact zero.
    /// `None` means incomplete coverage cannot establish either answer.
    pub has_unread: Option<bool>,
    /// Eligible unread triggers; incomplete coverage preserves the known lower bound.
    pub mentions: UnreadCount,
}

impl BadgeTotals {
    pub(crate) fn empty() -> Self {
        Self {
            has_unread: Some(false),
            mentions: UnreadCount::Exact(0),
        }
    }

    pub(crate) fn include(&mut self, unread: UnreadCount, mentions: UnreadCount) {
        let has_unread = match unread {
            UnreadCount::Exact(0) => Some(false),
            UnreadCount::AtLeast(0) => None,
            UnreadCount::Exact(_) | UnreadCount::AtLeast(_) => Some(true),
            _ => None,
        };
        self.has_unread = match (self.has_unread, has_unread) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
        self.mentions = add_counts(self.mentions, mentions);
    }
}

fn add_counts(left: UnreadCount, right: UnreadCount) -> UnreadCount {
    match (left, right) {
        (UnreadCount::Exact(left), UnreadCount::Exact(right)) => left
            .checked_add(right)
            .map_or(UnreadCount::AtLeast(u64::MAX), UnreadCount::Exact),
        (
            UnreadCount::Exact(left) | UnreadCount::AtLeast(left),
            UnreadCount::Exact(right) | UnreadCount::AtLeast(right),
        ) => UnreadCount::AtLeast(left.saturating_add(right)),
        _ => UnreadCount::AtLeast(0),
    }
}

/// Account-isolated rail badges. `None` always means unknown, never zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadgeSnapshot {
    /// Channel totals for each visible account; excludes direct conversations.
    /// Missing, closed or not-yet-reconciled sessions have unknown totals.
    pub accounts: BTreeMap<AccountId, Option<BadgeTotals>>,
    /// Channel totals for each server's active account, or unknown without an open active account.
    pub servers: BTreeMap<ServerId, Option<BadgeTotals>>,
    /// Unified direct-conversation totals across visible accounts.
    /// Unknown if any contributing account session is unavailable or not yet reconciled.
    pub direct_messages: Option<BadgeTotals>,
}

/// One changed aggregate image. Unchanged source publications produce no badge update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadgeUpdate {
    /// Monotonic subscription-local version, independent of resource/registry versions.
    pub version: u64,
    /// Full replacement image, including removals and unknown states.
    pub snapshot: BadgeSnapshot,
}

/// Captured aggregate baseline and bounded, pull-driven updates; no extra runtime or task.
pub struct BadgeSubscription {
    /// Subscription-local baseline version.
    pub version: u64,
    /// Initial badge values. Source changes after capture remain pending in the stream.
    pub snapshot: BadgeSnapshot,
    /// Changed images only; ends when the SDK registry watch closes.
    pub stream: Pin<Box<dyn Stream<Item = BadgeUpdate> + Send>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_bounds_do_not_turn_unknown_unread_into_zero() {
        let mut totals = BadgeTotals::empty();
        totals.include(UnreadCount::AtLeast(0), UnreadCount::AtLeast(2));
        assert_eq!(totals.has_unread, None);
        totals.include(UnreadCount::Exact(1), UnreadCount::Exact(3));
        assert_eq!(totals.has_unread, Some(true));
        assert_eq!(totals.mentions, UnreadCount::AtLeast(5));
    }

    #[test]
    fn overflow_is_a_lower_bound_instead_of_an_exact_saturated_total() {
        assert_eq!(
            add_counts(UnreadCount::Exact(u64::MAX), UnreadCount::Exact(1)),
            UnreadCount::AtLeast(u64::MAX)
        );
    }
}
