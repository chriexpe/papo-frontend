//! Shared evidence-aware trigger classification and committed eligibility; delivery suppression stays local.

#[cfg(test)]
mod tests;

use super::{NotificationDelivery, NotificationTarget};
use crate::{
    BackendMode, ChannelRecord, DirectRecord, Generation, ResourceSnapshot, ResourceTable, Session,
    Uuid, reading::evidence::ReadEvidence,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Admitted channel/DM metadata view consumed by notification policy.
/// Never a complete `ResourceSnapshot`: only channel and DM lists are carried here, so unloaded
/// resource groups (server, roles, members, profiles and blocks) remain structurally absent.
pub(crate) struct PolicyResources {
    generation: Generation,
    channels: ResourceTable<ChannelRecord>,
    dms: ResourceTable<DirectRecord>,
}

impl PolicyResources {
    pub(crate) fn from_snapshot(snapshot: &ResourceSnapshot) -> Self {
        Self {
            generation: snapshot.generation,
            channels: snapshot.channels.clone(),
            dms: snapshot.dms.clone(),
        }
    }

    pub(crate) fn channels_and_dms(
        generation: Generation,
        channels: Vec<ChannelRecord>,
        dms: Vec<DirectRecord>,
    ) -> Self {
        Self {
            generation,
            channels: ResourceTable::from_rows(channels.into_iter().map(|row| (row.id, row))),
            dms: ResourceTable::from_rows(dms.into_iter().map(|row| (row.id, row))),
        }
    }

    pub(crate) const fn generation(&self) -> Generation {
        self.generation
    }

    #[cfg(test)]
    pub(crate) fn channels(&self) -> &ResourceTable<ChannelRecord> {
        &self.channels
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
    enabled: Option<bool>,
    triggers_enabled: bool,
    all_messages: bool,
}

impl ChannelPolicy {
    pub(crate) fn new(
        settings: &NotificationSettings,
        backend: &papo_api::wire::Notifications,
        channel: Uuid,
        dm: bool,
        backend_mode: Option<&BackendMode>,
    ) -> Self {
        let mode = settings.channels.get(&channel).copied().unwrap_or(
            if backend_mode == Some(&BackendMode::All) {
                ChannelNotificationMode::All
            } else {
                ChannelNotificationMode::Mentions
            },
        );
        let enabled = if !settings.enabled
            || backend.enabled == Some(false)
            || settings.muted.contains(&channel)
            || mode == ChannelNotificationMode::Off
            || backend_mode == Some(&BackendMode::Off)
        {
            Some(false)
        } else if dm
            || matches!(
                backend_mode,
                Some(BackendMode::OnlyMentions | BackendMode::All)
            )
        {
            Some(true)
        } else {
            None
        };
        Self {
            enabled,
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
        matches!(self.enabled, Some(false)) || !self.triggers_enabled
    }

    /// A missing or newer backend mode may suppress every trigger for this channel.
    pub(crate) const fn badges_uncertain(self) -> bool {
        self.enabled.is_none() && self.triggers_enabled
    }

    /// None means unresolved classification could change the eligible trigger count.
    pub(crate) const fn badge_eligible(self, classification: Classification) -> Option<bool> {
        if self.badges_disabled() {
            return Some(false);
        }
        match classification {
            Classification::KnownTrigger(_) => self.enabled,
            Classification::KnownNonTrigger => Some(false),
            Classification::Unknown => None,
        }
    }

    /// Authoritative live/inbox positives survive unknown historical classification; All admits ordinary messages.
    pub(crate) const fn delivery_eligible(self, triggered: bool) -> Option<bool> {
        if self.all_messages || (triggered && self.triggers_enabled) {
            self.enabled
        } else {
            Some(false)
        }
    }
}

#[derive(Clone)]
pub(crate) struct Candidate {
    pub channel: Uuid,
    pub message: Uuid,
    pub author: Option<Uuid>,
    pub external_author: Option<String>,
    pub content: String,
    pub attachments: Vec<papo_api::wire::Attachment>,
    pub triggered: bool,
    pub read: bool,
    pub trigger_revision: Option<u64>,
    pub observed_at_us: i64,
    pub message_point: Option<crate::MessageKey>,
}
pub(super) fn contains_channel(resources: &PolicyResources, channel: Uuid) -> bool {
    resources.channels.get(channel).is_some() || resources.dms.get(channel).is_some()
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
/// A later admitted classification or mode can still permit delivery. Leave the identity unclaimed.
#[derive(Debug)]
pub(super) struct UnresolvedPolicy;

pub(super) fn decide(
    session: &Session,
    generation: crate::Generation,
    settings: &NotificationSettings,
    backend: &papo_api::wire::Notifications,
    candidate: &Candidate,
    resources: &PolicyResources,
    read_evidence: ReadEvidence,
) -> Result<Option<NotificationDelivery>, UnresolvedPolicy> {
    let dm = resources.dms.get(candidate.channel).is_some();
    let ordinary = resources.channels.get(candidate.channel);
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
        return Ok(None);
    }
    if dm && ordinary.is_some() {
        // Reading can resume before the new resource image is published. An older ambiguous
        // image cannot prove which policy applies to an otherwise deliverable candidate.
        return Err(UnresolvedPolicy);
    }
    let backend_mode = ordinary.and_then(|channel| channel.notification_settings.as_ref());
    let policy = ChannelPolicy::new(settings, backend, candidate.channel, dm, backend_mode);
    if !policy
        .delivery_eligible(candidate.triggered)
        .ok_or(UnresolvedPolicy)?
    {
        return Ok(None);
    }
    let allow_preview = settings.preview && backend.message_preview != Some(false);
    let author_name = candidate.external_author.clone().or_else(|| {
        let uid = candidate.author?;
        let snapshot = session.inner.resources.borrow();
        let user = snapshot.users.get(uid)?;
        user.nickname
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .or_else(|| {
                user.username
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
            })
            .map(str::to_owned)
    });
    Ok(Some(NotificationDelivery {
        target: NotificationTarget {
            account: session.account(),
            generation,
            channel: candidate.channel,
            message: candidate.message,
        },
        author: candidate.author,
        author_name,
        is_dm: dm,
        preview: allow_preview.then(|| candidate.content.chars().take(512).collect()),
        attachments: if allow_preview {
            candidate.attachments.clone()
        } else {
            Vec::new()
        },
        sound: settings.sound && backend.sound != Some(false),
    }))
}

/// Broad at-most-once claim admission, not badge eligibility: suppressed candidates still consume identity.
pub(super) fn claim_eligible(
    resources: &PolicyResources,
    settings: &NotificationSettings,
    candidate: &Candidate,
) -> bool {
    if candidate.triggered || resources.dms.get(candidate.channel).is_some() {
        return true;
    }
    match settings.channels.get(&candidate.channel) {
        Some(ChannelNotificationMode::All) => true,
        Some(ChannelNotificationMode::Mentions | ChannelNotificationMode::Off) => false,
        None => resources
            .channels
            .get(candidate.channel)
            .is_some_and(|channel| channel.notification_settings == Some(BackendMode::All)),
    }
}
