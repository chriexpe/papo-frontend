//! An account's live activity from generation-scoped SDK memory: each person's presence mark
//! and status phrase, and who is typing in which channel. Custom phrases never create typing
//! activity.
//!
//! Owns the projection only; the live state, its expiry and reporting are the SDK's, and
//! wording, plurals and drawing are the client's.

use futures_util::{Stream, StreamExt, stream};
use papo_sdk::{
    ActivitySnapshot, ConnectionState, Generation, Presence, PresenceStatus, ResourceSnapshot,
    Session, Uuid,
};
use std::{collections::BTreeMap, pin::Pin, sync::Arc};

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

/// The mark a person's presence draws, a shape first and a colour second.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresenceMark {
    /// Connected; also a backend status this SDK does not know yet, which still means connected.
    Available,
    /// Backend automatic or manual away.
    Away,
    /// Backend manual busy.
    Busy,
    /// No live connection.
    Offline,
}

/// An account's current live activity.
#[derive(Clone, Debug)]
pub struct AccountActivity {
    /// Connection owning these entries.
    pub generation: Generation,
    /// Whether this reads a current online connection. While false nobody's presence is known
    /// and nobody types.
    pub live: bool,
    /// Live backend presence of everyone connected, by person.
    pub presence: BTreeMap<Uuid, Presence>,
    /// Other people typing, by channel, each in a stable order. Entries expire ten seconds
    /// after their newest start in the SDK.
    pub typing: BTreeMap<Uuid, Vec<TypingItem>>,
}

impl AccountActivity {
    /// Person `user`'s mark; none while not live.
    #[must_use]
    pub fn presence_mark(&self, user: Uuid) -> Option<PresenceMark> {
        if !self.live {
            return None;
        }
        let mark = match self.presence.get(&user).map(|presence| presence.status) {
            None => PresenceMark::Offline,
            Some(PresenceStatus::Away) => PresenceMark::Away,
            Some(PresenceStatus::Busy) => PresenceMark::Busy,
            Some(_) => PresenceMark::Available,
        };
        Some(mark)
    }

    /// Person `user`'s custom status phrase while connected.
    #[must_use]
    pub fn status_phrase(&self, user: Uuid) -> Option<&str> {
        self.presence.get(&user)?.status_message.as_deref()
    }

    /// Who else is typing in `channel`.
    #[must_use]
    pub fn typing_in(&self, channel: Uuid) -> &[TypingItem] {
        self.typing.get(&channel).map_or(&[], Vec::as_slice)
    }
}

/// An atomic activity baseline and coalesced subsequent views.
pub struct ActivitySubscription {
    /// Current view at subscription capture.
    pub snapshot: AccountActivity,
    /// Views after any relevant live/resource/connection change; ends on revocation.
    pub stream: Pin<Box<dyn Stream<Item = AccountActivity> + Send>>,
}

/// Read-only account activity plus best-effort typing reporting, without native UI or durable
/// state.
#[derive(Clone)]
pub struct ActivityService {
    session: Session,
}

impl ActivityService {
    /// Binds to an account, without loading a roster/profile/media.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Reports a start in `channel` on this generation; no stop command or automatic replay
    /// exists in the backend.
    /// # Errors
    /// Returns a lifecycle error if the session lacks a current online connection.
    pub fn report_typing(&self, channel: Uuid) -> Result<(), papo_sdk::ClientError> {
        self.session.report_typing(channel)
    }

    /// Captures the activity watch before reading related metadata, so no event falls between
    /// capture and stream.
    #[must_use]
    pub fn subscribe(&self) -> ActivitySubscription {
        let mut activity = self.session.subscribe_activity();
        let mut resources = self.session.subscribe_resources();
        let mut connection = self.session.subscribe_connection();
        let live = activity.borrow_and_update().clone();
        let image = Arc::clone(&resources.borrow_and_update());
        let status = *connection.borrow_and_update();
        let snapshot = project(&self.session, &live, &image, status);
        let session = self.session.clone();
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
                let view = project(&session, &live, &image, status);
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
    activity: &ActivitySnapshot,
    resources: &ResourceSnapshot,
    status: papo_sdk::ConnectionStatus,
) -> AccountActivity {
    let mut view = AccountActivity {
        generation: status.generation,
        live: false,
        presence: BTreeMap::new(),
        typing: BTreeMap::new(),
    };
    if session.is_closed()
        || status.state != ConnectionState::Online
        || activity.generation != status.generation
        || session.generation() != status.generation
    {
        return view;
    }
    view.live = true;
    view.presence.clone_from(&activity.presence);
    let matching_resources = resources.generation == activity.generation;
    let now = tokio::time::Instant::now();
    // The map's order (channel, then person) keeps each name in its place while their client
    // refreshes the start; an order by start would reshuffle the names on every refresh.
    let typing = activity
        .typing
        .values()
        .filter(|entry| entry.user != session.account().user() && entry.expires_at > now);
    for entry in typing {
        let presence = activity.presence.get(&entry.user);
        let member = matching_resources
            .then(|| resources.users.get(entry.user))
            .flatten();
        view.typing
            .entry(entry.channel)
            .or_default()
            .push(TypingItem {
                user: entry.user,
                nickname: presence
                    .and_then(|presence| presence.nickname.clone())
                    .or_else(|| member.and_then(|member| member.nickname.clone())),
                phrase: presence
                    .and_then(|presence| presence.typing_phrase.clone())
                    .or_else(|| member.and_then(|member| member.typing.clone())),
            });
    }
    view
}
