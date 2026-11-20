//! Phase-aware voice error classification, ported from `papo-core`'s `fatal(code, joining)` contract.

use super::model::{CallEndReason, CallWarning};

/// Known backend voice error codes from `backend/internal/webrtc/events.go`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoiceErrorCode {
    /// `voice-not-found`: peer or publisher track not found (subscription race after join).
    NotFound,
    /// `voice-forbidden`: missing `connect_voice` permission or invalid channel type.
    Forbidden,
    /// `voice-room-full`: max room peers (joining) or all video slots occupied (after join).
    RoomFull,
    /// `voice-already-in-room`: user already occupies a room.
    AlreadyInRoom,
    /// `voice-codec-unsupported`: room video codec unsupported.
    CodecUnsupported,
    /// `voice-invalid-sdp`: SDP validation or application failed.
    InvalidSdp,
    /// `voice-rate-limited`: signal or subscribe token bucket exhausted.
    RateLimited,
    /// `voice-room-closed`: room was destroyed or closed.
    RoomClosed,
}

impl VoiceErrorCode {
    /// Parses a backend `error.code` string; non-voice or unknown codes return `None`.
    #[must_use]
    pub fn from_wire(code: &str) -> Option<Self> {
        match code {
            "voice-not-found" => Some(Self::NotFound),
            "voice-forbidden" => Some(Self::Forbidden),
            "voice-room-full" => Some(Self::RoomFull),
            "voice-already-in-room" => Some(Self::AlreadyInRoom),
            "voice-codec-unsupported" => Some(Self::CodecUnsupported),
            "voice-invalid-sdp" => Some(Self::InvalidSdp),
            "voice-rate-limited" => Some(Self::RateLimited),
            "voice-room-closed" => Some(Self::RoomClosed),
            _ => None,
        }
    }
}

/// Outcome of classifying a backend voice error in the current call phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceErrorDisposition {
    /// Ends the active call attempt immediately with the mapped reason.
    Fatal(CallEndReason),
    /// Non-fatal subscription or signalling race after join; keep the call alive.
    NonFatal {
        /// Optional warning to surface on the active call snapshot.
        warning: Option<CallWarning>,
        /// Whether pending video subscriptions should back off and retry.
        retry_subscriptions: bool,
    },
}

/// Classifies a backend voice error according to whether the call is still in `Joining`.
///
/// Always fatal: `room-closed`, `forbidden`, `invalid-sdp`, `codec-unsupported`.
/// Fatal only while joining: `room-full`, `already-in-room`, `rate-limited`.
/// `not-found` is non-fatal (a routine publisher stop/renegotiation race during subscribe).
#[must_use]
pub const fn classify_voice_error(code: VoiceErrorCode, joining: bool) -> VoiceErrorDisposition {
    match code {
        VoiceErrorCode::RoomClosed => VoiceErrorDisposition::Fatal(CallEndReason::RoomClosed),
        VoiceErrorCode::Forbidden => VoiceErrorDisposition::Fatal(CallEndReason::Forbidden),
        VoiceErrorCode::InvalidSdp => VoiceErrorDisposition::Fatal(CallEndReason::InvalidSdp),
        VoiceErrorCode::CodecUnsupported => {
            VoiceErrorDisposition::Fatal(CallEndReason::CodecUnsupported)
        }
        VoiceErrorCode::RoomFull if joining => {
            VoiceErrorDisposition::Fatal(CallEndReason::RoomFull)
        }
        VoiceErrorCode::AlreadyInRoom if joining => {
            VoiceErrorDisposition::Fatal(CallEndReason::AlreadyInRoom)
        }
        VoiceErrorCode::RateLimited if joining => {
            VoiceErrorDisposition::Fatal(CallEndReason::RateLimited)
        }
        VoiceErrorCode::RoomFull => VoiceErrorDisposition::NonFatal {
            warning: Some(CallWarning::VideoSlotsFull),
            retry_subscriptions: true,
        },
        VoiceErrorCode::RateLimited => VoiceErrorDisposition::NonFatal {
            warning: Some(CallWarning::RateLimited),
            retry_subscriptions: true,
        },
        VoiceErrorCode::NotFound => VoiceErrorDisposition::NonFatal {
            warning: None,
            retry_subscriptions: true,
        },
        VoiceErrorCode::AlreadyInRoom => VoiceErrorDisposition::NonFatal {
            warning: None,
            retry_subscriptions: false,
        },
    }
}

/// Returns whether a wire code ends the call in the given phase (ported `papo-core::state::call::fatal`).
#[must_use]
pub fn is_fatal_voice_code(code: &str, joining: bool) -> bool {
    VoiceErrorCode::from_wire(code).is_some_and(|parsed| {
        matches!(
            classify_voice_error(parsed, joining),
            VoiceErrorDisposition::Fatal(_)
        )
    })
}

#[cfg(test)]
mod tests {
    //! Ported `papo-core/src/state/call.rs` error classification tests.

    use super::*;

    #[test]
    fn only_call_ending_errors_end_the_call() {
        for code in [
            "voice-room-closed",
            "voice-forbidden",
            "voice-invalid-sdp",
            "voice-codec-unsupported",
        ] {
            assert!(
                is_fatal_voice_code(code, true),
                "{code} fatal while joining"
            );
            assert!(is_fatal_voice_code(code, false), "{code} fatal after join");
        }
        for code in [
            "voice-room-full",
            "voice-already-in-room",
            "voice-rate-limited",
        ] {
            assert!(
                is_fatal_voice_code(code, true),
                "{code} fatal while joining"
            );
            assert!(
                !is_fatal_voice_code(code, false),
                "{code} non-fatal after join"
            );
        }
        assert!(!is_fatal_voice_code("voice-not-found", true));
        assert!(!is_fatal_voice_code("voice-not-found", false));
        assert!(!is_fatal_voice_code("unrelated-error", true));
    }

    #[test]
    fn post_join_not_found_and_rate_limit_request_subscription_retry() {
        assert_eq!(
            classify_voice_error(VoiceErrorCode::NotFound, false),
            VoiceErrorDisposition::NonFatal {
                warning: None,
                retry_subscriptions: true,
            }
        );
        assert_eq!(
            classify_voice_error(VoiceErrorCode::RateLimited, false),
            VoiceErrorDisposition::NonFatal {
                warning: Some(CallWarning::RateLimited),
                retry_subscriptions: true,
            }
        );
        assert_eq!(
            classify_voice_error(VoiceErrorCode::RoomFull, false),
            VoiceErrorDisposition::NonFatal {
                warning: Some(CallWarning::VideoSlotsFull),
                retry_subscriptions: true,
            }
        );
    }
}
