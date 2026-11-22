//! Enum value types and capability mappings for calls across FFI.
//! Live snapshots live in `call_snapshot.rs` and UI views in `call_view.rs`.

use papo_sdk::{
    CallEndReason, CallPhase, CallWarning, CameraFacing, CaptureState, MediaLeaseError,
    MediaTransportState, SpeakingState, VideoCodec, VideoKind,
};
use papo_sdk_ui::VoiceCapability;

/// Voice connect capability projected from admitted permissions (`connect_voice`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiVoiceCapability {
    /// Voice channel with admitted `connect_voice` while online.
    Allowed,
    /// Denied or channel deleted.
    Denied,
    /// Channel is not a voice channel.
    NotVoiceChannel,
    /// Offline or missing permission evidence.
    Unknown,
}

impl From<VoiceCapability> for FfiVoiceCapability {
    fn from(v: VoiceCapability) -> Self {
        match v {
            VoiceCapability::Allowed => Self::Allowed,
            VoiceCapability::Denied => Self::Denied,
            VoiceCapability::NotVoiceChannel => Self::NotVoiceChannel,
            VoiceCapability::Unknown | _ => Self::Unknown,
        }
    }
}

/// Video stream source kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiVideoKind {
    /// Camera video stream.
    Camera,
    /// Screen share stream.
    Screen,
}

impl From<VideoKind> for FfiVideoKind {
    fn from(v: VideoKind) -> Self {
        match v {
            VideoKind::Camera => Self::Camera,
            VideoKind::Screen => Self::Screen,
        }
    }
}

impl From<FfiVideoKind> for VideoKind {
    fn from(v: FfiVideoKind) -> Self {
        match v {
            FfiVideoKind::Camera => Self::Camera,
            FfiVideoKind::Screen => Self::Screen,
        }
    }
}

/// SFU room video codec announced at join.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiVideoCodec {
    /// VP8 default codec.
    Vp8,
    /// VP9 codec.
    Vp9,
    /// H.264 Main codec.
    H264,
    /// Unrecognized backend codec string.
    Unknown,
}

impl From<VideoCodec> for FfiVideoCodec {
    fn from(v: VideoCodec) -> Self {
        match v {
            VideoCodec::Vp8 => Self::Vp8,
            VideoCodec::Vp9 => Self::Vp9,
            VideoCodec::H264 => Self::H264,
            VideoCodec::Unknown | _ => Self::Unknown,
        }
    }
}

/// Desired local camera facing direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiCameraFacing {
    /// Front camera.
    Front,
    /// Back camera.
    Back,
}

impl From<CameraFacing> for FfiCameraFacing {
    fn from(v: CameraFacing) -> Self {
        match v {
            CameraFacing::Front => Self::Front,
            CameraFacing::Back => Self::Back,
        }
    }
}

impl From<FfiCameraFacing> for CameraFacing {
    fn from(v: FfiCameraFacing) -> Self {
        match v {
            FfiCameraFacing::Front => Self::Front,
            FfiCameraFacing::Back => Self::Back,
        }
    }
}

/// WebRTC media transport connection state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiMediaTransportState {
    /// Initial peer connection.
    New,
    /// ICE/DTLS negotiating.
    Connecting,
    /// Connected.
    Connected,
    /// Temporarily disconnected.
    Disconnected,
    /// Permanently failed.
    Failed,
    /// Closed.
    Closed,
}

impl From<MediaTransportState> for FfiMediaTransportState {
    fn from(v: MediaTransportState) -> Self {
        match v {
            MediaTransportState::New => Self::New,
            MediaTransportState::Connecting => Self::Connecting,
            MediaTransportState::Connected => Self::Connected,
            MediaTransportState::Disconnected => Self::Disconnected,
            MediaTransportState::Failed => Self::Failed,
            MediaTransportState::Closed | _ => Self::Closed,
        }
    }
}

/// Actual physical audio/camera device capture state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiCaptureState {
    /// Intentionally stopped or idle.
    Inactive,
    /// Actively capturing.
    Active,
    /// Hardware unavailable or permission lost.
    Unavailable,
}

impl From<CaptureState> for FfiCaptureState {
    fn from(v: CaptureState) -> Self {
        match v {
            CaptureState::Inactive => Self::Inactive,
            CaptureState::Active => Self::Active,
            CaptureState::Unavailable | _ => Self::Unavailable,
        }
    }
}

/// Non-fatal call warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiCallWarning {
    /// Microphone unavailable (listen-only).
    MicrophoneUnavailable,
    /// Camera unavailable (audio unaffected).
    CameraUnavailable,
    /// Hardware encoder/decoder fell back to software.
    CodecFallback,
    /// ICE disconnected transiently.
    IceDisconnected,
    /// All SFU video receive slots currently busy.
    VideoSlotsFull,
    /// Signalling rate-limited transiently.
    RateLimited,
}

impl From<CallWarning> for FfiCallWarning {
    fn from(v: CallWarning) -> Self {
        match v {
            CallWarning::MicrophoneUnavailable => Self::MicrophoneUnavailable,
            CallWarning::CameraUnavailable => Self::CameraUnavailable,
            CallWarning::CodecFallback => Self::CodecFallback,
            CallWarning::IceDisconnected => Self::IceDisconnected,
            CallWarning::VideoSlotsFull => Self::VideoSlotsFull,
            CallWarning::RateLimited | _ => Self::RateLimited,
        }
    }
}

/// Media lease queue command failure carried inside `CallEndReason::MediaCommandFailed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiMediaLeaseError {
    /// Lease already stopped or closed.
    Closed,
    /// Bounded command queue overflowed.
    Full,
}

impl From<MediaLeaseError> for FfiMediaLeaseError {
    fn from(v: MediaLeaseError) -> Self {
        match v {
            MediaLeaseError::Closed => Self::Closed,
            MediaLeaseError::Full | _ => Self::Full,
        }
    }
}

/// Typed terminal end reason for one call attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiCallEndReason {
    /// Explicit local leave.
    LocalLeave,
    /// Switched to another channel or account call.
    Switched,
    /// Host foreground service / OS terminated the call.
    HostTerminated,
    /// Removed by server (`voice_leave`).
    ServerRemoved,
    /// Voice channel closed or deleted.
    RoomClosed,
    /// Voice permission revoked or denied.
    Forbidden,
    /// Permission evidence became uncertain.
    PermissionUnknown,
    /// Voice room full.
    RoomFull,
    /// Already connected in another room.
    AlreadyInRoom,
    /// Join rate-limited.
    RateLimited,
    /// Server rejected SDP.
    InvalidSdp,
    /// Room video codec unsupported.
    CodecUnsupported,
    /// Join timeout exceeded.
    JoinTimeout,
    /// ICE server fetch failed.
    IceFetchFailed,
    /// Media device startup failed.
    DeviceUnavailable,
    /// WebRTC negotiation or transport failed.
    NegotiationFailed,
    /// Signalling command FIFO overflowed.
    CommandOverflow,
    /// Required media command failed.
    MediaCommandFailed {
        /// Underlying media lease queue error.
        error: FfiMediaLeaseError,
    },
    /// Previous lease drain missed timeout.
    ReleasePending,
    /// Gateway connection lost or superseded.
    ConnectionLost,
    /// Account logged out.
    LoggedOut,
    /// Account removed.
    AccountRemoved,
    /// Client shut down.
    Shutdown,
}

impl From<CallEndReason> for FfiCallEndReason {
    fn from(v: CallEndReason) -> Self {
        match v {
            CallEndReason::LocalLeave => Self::LocalLeave,
            CallEndReason::Switched => Self::Switched,
            CallEndReason::HostTerminated => Self::HostTerminated,
            CallEndReason::ServerRemoved => Self::ServerRemoved,
            CallEndReason::RoomClosed => Self::RoomClosed,
            CallEndReason::Forbidden => Self::Forbidden,
            CallEndReason::PermissionUnknown => Self::PermissionUnknown,
            CallEndReason::RoomFull => Self::RoomFull,
            CallEndReason::AlreadyInRoom => Self::AlreadyInRoom,
            CallEndReason::RateLimited => Self::RateLimited,
            CallEndReason::InvalidSdp => Self::InvalidSdp,
            CallEndReason::CodecUnsupported => Self::CodecUnsupported,
            CallEndReason::JoinTimeout => Self::JoinTimeout,
            CallEndReason::IceFetchFailed => Self::IceFetchFailed,
            CallEndReason::DeviceUnavailable => Self::DeviceUnavailable,
            CallEndReason::NegotiationFailed => Self::NegotiationFailed,
            CallEndReason::CommandOverflow => Self::CommandOverflow,
            CallEndReason::MediaCommandFailed(err) => {
                Self::MediaCommandFailed { error: err.into() }
            }
            CallEndReason::ReleasePending => Self::ReleasePending,
            CallEndReason::ConnectionLost => Self::ConnectionLost,
            CallEndReason::LoggedOut => Self::LoggedOut,
            CallEndReason::AccountRemoved => Self::AccountRemoved,
            CallEndReason::Shutdown | _ => Self::Shutdown,
        }
    }
}

/// Lifecycle phase of a call attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiCallPhase {
    /// Idle (no active call).
    Idle,
    /// Fetching ICE servers and starting media lease.
    Preparing,
    /// Waiting for `voice_joined`.
    Joining,
    /// Joined room, negotiating WebRTC.
    Joined,
    /// Media transport connected.
    Connected,
    /// Terminated with typed end reason.
    Ended {
        /// Terminal cause.
        reason: FfiCallEndReason,
    },
}

impl From<CallPhase> for FfiCallPhase {
    fn from(v: CallPhase) -> Self {
        match v {
            CallPhase::Preparing => Self::Preparing,
            CallPhase::Joining => Self::Joining,
            CallPhase::Joined => Self::Joined,
            CallPhase::Connected => Self::Connected,
            CallPhase::Ended(reason) => Self::Ended {
                reason: reason.into(),
            },
            // Idle and unknown future variants project as Idle.
            _ => Self::Idle,
        }
    }
}

/// Server-driven active-speaker status for a voice occupant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiSpeakingState {
    /// Active-speaker status unknown so far.
    Unknown,
    /// Participant is currently active speaker and unmuted.
    Selected,
    /// Participant is not active speaker (or muted).
    NotSelected,
}

impl From<SpeakingState> for FfiSpeakingState {
    fn from(s: SpeakingState) -> Self {
        match s {
            SpeakingState::Unknown => Self::Unknown,
            SpeakingState::Selected => Self::Selected,
            SpeakingState::NotSelected | _ => Self::NotSelected,
        }
    }
}
