//! Route reduction (`voice_audio_routes`, `voice_video_routes`) and room/participant projections.
//! Cold presence, join, state updates, leave and speaker reduction live in `reducer.rs`.

use crate::{
    ResourceSnapshot, Uuid,
    permissions::{VoiceAccess, voice_access},
};
use papo_api::gateway::{VoiceAudioRoutesEvent, VoiceVideoRoutesEvent};
use std::collections::BTreeMap;

use super::{
    media_port::AudioRoute,
    model::{RoomMember, RoomsSnapshot, SpeakingState, VideoKind, VoiceDeviceState, VoiceRoom},
    reducer::{ActivePeerRef, ReducedVideoRoute, VoiceReducer},
};

impl VoiceReducer {
    /// Applies owner-scoped `voice_audio_routes`, enforcing `peer_id` and per-type `revision`.
    pub(crate) fn apply_audio_routes(
        &mut self,
        active_peer: Option<ActivePeerRef<'_>>,
        event: &VoiceAudioRoutesEvent,
    ) -> Option<(u64, Vec<AudioRoute>)> {
        let active = active_peer?;
        let Ok(channel) = event.channel_id.parse::<Uuid>() else {
            return None;
        };
        if channel != active.channel || event.peer_id != active.peer_id {
            return None;
        }
        if self
            .last_audio_revision
            .is_some_and(|prev| event.revision < prev)
        {
            return None;
        }
        self.last_audio_revision = Some(event.revision);
        self.audio_routes_known = true;
        self.audio_routes.clear();
        let mut out = Vec::with_capacity(event.routes.len());
        for route in &event.routes {
            let Ok(user) = route.user_id.parse::<Uuid>() else {
                continue;
            };
            if route.track_id.is_empty() {
                continue;
            }
            self.audio_routes.insert(route.track_id.clone(), user);
            out.push(AudioRoute {
                track_id: route.track_id.clone(),
                user,
            });
        }
        Some((event.revision, out))
    }

    /// Applies owner-scoped `voice_video_routes`, enforcing `peer_id`, per-type `revision` and current room membership.
    pub(crate) fn apply_video_routes(
        &mut self,
        active_peer: Option<ActivePeerRef<'_>>,
        event: &VoiceVideoRoutesEvent,
    ) -> Option<u64> {
        let active = active_peer?;
        let Ok(channel) = event.channel_id.parse::<Uuid>() else {
            return None;
        };
        if channel != active.channel || event.peer_id != active.peer_id {
            return None;
        }
        if self
            .last_video_revision
            .is_some_and(|prev| event.revision < prev)
        {
            return None;
        }
        self.last_video_revision = Some(event.revision);
        let members = self.rooms.get(&channel).map(|room| &room.members);
        self.video_routes.clear();
        for route in &event.routes {
            let Ok(participant) = route.user_id.parse::<Uuid>() else {
                continue;
            };
            let Some(kind) = VideoKind::from_wire(&route.kind) else {
                continue;
            };
            if route.track_id.is_empty() {
                continue;
            }
            // Drop routes naming a publisher who has already left the room.
            if !members.is_some_and(|m| m.contains_key(&participant)) {
                continue;
            }
            self.video_routes.insert(
                route.track_id.clone(),
                ReducedVideoRoute {
                    track_id: route.track_id.clone(),
                    participant,
                    kind,
                    first_seq: route.first_seq,
                },
            );
        }
        Some(event.revision)
    }

    /// Projects visible voice rooms filtered by admitted `connect_voice` permission.
    #[must_use]
    pub(crate) fn project_rooms(
        &self,
        version: u64,
        online: bool,
        resources: &ResourceSnapshot,
        self_user: Uuid,
    ) -> RoomsSnapshot {
        let mut snapshot = RoomsSnapshot::empty(self.generation);
        snapshot.version = version;
        if !online {
            return snapshot;
        }
        for (&channel, raw) in &self.rooms {
            if raw.members.is_empty() {
                continue;
            }
            if voice_access(resources, self_user, channel) != VoiceAccess::Allowed {
                continue;
            }
            let members = raw
                .members
                .iter()
                .map(|(&user, &device)| RoomMember { user, device })
                .collect();
            snapshot.rooms.insert(
                channel,
                VoiceRoom {
                    channel,
                    members,
                    speakers: raw.speakers.clone(),
                },
            );
        }
        snapshot
    }

    /// Returns the raw members and speaking state for a single active call channel.
    #[must_use]
    pub(crate) fn room_members(
        &self,
        channel: Uuid,
    ) -> Vec<(Uuid, Option<VoiceDeviceState>, SpeakingState)> {
        let Some(room) = self.rooms.get(&channel) else {
            return Vec::new();
        };
        room.members
            .iter()
            .map(|(&user, &device)| {
                let speaking = if device.is_some_and(|d| d.muted) {
                    SpeakingState::NotSelected
                } else {
                    match &room.speakers {
                        None => SpeakingState::Unknown,
                        Some(set) if set.contains(&user) => SpeakingState::Selected,
                        Some(_) => SpeakingState::NotSelected,
                    }
                };
                (user, device, speaking)
            })
            .collect()
    }

    /// Returns whether `user` is currently publishing `kind` in `channel`.
    #[must_use]
    pub(crate) fn is_publishing(&self, channel: Uuid, user: Uuid, kind: VideoKind) -> bool {
        self.rooms
            .get(&channel)
            .and_then(|room| room.members.get(&user))
            .copied()
            .flatten()
            .is_some_and(|device| match kind {
                VideoKind::Camera => device.camera_on,
                VideoKind::Screen => device.screen_sharing,
            })
    }

    /// Current audio route knowledge flag for the active call.
    #[must_use]
    pub(crate) const fn audio_routes_known(&self) -> bool {
        self.audio_routes_known
    }

    /// Current SFU audio routes (`track_id` → `user`).
    #[must_use]
    pub(crate) fn audio_routes(&self) -> &BTreeMap<String, Uuid> {
        &self.audio_routes
    }

    /// Latest audio route revision applied for this peer (0 if none yet).
    #[must_use]
    pub(crate) fn audio_revision(&self) -> u64 {
        self.last_audio_revision.unwrap_or(0)
    }

    /// Current audio routes as a `Vec<AudioRoute>` for `MediaCommand::SetAudioRoutes`.
    #[must_use]
    pub(crate) fn audio_route_vec(&self) -> Vec<AudioRoute> {
        self.audio_routes
            .iter()
            .map(|(track_id, &user)| AudioRoute {
                track_id: track_id.clone(),
                user,
            })
            .collect()
    }

    /// Current SFU video routes (`track_id` → route).
    #[must_use]
    pub(crate) fn video_routes(&self) -> &BTreeMap<String, ReducedVideoRoute> {
        &self.video_routes
    }
}
