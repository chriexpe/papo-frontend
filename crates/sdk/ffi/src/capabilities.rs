//! Live channel activity (presence, typing) and server/channel capability projections across FFI.
//! All permission math remains inside `papo-sdk` / `papo-sdk-ui` (authoritative backend checks apply).

use std::sync::Arc;

use papo_sdk::{Presence, PresenceStatus};
use papo_sdk_ui::{
    ActivityService, ActivityView, CapabilityService, CapabilityView, ChannelCapabilities,
    ServerCapabilities, TypingItem,
};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid},
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

/// Live presence state for one user on an account generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPresenceStatus {
    /// Actively online.
    Online,
    /// Automatically or manually away.
    Away,
    /// Manually busy.
    Busy,
    /// Unrecognized backend presence string.
    Unknown,
}

/// User presence record without guessed local fallbacks.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPresenceItem {
    /// User UUID string.
    pub user_id: String,
    /// Backend presence state.
    pub status: FfiPresenceStatus,
    /// Optional backend custom status phrase.
    pub status_message: Option<String>,
    /// Optional typing phrase delivered in a live presence update.
    pub typing_phrase: Option<String>,
    /// Optional nickname delivered in a live presence update.
    pub nickname: Option<String>,
}

impl From<Presence> for FfiPresenceItem {
    fn from(value: Presence) -> Self {
        let status = match value.status {
            PresenceStatus::Online => FfiPresenceStatus::Online,
            PresenceStatus::Away => FfiPresenceStatus::Away,
            PresenceStatus::Busy => FfiPresenceStatus::Busy,
            // Unknown and future backend variants collapse to Unknown without guessing.
            _ => FfiPresenceStatus::Unknown,
        };
        Self {
            user_id: value.user.to_string(),
            status,
            status_message: value.status_message,
            typing_phrase: value.typing_phrase,
            nickname: value.nickname,
        }
    }
}

/// Active typist in a channel.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTypingItem {
    /// User UUID string.
    pub user_id: String,
    /// Current nickname/member label.
    pub nickname: Option<String>,
    /// Backend custom phrase.
    pub phrase: Option<String>,
}

impl From<TypingItem> for FfiTypingItem {
    fn from(value: TypingItem) -> Self {
        Self {
            user_id: value.user.to_string(),
            nickname: value.nickname,
            phrase: value.phrase,
        }
    }
}

/// Live channel activity projection.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiActivityView {
    /// Session generation owning this projection.
    pub generation: u64,
    /// Live user presence entries (empty when offline/stale).
    pub presence: Vec<FfiPresenceItem>,
    /// Active typists (10-second SDK timeout).
    pub typing: Vec<FfiTypingItem>,
}

impl From<ActivityView> for FfiActivityView {
    fn from(value: ActivityView) -> Self {
        Self {
            generation: value.generation.get(),
            presence: value.presence.into_iter().map(Into::into).collect(),
            typing: value.typing.into_iter().map(Into::into).collect(),
        }
    }
}

/// Listener for live channel presence and typing updates.
#[uniffi::export(with_foreign)]
pub trait FfiActivityListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, view: FfiActivityView);
    /// Subsequent live activity changes.
    fn on_update(&self, view: FfiActivityView);
    /// Terminal notification on closure.
    fn on_closed(&self);
}

/// Server-wide UI capability flags computed by `papo-sdk-ui`.
// Mirrors `papo_sdk_ui::ServerCapabilities` boolean affordance flags verbatim.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerCapabilities {
    /// Manage server settings.
    pub manage_server: bool,
    /// Create/edit/delete/move channels.
    pub manage_channels: bool,
    /// Create/edit/delete/assign roles.
    pub manage_roles: bool,
    /// Manage server webhooks.
    pub manage_webhooks: bool,
    /// Kick/ban non-owner members.
    pub ban_nonowner: bool,
    /// Server owner ability to reset another user's password.
    pub reset_other_password: bool,
}

impl From<ServerCapabilities> for FfiServerCapabilities {
    fn from(v: ServerCapabilities) -> Self {
        Self {
            manage_server: v.manage_server,
            manage_channels: v.manage_channels,
            manage_roles: v.manage_roles,
            manage_webhooks: v.manage_webhooks,
            ban_nonowner: v.ban_nonowner,
            reset_other_password: v.reset_other_password,
        }
    }
}

/// Per-channel UI capability flags computed by `papo-sdk-ui`.
// Mirrors `papo_sdk_ui::ChannelCapabilities` boolean affordance flags verbatim.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelCapabilities {
    /// Can read history, pins, and remove own reactions.
    pub read: bool,
    /// Can send messages and add reactions.
    pub send: bool,
    /// Can send file attachments.
    pub attach: bool,
    /// Can pin messages.
    pub pin: bool,
    /// Can unpin messages.
    pub unpin: bool,
    /// Can add reactions.
    pub add_reaction: bool,
    /// Can remove own reactions.
    pub remove_reaction: bool,
    /// Can report typing.
    pub type_here: bool,
    /// Can connect to voice channel.
    pub connect_voice: bool,
    /// Can delete others' messages.
    pub delete_others: bool,
    /// True when direct conversation.
    pub direct: bool,
}

impl From<ChannelCapabilities> for FfiChannelCapabilities {
    fn from(v: ChannelCapabilities) -> Self {
        Self {
            read: v.read,
            send: v.send,
            attach: v.attach,
            pin: v.pin,
            unpin: v.unpin,
            add_reaction: v.add_reaction,
            remove_reaction: v.remove_reaction,
            type_here: v.type_here,
            connect_voice: v.connect_voice,
            delete_others: v.delete_others,
            direct: v.direct,
        }
    }
}

/// Admitted UI capability snapshot for a specific channel under one account generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCapabilityView {
    /// Session connection epoch owning these flags.
    pub generation: u64,
    /// Server-wide UI affordances if admitted and online; `None` if unavailable/stale.
    pub server: Option<FfiServerCapabilities>,
    /// Target channel UI affordances if admitted and online; `None` if unavailable/denied/stale.
    pub channel: Option<FfiChannelCapabilities>,
}

impl From<CapabilityView> for FfiCapabilityView {
    fn from(value: CapabilityView) -> Self {
        Self {
            generation: value.generation.get(),
            server: value
                .global
                .ok()
                .map(ServerCapabilities::from_grants)
                .map(Into::into),
            channel: value.channel.ok().map(Into::into),
        }
    }
}

/// Listener for reactive channel and server capability snapshots.
#[uniffi::export(with_foreign)]
pub trait FfiCapabilityListener: Send + Sync {
    /// Initial atomic capability view.
    fn on_snapshot(&self, view: FfiCapabilityView);
    /// Subsequent capability views on resource or connection changes.
    fn on_update(&self, view: FfiCapabilityView);
    /// Terminal notification on closure.
    fn on_closed(&self);
}

#[uniffi::export]
impl FfiClient {
    /// Sends a best-effort live typing command in `channel_id`.
    ///
    /// # Errors
    /// Returns [`FfiClientError::InvalidArgument`], offline/lifecycle, or session errors.
    pub fn report_typing(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<(), FfiClientError> {
        let session = self.session_by_id(&account)?;
        let channel = parse_uuid(&channel_id)?;
        ActivityService::new(session, channel)
            .report_typing()
            .map_err(FfiClientError::from)
    }

    /// Subscribes to live presence and typing state for `(account, channel_id)`.
    ///
    /// # Errors
    /// Returns invalid argument, missing session, or worker failures.
    pub fn subscribe_activity(
        &self,
        account: FfiAccountId,
        channel_id: String,
        listener: Arc<dyn FfiActivityListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let channel = parse_uuid(&channel_id)?;
        let sub = ActivityService::new(session, channel).subscribe();
        let baseline = FfiActivityView::from(sub.snapshot);
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
            move |view| {
                l_snap.on_snapshot(view);
                true
            },
            move |view| {
                l_upd.on_update(view.into());
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Reads current online global server capabilities without subscribing.
    ///
    /// # Errors
    /// Returns missing session or `NotFound` / `SignedOut` when unavailable.
    pub fn server_capabilities(
        &self,
        account: FfiAccountId,
    ) -> Result<Option<FfiServerCapabilities>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        Ok(CapabilityService::new(session)
            .global()
            .ok()
            .map(|g| ServerCapabilities::from_grants(g.value).into()))
    }

    /// Subscribes to reactive server and channel capabilities for `(account, channel_id)`.
    ///
    /// # Errors
    /// Returns invalid argument, missing session, or worker failures.
    pub fn subscribe_channel_capabilities(
        &self,
        account: FfiAccountId,
        channel_id: String,
        listener: Arc<dyn FfiCapabilityListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let channel = parse_uuid(&channel_id)?;
        let sub = CapabilityService::new(session).subscribe(channel);
        let baseline = FfiCapabilityView::from(sub.snapshot);
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
            move |view| {
                l_snap.on_snapshot(view);
                true
            },
            move |view| {
                l_upd.on_update(view.into());
                true
            },
            move || l_end.on_closed(),
        )
    }
}
