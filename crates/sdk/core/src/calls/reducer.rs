//! Pure deterministic voice reducer: cold presence, live rooms, speakers and audio/video routes.
//! Owns no sockets, timers or media threads; callers apply its returned actions.

use crate::{Generation, Uuid};
use papo_api::gateway::{
    ActiveSpeakerUpdateEvent, PresenceSyncEvent, VoiceJoinedEvent, VoiceLeaveEvent,
    VoiceStateUpdateEvent,
};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    media_port::VideoRouteFence,
    model::{VideoCodec, VideoKind, VoiceDeviceState},
};

/// One decoded SFU video route entry retained by the reducer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReducedVideoRoute {
    /// SFU track identifier (`papo-video-N`).
    pub track_id: String,
    /// Publisher user identity.
    pub participant: Uuid,
    /// Stream kind (`Camera` or `Screen`).
    pub kind: VideoKind,
    /// Validated RTP assignment fence, if supplied.
    pub fence: Option<VideoRouteFence>,
    /// Legacy sequence hint retained for the public call snapshot only.
    pub first_seq: Option<u16>,
}

/// Internal per-room live occupancy and speaker state for one connection generation.
#[derive(Clone, Debug, Default)]
pub(super) struct RawRoom {
    pub(super) members: BTreeMap<Uuid, Option<VoiceDeviceState>>,
    /// `None` until `voice_joined` or `active_speaker_update` provides an authoritative set.
    pub(super) speakers: Option<BTreeSet<Uuid>>,
}

/// Active peer context supplied to the reducer when matching owner-scoped events.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ActivePeerRef<'a> {
    /// Channel of the active call.
    pub channel: Uuid,
    /// Opaque `peer_id` assigned by `voice_joined`.
    pub peer_id: &'a str,
}

/// Outcome of applying `voice_joined` to the reducer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct JoinedAdmission {
    /// Channel admitted.
    pub channel: Uuid,
    /// Opaque `peer_id` assigned by the backend.
    pub peer_id: String,
    /// Room video codec advertised by the backend.
    pub video_codec: VideoCodec,
}

/// Active-call route changes triggered when a remote occupant departs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RouteCleanup {
    /// True when active audio routes lost entries for the departed member.
    pub audio_changed: bool,
    /// True when active video routes lost entries for the departed member.
    pub video_changed: bool,
}

/// Outcome of applying `voice_leave` to the reducer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LeaveOutcome {
    /// True when the departing member is `self_user` AND `peer_id` matches the active peer.
    pub ended_active_call: bool,
    /// Route cleanups for a departed remote occupant in the active channel.
    pub routes: RouteCleanup,
    /// True when visible room occupancy changed.
    pub rooms_changed: bool,
}

/// Pure in-memory voice reducer for one account session generation.
#[derive(Clone, Debug)]
pub(crate) struct VoiceReducer {
    pub(super) generation: Generation,
    pub(super) rooms: BTreeMap<Uuid, RawRoom>,
    /// Active call's audio routes (`track_id` → `user`), cleared on attempt change.
    pub(super) audio_routes_known: bool,
    pub(super) audio_routes: BTreeMap<String, Uuid>,
    pub(super) last_audio_revision: Option<u64>,
    /// Active call's video routes (`track_id` → route), cleared on attempt change.
    pub(super) video_routes: BTreeMap<String, ReducedVideoRoute>,
    pub(super) last_video_revision: Option<u64>,
}

impl VoiceReducer {
    /// Starts an empty reducer for `generation`.
    #[must_use]
    pub(crate) fn new(generation: Generation) -> Self {
        Self {
            generation,
            rooms: BTreeMap::new(),
            audio_routes_known: false,
            audio_routes: BTreeMap::new(),
            last_audio_revision: None,
            video_routes: BTreeMap::new(),
            last_video_revision: None,
        }
    }

    /// Resets all live rooms and routes for a new connection generation.
    pub(crate) fn reset_generation(&mut self, generation: Generation) {
        self.generation = generation;
        self.rooms.clear();
        self.clear_attempt_routes();
    }

    /// Clears per-attempt route state and active-room speaker cache on join/leave/rejoin.
    pub(crate) fn clear_attempt_routes(&mut self) {
        self.audio_routes_known = false;
        self.audio_routes.clear();
        self.last_audio_revision = None;
        self.video_routes.clear();
        self.last_video_revision = None;
    }

    /// Clears stale speaker state for a channel when leaving or starting a fresh join attempt.
    pub(crate) fn clear_room_speakers(&mut self, channel: Uuid) {
        if let Some(room) = self.rooms.get_mut(&channel) {
            room.speakers = None;
        }
    }

    /// Seeds cold room occupancy from `presence_sync.user_voice`, preserving any known live device state.
    pub(crate) fn apply_presence_sync(&mut self, event: &PresenceSyncEvent) {
        let mut next: BTreeMap<Uuid, RawRoom> = BTreeMap::new();
        for member in &event.members {
            let Ok(user) = member.user_id.parse::<Uuid>() else {
                continue;
            };
            for raw_channel in &member.user_voice {
                let Ok(channel) = raw_channel.parse::<Uuid>() else {
                    continue;
                };
                let existing_room = self.rooms.get(&channel);
                let preserved_device = existing_room
                    .and_then(|room| room.members.get(&user))
                    .copied()
                    .flatten();
                let entry = next.entry(channel).or_insert_with(|| RawRoom {
                    members: BTreeMap::new(),
                    speakers: existing_room.and_then(|room| room.speakers.clone()),
                });
                entry.members.insert(user, preserved_device);
            }
        }
        for room in next.values_mut() {
            if let Some(speakers) = room.speakers.as_mut() {
                speakers.retain(|speaker| room.members.contains_key(speaker));
            }
        }
        self.rooms = next;
    }

    /// Applies `voice_joined` for the active attempt's channel, seeding members and active speakers.
    pub(crate) fn apply_voice_joined(
        &mut self,
        expected_channel: Uuid,
        event: &VoiceJoinedEvent,
    ) -> Option<JoinedAdmission> {
        let Ok(channel) = event.channel_id.parse::<Uuid>() else {
            return None;
        };
        if channel != expected_channel || event.peer_id.is_empty() {
            return None;
        }
        let mut members = BTreeMap::new();
        for state in &event.members {
            let Ok(user) = state.user_id.parse::<Uuid>() else {
                continue;
            };
            members.insert(
                user,
                Some(VoiceDeviceState {
                    muted: state.muted,
                    camera_on: state.camera_on,
                    screen_sharing: state.screen_sharing,
                }),
            );
        }
        let mut speakers = BTreeSet::new();
        for raw_user in &event.active_speakers {
            if let Ok(user) = raw_user.parse::<Uuid>()
                && members
                    .get(&user)
                    .is_some_and(|device| !device.is_some_and(|d| d.muted))
            {
                speakers.insert(user);
            }
        }
        self.rooms.insert(
            channel,
            RawRoom {
                members,
                speakers: Some(speakers),
            },
        );
        self.clear_attempt_routes();
        Some(JoinedAdmission {
            channel,
            peer_id: event.peer_id.clone(),
            video_codec: VideoCodec::from_wire(&event.video_codec),
        })
    }

    /// Applies `voice_state_update`, upserting the member and clearing speaking when muted.
    pub(crate) fn apply_state_update(
        &mut self,
        event: &VoiceStateUpdateEvent,
    ) -> Option<(Uuid, Uuid)> {
        let (Ok(channel), Ok(user)) = (
            event.channel_id.parse::<Uuid>(),
            event.user_id.parse::<Uuid>(),
        ) else {
            return None;
        };
        let room = self.rooms.entry(channel).or_default();
        room.members.insert(
            user,
            Some(VoiceDeviceState {
                muted: event.muted,
                camera_on: event.camera_on,
                screen_sharing: event.screen_sharing,
            }),
        );
        if event.muted
            && let Some(speakers) = room.speakers.as_mut()
        {
            speakers.remove(&user);
        }
        Some((channel, user))
    }

    /// Applies `voice_leave`, ignoring a late self-leave whose `peer_id` differs from the active peer.
    pub(crate) fn apply_leave(
        &mut self,
        self_user: Uuid,
        active_peer: Option<ActivePeerRef<'_>>,
        event: &VoiceLeaveEvent,
    ) -> LeaveOutcome {
        let (Ok(channel), Ok(user)) = (
            event.channel_id.parse::<Uuid>(),
            event.user_id.parse::<Uuid>(),
        ) else {
            return LeaveOutcome {
                ended_active_call: false,
                routes: RouteCleanup::default(),
                rooms_changed: false,
            };
        };

        let is_self = user == self_user;
        if is_self {
            if let Some(active) = active_peer
                && active.channel == channel
            {
                if event.peer_id != active.peer_id {
                    // Late self-leave from an earlier peer on the same channel: ignore completely.
                    return LeaveOutcome {
                        ended_active_call: false,
                        routes: RouteCleanup::default(),
                        rooms_changed: false,
                    };
                }
                let rooms_changed = self.remove_member(channel, user);
                self.clear_attempt_routes();
                return LeaveOutcome {
                    ended_active_call: true,
                    routes: RouteCleanup::default(),
                    rooms_changed,
                };
            }
            let rooms_changed = self.remove_member(channel, user);
            return LeaveOutcome {
                ended_active_call: false,
                routes: RouteCleanup::default(),
                rooms_changed,
            };
        }

        let rooms_changed = self.remove_member(channel, user);
        let mut routes = RouteCleanup::default();
        if active_peer.is_some_and(|active| active.channel == channel) {
            let before_video = self.video_routes.len();
            self.video_routes
                .retain(|_, route| route.participant != user);
            routes.video_changed = self.video_routes.len() != before_video;
            let before_audio = self.audio_routes.len();
            self.audio_routes.retain(|_, owner| *owner != user);
            routes.audio_changed = self.audio_routes.len() != before_audio;
        }
        LeaveOutcome {
            ended_active_call: false,
            routes,
            rooms_changed,
        }
    }

    /// Applies `active_speaker_update`, keeping only non-muted occupants of that channel.
    pub(crate) fn apply_active_speakers(
        &mut self,
        event: &ActiveSpeakerUpdateEvent,
    ) -> Option<Uuid> {
        let Ok(channel) = event.channel_id.parse::<Uuid>() else {
            return None;
        };
        let room = self.rooms.get_mut(&channel)?;
        let mut speakers = BTreeSet::new();
        for raw_user in &event.user_ids {
            if let Ok(user) = raw_user.parse::<Uuid>()
                && room
                    .members
                    .get(&user)
                    .is_some_and(|device| !device.is_some_and(|d| d.muted))
            {
                speakers.insert(user);
            }
        }
        room.speakers = Some(speakers);
        Some(channel)
    }

    fn remove_member(&mut self, channel: Uuid, user: Uuid) -> bool {
        let Some(room) = self.rooms.get_mut(&channel) else {
            return false;
        };
        let removed = room.members.remove(&user).is_some();
        if let Some(speakers) = room.speakers.as_mut() {
            speakers.remove(&user);
        }
        if room.members.is_empty() {
            self.rooms.remove(&channel);
        }
        removed
    }
}
