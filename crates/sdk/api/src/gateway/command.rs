//! Owns client-to-server gateway command shapes and handler-level validation; the server owns authorization.

use serde::Serialize;

/// Video source requested from a publisher. Audio is not subscribed this way.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Camera video.
    Video,
    /// Screen-share video.
    Screen,
}

/// A client-to-server gateway command on the owning connection.
///
/// Voice join checks `connect_voice`; subsequent voice commands require membership
/// on this same connection. Sending does not imply acceptance by the server.
/// SDP and ICE payloads are deliberately not exposed through `Debug`.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Command {
    /// Report actual user activity, not a keepalive.
    PresenceActivity,
    /// Request a heartbeat acknowledgement. The gateway also sends its own heartbeats.
    Heartbeat,
    /// Report typing activity. The backend always broadcasts a start, never a stop.
    Typing {
        /// Nonempty channel identifier.
        channel_id: String,
    },
    /// Join voice on this socket.
    VoiceJoin {
        /// Nonempty voice channel identifier.
        channel_id: String,
    },
    /// Leave voice on this socket.
    VoiceLeave {
        /// Nonempty voice channel identifier.
        channel_id: String,
    },
    /// Send a client-initiated SDP offer.
    VoiceOffer {
        /// Nonempty voice channel identifier.
        channel_id: String,
        /// Nonempty SDP offer.
        sdp: String,
    },
    /// Answer a server-initiated SDP renegotiation.
    VoiceAnswer {
        /// Nonempty voice channel identifier.
        channel_id: String,
        /// Nonempty SDP answer.
        sdp: String,
    },
    /// Send a trickle ICE candidate. Null mid and index become empty string and zero on the server.
    VoiceIceCandidate {
        /// Nonempty voice channel identifier.
        channel_id: String,
        /// Nonempty candidate.
        candidate: String,
        /// Optional SDP media identifier.
        sdp_mid: Option<String>,
        /// Optional SDP media line index; the backend accepts signed integers.
        sdp_mline_index: Option<i64>,
    },
    /// Subscribe to a publisher's video or screen track.
    TrackSubscribe {
        /// Nonempty voice channel identifier.
        channel_id: String,
        /// Nonempty publisher identifier.
        publisher_id: String,
        /// Source kind.
        kind: TrackKind,
    },
    /// Stop forwarding a publisher's video or screen track.
    TrackUnsubscribe {
        /// Nonempty voice channel identifier.
        channel_id: String,
        /// Nonempty publisher identifier.
        publisher_id: String,
        /// Source kind.
        kind: TrackKind,
    },
    /// Set microphone mute state.
    VoiceMute {
        /// Nonempty voice channel identifier.
        channel_id: String,
        /// Whether the microphone is muted.
        muted: bool,
    },
    /// Set camera state.
    VoiceCamera {
        /// Nonempty voice channel identifier.
        channel_id: String,
        /// Whether the camera is on.
        on: bool,
    },
    /// Mark screen sharing as started; the track arrives in a subsequent negotiation.
    ScreenShareStart {
        /// Nonempty voice channel identifier.
        channel_id: String,
    },
    /// Mark screen sharing as stopped.
    ScreenShareStop {
        /// Nonempty voice channel identifier.
        channel_id: String,
    },
}

impl Command {
    /// Serialize the exact JSON sent to the gateway.
    ///
    /// # Errors
    /// Returns `InvalidCommand` for the backend handlers' empty required strings,
    /// or `Encode` if JSON serialization fails. Authorization is not checked here.
    pub fn encode(&self) -> Result<String, super::Error> {
        let valid = match self {
            Self::PresenceActivity | Self::Heartbeat => true,
            Self::Typing { channel_id }
            | Self::VoiceJoin { channel_id }
            | Self::VoiceLeave { channel_id }
            | Self::VoiceMute { channel_id, .. }
            | Self::VoiceCamera { channel_id, .. }
            | Self::ScreenShareStart { channel_id }
            | Self::ScreenShareStop { channel_id } => !channel_id.is_empty(),
            Self::VoiceOffer { channel_id, sdp } | Self::VoiceAnswer { channel_id, sdp } => {
                !channel_id.is_empty() && !sdp.is_empty()
            }
            Self::VoiceIceCandidate {
                channel_id,
                candidate,
                ..
            } => !channel_id.is_empty() && !candidate.is_empty(),
            Self::TrackSubscribe {
                channel_id,
                publisher_id,
                ..
            }
            | Self::TrackUnsubscribe {
                channel_id,
                publisher_id,
                ..
            } => !channel_id.is_empty() && !publisher_id.is_empty(),
        };
        if !valid {
            return Err(super::Error::InvalidCommand);
        }
        serde_json::to_string(self).map_err(|_| super::Error::Encode)
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
