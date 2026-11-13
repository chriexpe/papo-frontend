//! Presence and typing affordances from generation-scoped SDK memory; custom phrases never create typing activity.

use futures_util::{Stream, StreamExt, stream};
use papo_sdk::{
    ActivitySnapshot, ConnectionState, Generation, Presence, ResourceSnapshot, Session, Uuid,
};
use std::{pin::Pin, sync::Arc};

/// One active typist. Clients own ellipsis, pluralization and avatar presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypingItem {
    /// Backend person identity.
    pub user: Uuid,
    /// Current live nickname, falling back to requested profile or lightweight member metadata.
    pub nickname: Option<String>,
    /// Backend custom phrase, without localized fallback text.
    pub phrase: Option<String>,
}
/// Current live channel activity, independent of durable member/profile projections.
#[derive(Clone, Debug)]
pub struct ActivityView {
    /// Connection owning these entries.
    pub generation: Generation,
    /// Live backend presence. Empty while stale/offline.
    pub presence: Vec<Presence>,
    /// Active other users in this channel, expiring after ten seconds in the SDK.
    pub typing: Vec<TypingItem>,
}
/// An atomic activity baseline and coalesced subsequent views.
pub struct ActivitySubscription {
    /// Current view at subscription capture.
    pub snapshot: ActivityView,
    /// Views after any relevant live/resource/connection change; ends on revocation.
    pub stream: Pin<Box<dyn Stream<Item = ActivityView> + Send>>,
}
/// Read-only channel activity plus best-effort typing reporting, without native UI or durable state.
#[derive(Clone)]
pub struct ActivityService {
    session: Session,
    channel: Uuid,
}
impl ActivityService {
    /// Binds to an account and channel, without loading a roster/profile/media.
    #[must_use]
    pub fn new(session: Session, channel: Uuid) -> Self {
        Self { session, channel }
    }
    /// Reports a start on this generation; no stop command or automatic replay exists in the backend.
    /// # Errors
    /// Returns a lifecycle error if the session lacks a current online connection.
    pub fn report_typing(&self) -> Result<(), papo_sdk::ClientError> {
        self.session.report_typing(self.channel)
    }
    /// Captures the activity watch before reading related metadata, so no event falls between capture and stream.
    #[must_use]
    pub fn subscribe(&self) -> ActivitySubscription {
        let mut activity = self.session.subscribe_activity();
        let mut resources = self.session.subscribe_resources();
        let mut connection = self.session.subscribe_connection();
        let live = activity.borrow_and_update().clone();
        let image = Arc::clone(&resources.borrow_and_update());
        let status = *connection.borrow_and_update();
        let snapshot = project(&self.session, self.channel, &live, &image, status);
        let session = self.session.clone();
        let channel = self.channel;
        let changes = stream::unfold(
            (activity, resources, connection, session),
            move |(mut activity, mut resources, mut connection, session)| async move {
                tokio::select! {
                    result = activity.changed() => result.ok()?,
                    result = resources.changed() => result.ok()?,
                    result = connection.changed() => result.ok()?,
                }
                if session.is_closed() {
                    return None;
                }
                let live = activity.borrow_and_update().clone();
                let image = Arc::clone(&resources.borrow_and_update());
                let status = *connection.borrow_and_update();
                let view = project(&session, channel, &live, &image, status);
                Some((view, (activity, resources, connection, session)))
            },
        );
        ActivitySubscription {
            snapshot,
            stream: Box::pin(changes.fuse()),
        }
    }
}
fn project(
    session: &Session,
    channel: Uuid,
    activity: &ActivitySnapshot,
    resources: &ResourceSnapshot,
    status: papo_sdk::ConnectionStatus,
) -> ActivityView {
    let mut view = ActivityView {
        generation: status.generation,
        presence: Vec::new(),
        typing: Vec::new(),
    };
    if session.is_closed()
        || status.state != ConnectionState::Online
        || activity.generation != status.generation
        || session.generation() != status.generation
    {
        return view;
    }
    view.presence = activity.presence.values().cloned().collect();
    let matching_resources = resources.generation == activity.generation;
    view.typing = activity
        .typing
        .values()
        .filter(|entry| {
            entry.channel == channel
                && entry.user != session.account().user()
                && entry.expires_at > tokio::time::Instant::now()
        })
        .map(|entry| {
            let id = entry.user.to_string();
            let presence = activity.presence.get(&entry.user);
            let profile = matching_resources
                .then(|| {
                    resources
                        .profiles
                        .iter()
                        .find(|profile| profile.id.as_deref() == Some(id.as_str()))
                })
                .flatten();
            let member = matching_resources
                .then(|| {
                    resources
                        .members
                        .iter()
                        .find(|member| member.id.as_deref() == Some(id.as_str()))
                })
                .flatten();
            TypingItem {
                user: entry.user,
                nickname: presence
                    .and_then(|presence| presence.nickname.clone())
                    .or_else(|| profile.and_then(|profile| profile.nickname.clone()))
                    .or_else(|| member.and_then(|member| member.nickname.clone())),
                phrase: presence
                    .and_then(|presence| presence.typing_phrase.clone())
                    .or_else(|| profile.and_then(|profile| profile.typing.clone()))
                    .or_else(|| member.and_then(|member| member.typing.clone())),
            }
        })
        .collect();
    view
}
