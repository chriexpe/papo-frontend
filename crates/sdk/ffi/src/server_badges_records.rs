//! Account and server-rail unread badge counter DTOs across FFI.
//! Registry discovery and server/account list DTOs live in `server_records.rs`.

use std::sync::Arc;

use papo_sdk_ui::{AccountBadges, BadgeAggregate, ServerBadgesSnapshot};

use crate::ids::FfiAccountId;

/// Exact or lower-bound unread trigger count without losing incomplete evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiUnreadCount {
    /// Complete coverage proved exact unread trigger count.
    Exact {
        /// Exact unread count.
        count: u64,
    },
    /// Coverage is incomplete; at least `count` unread triggers exist.
    AtLeast {
        /// Proven lower bound.
        count: u64,
    },
}

impl From<papo_sdk::UnreadCount> for FfiUnreadCount {
    fn from(value: papo_sdk::UnreadCount) -> Self {
        match value {
            papo_sdk::UnreadCount::Exact(count) => Self::Exact { count },
            papo_sdk::UnreadCount::AtLeast(count) => Self::AtLeast { count },
            _ => Self::AtLeast { count: 0 },
        }
    }
}

/// Aggregate badge counters for an account or rail item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiBadgeAggregate {
    /// Eligible unread mention/trigger total preserving lower bounds.
    pub mention_count: FfiUnreadCount,
    /// `None` when unread evidence is unknown/cold.
    pub has_unread: Option<bool>,
}

impl From<&BadgeAggregate> for FfiBadgeAggregate {
    fn from(value: &BadgeAggregate) -> Self {
        Self {
            mention_count: value.mention_count.into(),
            has_unread: value.has_unread,
        }
    }
}

/// One account's complete and DM-only badge totals.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiAccountBadgesEntry {
    /// Account owning these totals.
    pub account: FfiAccountId,
    /// All accessible channels on the account.
    pub all: FfiBadgeAggregate,
    /// Only direct conversations on the account.
    pub dms: FfiBadgeAggregate,
}

/// One server rail entry's active-account badge totals.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerBadgesEntry {
    /// Server identity.
    pub server_id: String,
    /// All accessible channels of the active account.
    pub all: FfiBadgeAggregate,
    /// Direct conversations of the active account.
    pub dms: FfiBadgeAggregate,
}

/// Coherent snapshot of account and server-rail badge totals.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerBadgesSnapshot {
    /// Monotonic aggregate publication version.
    pub version: u64,
    /// Per-account badge totals.
    pub accounts: Vec<FfiAccountBadgesEntry>,
    /// Per-server active-account totals.
    pub servers: Vec<FfiServerBadgesEntry>,
}

/// Converts UI `ServerBadgesSnapshot` into FFI snapshot.
pub(crate) fn map_badges_snapshot(
    version: u64,
    snap: ServerBadgesSnapshot,
) -> FfiServerBadgesSnapshot {
    let map_account_badges = |b: &Arc<AccountBadges>| ((&b.all).into(), (&b.dms).into());
    FfiServerBadgesSnapshot {
        version,
        accounts: snap
            .accounts
            .iter()
            .map(|(account, badges)| {
                let (all, dms) = map_account_badges(badges);
                FfiAccountBadgesEntry {
                    account: FfiAccountId::from_sdk(*account),
                    all,
                    dms,
                }
            })
            .collect(),
        servers: snap
            .servers
            .iter()
            .map(|(server, badges)| {
                let (all, dms) = map_account_badges(badges);
                FfiServerBadgesEntry {
                    server_id: server.to_string(),
                    all,
                    dms,
                }
            })
            .collect(),
    }
}
