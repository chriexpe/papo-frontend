//! Single notification decision for all producers; backend remains the authorization authority.

use super::{NotificationDelivery, NotificationTarget};
use crate::{
    Channel, Generation, ResourceSnapshot, Session, Uuid, reading::evidence::ReadEvidence,
};
use papo_api::wire::DirectConversation;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[cfg(test)]
/// Provenance of channel/DM inputs supplied to notification policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceScope {
    /// Derived from the session's admitted `ResourceSnapshot`.
    AdmittedSnapshot,
    /// Fetched in background and admitted through `resource_admission` for channels and DMs only;
    /// server, roles, members, profiles and blocks were not loaded.
    ChannelsAndDmsOnly,
}

/// Admitted channel/DM metadata view consumed by notification policy.
/// Never impersonates a complete `ResourceSnapshot`: unloaded resource groups remain absent.
pub(crate) enum PolicyResources {
    /// Derived from the session's admitted `ResourceSnapshot`.
    AdmittedSnapshot {
        generation: Generation,
        channels: Arc<Vec<Channel>>,
        dms: Arc<Vec<DirectConversation>>,
    },
    /// Fetched in background and admitted through `resource_admission` for channels and DMs only;
    /// server, roles, members, profiles and blocks were not loaded.
    ChannelsAndDmsOnly {
        generation: Generation,
        channels: Arc<Vec<Channel>>,
        dms: Arc<Vec<DirectConversation>>,
    },
}

impl PolicyResources {
    pub(crate) fn from_snapshot(snapshot: &ResourceSnapshot) -> Self {
        Self::AdmittedSnapshot {
            generation: snapshot.generation,
            channels: Arc::clone(&snapshot.channels),
            dms: Arc::clone(&snapshot.dms),
        }
    }

    pub(crate) fn channels_and_dms(
        generation: Generation,
        channels: Vec<Channel>,
        dms: Vec<DirectConversation>,
    ) -> Self {
        Self::ChannelsAndDmsOnly {
            generation,
            channels: Arc::new(channels),
            dms: Arc::new(dms),
        }
    }

    pub(crate) const fn generation(&self) -> Generation {
        match self {
            Self::AdmittedSnapshot { generation, .. }
            | Self::ChannelsAndDmsOnly { generation, .. } => *generation,
        }
    }

    pub(crate) fn channels(&self) -> &[Channel] {
        match self {
            Self::AdmittedSnapshot { channels, .. } | Self::ChannelsAndDmsOnly { channels, .. } => {
                channels
            }
        }
    }

    pub(crate) fn dms(&self) -> &[DirectConversation] {
        match self {
            Self::AdmittedSnapshot { dms, .. } | Self::ChannelsAndDmsOnly { dms, .. } => dms,
        }
    }

    #[cfg(test)]
    pub(crate) const fn scope(&self) -> ResourceScope {
        match self {
            Self::AdmittedSnapshot { .. } => ResourceScope::AdmittedSnapshot,
            Self::ChannelsAndDmsOnly { .. } => ResourceScope::ChannelsAndDmsOnly,
        }
    }
}

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
#[derive(Clone, Serialize, Deserialize)]
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
pub(super) fn contains_channel(resources: &PolicyResources, channel: Uuid) -> bool {
    resources
        .channels()
        .iter()
        .any(|c| c.id.as_deref().and_then(|s| s.parse().ok()) == Some(channel))
        || resources
            .dms()
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
    resources: &PolicyResources,
    read_evidence: ReadEvidence,
) -> Option<NotificationDelivery> {
    let dm = resources
        .dms()
        .iter()
        .any(|d| d.id.parse().ok() == Some(candidate.channel));
    if read_evidence != ReadEvidence::Unread
        || resources.generation() != generation
        || !contains_channel(resources, candidate.channel)
        || candidate.read
        || candidate.author == Some(session.account().user())
        || !settings.enabled
        || backend.enabled == Some(false)
        || settings.muted.contains(&candidate.channel)
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
    let backend_mode = resources
        .channels()
        .iter()
        .find(|c| c.id.as_deref().and_then(|s| s.parse().ok()) == Some(candidate.channel))
        .and_then(|c| c.notification_settings.as_deref())
        .unwrap_or("only_mentions");
    if backend_mode == "off" {
        return None;
    }
    let mode = settings
        .channels
        .get(&candidate.channel)
        .copied()
        .unwrap_or(if backend_mode == "all" {
            ChannelNotificationMode::All
        } else {
            ChannelNotificationMode::Mentions
        });
    if mode == ChannelNotificationMode::Off {
        return None;
    }
    if dm {
        if !settings.direct_messages {
            return None;
        }
    } else if mode != ChannelNotificationMode::All
        && (!candidate.triggered || !settings.mentions || backend.mentions == Some(false))
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

/// Eligibility is shared with claim admission, independently of suppression.
pub(super) fn eligible(
    resources: &PolicyResources,
    settings: &NotificationSettings,
    candidate: &Candidate,
) -> bool {
    if candidate.triggered
        || resources
            .dms()
            .iter()
            .any(|dm| dm.id.parse().ok() == Some(candidate.channel))
    {
        return true;
    }
    match settings.channels.get(&candidate.channel) {
        Some(ChannelNotificationMode::All) => true,
        Some(ChannelNotificationMode::Mentions | ChannelNotificationMode::Off) => false,
        None => resources.channels().iter().any(|channel| {
            channel.id.as_deref().and_then(|id| id.parse().ok()) == Some(candidate.channel)
                && channel.notification_settings.as_deref() == Some("all")
        }),
    }
}
