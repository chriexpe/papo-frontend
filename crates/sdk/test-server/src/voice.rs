//! Deterministic loopback voice REST/WS handlers and fault scripts mirroring the 14-0 backend.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use papo_api::{gateway::KnownEvent, wire::ICEServer};
use serde_json::{Value, json};

use crate::{
    TestServer,
    presence::Presence,
    response::{Reply, problem},
    server::State,
};

/// Scriptable voice fault injected into `TestServer` for deterministic loopback testing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoiceFault {
    /// Reject the next `voice_join` with this backend voice error code (e.g. `"voice-room-full"`).
    RejectNextJoin(String),
    /// Accept the next `voice_join` silently without emitting `voice_joined` (tests join timeout).
    DropNextVoiceJoined,
    /// Hold the next `voice_leave` broadcast until `TestServer::flush_delayed_voice_leaves` is called.
    DelayNextVoiceLeave,
    /// Reject the next `track_subscribe` with this backend voice error code (e.g. `"voice-not-found"`).
    RejectNextSubscribe(String),
    /// Suppress automatic `voice_answer` for the next `voice_offer`.
    DropNextVoiceAnswer,
}

/// One inbound voice gateway command observed by `TestServer`, without logging secrets or SDP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedVoiceCommand {
    /// Authenticated user who sent the command.
    pub user_id: String,
    /// Owning WebSocket connection identifier.
    pub client_id: String,
    /// Inbound command `"type"` discriminator.
    pub kind: String,
    /// Channel identifier from the command.
    pub channel_id: String,
}

#[derive(Clone)]
pub(crate) struct VoicePeer {
    pub(crate) peer_id: String,
    pub(crate) user_id: String,
    pub(crate) client_id: String,
    pub(crate) muted: bool,
    pub(crate) camera_on: bool,
    pub(crate) screen_sharing: bool,
    pub(crate) routes_revision: u64,
    /// Slot index -> `(publisher_id, kind, first_seq)`.
    pub(crate) video_slots: Vec<Option<(String, String, Option<u16>)>>,
    /// Tracks whether a video slot ever carried an occupant before (`first_seq` boundary).
    pub(crate) video_slot_used: Vec<bool>,
    pub(crate) video_slot_seq: Vec<u16>,
}

#[derive(Default)]
pub(crate) struct VoiceRoomState {
    pub(crate) peers: BTreeMap<String, VoicePeer>,
    pub(crate) active_speakers: Vec<String>,
}

/// In-memory voice state held inside `TestServer`.
pub(crate) struct VoiceState {
    pub(crate) ice_servers: Vec<ICEServer>,
    pub(crate) video_codec: String,
    next_peer_seq: u64,
    pub(crate) rooms: BTreeMap<String, VoiceRoomState>,
    pub(crate) faults: VecDeque<VoiceFault>,
    pub(crate) delayed_leaves: Vec<Value>,
    pub(crate) recorded: Vec<RecordedVoiceCommand>,
}

impl Default for VoiceState {
    fn default() -> Self {
        Self {
            ice_servers: vec![ICEServer {
                urls: Some(vec!["stun:127.0.0.1:3478".into()]),
                username: None,
                credential: None,
            }],
            video_codec: "h264".into(),
            next_peer_seq: 1,
            rooms: BTreeMap::new(),
            faults: VecDeque::new(),
            delayed_leaves: Vec::new(),
            recorded: Vec::new(),
        }
    }
}

impl VoiceState {
    /// Lists every user currently occupying at least one voice channel (unfiltered, matching 14-0).
    pub(crate) fn all_user_voice_channels(&self) -> BTreeMap<String, Vec<String>> {
        let mut by_user: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (channel_id, room) in &self.rooms {
            for user_id in room.peers.keys() {
                by_user
                    .entry(user_id.clone())
                    .or_default()
                    .push(channel_id.clone());
            }
        }
        by_user
    }

    /// Removes any peer owned by `client_id` when its WebSocket closes.
    pub(crate) fn client_offline(&mut self, presence: &Presence, user_id: &str, client_id: &str) {
        let channels: Vec<String> = self
            .rooms
            .iter()
            .filter(|(_, room)| {
                room.peers
                    .get(user_id)
                    .is_some_and(|peer| peer.client_id == client_id)
            })
            .map(|(channel_id, _)| channel_id.clone())
            .collect();
        for channel_id in channels {
            self.remove_peer(presence, &channel_id, user_id);
        }
    }

    pub(crate) fn next_peer_id(&mut self) -> String {
        let seq = self.next_peer_seq;
        self.next_peer_seq = self.next_peer_seq.saturating_add(1);
        format!("peer-{seq}")
    }

    pub(crate) fn take_fault(
        &mut self,
        predicate: impl Fn(&VoiceFault) -> bool,
    ) -> Option<VoiceFault> {
        let index = self.faults.iter().position(predicate)?;
        self.faults.remove(index)
    }

    pub(crate) fn remove_peer(&mut self, presence: &Presence, channel_id: &str, user_id: &str) {
        let (departed_peer_id, room_empty) = {
            let Some(room) = self.rooms.get_mut(channel_id) else {
                return;
            };
            let Some(departed) = room.peers.remove(user_id) else {
                return;
            };
            room.active_speakers.retain(|speaker| speaker != user_id);
            (departed.peer_id, room.peers.is_empty())
        };
        if room_empty {
            self.rooms.remove(channel_id);
        }
        let leave_payload = json!({
            "type": "voice_leave",
            "channel_id": channel_id,
            "user_id": user_id,
            "peer_id": departed_peer_id,
        });
        if self
            .take_fault(|f| matches!(f, VoiceFault::DelayNextVoiceLeave))
            .is_some()
        {
            self.delayed_leaves.push(leave_payload);
        } else {
            presence.emit(None, leave_payload);
        }
    }
}

/// Handles `GET /voice/ice-servers`.
pub(crate) fn handle_rest(
    state: &State,
    method: &str,
    path: &str,
    user_id: Option<&str>,
) -> Option<Reply> {
    if method != "GET" || path != "/voice/ice-servers" {
        return None;
    }
    if user_id.is_none() {
        return Some(problem(401, "unauthorized", path));
    }
    Some(Reply::json(
        200,
        json!({ "ice_servers": state.voice.ice_servers }),
    ))
}

pub(crate) use crate::voice_ws::{VoiceWsOutcome, handle_ws};

impl TestServer {
    /// Creates a deterministic voice channel and broadcasts `channel_create`.
    ///
    /// # Errors
    /// Returns [`crate::Error::Configuration`] for an empty name or exhausted counters.
    pub async fn create_voice_channel(
        &self,
        name: &str,
    ) -> Result<papo_api::wire::Channel, crate::Error> {
        let mut channel = self.create_channel(name).await?;
        channel.r#type = Some("voice".into());
        self.set_channel(channel.clone()).await?;
        Ok(channel)
    }

    /// Replaces the ICE servers returned by `GET /voice/ice-servers`.
    pub async fn set_ice_servers(&self, servers: Vec<ICEServer>) {
        self.state.lock().await.voice.ice_servers = servers;
    }

    /// Sets the `video_codec` announced in `voice_joined` (`"vp8"`, `"vp9"`, `"h264"`).
    pub async fn set_voice_video_codec(&self, codec: &str) {
        self.state.lock().await.voice.video_codec = codec.to_owned();
    }

    /// Seeds a remote occupant in a voice channel (visible in `presence_sync.user_voice` and `voice_joined`).
    pub async fn seed_voice_occupant(
        &self,
        channel_id: &str,
        user_id: &str,
        muted: bool,
        camera_on: bool,
        screen_sharing: bool,
    ) {
        let mut state = self.state.lock().await;
        let peer_id = state.voice.next_peer_id();
        let room = state.voice.rooms.entry(channel_id.to_owned()).or_default();
        room.peers.insert(
            user_id.to_owned(),
            VoicePeer {
                peer_id,
                user_id: user_id.to_owned(),
                client_id: format!("seeded-{user_id}"),
                muted,
                camera_on,
                screen_sharing,
                routes_revision: 0,
                video_slots: vec![None; 6],
                video_slot_used: vec![false; 6],
                video_slot_seq: vec![1000; 6],
            },
        );
    }

    /// Queues a deterministic voice fault for subsequent voice gateway commands.
    pub async fn push_voice_fault(&self, fault: VoiceFault) {
        self.state.lock().await.voice.faults.push_back(fault);
    }

    /// Broadcasts any `voice_leave` events held by `VoiceFault::DelayNextVoiceLeave`.
    pub async fn flush_delayed_voice_leaves(&self) {
        let mut state = self.state.lock().await;
        let leaves = std::mem::take(&mut state.voice.delayed_leaves);
        for payload in leaves {
            state.presence.emit(None, payload);
        }
    }

    /// Sets a voice channel's role overwrites `(role_id, read_channel, connect_voice)`.
    ///
    /// # Errors
    /// Returns [`crate::Error::Configuration`] if the channel does not exist.
    pub async fn set_voice_channel_overwrites(
        &self,
        channel_id: &str,
        overwrites: Vec<(&str, bool, bool)>,
    ) -> Result<(), crate::Error> {
        let mut channel = self
            .state
            .lock()
            .await
            .resources
            .channels
            .get(channel_id)
            .cloned()
            .ok_or(crate::Error::Configuration)?;
        channel.permissions = Some(
            overwrites
                .into_iter()
                .map(
                    |(role_id, read, voice)| papo_api::wire::ChannelPermissionEntry {
                        role_id: Some(role_id.to_owned()),
                        role_name: None,
                        permissions: Some(papo_api::wire::ChannelPermissions {
                            read_channel: Some(read),
                            send_messages: Some(false),
                            delete_messages: Some(false),
                            connect_voice: Some(voice),
                        }),
                    },
                )
                .collect(),
        );
        self.set_channel(channel).await
    }

    /// Installs a single role with default permissions (valid for resource admission).
    pub async fn set_voice_role(&self, role_id: &str, name: &str) {
        self.set_roles(vec![papo_api::wire::Role {
            id: Some(role_id.to_owned()),
            name: Some(name.to_owned()),
            permissions: Some(papo_api::wire::RolePermissions::default()),
            ..papo_api::wire::Role::default()
        }])
        .await;
    }

    /// Sets a member profile with optional nickname, avatar format and role ids.
    ///
    /// # Errors
    /// Returns [`crate::Error::Configuration`] for an invalid user id or empty username.
    pub async fn set_voice_member_profile(
        &self,
        user_id: &str,
        username: &str,
        nickname: Option<&str>,
        avatar_format: Option<&str>,
        roles: &[(&str, &str)],
    ) -> Result<(), crate::Error> {
        self.set_profile(papo_api::wire::UserProfile {
            id: Some(user_id.to_owned()),
            username: Some(username.to_owned()),
            nickname: match nickname {
                Some(text) => papo_api::NullableText::Value(text.to_owned()),
                None => papo_api::NullableText::Missing,
            },
            avatar_format: avatar_format.map(str::to_owned),
            roles: Some(
                roles
                    .iter()
                    .map(|(id, name)| papo_api::wire::RoleSummary {
                        id: Some((*id).to_owned()),
                        name: Some((*name).to_owned()),
                        color: None,
                    })
                    .collect(),
            ),
            ..papo_api::wire::UserProfile::default()
        })
        .await
    }

    /// Broadcasts `active_speaker_update` for `channel_id` to connected sockets.
    pub async fn emit_active_speakers(&self, channel_id: &str, user_ids: &[&str]) {
        let mut state = self.state.lock().await;
        if let Some(room) = state.voice.rooms.get_mut(channel_id) {
            room.active_speakers = user_ids.iter().map(|id| (*id).to_owned()).collect();
        }
        state.presence.emit(
            None,
            json!({
                "type": "active_speaker_update",
                "channel_id": channel_id,
                "user_ids": user_ids,
            }),
        );
    }

    /// Emits a typed gateway event (e.g. stale `peer_id` answer/routes/error or speakers) to sockets.
    ///
    /// # Errors
    /// Returns [`crate::Error::Configuration`] if serialization fails.
    pub async fn emit_voice_event(
        &self,
        recipient_user: Option<&str>,
        event: KnownEvent,
    ) -> Result<(), crate::Error> {
        let payload = serde_json::to_value(event).map_err(|_| crate::Error::Configuration)?;
        self.state
            .lock()
            .await
            .presence
            .emit(recipient_user, payload);
        Ok(())
    }

    /// Returns all inbound voice commands received so far in arrival order.
    pub async fn recorded_voice_commands(&self) -> Vec<RecordedVoiceCommand> {
        self.state.lock().await.voice.recorded.clone()
    }

    /// Emits a message event directly to connected gateways (used in candidate-flood concurrency tests).
    pub async fn emit_chat_message(&self, channel_id: &str, message_id: &str, author_id: &str) {
        let state = self.state.lock().await;
        let excluded: BTreeSet<String> = BTreeSet::new();
        state.presence.emit_except(
            excluded,
            json!({
                "type": "message",
                "id": message_id,
                "channel_id": channel_id,
                "author_id": author_id,
                "content": "ping",
                "created_at": crate::resources::CREATED_AT,
                "reply_to": null,
            }),
        );
    }
}
