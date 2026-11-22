//! Notification target validation, inbox records, settings, status, and push metadata across FFI.
//! Delivery subscriptions, policy RPCs, and push lifecycle methods live in `notifications.rs`.

use std::collections::{BTreeMap, BTreeSet};

use papo_sdk::{
    ChannelNotificationMode, InboxCursor, NotificationDelivery, NotificationRecord,
    NotificationSettings, NotificationStatus, NotificationTarget, PushDevice, PushRegistration,
};

use crate::{
    ids::{FfiAccountId, parse_uuid, parse_uuid_list},
    messages::FfiExternalAuthor,
    operation_errors::FfiNotificationError,
};

/// Scoped target validated before notification display or tap navigation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiNotificationTarget {
    /// Owning signed-in account.
    pub account: FfiAccountId,
    /// Validating connection generation.
    pub generation: u64,
    /// Target channel or DM UUID string.
    pub channel_id: String,
    /// Canonical message UUID string.
    pub message_id: String,
}

impl FfiNotificationTarget {
    /// Converts to SDK `NotificationTarget`.
    ///
    /// # Errors
    /// Returns [`FfiNotificationError::InvalidData`] if account or UUIDs fail validation.
    pub(crate) fn into_sdk(self) -> Result<NotificationTarget, FfiNotificationError> {
        Ok(NotificationTarget {
            account: self
                .account
                .into_sdk()
                .map_err(|_| FfiNotificationError::InvalidData)?,
            generation: papo_sdk::Generation::from_raw(self.generation),
            channel: parse_uuid(&self.channel_id).map_err(|_| FfiNotificationError::InvalidData)?,
            message: parse_uuid(&self.message_id).map_err(|_| FfiNotificationError::InvalidData)?,
        })
    }
}

/// At-most-once FULL-committed platform handoff claim emitted by live or background sync.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiNotificationDelivery {
    /// Scoped account/channel/message target (revalidate via `validate_notification_target` before showing/tapping).
    pub target: FfiNotificationTarget,
    /// Optional privacy-filtered preview text (absent when policy disallows lock-screen preview).
    pub preview: Option<String>,
    /// True if shared policy allows notification sound.
    pub sound: bool,
}

/// Converts `papo_sdk::NotificationDelivery` into FFI record.
pub(crate) fn map_delivery(d: NotificationDelivery) -> FfiNotificationDelivery {
    FfiNotificationDelivery {
        target: FfiNotificationTarget {
            account: FfiAccountId::from_sdk(d.target.account),
            generation: d.target.generation.get(),
            channel_id: d.target.channel.to_string(),
            message_id: d.target.message.to_string(),
        },
        preview: d.preview,
        sound: d.sound,
    }
}

/// Exact backend cursor tuple for inbox pagination.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiInboxCursor {
    /// Exact notification creation timestamp.
    pub created_at: String,
    /// Notification row UUID string.
    pub id: String,
}

impl From<&InboxCursor> for FfiInboxCursor {
    fn from(v: &InboxCursor) -> Self {
        Self {
            created_at: v.created_at.clone(),
            id: v.id.clone(),
        }
    }
}

impl From<FfiInboxCursor> for InboxCursor {
    fn from(v: FfiInboxCursor) -> Self {
        Self {
            created_at: v.created_at,
            id: v.id,
        }
    }
}

/// Single authoritative backend notification inbox row with provenance.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiNotificationRecord {
    /// Owning account.
    pub account: FfiAccountId,
    /// Generation that fetched this inbox row.
    pub generation: u64,
    /// Notification row UUID.
    pub id: Option<String>,
    /// Canonical message UUID.
    pub message_id: Option<String>,
    /// Channel or DM UUID.
    pub channel_id: Option<String>,
    /// Author user UUID.
    pub author_id: Option<String>,
    /// Webhook UUID if posted by webhook.
    pub webhook_id: Option<String>,
    /// External bridged author details.
    pub external_author: Option<FfiExternalAuthor>,
    /// Preview message content string.
    pub message_content: Option<String>,
    /// Backend read flag.
    pub read: Option<bool>,
    /// Exact backend creation timestamp.
    pub created_at: Option<String>,
    /// True if message was an actual direct mention of this user.
    pub actual_mention: bool,
}

impl From<&NotificationRecord> for FfiNotificationRecord {
    fn from(r: &NotificationRecord) -> Self {
        Self {
            account: FfiAccountId::from_sdk(r.account),
            generation: r.generation.get(),
            id: r.record.id.clone(),
            message_id: r.record.message_id.clone(),
            channel_id: r.record.channel_id.clone(),
            author_id: r.record.author_id.clone(),
            webhook_id: r.record.webhook_id.clone(),
            external_author: r.record.external_author.as_ref().map(Into::into),
            message_content: r.record.message_content.clone(),
            read: r.record.read,
            created_at: r.record.created_at.clone(),
            actual_mention: r.actual_mention,
        }
    }
}

/// One account's authoritative notification inbox page.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiInboxPage {
    /// Notification rows in backend order.
    pub records: Vec<FfiNotificationRecord>,
    /// Continuation cursor when `has_more` is true.
    pub next_cursor: Option<FfiInboxCursor>,
}

/// Multi-account inbox outcome (isolation prevents one failing server hiding others).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAccountInbox {
    /// Complete inbox page loaded for `account`.
    Ready {
        /// Owning account.
        account: FfiAccountId,
        /// Loaded page.
        page: FfiInboxPage,
    },
    /// Failed independently; offer user retry for `account`.
    Retry {
        /// Failed account.
        account: FfiAccountId,
    },
}

/// Device-local notification mode override for one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiChannelNotificationMode {
    /// Suppress alerts on this channel on this device.
    Off,
    /// Alerts for mentions, replies, and DMs only.
    Mentions,
    /// Alerts for every message in this channel.
    All,
}

/// Per-channel override entry inside `FfiNotificationSettings`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelNotificationOverride {
    /// Channel or DM UUID string.
    pub channel_id: String,
    /// Device mode override.
    pub mode: FfiChannelNotificationMode,
}

/// Durable device-level notification settings scoped per signed-in account.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
#[allow(clippy::struct_excessive_bools)]
pub struct FfiNotificationSettings {
    /// Master switch for notifications on this account.
    pub enabled: bool,
    /// Notify on mentions/replies.
    pub mentions: bool,
    /// Notify on DMs.
    pub direct_messages: bool,
    /// Allow message text preview in OS notifications.
    pub preview: bool,
    /// Play sound.
    pub sound: bool,
    /// Explicitly muted channel UUID strings.
    pub muted_channel_ids: Vec<String>,
    /// Explicit per-channel override modes.
    pub channel_overrides: Vec<FfiChannelNotificationOverride>,
}

/// Observable notification producer health.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiNotificationStatus {
    /// Producer healthy.
    Ready,
    /// Producer fault; manual inbox refresh still available.
    Fault,
    /// Account revoked.
    Closed,
}

/// Queue overflow counters without leaking message IDs or contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiNotificationDiagnostics {
    /// Deferred evidence candidates dropped on full queue.
    pub deferred_evictions: u64,
    /// Deferred lookup candidates dropped on full queue.
    pub lookup_evictions: u64,
}

/// Host push device registration metadata (token never enters SDK SQLite or logs).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPushDevice {
    /// Provider token (e.g., FCM token).
    pub token: String,
    /// Platform tag (`android`).
    pub platform: String,
    /// Non-secret user-visible device label.
    pub device_name: Option<String>,
    /// Push provider name (`fcm`).
    pub provider: String,
}

/// Push token registration state machine result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPushRegistration {
    /// Backend confirmed active token ownership.
    Registered,
    /// Backend confirmed removal or scoped 404.
    Removed,
    /// Network write outcome uncertain; host should explicitly reconcile.
    Unknown,
}

/// Listener for live FULL-committed notification deliveries claimed on an account.
#[uniffi::export(with_foreign)]
pub trait FfiNotificationDeliveryListener: Send + Sync {
    /// Emitted whenever a live notification is claimed at most once for this device.
    fn on_delivery(&self, delivery: FfiNotificationDelivery);
    /// Terminal notification on closure or fault.
    fn on_closed(&self);
}

/// Listener for notification producer health updates.
#[uniffi::export(with_foreign)]
pub trait FfiNotificationStatusListener: Send + Sync {
    /// Atomic initial status.
    fn on_snapshot(&self, status: FfiNotificationStatus);
    /// Subsequent status updates.
    fn on_update(&self, status: FfiNotificationStatus);
    /// Terminal notification on account revocation.
    fn on_closed(&self);
}

/// Maps SDK `NotificationStatus` into FFI enum.
pub(crate) fn map_status(s: NotificationStatus) -> FfiNotificationStatus {
    match s {
        NotificationStatus::Ready => FfiNotificationStatus::Ready,
        NotificationStatus::Fault => FfiNotificationStatus::Fault,
        // Closed and future variants collapse cleanly to Closed.
        _ => FfiNotificationStatus::Closed,
    }
}

/// Maps SDK `NotificationSettings` into FFI record.
pub(crate) fn map_settings_from_sdk(s: &NotificationSettings) -> FfiNotificationSettings {
    FfiNotificationSettings {
        enabled: s.enabled,
        mentions: s.mentions,
        direct_messages: s.direct_messages,
        preview: s.preview,
        sound: s.sound,
        muted_channel_ids: s.muted.iter().map(ToString::to_string).collect(),
        channel_overrides: s
            .channels
            .iter()
            .map(|(ch, mode)| FfiChannelNotificationOverride {
                channel_id: ch.to_string(),
                mode: match mode {
                    ChannelNotificationMode::Off => FfiChannelNotificationMode::Off,
                    ChannelNotificationMode::Mentions => FfiChannelNotificationMode::Mentions,
                    ChannelNotificationMode::All => FfiChannelNotificationMode::All,
                },
            })
            .collect(),
    }
}

/// Converts FFI notification settings into SDK settings with UUID validation.
///
/// # Errors
/// Returns [`FfiNotificationError::InvalidData`] if any channel UUID is malformed.
pub(crate) fn map_settings_to_sdk(
    s: FfiNotificationSettings,
) -> Result<NotificationSettings, FfiNotificationError> {
    let muted: BTreeSet<_> = parse_uuid_list(s.muted_channel_ids)
        .map_err(|_| FfiNotificationError::InvalidData)?
        .into_iter()
        .collect();
    let mut channels = BTreeMap::new();
    for entry in s.channel_overrides {
        let ch = parse_uuid(&entry.channel_id).map_err(|_| FfiNotificationError::InvalidData)?;
        let mode = match entry.mode {
            FfiChannelNotificationMode::Off => ChannelNotificationMode::Off,
            FfiChannelNotificationMode::Mentions => ChannelNotificationMode::Mentions,
            FfiChannelNotificationMode::All => ChannelNotificationMode::All,
        };
        channels.insert(ch, mode);
    }
    Ok(NotificationSettings {
        enabled: s.enabled,
        mentions: s.mentions,
        direct_messages: s.direct_messages,
        preview: s.preview,
        sound: s.sound,
        muted,
        channels,
    })
}

/// Converts FFI push device into SDK `PushDevice`.
pub(crate) fn map_push_device(d: FfiPushDevice) -> PushDevice {
    PushDevice {
        token: d.token,
        platform: d.platform,
        device_name: d.device_name,
        provider: d.provider,
    }
}

/// Maps SDK `PushRegistration` into FFI enum.
pub(crate) fn map_push_reg(r: PushRegistration) -> FfiPushRegistration {
    match r {
        PushRegistration::Registered => FfiPushRegistration::Registered,
        PushRegistration::Removed => FfiPushRegistration::Removed,
        // Unknown and future variants map to Unknown.
        _ => FfiPushRegistration::Unknown,
    }
}
