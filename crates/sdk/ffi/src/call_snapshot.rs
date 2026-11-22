//! Attempt-level snapshot records and device/playback structs for active calls across FFI.
//! Joined UI room/participant views live in `call_view.rs`.

use std::sync::Arc;

use papo_sdk::{
    CallParticipant, CallSettings, CallSnapshot, PlaybackSettings, VideoBinding, VideoTarget,
    VoiceDeviceState,
};

use crate::{
    call_enums::{
        FfiCallPhase, FfiCallWarning, FfiCameraFacing, FfiCaptureState, FfiMediaTransportState,
        FfiSpeakingState, FfiVideoCodec, FfiVideoKind,
    },
    ids::{FfiAccountId, parse_uuid},
    operation_errors::FfiCallError,
};

/// Desired local call settings (`muted = true`, `camera_on = false` on join).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCallSettings {
    /// Desired local microphone mute intent.
    pub muted: bool,
    /// Desired local camera publish intent.
    pub camera_on: bool,
    /// Desired camera facing direction.
    pub camera_facing: FfiCameraFacing,
}

impl From<CallSettings> for FfiCallSettings {
    fn from(s: CallSettings) -> Self {
        Self {
            muted: s.muted,
            camera_on: s.camera_on,
            camera_facing: s.camera_facing.into(),
        }
    }
}

/// Independent local receiver playback settings (`0..=200` percent + mute).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPlaybackSettings {
    /// Receiver volume gain percentage (`0..=200`).
    pub gain_percent: u16,
    /// Local receiver mute override.
    pub muted: bool,
}

impl From<PlaybackSettings> for FfiPlaybackSettings {
    fn from(p: PlaybackSettings) -> Self {
        Self {
            gain_percent: p.gain_percent,
            muted: p.muted,
        }
    }
}

/// Server-reported participant device state flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiVoiceDeviceState {
    /// Server-side muted state.
    pub muted: bool,
    /// Active camera publish state.
    pub camera_on: bool,
    /// Active screen-share state.
    pub screen_sharing: bool,
}

impl From<VoiceDeviceState> for FfiVoiceDeviceState {
    fn from(d: VoiceDeviceState) -> Self {
        Self {
            muted: d.muted,
            camera_on: d.camera_on,
            screen_sharing: d.screen_sharing,
        }
    }
}

/// Requested remote video stream target supplied to `set_video_interest`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiVideoTarget {
    /// Publishing user UUID string.
    pub participant_user_id: String,
    /// Requested media kind.
    pub kind: FfiVideoKind,
}

impl FfiVideoTarget {
    /// Parses into `papo_sdk::VideoTarget`.
    ///
    /// # Errors
    /// Returns [`FfiCallError::InvalidArgument`] if participant UUID is malformed.
    pub(crate) fn into_sdk(self) -> Result<VideoTarget, FfiCallError> {
        Ok(VideoTarget {
            participant: parse_uuid(&self.participant_user_id)
                .map_err(|_| FfiCallError::InvalidArgument)?,
            kind: self.kind.into(),
        })
    }
}

/// Attached host video route on a call snapshot.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiVideoBinding {
    /// SFU track ID.
    pub track_id: String,
    /// Publishing user UUID string.
    pub participant_user_id: String,
    /// Stream kind.
    pub kind: FfiVideoKind,
    /// Legacy translated sequence hint if present.
    pub first_seq: Option<u16>,
    /// Attached host destination ID if bound with valid route fence.
    pub destination_id: Option<u64>,
}

impl From<&VideoBinding> for FfiVideoBinding {
    fn from(b: &VideoBinding) -> Self {
        Self {
            track_id: b.track_id.clone(),
            participant_user_id: b.participant.to_string(),
            kind: b.kind.into(),
            first_seq: b.first_seq,
            destination_id: b.destination_id.map(papo_sdk::VideoDestinationId::raw),
        }
    }
}

/// Attempt-level participant state row inside `FfiCallSnapshot`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCallParticipant {
    /// Owning account.
    pub account: FfiAccountId,
    /// Attempt raw ID.
    pub attempt_id: u64,
    /// Participant user UUID string.
    pub user_id: String,
    /// Known server device flags.
    pub device: Option<FfiVoiceDeviceState>,
    /// Server active-speaker state.
    pub speaking: FfiSpeakingState,
    /// Independent receiver playback settings.
    pub playback: FfiPlaybackSettings,
}

impl From<&CallParticipant> for FfiCallParticipant {
    fn from(p: &CallParticipant) -> Self {
        Self {
            account: FfiAccountId::from_sdk(p.account),
            attempt_id: p.attempt.raw(),
            user_id: p.user.to_string(),
            device: p.device.map(Into::into),
            speaking: p.speaking.into(),
            playback: p.playback.into(),
        }
    }
}

/// Authoritative SFU audio slot route entry.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiAudioTrackRoute {
    /// SFU track identifier.
    pub track_id: String,
    /// User UUID string mapped to that track.
    pub user_id: String,
}

/// Direct snapshot emitted by an individual `FfiCallHandle`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCallSnapshot {
    /// Monotonic attempt version.
    pub version: u64,
    /// Owning account.
    pub account: FfiAccountId,
    /// Connection epoch owning this attempt.
    pub generation: u64,
    /// Unique call attempt raw ID.
    pub attempt_id: u64,
    /// Target voice channel UUID string.
    pub channel_id: String,
    /// Lifecycle phase.
    pub phase: FfiCallPhase,
    /// Backend peer ID once joined.
    pub peer_id: Option<String>,
    /// Announced room video codec.
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
    /// Active participants.
    pub participants: Vec<FfiCallParticipant>,
    /// Whether `voice_audio_routes` was received.
    pub audio_routes_known: bool,
    /// Active SFU audio track mappings.
    pub audio_routes: Vec<FfiAudioTrackRoute>,
    /// Active SFU video slot bindings.
    pub video_bindings: Vec<FfiVideoBinding>,
    /// Informative host audio focus flag.
    pub host_audio_focus: bool,
}

impl From<&CallSnapshot> for FfiCallSnapshot {
    fn from(s: &CallSnapshot) -> Self {
        Self {
            version: s.version,
            account: FfiAccountId::from_sdk(s.account),
            generation: s.generation.get(),
            attempt_id: s.attempt.raw(),
            channel_id: s.channel.to_string(),
            phase: s.phase.into(),
            peer_id: s.peer_id.clone(),
            video_codec: s.video_codec.map(Into::into),
            settings: s.settings.into(),
            mic_capture: s.mic_capture.into(),
            camera_capture: s.camera_capture.into(),
            transport: s.transport.into(),
            warnings: s.warnings.iter().copied().map(Into::into).collect(),
            participants: s.participants.iter().map(Into::into).collect(),
            audio_routes_known: s.audio_routes_known,
            audio_routes: s
                .audio_routes
                .iter()
                .map(|(track_id, user)| FfiAudioTrackRoute {
                    track_id: track_id.clone(),
                    user_id: user.to_string(),
                })
                .collect(),
            video_bindings: s.video_bindings.iter().map(Into::into).collect(),
            host_audio_focus: s.host_audio_focus,
        }
    }
}

/// Cross-account active call handle paired with its owning account identity.
#[derive(Clone, uniffi::Record)]
pub struct FfiActiveAccountCall {
    /// Account currently holding the single client-wide media lease.
    pub account: FfiAccountId,
    /// Handle scoped to the active attempt.
    pub handle: Arc<crate::calls::FfiCallHandle>,
}
