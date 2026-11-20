//! Pure-Rust call state, configuration, participant and error model; owns no I/O or media threads.

use std::time::Duration;

pub use super::snapshots::{
    CallParticipant, CallSnapshot, PlaybackSettings, RoomMember, RoomsSnapshot, SpeakingState,
    VideoBinding, VideoTarget, VoiceDeviceState, VoiceRoom,
};

/// Maximum local per-user playback gain in percent (`0..=200`).
pub const MAX_PLAYBACK_GAIN_PERCENT: u16 = 200;

/// Monotonically increasing identifier for a single call attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallAttemptId(u64);

impl CallAttemptId {
    /// Creates an attempt identifier from a positive monotonic sequence number.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Raw monotonic value for diagnostics and correlation.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Configurable timeouts and queue bounds for session-owned call control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallConfig {
    /// Maximum time to wait for `voice_joined` after sending `voice_join`.
    pub join_timeout: Duration,
    /// Maximum time to wait for media teardown (`Drained`) during leave, switch or shutdown.
    pub drain_timeout: Duration,
    /// Capacity of the bounded voice command FIFO on `AccountSocket`.
    pub command_queue_capacity: usize,
}

impl Default for CallConfig {
    fn default() -> Self {
        Self {
            join_timeout: Duration::from_secs(15),
            drain_timeout: Duration::from_secs(2),
            command_queue_capacity: 256,
        }
    }
}

/// Typed call failures; clients translate these into user-facing text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CallError {
    /// Another account already holds the client-wide call lease.
    #[error("another account already holds the active call")]
    Busy,
    /// The session is not currently connected (`ConnectionState::Online`).
    #[error("session is offline")]
    Offline,
    /// Admitted permissions deny `connect_voice` on this channel (or the channel was deleted).
    #[error("voice permission denied")]
    Forbidden,
    /// Required channel or permission inputs are absent or uncertain.
    #[error("voice permission is unknown")]
    PermissionUnknown,
    /// The target channel is not a voice channel.
    #[error("channel is not a voice channel")]
    NotVoiceChannel,
    /// No media engine factory was configured on `ClientConfig`.
    #[error("media engine unavailable")]
    MediaUnavailable,
    /// A previous media lease missed its drain deadline and still holds hardware devices.
    #[error("previous call media release is still pending")]
    ReleasePending,
    /// This `CallHandle` or host lease belongs to an attempt that has already ended.
    #[error("call attempt has ended")]
    Ended,
    /// The target account session was not found.
    #[error("account session not found")]
    AccountNotFound,
    /// The client or session has been closed.
    #[error("session closed")]
    Closed,
}

/// Lifecycle phase of a single call attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallPhase {
    /// No call is active on this session.
    Idle,
    /// Fetching ICE servers and starting the media lease; `voice_join` has not been sent.
    Preparing,
    /// `voice_join` has been queued on the socket; awaiting `voice_joined`.
    Joining,
    /// `voice_joined` admitted membership; WebRTC negotiation is in progress.
    Joined,
    /// Media transport reached `Connected`.
    Connected,
    /// Terminal state for this attempt.
    Ended(CallEndReason),
}

impl CallPhase {
    /// Whether this phase represents an active (non-idle, non-ended) attempt.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Joining | Self::Joined | Self::Connected
        )
    }

    /// Returns the terminal reason when in `Ended`.
    #[must_use]
    pub const fn end_reason(self) -> Option<CallEndReason> {
        match self {
            Self::Ended(reason) => Some(reason),
            _ => None,
        }
    }
}

/// Typed reason a call attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallEndReason {
    /// The caller explicitly left this attempt.
    LocalLeave,
    /// Replaced by a new call attempt (same-account channel switch/rejoin or cross-account switch).
    Switched,
    /// Ended by a host lifecycle callback (for example, OS foreground service stopped).
    HostTerminated,
    /// The server reported a self-`voice_leave` matching this attempt's `peer_id`.
    ServerRemoved,
    /// The voice channel was deleted or closed by the server (`voice-room-closed`).
    RoomClosed,
    /// Admitted `connect_voice` permission was revoked or the server returned `voice-forbidden`.
    Forbidden,
    /// Admitted permission evidence became uncertain (`Unknown`) after a resource change.
    PermissionUnknown,
    /// The voice room was full during join (`voice-room-full`).
    RoomFull,
    /// The account was already in a room during join (`voice-already-in-room`).
    AlreadyInRoom,
    /// Signalling was rate-limited during join (`voice-rate-limited`).
    RateLimited,
    /// The server rejected the SDP (`voice-invalid-sdp`).
    InvalidSdp,
    /// The room's video codec is unsupported (`voice-codec-unsupported` or media engine rejection).
    CodecUnsupported,
    /// `voice_joined` did not arrive before `CallConfig::join_timeout`.
    JoinTimeout,
    /// Fetching `/voice/ice-servers` failed.
    IceFetchFailed,
    /// Starting the media engine failed.
    DeviceUnavailable,
    /// WebRTC negotiation or transport failed fatally.
    NegotiationFailed,
    /// The bounded voice command FIFO overflowed.
    CommandOverflow,
    /// A required command was rejected by the media lease.
    MediaCommandFailed(super::media_port::MediaLeaseError),
    /// Waiting for a prior call's media drain missed its deadline.
    ReleasePending,
    /// The gateway connection was lost or replaced; calls never auto-rejoin.
    ConnectionLost,
    /// The account logged out.
    LoggedOut,
    /// The account was removed.
    AccountRemoved,
    /// The client shut down.
    Shutdown,
}

/// Non-fatal call warning; does not end the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum CallWarning {
    /// No microphone is available; call continues listen-only.
    MicrophoneUnavailable,
    /// Camera capture failed; audio continues unaffected.
    CameraUnavailable,
    /// Media engine fell back to a software codec.
    CodecFallback,
    /// ICE transport is temporarily disconnected and attempting recovery.
    IceDisconnected,
    /// All SFU video receive slots are currently full.
    VideoSlotsFull,
    /// Signalling or subscription was rate-limited after join; retrying with backoff.
    RateLimited,
}

/// Video stream source kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VideoKind {
    /// Camera stream (`video` on the wire).
    Camera,
    /// Screen-share stream (`screen` on the wire).
    Screen,
}

impl VideoKind {
    /// Parses the backend wire kind (`video` or `screen`).
    #[must_use]
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "video" => Some(Self::Camera),
            "screen" => Some(Self::Screen),
            _ => None,
        }
    }

    /// Converts to the `papo-api` gateway command track kind.
    #[must_use]
    pub const fn to_api(self) -> papo_api::gateway::TrackKind {
        match self {
            Self::Camera => papo_api::gateway::TrackKind::Video,
            Self::Screen => papo_api::gateway::TrackKind::Screen,
        }
    }
}

/// Room video codec announced by `voice_joined`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VideoCodec {
    /// VP8 software default.
    Vp8,
    /// VP9 (out of S9 scope).
    Vp9,
    /// H.264 Main profile (hardware-first).
    H264,
    /// Unrecognized or empty codec string from the backend.
    Unknown,
}

impl VideoCodec {
    /// Parses the backend `video_codec` string.
    #[must_use]
    pub fn from_wire(raw: &str) -> Self {
        match raw.to_ascii_lowercase().as_str() {
            "vp8" => Self::Vp8,
            "vp9" => Self::Vp9,
            "h264" => Self::H264,
            _ => Self::Unknown,
        }
    }
}

/// Desired local camera facing direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CameraFacing {
    /// Front-facing camera.
    #[default]
    Front,
    /// Rear-facing camera.
    Back,
}

/// Desired local call settings; calls always start muted with camera off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallSettings {
    /// Desired local microphone mute intent (`true` at join).
    pub muted: bool,
    /// Desired local camera publish intent (`false` at join).
    pub camera_on: bool,
    /// Desired camera facing direction.
    pub camera_facing: CameraFacing,
}

impl Default for CallSettings {
    fn default() -> Self {
        Self {
            muted: true,
            camera_on: false,
            camera_facing: CameraFacing::Front,
        }
    }
}
