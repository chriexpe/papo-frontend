//! View-ready call and admitted voice-room projections joined with resources across FFI.
//! Attempt-level snapshots live in `call_snapshot.rs`.

use papo_sdk_ui::{
    ActiveCallView, CallParticipantItem, CallView, ParticipantVideoBinding, RoomParticipantItem,
    VoiceRoomView,
};

use crate::{
    call_enums::{
        FfiCallPhase, FfiCallWarning, FfiCaptureState, FfiMediaTransportState, FfiSpeakingState,
        FfiVideoCodec, FfiVideoKind,
    },
    call_snapshot::{FfiCallSettings, FfiPlaybackSettings, FfiVoiceDeviceState},
    ids::FfiAccountId,
};

/// View-ready video binding attached to its publisher participant.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiParticipantVideoBinding {
    /// SFU track identifier.
    pub track_id: String,
    /// Stream kind.
    pub kind: FfiVideoKind,
    /// Legacy first RTP sequence hint if available.
    pub first_seq: Option<u16>,
    /// Bound host destination ID if active.
    pub destination_id: Option<u64>,
}

impl From<&ParticipantVideoBinding> for FfiParticipantVideoBinding {
    fn from(v: &ParticipantVideoBinding) -> Self {
        Self {
            track_id: v.track_id.clone(),
            kind: v.kind.into(),
            first_seq: v.first_seq,
            destination_id: v.destination_id.map(papo_sdk::VideoDestinationId::raw),
        }
    }
}

/// View-ready active call participant joining admitted username/nickname/avatar and video tracks.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCallParticipantItem {
    /// Owning account.
    pub account: FfiAccountId,
    /// Attempt raw ID.
    pub attempt_id: u64,
    /// User UUID string.
    pub user_id: String,
    /// Admitted username.
    pub username: Option<String>,
    /// Admitted nickname.
    pub nickname: Option<String>,
    /// Profile avatar format token.
    pub avatar_format: Option<String>,
    /// Known server device flags.
    pub device: Option<FfiVoiceDeviceState>,
    /// Server speaking state.
    pub speaking: FfiSpeakingState,
    /// Independent receiver playback settings.
    pub playback: FfiPlaybackSettings,
    /// Attached participant video tracks.
    pub video_tracks: Vec<FfiParticipantVideoBinding>,
}

impl From<&CallParticipantItem> for FfiCallParticipantItem {
    fn from(p: &CallParticipantItem) -> Self {
        Self {
            account: FfiAccountId::from_sdk(p.key.account),
            attempt_id: p.key.attempt.raw(),
            user_id: p.user.to_string(),
            username: p.username.clone(),
            nickname: p.nickname.clone(),
            avatar_format: p.avatar_format.clone(),
            device: p.device.map(Into::into),
            speaking: p.speaking.into(),
            playback: p.playback.into(),
            video_tracks: p.video_tracks.iter().map(Into::into).collect(),
        }
    }
}

/// View-ready occupant in a visible voice room list.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoomParticipantItem {
    /// User UUID string.
    pub user_id: String,
    /// Admitted username.
    pub username: Option<String>,
    /// Admitted nickname.
    pub nickname: Option<String>,
    /// Profile avatar format token.
    pub avatar_format: Option<String>,
    /// Known server device flags (`None` on cold sync).
    pub device: Option<FfiVoiceDeviceState>,
    /// Server active-speaker state.
    pub speaking: FfiSpeakingState,
}

impl From<&RoomParticipantItem> for FfiRoomParticipantItem {
    fn from(r: &RoomParticipantItem) -> Self {
        Self {
            user_id: r.user.to_string(),
            username: r.username.clone(),
            nickname: r.nickname.clone(),
            avatar_format: r.avatar_format.clone(),
            device: r.device.map(Into::into),
            speaking: r.speaking.into(),
        }
    }
}

/// Visible admitted voice room occupancy projection.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiVoiceRoomView {
    /// Voice channel UUID string.
    pub channel_id: String,
    /// Sorted occupants with admitted names and speaking state.
    pub participants: Vec<FfiRoomParticipantItem>,
}

impl From<&VoiceRoomView> for FfiVoiceRoomView {
    fn from(v: &VoiceRoomView) -> Self {
        Self {
            channel_id: v.channel.to_string(),
            participants: v.participants.iter().map(Into::into).collect(),
        }
    }
}

/// View-ready projection of the active call attempt joined with resources.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiActiveCallView {
    /// Monotonic version within this attempt.
    pub version: u64,
    /// Owning account.
    pub account: FfiAccountId,
    /// Connection epoch owning this attempt.
    pub generation: u64,
    /// Attempt raw ID.
    pub attempt_id: u64,
    /// Voice channel UUID string.
    pub channel_id: String,
    /// Call phase (`Preparing` -> `Joining` -> `Joined` -> `Connected`).
    pub phase: FfiCallPhase,
    /// Room video codec if announced.
    pub video_codec: Option<FfiVideoCodec>,
    /// Desired local settings.
    pub settings: FfiCallSettings,
    /// Actual microphone capture state.
    pub mic_capture: FfiCaptureState,
    /// Actual camera capture state.
    pub camera_capture: FfiCaptureState,
    /// WebRTC transport state.
    pub transport: FfiMediaTransportState,
    /// Active non-fatal warnings.
    pub warnings: Vec<FfiCallWarning>,
    /// Active call participants with admitted profile/video metadata.
    pub participants: Vec<FfiCallParticipantItem>,
    /// Whether `voice_audio_routes` has been received.
    pub audio_routes_known: bool,
    /// Informative host audio focus flag.
    pub host_audio_focus: bool,
}

impl From<&ActiveCallView> for FfiActiveCallView {
    fn from(a: &ActiveCallView) -> Self {
        Self {
            version: a.version,
            account: FfiAccountId::from_sdk(a.account),
            generation: a.generation.get(),
            attempt_id: a.attempt.raw(),
            channel_id: a.channel.to_string(),
            phase: a.phase.into(),
            video_codec: a.video_codec.map(Into::into),
            settings: a.settings.into(),
            mic_capture: a.mic_capture.into(),
            camera_capture: a.camera_capture.into(),
            transport: a.transport.into(),
            warnings: a.warnings.iter().copied().map(Into::into).collect(),
            participants: a.participants.iter().map(Into::into).collect(),
            audio_routes_known: a.audio_routes_known,
            host_audio_focus: a.host_audio_focus,
        }
    }
}

/// Complete atomic call + voice-room projection for one account.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCallView {
    /// Connection generation owning this projection.
    pub generation: u64,
    /// Active call view if one exists on this account.
    pub active: Option<FfiActiveCallView>,
    /// Visible admitted voice rooms sorted by channel ID.
    pub rooms: Vec<FfiVoiceRoomView>,
}

impl From<&CallView> for FfiCallView {
    fn from(v: &CallView) -> Self {
        Self {
            generation: v.generation.get(),
            active: v.active.as_ref().map(Into::into),
            rooms: v.rooms.values().map(Into::into).collect(),
        }
    }
}
