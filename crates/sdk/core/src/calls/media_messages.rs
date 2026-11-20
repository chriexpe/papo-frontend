//! Redacted command, report and status messages exchanged across the pure-Rust media port.
//! Traits, lease factories and bounded report sinks live in `media_port.rs`.

use crate::Uuid;
use std::fmt;

use super::{
    media_port::{AudioRoute, VideoSlotBinding},
    model::{CallAttemptId, CameraFacing, PlaybackSettings, VideoCodec},
};

/// WebRTC media transport state reported by the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaTransportState {
    /// Peer connection not yet negotiating.
    #[default]
    New,
    /// ICE/DTLS connecting.
    Connecting,
    /// Media transport connected.
    Connected,
    /// Temporarily disconnected; may recover.
    Disconnected,
    /// Permanently failed.
    Failed,
    /// Closed.
    Closed,
}

/// Actual device capture state, independent of desired mute/camera intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CaptureState {
    /// Device is intentionally stopped or not opened.
    #[default]
    Inactive,
    /// Device is actively capturing.
    Active,
    /// Device could not be opened or was lost.
    Unavailable,
}

/// Non-fatal warnings reported by the media engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaWarning {
    /// Microphone could not be opened; listen-only mode.
    MicrophoneUnavailable,
    /// Camera could not be opened; audio continues unaffected.
    CameraUnavailable,
    /// Hardware codec unavailable; using software fallback.
    CodecFallback,
    /// ICE temporarily disconnected.
    IceDisconnected,
}

/// Fatal media engine failures that terminate the call attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaFatalError {
    /// The server's room video codec is not supported by this engine.
    UnsupportedCodec,
    /// SDP negotiation failed unrecoverably.
    NegotiationFailed,
    /// ICE/DTLS transport failed permanently.
    TransportFailed,
    /// Fatal audio/media pipeline error.
    DeviceError,
    /// Unexpected internal media engine failure.
    Internal,
}

/// Commands sent from the session call coordinator to the active `MediaLease`.
///
/// `Debug` redacts SDP and ICE candidate strings.
#[derive(Clone)]
#[non_exhaustive]
pub enum MediaCommand {
    /// Room codec learned from `voice_joined`; begins initial SDP offer generation.
    StartNegotiation {
        /// Room video codec announced by the SFU.
        video_codec: VideoCodec,
    },
    /// Remote SDP answer from the SFU.
    RemoteAnswer {
        /// SDP payload (redacted in `Debug`).
        sdp: String,
    },
    /// Remote SDP offer from a server-initiated renegotiation.
    RemoteOffer {
        /// SDP payload (redacted in `Debug`).
        sdp: String,
    },
    /// Remote trickle ICE candidate from the SFU.
    RemoteIceCandidate {
        /// Candidate string (redacted in `Debug`).
        candidate: String,
        /// Optional SDP media identifier.
        sdp_mid: Option<String>,
        /// Optional SDP m-line index.
        sdp_mline_index: Option<i64>,
    },
    /// Update desired microphone mute state.
    SetMuted {
        /// Whether local microphone should be muted.
        muted: bool,
    },
    /// Update desired camera publication state.
    SetCamera {
        /// Whether local camera should publish.
        on: bool,
    },
    /// Update desired camera facing direction.
    SetCameraFacing {
        /// Front or back camera.
        facing: CameraFacing,
    },
    /// Update local receiver gain and mute for a remote user.
    SetPlayback {
        /// Remote user identity.
        user: Uuid,
        /// Independent local gain (`0..=200`) and mute.
        playback: PlaybackSettings,
    },
    /// Replace active SFU audio slot-to-user routes.
    SetAudioRoutes {
        /// Monotonic revision from `voice_audio_routes`.
        revision: u64,
        /// Full replacement snapshot of active audio routes.
        routes: Vec<AudioRoute>,
    },
    /// Replace active SFU video slot assignments and host destinations.
    SetVideoSlotPlan {
        /// Monotonic per-attempt slot-plan revision incremented on every backend video-route
        /// update, departed-member cleanup, and local destination bind/unbind.
        revision: u64,
        /// Full replacement snapshot of active video slot bindings.
        slots: Vec<VideoSlotBinding>,
    },
    /// Begin teardown and emit `MediaReportKind::Drained` once every worker and device is released.
    Stop,
}

impl fmt::Debug for MediaCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartNegotiation { video_codec } => f
                .debug_struct("StartNegotiation")
                .field("video_codec", video_codec)
                .finish(),
            Self::RemoteAnswer { .. } => f
                .debug_struct("RemoteAnswer")
                .field("sdp", &"<redacted>")
                .finish(),
            Self::RemoteOffer { .. } => f
                .debug_struct("RemoteOffer")
                .field("sdp", &"<redacted>")
                .finish(),
            Self::RemoteIceCandidate {
                sdp_mid,
                sdp_mline_index,
                ..
            } => f
                .debug_struct("RemoteIceCandidate")
                .field("candidate", &"<redacted>")
                .field("sdp_mid", sdp_mid)
                .field("sdp_mline_index", sdp_mline_index)
                .finish(),
            Self::SetMuted { muted } => f.debug_struct("SetMuted").field("muted", muted).finish(),
            Self::SetCamera { on } => f.debug_struct("SetCamera").field("on", on).finish(),
            Self::SetCameraFacing { facing } => f
                .debug_struct("SetCameraFacing")
                .field("facing", facing)
                .finish(),
            Self::SetPlayback { user, playback } => f
                .debug_struct("SetPlayback")
                .field("user", user)
                .field("playback", playback)
                .finish(),
            Self::SetAudioRoutes { revision, routes } => f
                .debug_struct("SetAudioRoutes")
                .field("revision", revision)
                .field("routes", routes)
                .finish(),
            Self::SetVideoSlotPlan { revision, slots } => f
                .debug_struct("SetVideoSlotPlan")
                .field("revision", revision)
                .field("slots", slots)
                .finish(),
            Self::Stop => f.write_str("Stop"),
        }
    }
}

/// Attempt-tagged report emitted by the media engine to the session coordinator.
#[derive(Clone)]
pub struct MediaReport {
    /// Call attempt that produced this report.
    pub attempt: CallAttemptId,
    /// Typed report payload.
    pub kind: MediaReportKind,
}

impl fmt::Debug for MediaReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaReport")
            .field("attempt", &self.attempt)
            .field("kind", &self.kind)
            .finish()
    }
}

/// Payload of a `MediaReport`; `Debug` redacts SDP and ICE candidates.
#[derive(Clone)]
#[non_exhaustive]
pub enum MediaReportKind {
    /// Local SDP offer to transmit via `voice_offer`.
    LocalOffer {
        /// SDP offer (redacted in `Debug`).
        sdp: String,
    },
    /// Local SDP answer to transmit via `voice_answer`.
    LocalAnswer {
        /// SDP answer (redacted in `Debug`).
        sdp: String,
    },
    /// Local trickle ICE candidate to transmit via `voice_ice_candidate`.
    LocalIceCandidate {
        /// Candidate payload (redacted in `Debug`).
        candidate: String,
        /// Optional SDP media identifier.
        sdp_mid: Option<String>,
        /// Optional SDP m-line index.
        sdp_mline_index: Option<i64>,
    },
    /// WebRTC transport state change.
    TransportState(MediaTransportState),
    /// Actual local microphone capture state change.
    MicCapture(CaptureState),
    /// Actual local camera capture state change.
    CameraCapture(CaptureState),
    /// Non-fatal media warning.
    Warning(MediaWarning),
    /// Fatal media failure ending the call.
    Fatal(MediaFatalError),
    /// Teardown complete: outputs revoked, pipelines `NULL`, workers joined, devices released.
    Drained,
}

impl fmt::Debug for MediaReportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalOffer { .. } => f
                .debug_struct("LocalOffer")
                .field("sdp", &"<redacted>")
                .finish(),
            Self::LocalAnswer { .. } => f
                .debug_struct("LocalAnswer")
                .field("sdp", &"<redacted>")
                .finish(),
            Self::LocalIceCandidate {
                sdp_mid,
                sdp_mline_index,
                ..
            } => f
                .debug_struct("LocalIceCandidate")
                .field("candidate", &"<redacted>")
                .field("sdp_mid", sdp_mid)
                .field("sdp_mline_index", sdp_mline_index)
                .finish(),
            Self::TransportState(state) => f.debug_tuple("TransportState").field(state).finish(),
            Self::MicCapture(state) => f.debug_tuple("MicCapture").field(state).finish(),
            Self::CameraCapture(state) => f.debug_tuple("CameraCapture").field(state).finish(),
            Self::Warning(warning) => f.debug_tuple("Warning").field(warning).finish(),
            Self::Fatal(error) => f.debug_tuple("Fatal").field(error).finish(),
            Self::Drained => f.write_str("Drained"),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Verifies ICE credentials, SDP and ICE candidates are redacted in `Debug` output.

    use super::*;
    use crate::calls::media_port::IceServerConfig;

    #[test]
    fn ice_server_and_signalling_debug_redacts_secrets() {
        let server = IceServerConfig {
            urls: vec!["turn:turn.example:3478".into()],
            username: Some("1790000000:user-secret".into()),
            credential: Some("super-secret-turn-hmac+abc/==".into()),
        };
        let debug = format!("{server:?}");
        assert!(!debug.contains("super-secret-turn-hmac"));
        assert!(!debug.contains("user-secret"));
        assert!(!debug.contains("turn.example"));

        let offer = MediaCommand::RemoteOffer {
            sdp: "a=ice-pwd:verysecretpassword".into(),
        };
        assert!(!format!("{offer:?}").contains("verysecretpassword"));

        let cand = MediaReport {
            attempt: CallAttemptId::from_raw(1),
            kind: MediaReportKind::LocalIceCandidate {
                candidate: "candidate:1 1 UDP 2122260223 192.0.2.1 54321 typ host".into(),
                sdp_mid: Some("0".into()),
                sdp_mline_index: Some(0),
            },
        };
        assert!(!format!("{cand:?}").contains("192.0.2.1"));
    }
}
