//! Unified cross-account direct message list and admitted live server info subscriptions across FFI.
//! Per-account channel lists and read-state badges live in `channels.rs`.

use std::sync::Arc;

use papo_sdk_ui::{DirectMessageItem, DirectMessageService, ServerInfoService, ServerInfoView};

use crate::{
    channels::{FfiChannelBadges, FfiReadState},
    client::FfiClient,
    define_ffi_vector_diff,
    diffs::translate_diffs,
    errors::FfiClientError,
    ids::FfiAccountId,
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

/// Projected unified direct-conversation row across one or all signed-in accounts.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDirectMessageItem {
    /// Owning signed-in account.
    pub account: FfiAccountId,
    /// Remote DM channel UUID.
    pub channel_id: String,
    /// Resource generation owning this row.
    pub generation: u64,
    /// Other participant user UUID.
    pub user_id: String,
    /// Admitted username.
    pub username: Option<String>,
    /// Admitted nickname.
    pub nickname: Option<String>,
    /// Exact last-activity or creation timestamp string.
    pub activity_at: String,
    /// SDK reading projection if verified.
    pub unread: Option<FfiReadState>,
    /// SDK badge overlay if available.
    pub badges: Option<FfiChannelBadges>,
}

impl From<Arc<DirectMessageItem>> for FfiDirectMessageItem {
    fn from(item: Arc<DirectMessageItem>) -> Self {
        Self {
            account: FfiAccountId::from_sdk(item.id.account),
            channel_id: item.id.channel.to_string(),
            generation: item.generation.get(),
            user_id: item.user.to_string(),
            username: item.username.clone(),
            nickname: item.nickname.clone(),
            activity_at: item.activity_at.clone(),
            unread: item.unread.as_ref().map(Into::into),
            badges: item.badges.as_deref().map(Into::into),
        }
    }
}

define_ffi_vector_diff!(
    "Ordered diff over direct message conversations.",
    FfiDirectMessageDiff,
    FfiDirectMessageItem
);

/// Atomic initial baseline of direct conversations sorted newest activity first.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDirectMessageSnapshot {
    /// Projection version.
    pub version: u64,
    /// Sorted DM rows.
    pub items: Vec<FfiDirectMessageItem>,
}

/// Ordered diff batch over unified direct conversations.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDirectMessageUpdate {
    /// New projection version.
    pub version: u64,
    /// Sequential diff list.
    pub diffs: Vec<FfiDirectMessageDiff>,
}

/// Listener for unified DM list updates.
#[uniffi::export(with_foreign)]
pub trait FfiDirectMessageListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, snapshot: FfiDirectMessageSnapshot);
    /// Subsequent diff batch.
    fn on_update(&self, update: FfiDirectMessageUpdate);
    /// Terminal notification on closure.
    fn on_closed(&self);
}

/// Admitted server metadata projected for an account session.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerInfoView {
    /// Connection epoch owning this metadata.
    pub generation: u64,
    /// Admitted server UUID if synced.
    pub server_id: Option<String>,
    /// Server display name.
    pub name: Option<String>,
    /// Owner user UUID string.
    pub owner_id: Option<String>,
    /// Owner username.
    pub owner_username: Option<String>,
    /// Public vs server-password flag.
    pub public_server: Option<bool>,
    /// Declared icon format if any (fetch bytes via `FfiMediaSource::ServerIcon`).
    pub icon_format: Option<String>,
    /// Server creation timestamp string.
    pub created_at: Option<String>,
    /// Server channel count reported by server metadata.
    pub channel_count: Option<i64>,
    /// Server member count reported by server metadata.
    pub member_count: Option<i64>,
    /// Server role count reported by server metadata.
    pub role_count: Option<i64>,
}

impl From<ServerInfoView> for FfiServerInfoView {
    fn from(value: ServerInfoView) -> Self {
        match value.server {
            Some(server) => Self {
                generation: value.generation.get(),
                server_id: Some(server.id.to_string()),
                name: server.name.clone(),
                owner_id: server.owner_id.map(|u| u.to_string()),
                owner_username: server.owner_username.clone(),
                public_server: server.public,
                icon_format: server.icon_format.clone(),
                created_at: server.created_at.clone(),
                channel_count: server.channel_count,
                member_count: server.member_count,
                role_count: server.role_count,
            },
            None => Self {
                generation: value.generation.get(),
                server_id: None,
                name: None,
                owner_id: None,
                owner_username: None,
                public_server: None,
                icon_format: None,
                created_at: None,
                channel_count: None,
                member_count: None,
                role_count: None,
            },
        }
    }
}

/// Listener for live server info changes.
#[uniffi::export(with_foreign)]
pub trait FfiServerInfoListener: Send + Sync {
    /// Atomic baseline server view.
    fn on_snapshot(&self, view: FfiServerInfoView);
    /// Subsequent updates on metadata or epoch changes.
    fn on_update(&self, view: FfiServerInfoView);
    /// Terminal notification on session closure.
    fn on_closed(&self);
}

#[uniffi::export]
impl FfiClient {
    /// Subscribes to unified DM rows dynamically following all signed-in accounts on this client.
    ///
    /// # Errors
    /// Returns shutdown or worker errors.
    pub fn subscribe_direct_messages(
        &self,
        listener: Arc<dyn FfiDirectMessageListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let sub = DirectMessageService::for_client(self.handle.clone()).subscribe();
        let baseline = FfiDirectMessageSnapshot {
            version: sub.version,
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
                    translate_diffs(upd.diffs, |d| FfiDirectMessageDiff::from_sdk(d, Into::into))
                else {
                    return false;
                };
                l_upd.on_update(FfiDirectMessageUpdate {
                    version: upd.version,
                    diffs,
                });
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Subscribes to live admitted server metadata for an account session.
    ///
    /// # Errors
    /// Returns missing/revoked session or worker failures.
    pub fn subscribe_server_info(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiServerInfoListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let sub = ServerInfoService::new(session).subscribe();
        let baseline = FfiServerInfoView::from(sub.snapshot);
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
