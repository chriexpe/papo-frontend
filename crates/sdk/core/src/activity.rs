//! Generation-scoped live presence and typing with bounded typing memory; no SQLite state or profile loading.

use crate::{
    ClientError, ConnectionState, Generated, Generation, Session, observable::StateCell,
    session::SessionInner,
};
use papo_api::{
    Uuid,
    gateway::{Event, KnownEvent},
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};

/// Backend presence states. Unknown future values do not become localized status text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresenceStatus {
    /// At least one backend connection is active.
    Online,
    /// Backend automatic or manual away state.
    Away,
    /// Backend manual busy state.
    Busy,
    /// A future backend presence value.
    Unknown,
}
impl PresenceStatus {
    fn from_wire(value: &str) -> Self {
        match value {
            "online" => Self::Online,
            "away" => Self::Away,
            "busy" => Self::Busy,
            _ => Self::Unknown,
        }
    }
}

/// Backend presence metadata from this connection, never recovered from the account cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presence {
    /// Backend user identity.
    pub user: Uuid,
    /// Backend status, including future statuses, without client inference.
    pub status: PresenceStatus,
    /// Optional backend custom status phrase.
    pub status_message: Option<String>,
    /// Optional typing phrase delivered in a live presence update.
    pub typing_phrase: Option<String>,
    /// Optional nickname delivered in a live presence update.
    pub nickname: Option<String>,
}

/// A typing start expires ten seconds after receipt unless refreshed or explicitly stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Typing {
    /// Channel in which activity was reported.
    pub channel: Uuid,
    /// Backend user identity, never an external webhook author.
    pub user: Uuid,
    /// Monotonic expiry, not a durable timestamp.
    pub expires_at: Instant,
}

/// Atomic memory-only live image, capacity-one coalescing on subscription.
#[derive(Clone, Debug)]
pub struct ActivitySnapshot {
    /// Monotonic revision in this session.
    pub version: u64,
    /// Connection owning every entry.
    pub generation: Generation,
    /// Last live presence by backend user identity.
    pub presence: BTreeMap<Uuid, Presence>,
    /// Active typing keyed by channel and user; at most 1024 entries.
    pub typing: BTreeMap<(Uuid, Uuid), Typing>,
}
impl ActivitySnapshot {
    pub(crate) fn empty(generation: Generation) -> Self {
        Self {
            version: 0,
            generation,
            presence: BTreeMap::new(),
            typing: BTreeMap::new(),
        }
    }
    fn apply(&mut self, event: &Event, now: Instant) -> bool {
        let Event::Known(event) = event else {
            return false;
        };
        match event {
            KnownEvent::PresenceSync(event) => {
                self.presence = event
                    .members
                    .iter()
                    .filter_map(|member| {
                        if member.status == "offline" {
                            return None;
                        }
                        let user = member.user_id.parse().ok()?;
                        Some((
                            user,
                            Presence {
                                user,
                                status: PresenceStatus::from_wire(&member.status),
                                status_message: member.status_message.clone(),
                                typing_phrase: None,
                                nickname: None,
                            },
                        ))
                    })
                    .collect();
            }
            KnownEvent::PresenceUpdate(event) => {
                let Ok(user) = event.user_id.parse() else {
                    return false;
                };
                self.presence.insert(
                    user,
                    Presence {
                        user,
                        status: PresenceStatus::from_wire(&event.status),
                        status_message: event.status_message.clone(),
                        typing_phrase: event.typing.clone(),
                        nickname: event.nickname.clone(),
                    },
                );
                if event.status == "offline" {
                    self.typing.retain(|(_, id), _| *id != user);
                    self.presence.remove(&user);
                }
            }
            KnownEvent::Typing(event) => {
                let (Ok(channel), Ok(user)) = (event.channel_id.parse(), event.user_id.parse())
                else {
                    return false;
                };
                self.expire(now);
                let key = (channel, user);
                if event.is_typing {
                    if self.typing.len() >= 1024 && !self.typing.contains_key(&key) {
                        return false;
                    }
                    self.typing.insert(
                        key,
                        Typing {
                            channel,
                            user,
                            expires_at: now + Duration::from_secs(10),
                        },
                    );
                } else {
                    self.typing.remove(&key);
                }
            }
            _ => return false,
        }
        true
    }
    fn expire(&mut self, now: Instant) -> bool {
        let before = self.typing.len();
        self.typing.retain(|_, typing| typing.expires_at > now);
        before != self.typing.len()
    }
}

pub(crate) fn receive(session: &SessionInner, generation: Generation, event: &Event) {
    let _ = session.epochs.apply(generation, || {
        session.activity.send_if_modified(|snapshot| {
            if snapshot.generation != generation {
                *snapshot = ActivitySnapshot::empty(generation);
            }
            let changed = snapshot.apply(event, Instant::now());
            if changed {
                snapshot.version = snapshot.version.saturating_add(1);
            }
            changed
        });
    });
}

pub(crate) fn clear(session: &SessionInner, generation: Generation) {
    session.activity.send_if_modified(|snapshot| {
        let version = snapshot.version.saturating_add(1);
        *snapshot = ActivitySnapshot::empty(generation);
        snapshot.version = version;
        true
    });
}

pub(crate) async fn expire_loop(session: Arc<SessionInner>) {
    let mut source = session.activity.subscribe();
    loop {
        let deadline = source
            .borrow_and_update()
            .typing
            .values()
            .map(|entry| entry.expires_at)
            .min();
        tokio::select! {
            biased;
            () = session.root.cancelled() => return,
            changed = source.changed() => if changed.is_err() { return; },
            () = async {
                if let Some(deadline) = deadline { tokio::time::sleep_until(deadline).await; }
                else { std::future::pending::<()>().await; }
            } => session.activity.send_if_modified(|snapshot| {
                let changed = snapshot.expire(Instant::now());
                if changed { snapshot.version = snapshot.version.saturating_add(1); }
                changed
            }),
        }
    }
}

impl Session {
    /// Subscribes to live-only metadata; connection replacement clears it and revocation closes it.
    #[must_use]
    pub fn subscribe_activity(&self) -> watch::Receiver<ActivitySnapshot> {
        self.inner.activity.subscribe()
    }
    /// Coalesces a best-effort typing start onto this connection. No durable work or retries.
    /// Backend checks read permission; it does not support an outgoing stop command.
    /// # Errors
    /// Returns `Closed` or `Superseded` when no current online connection exists.
    pub fn report_typing(&self, channel: Uuid) -> Result<(), ClientError> {
        let status = *self.inner.connection.borrow();
        self.inner.epochs.apply(status.generation, || {
            if status.state != ConnectionState::Online {
                return Err(ClientError::Superseded);
            }
            self.inner.typing_command.send_replace(Some(Generated {
                generation: status.generation,
                value: (channel, Instant::now()),
            }));
            Ok(())
        })?
    }
}

pub(crate) fn cell(generation: Generation) -> Arc<StateCell<ActivitySnapshot>> {
    StateCell::new(ActivitySnapshot::empty(generation))
}

#[cfg(test)]
mod tests;
