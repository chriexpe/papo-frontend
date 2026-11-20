//! Observable call and room snapshot types published by `SessionCalls`.
//! Configuration, errors and lifecycle enums live in `model.rs`.

use crate::{AccountId, Generation, Uuid};
use std::collections::{BTreeMap, BTreeSet};

use super::{
    media_port::{CaptureState, MediaTransportState, VideoDestinationId},
    model::{
        CallAttemptId, CallEndReason, CallPhase, CallSettings, CallWarning,
        MAX_PLAYBACK_GAIN_PERCENT, VideoCodec, VideoKind,
    },
};

/// Independent local receiver playback settings for one remote user.
///
/// Gain (`0..=200`) and receiver mute are independent and never alter the sender's mute state
/// or active-speaker selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackSettings {
    /// Volume gain percentage, clamped to `0..=200`. Default `100`.
    pub gain_percent: u16,
    /// Local receiver mute. Default `false`.
    pub muted: bool,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            gain_percent: 100,
            muted: false,
        }
    }
}

impl PlaybackSettings {
    /// Constructs playback settings, clamping `gain_percent` to `0..=200`.
    #[must_use]
    pub fn new(gain_percent: u16, muted: bool) -> Self {
        Self {
            gain_percent: gain_percent.min(MAX_PLAYBACK_GAIN_PERCENT),
            muted,
        }
    }
}

/// Server-driven active-speaker state for a participant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpeakingState {
    /// Server active-speaker selection has not been received yet for this room or attempt.
    Unknown,
    /// Participant is in the SFU's current active-speaker set and is not muted.
    Selected,
    /// Server active-speaker selection is known and this participant is not speaking (or is muted).
    NotSelected,
}

/// Known remote/self device flags from `voice_joined` or `voice_state_update`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceDeviceState {
    /// Whether the participant's microphone is muted on the server.
    pub muted: bool,
    /// Whether the participant is actively publishing camera video.
    pub camera_on: bool,
    /// Whether the participant is actively publishing screen video.
    pub screen_sharing: bool,
}

/// One occupant of a voice room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomMember {
    /// Backend user identity.
    pub user: Uuid,
    /// Device state; `None` for cold presence (`presence_sync.user_voice`) until live state arrives.
    pub device: Option<VoiceDeviceState>,
}

/// Live occupancy snapshot of one admitted voice channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceRoom {
    /// Voice channel identity.
    pub channel: Uuid,
    /// Occupants sorted by user UUID.
    pub members: Vec<RoomMember>,
    /// Active speaker set if known for this room (`None` before any speaker event).
    pub speakers: Option<BTreeSet<Uuid>>,
}

impl VoiceRoom {
    /// Computes `SpeakingState` for a member of this room.
    #[must_use]
    pub fn speaking_state(&self, user: Uuid, device: Option<VoiceDeviceState>) -> SpeakingState {
        if device.is_some_and(|state| state.muted) {
            return SpeakingState::NotSelected;
        }
        match &self.speakers {
            None => SpeakingState::Unknown,
            Some(selected) if selected.contains(&user) => SpeakingState::Selected,
            Some(_) => SpeakingState::NotSelected,
        }
    }
}

/// Observable voice room occupancy for one account generation, filtered by admitted `connect_voice`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomsSnapshot {
    /// Monotonic publication version within this session.
    pub version: u64,
    /// Connection generation owning these rooms.
    pub generation: Generation,
    /// Non-empty visible rooms keyed by channel UUID.
    pub rooms: BTreeMap<Uuid, VoiceRoom>,
}

impl RoomsSnapshot {
    /// Empty room image for a generation.
    #[must_use]
    pub(crate) fn empty(generation: Generation) -> Self {
        Self {
            version: 0,
            generation,
            rooms: BTreeMap::new(),
        }
    }
}

/// A participant inside the active call, keyed by `(account, attempt, user)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallParticipant {
    /// Account observing the call.
    pub account: AccountId,
    /// Call attempt owning this participant row.
    pub attempt: CallAttemptId,
    /// Backend user identity.
    pub user: Uuid,
    /// Known server-reported device flags (`None` only if still cold before `voice_joined`).
    pub device: Option<VoiceDeviceState>,
    /// Server active-speaker state; unaffected by local playback mute or gain.
    pub speaking: SpeakingState,
    /// Local receiver playback settings for this participant.
    pub playback: PlaybackSettings,
}

/// One requested remote video stream expressed by the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VideoTarget {
    /// Publisher user identity.
    pub participant: Uuid,
    /// Requested source kind (camera or screen).
    pub kind: VideoKind,
}

/// One active SFU video slot binding driven exclusively by `voice_video_routes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoBinding {
    /// SFU track identifier (`papo-video-N`).
    pub track_id: String,
    /// Publisher user identity.
    pub participant: Uuid,
    /// Source kind (`Camera` or `Screen`).
    pub kind: VideoKind,
    /// First translated RTP sequence number for this occupant when the slot was reused.
    pub first_seq: Option<u16>,
    /// Currently bound host video destination identity, if attached by the client.
    pub destination_id: Option<VideoDestinationId>,
}

/// Observable state of a single call attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSnapshot {
    /// Monotonic publication version within this attempt.
    pub version: u64,
    /// Account owning this attempt.
    pub account: AccountId,
    /// Connection generation owning this attempt.
    pub generation: Generation,
    /// Unique identifier for this call attempt.
    pub attempt: CallAttemptId,
    /// Voice channel for this call attempt.
    pub channel: Uuid,
    /// Lifecycle phase (`Preparing` → `Joining` → `Joined` → `Connected` → `Ended`).
    pub phase: CallPhase,
    /// Opaque backend peer identifier once `voice_joined` has been accepted.
    pub peer_id: Option<String>,
    /// Room video codec announced by `voice_joined`.
    pub video_codec: Option<VideoCodec>,
    /// Desired local settings (mute intent, camera intent, camera facing).
    pub settings: CallSettings,
    /// Actual local microphone capture state (independent of mute intent).
    pub mic_capture: CaptureState,
    /// Actual local camera capture state (independent of camera intent).
    pub camera_capture: CaptureState,
    /// Media engine WebRTC transport state.
    pub transport: MediaTransportState,
    /// Non-fatal call warnings.
    pub warnings: Vec<CallWarning>,
    /// Per-user local playback overrides.
    pub playback: BTreeMap<Uuid, PlaybackSettings>,
    /// Active call participants (including self), sorted by user UUID.
    pub participants: Vec<CallParticipant>,
    /// Whether `voice_audio_routes` has been received for this peer.
    pub audio_routes_known: bool,
    /// Current SFU audio track-to-user mapping (`track_id` → `user`).
    pub audio_routes: BTreeMap<String, Uuid>,
    /// Current SFU video slot bindings driven solely by `voice_video_routes`.
    pub video_bindings: Vec<VideoBinding>,
    /// Whether the host currently holds audio focus (informative; regaining never unmutes).
    pub host_audio_focus: bool,
}

impl CallSnapshot {
    /// Terminal end reason when `phase` is `CallPhase::Ended`.
    #[must_use]
    pub const fn end_reason(&self) -> Option<CallEndReason> {
        self.phase.end_reason()
    }
}
