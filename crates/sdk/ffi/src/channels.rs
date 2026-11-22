//! Account channel list, category hierarchy, read marks, and unread badge projections across FFI.
//! Unified direct messages and live server info live in `direct_messages.rs`.

use std::sync::Arc;

use papo_sdk::{ChannelKind, MessageKey};
use papo_sdk_ui::{ChannelListItem, ChannelListService};

use crate::{
    client::FfiClient,
    define_ffi_vector_diff,
    diffs::translate_diffs,
    errors::FfiClientError,
    ids::{FfiAccountId, FfiBackendMode},
    server_badges_records::FfiUnreadCount,
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

/// Admitted backend channel kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiChannelKind {
    /// Standard text channel.
    Text,
    /// Real-time voice/video channel.
    Voice,
    /// Collapsible category grouping channels by `parent_id`.
    Category,
}

impl From<ChannelKind> for FfiChannelKind {
    fn from(value: ChannelKind) -> Self {
        match value {
            ChannelKind::Text => Self::Text,
            ChannelKind::Voice => Self::Voice,
            ChannelKind::Category => Self::Category,
        }
    }
}

/// Chronological message key used in durable read marks and catch-up boundaries.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMessageKey {
    /// Microseconds since Unix epoch.
    pub created_at_us: i64,
    /// Canonical message UUID string.
    pub message_id: String,
}

impl From<MessageKey> for FfiMessageKey {
    fn from(value: MessageKey) -> Self {
        Self {
            created_at_us: value.created_at_us,
            message_id: value.message_id.to_string(),
        }
    }
}

/// SDK durable read cursors and unread count overlay for one channel/DM.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReadState {
    /// Furthest locally committed visible message.
    pub local_point: Option<FfiMessageKey>,
    /// Furthest server-acknowledged read mark.
    pub uploaded_point: Option<FfiMessageKey>,
    /// Unread message count preserving exact vs lower-bound semantics.
    pub unread_count: FfiUnreadCount,
    /// First unread cached message UUID if known.
    pub first_unread: Option<String>,
}

impl From<&papo_sdk::ReadState> for FfiReadState {
    fn from(value: &papo_sdk::ReadState) -> Self {
        Self {
            local_point: value.local_point.map(Into::into),
            uploaded_point: value.uploaded_point.map(Into::into),
            unread_count: value.unread_count.into(),
            first_unread: value.first_unread.map(|id| id.to_string()),
        }
    }
}

/// Independent unread presence and eligible mention count overlay for one channel/DM.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelBadges {
    /// `None` if unread state could not be established safely.
    pub has_unread: Option<bool>,
    /// Eligible mention/trigger count preserving lower bounds.
    pub mention_count: FfiUnreadCount,
    /// True when the channel is an admitted direct conversation.
    pub direct_message: bool,
}

impl From<&papo_sdk::ChannelBadges> for FfiChannelBadges {
    fn from(value: &papo_sdk::ChannelBadges) -> Self {
        Self {
            has_unread: value.has_unread,
            mention_count: value.mention_count.into(),
            direct_message: value.direct_message,
        }
    }
}

/// Projected channel row preserving backend order, category nesting, mode, read state, and badges.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelListItem {
    /// Channel UUID string.
    pub id: String,
    /// Raw backend channel name.
    pub name: Option<String>,
    /// Admitted channel kind.
    pub kind: FfiChannelKind,
    /// Optional backend sort position metadata.
    pub position: Option<i64>,
    /// Channel topic text.
    pub topic: Option<String>,
    /// Optional parent category channel UUID string.
    pub parent_id: Option<String>,
    /// Backend notification mode override.
    pub notification_settings: Option<FfiBackendMode>,
    /// Exact backend creation timestamp string.
    pub created_at: Option<String>,
    /// SDK reading projection if verified in this generation.
    pub unread: Option<FfiReadState>,
    /// SDK eligible mention/unread badge overlay if available.
    pub badges: Option<FfiChannelBadges>,
}

impl From<Arc<ChannelListItem>> for FfiChannelListItem {
    fn from(item: Arc<ChannelListItem>) -> Self {
        Self {
            id: item.id.to_string(),
            name: item.name.clone(),
            kind: item.kind.into(),
            position: item.position,
            topic: item.topic.clone(),
            parent_id: item.parent_id.map(|id| id.to_string()),
            notification_settings: item.notification_settings.as_ref().map(Into::into),
            created_at: item.created_at.clone(),
            unread: item.unread.as_ref().map(Into::into),
            badges: item.badges.as_deref().map(Into::into),
        }
    }
}

define_ffi_vector_diff!(
    "Ordered diff over channel list items.",
    FfiChannelDiff,
    FfiChannelListItem
);

/// Atomic initial baseline for an account's channel list.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelListSnapshot {
    /// Channel projection version.
    pub version: u64,
    /// Session generation owning this baseline.
    pub generation: u64,
    /// Channel items in backend order.
    pub items: Vec<FfiChannelListItem>,
}

/// Incremental ordered batch of channel diffs.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelListUpdate {
    /// New projection version after applying all diffs.
    pub version: u64,
    /// Session generation stamped on source resource snapshot.
    pub generation: u64,
    /// Sequential diff list.
    pub diffs: Vec<FfiChannelDiff>,
}

/// Listener for account channel list updates.
#[uniffi::export(with_foreign)]
pub trait FfiChannelListListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, snapshot: FfiChannelListSnapshot);
    /// Incremental ordered diff batch.
    fn on_update(&self, update: FfiChannelListUpdate);
    /// Terminal notification when session closes.
    fn on_closed(&self);
}

#[uniffi::export]
impl FfiClient {
    /// Subscribes to ordered channel list items and diffs for one account.
    ///
    /// # Errors
    /// Returns missing/revoked account or worker errors.
    pub fn subscribe_channels(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiChannelListListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let sub = ChannelListService::new(session).subscribe();
        let baseline = FfiChannelListSnapshot {
            version: sub.version,
            generation: sub.generation.get(),
            items: sub.snapshot.into_iter().map(Into::into).collect(),
        };
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            sub.stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |upd| {
                let Ok(diffs) =
                    translate_diffs(upd.diffs, |d| FfiChannelDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                l_upd.on_update(FfiChannelListUpdate {
                    version: upd.version,
                    generation: upd.generation.get(),
                    diffs,
                });
                true
            },
            move || l_end.on_closed(),
        )
    }
}
