//! Session-owned call control, voice reducer, media port and client-wide single-call lease.
//! `GStreamer`, UI projections and video rendering live outside `papo-sdk`.

pub(crate) mod attempt;
pub(crate) mod controls;
pub(crate) mod coordinator;
pub(crate) mod errors;
#[cfg(any(test, feature = "test-support"))]
pub mod fake_media;
pub(crate) mod fifo;
pub(crate) mod gateway_events;
pub(crate) mod handle;
pub(crate) mod lease;
pub(crate) mod lifecycle;
pub(crate) mod media_messages;
pub(crate) mod media_port;
pub(crate) mod media_reports;
pub(crate) mod model;
pub(crate) mod reducer;
pub(crate) mod reducer_routes;
pub(crate) mod snapshots;
pub(crate) mod startup;
pub(crate) mod subscriptions;

#[cfg(test)]
mod coordinator_tests;
#[cfg(test)]
mod finding_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod reducer_tests;
#[cfg(test)]
mod regression_tests;

pub use errors::{
    VoiceErrorCode, VoiceErrorDisposition, classify_voice_error, is_fatal_voice_code,
};
pub use handle::{CallHandle, Calls};
pub use media_port::{
    AudioRoute, CaptureState, IceServerConfig, InitialMediaSettings, MEDIA_REPORT_CHANNEL_CAPACITY,
    MediaCommand, MediaEngineFactory, MediaFatalError, MediaLease, MediaLeaseError, MediaReport,
    MediaReportKind, MediaReportSink, MediaReportSinkError, MediaStartError, MediaStartFuture,
    MediaTransportState, MediaWarning, VideoDestination, VideoDestinationId, VideoRouteFence,
    VideoSlotBinding,
};
pub use model::{
    CallAttemptId, CallConfig, CallEndReason, CallError, CallParticipant, CallPhase, CallSettings,
    CallSnapshot, CallWarning, CameraFacing, MAX_PLAYBACK_GAIN_PERCENT, PlaybackSettings,
    RoomMember, RoomsSnapshot, SpeakingState, VideoBinding, VideoCodec, VideoKind, VideoTarget,
    VoiceDeviceState, VoiceRoom,
};

#[cfg(any(test, feature = "test-support"))]
pub use fake_media::{
    FakeMediaFactory, FakeMediaLeaseHandle, FakeStartBehavior, FakeVideoDestination,
};

pub(crate) use coordinator::SessionCalls;
pub(crate) use fifo::{QueuedVoiceCommand, VoiceCommandReceiver};
pub(crate) use lease::ClientCallLease;

use crate::{Generation, session::SessionInner};
use papo_api::gateway::{Event, KnownEvent};

/// Returns true for voice-only gateway events that bypass the reconcile journal and `session.events`.
#[must_use]
pub(crate) fn is_voice_only_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Known(
            KnownEvent::VoiceJoined(_)
                | KnownEvent::VoiceAnswer(_)
                | KnownEvent::VoiceOffer(_)
                | KnownEvent::VoiceICECandidate(_)
                | KnownEvent::VoiceStateUpdate(_)
                | KnownEvent::VoiceLeave(_)
                | KnownEvent::ActiveSpeakerUpdate(_)
                | KnownEvent::VoiceAudioRoutes(_)
                | KnownEvent::VoiceVideoRoutes(_)
        )
    )
}

/// Applies an incoming gateway event once, in socket order, straight from the receive path.
pub(crate) fn receive_gateway_event(
    session: &SessionInner,
    generation: Generation,
    event: &Event,
    runtime: &tokio::runtime::Handle,
) {
    session.calls.on_gateway_event(generation, event, runtime);
}
