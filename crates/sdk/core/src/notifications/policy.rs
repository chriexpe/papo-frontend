//! Single notification decision for all producers; backend remains the authorization authority.

use super::{NotificationDelivery, NotificationTarget};
use crate::{ResourceSnapshot, Session, Uuid};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Effective per-channel SDK device override. Backend off always wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelNotificationMode {
    /// Suppress this channel on this device.
    Off,
    /// Mentions, replies, authorized everyone and DMs only.
    Mentions,
    /// Every new message in this channel.
    All,
}
/// Durable device policy, isolated per authenticated account.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent device notification switches"
)]
pub struct NotificationSettings {
    /// Master device-account switch.
    pub enabled: bool,
    /// Permit mention/reply triggers.
    pub mentions: bool,
    /// Permit direct-conversation alerts.
    pub direct_messages: bool,
    /// Permit preview bytes in platform handoffs; default false for lock-screen privacy.
    pub preview: bool,
    /// Permit sound.
    pub sound: bool,
    /// Muted channel IDs.
    pub muted: BTreeSet<Uuid>,
    /// Device-specific channel overrides; backend off cannot be overridden.
    pub channels: BTreeMap<Uuid, ChannelNotificationMode>,
}
impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mentions: true,
            direct_messages: true,
            preview: false,
            sound: true,
            muted: BTreeSet::new(),
            channels: BTreeMap::new(),
        }
    }
}
#[derive(Clone)]
pub(crate) struct Candidate {
    pub channel: Uuid,
    pub message: Uuid,
    pub author: Option<Uuid>,
    pub content: String,
    pub triggered: bool,
    pub read: bool,
    pub trigger_revision: Option<u64>,
    pub observed_at_us: i64,
    pub message_point: Option<crate::MessageKey>,
}
pub(super) fn contains_channel(resources: &ResourceSnapshot, channel: Uuid) -> bool {
    resources
        .channels
        .iter()
        .any(|c| c.id.as_deref().and_then(|s| s.parse().ok()) == Some(channel))
        || resources
            .dms
            .iter()
            .any(|d| d.id.parse().ok() == Some(channel))
}
pub(super) fn mentions(content: &str, user: Uuid) -> bool {
    crate::mentions::contains_user_mention(content, user)
}
pub(super) fn decide(
    session: &Session,
    generation: crate::Generation,
    settings: &NotificationSettings,
    backend: &papo_api::wire::Notifications,
    candidate: &Candidate,
    resources: &ResourceSnapshot,
) -> Option<NotificationDelivery> {
    let read_locally = session
        .reading_state(candidate.channel)
        .is_some_and(|state| {
            let point = state.local_point.max(state.uploaded_point);
            candidate
                .message_point
                .is_some_and(|message| point.is_some_and(|point| message <= point))
                || point.is_some_and(|point| point.message_id == candidate.message)
        });
    if resources.generation != generation
        || candidate.author == Some(session.account().user())
        || !allows(
            resources,
            settings,
            backend,
            candidate.channel,
            candidate.triggered,
        )
        || read_locally
        || candidate.read
        || session
            .inner
            .notifications
            .context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&candidate.channel)
    {
        return None;
    }
    Some(NotificationDelivery {
        target: NotificationTarget {
            account: session.account(),
            generation,
            channel: candidate.channel,
            message: candidate.message,
        },
        preview: (settings.preview && backend.message_preview != Some(false))
            .then(|| candidate.content.chars().take(512).collect()),
        sound: settings.sound && backend.sound != Some(false),
    })
}

/// Shared eligibility; read state, visibility, previews and claims are deliberately absent.
pub(crate) fn allows(
    resources: &ResourceSnapshot,
    settings: &NotificationSettings,
    backend: &papo_api::wire::Notifications,
    channel: Uuid,
    triggered: bool,
) -> bool {
    if !contains_channel(resources, channel)
        || !settings.enabled
        || backend.enabled == Some(false)
        || settings.muted.contains(&channel)
    {
        return false;
    }
    let backend_mode = resources
        .channels
        .iter()
        .find(|c| c.id.as_deref().and_then(|id| id.parse().ok()) == Some(channel))
        .and_then(|c| c.notification_settings.as_deref())
        .unwrap_or("only_mentions");
    if backend_mode == "off" {
        return false;
    }
    let mode = settings
        .channels
        .get(&channel)
        .copied()
        .unwrap_or(if backend_mode == "all" {
            ChannelNotificationMode::All
        } else {
            ChannelNotificationMode::Mentions
        });
    if mode == ChannelNotificationMode::Off {
        return false;
    }
    if resources
        .dms
        .iter()
        .any(|dm| dm.id.parse().ok() == Some(channel))
    {
        return settings.direct_messages;
    }
    mode == ChannelNotificationMode::All
        || (triggered && settings.mentions && backend.mentions != Some(false))
}

/// Delivery ledger admission, not settings eligibility. Suppressed triggers consume identity.
pub(super) fn claimable(
    resources: &ResourceSnapshot,
    settings: &NotificationSettings,
    candidate: &Candidate,
) -> bool {
    candidate.triggered
        || resources
            .dms
            .iter()
            .any(|dm| dm.id.parse().ok() == Some(candidate.channel))
        || match settings.channels.get(&candidate.channel) {
            Some(ChannelNotificationMode::All) => true,
            Some(ChannelNotificationMode::Mentions | ChannelNotificationMode::Off) => false,
            None => resources.channels.iter().any(|channel| {
                channel.id.as_deref().and_then(|id| id.parse().ok()) == Some(candidate.channel)
                    && channel.notification_settings.as_deref() == Some("all")
            }),
        }
}
