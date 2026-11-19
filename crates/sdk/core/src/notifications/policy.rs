//! Shared evidence-aware trigger classification and committed eligibility; delivery suppression stays local.

#[cfg(test)]
mod tests;

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

/// Admitted channel/DM metadata view consumed by notification policy.
/// Never a complete `ResourceSnapshot`: only channel and DM lists are carried here, so unloaded
/// resource groups (server, roles, members, profiles and blocks) remain structurally absent.
pub(crate) struct PolicyResources {
    generation: Generation,
    channels: Arc<Vec<Channel>>,
    dms: Arc<Vec<DirectConversation>>,
}

impl PolicyResources {
    pub(crate) fn from_snapshot(snapshot: &ResourceSnapshot) -> Self {
        Self {
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
        Self {
            generation,
            channels: Arc::new(channels),
            dms: Arc::new(dms),
        }
    }

    pub(crate) const fn generation(&self) -> Generation {
        self.generation
    }

    pub(crate) fn channels(&self) -> &[Channel] {
        &self.channels
    }

    pub(crate) fn dms(&self) -> &[DirectConversation] {
        &self.dms
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
/// Independently established positive evidence; current roles never prove historical authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Provenance {
    DirectMention,
    ReplyToAccount,
    DirectMessage,
}

/// Historical trigger evidence, before current committed settings are applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Classification {
    KnownTrigger(Provenance),
    KnownNonTrigger,
    Unknown,
}

/// Classifies held immutable message evidence without fetching parents or reconstructing permissions.
/// `parent_author` must belong to this message's held parent; absent/deleted/malformed parents stay unknown.
pub(crate) fn classify(
    message: &crate::Message,
    account: Uuid,
    dm: bool,
    parent_author: Option<Uuid>,
) -> Classification {
    let author = message.author_id.as_deref().and_then(|id| id.parse().ok());
    if author == Some(account) {
        return Classification::KnownNonTrigger;
    }
    let content = message.content.as_deref();
    if let Some(Classification::KnownTrigger(provenance)) =
        content.map(|content| classify_direct(content, account))
    {
        return Classification::KnownTrigger(provenance);
    }
    let parent = message
        .reply_to
        .as_deref()
        .and_then(|id| id.parse::<Uuid>().ok());
    if parent.is_some() && parent_author == Some(account) {
        return Classification::KnownTrigger(Provenance::ReplyToAccount);
    }
    let webhook = message
        .webhook_id
        .as_deref()
        .and_then(|id| id.parse::<Uuid>().ok())
        .is_some();
    if dm && (author.is_some() || message.author_id.is_none() || webhook) {
        return Classification::KnownTrigger(Provenance::DirectMessage);
    }
    // The pin cannot broadcast everyone without a user author. A webhook can still mention directly.
    let everyone_unknown = message
        .author_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
        && content.is_some_and(crate::mentions::contains_everyone_mention);
    if content.is_none()
        || (dm && author.is_none() && !webhook)
        || (message.reply_to.is_some() && (parent.is_none() || parent_author.is_none()))
        || everyone_unknown
    {
        Classification::Unknown
    } else {
        Classification::KnownNonTrigger
    }
}

/// Small effective image of committed channel settings, shared by badges and delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChannelPolicy {
    enabled: bool,
    triggers_enabled: bool,
    all_messages: bool,
}

impl ChannelPolicy {
    pub(crate) fn new(
        settings: &NotificationSettings,
        backend: &papo_api::wire::Notifications,
        channel: Uuid,
        dm: bool,
        backend_mode: Option<&str>,
    ) -> Self {
        let mode =
            settings
                .channels
                .get(&channel)
                .copied()
                .unwrap_or(if backend_mode == Some("all") {
                    ChannelNotificationMode::All
                } else {
                    ChannelNotificationMode::Mentions
                });
        Self {
            enabled: settings.enabled
                && backend.enabled != Some(false)
                && !settings.muted.contains(&channel)
                && backend_mode != Some("off")
                && mode != ChannelNotificationMode::Off,
            triggers_enabled: if dm {
                settings.direct_messages
            } else {
                settings.mentions && backend.mentions != Some(false)
            },
            all_messages: if dm {
                settings.direct_messages
            } else {
                mode == ChannelNotificationMode::All
            },
        }
    }

    /// True when no trigger could count, even without history or a read boundary.
    pub(crate) const fn badges_disabled(self) -> bool {
        !self.enabled || !self.triggers_enabled
    }

    /// None means unresolved classification could change the eligible trigger count.
    pub(crate) const fn badge_eligible(self, classification: Classification) -> Option<bool> {
        if self.badges_disabled() {
            return Some(false);
        }
        match classification {
            Classification::KnownTrigger(_) => Some(true),
            Classification::KnownNonTrigger => Some(false),
            Classification::Unknown => None,
        }
    }

    /// Authoritative live/inbox positives survive unknown historical classification; All admits ordinary messages.
    pub(crate) const fn delivery_eligible(self, triggered: bool) -> bool {
        self.enabled && (self.all_messages || (triggered && self.triggers_enabled))
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
/// Direct tokens independently prove a trigger; other branches need more than live content evidence.
pub(super) fn classify_direct(content: &str, account: Uuid) -> Classification {
    if crate::mentions::contains_user_mention(content, account) {
        Classification::KnownTrigger(Provenance::DirectMention)
    } else {
        Classification::Unknown
    }
}

pub(super) fn mentions(content: &str, user: Uuid) -> bool {
    matches!(
        classify_direct(content, user),
        Classification::KnownTrigger(_)
    )
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
        .and_then(|c| c.notification_settings.as_deref());
    let policy = ChannelPolicy::new(settings, backend, candidate.channel, dm, backend_mode);
    if !policy.delivery_eligible(candidate.triggered) {
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

/// Broad at-most-once claim admission, not badge eligibility: suppressed candidates still consume identity.
pub(super) fn claim_eligible(
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
