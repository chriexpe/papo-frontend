//! Pure-Rust media engine port implemented by `papo-call` (14b) and test fakes (14a).
//! Never imports `GStreamer`, UI frameworks or wire JSON; redacts ICE credentials and SDP in `Debug`.

use crate::Uuid;
use std::{any::Any, fmt, future::Future, pin::Pin, sync::Arc};
use tokio::sync::mpsc;

pub use super::media_messages::{
    CaptureState, MediaCommand, MediaFatalError, MediaReport, MediaReportKind, MediaTransportState,
    MediaWarning,
};
use super::model::{CallAttemptId, CameraFacing, VideoKind};

/// Bounded capacity for attempt-tagged reports flowing from the media engine to the SDK.
pub const MEDIA_REPORT_CHANNEL_CAPACITY: usize = 256;

/// STUN/TURN server configuration passed to the media engine; `Debug` redacts credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct IceServerConfig {
    /// Server URIs (`stun:...`, `turn:...`, `turns:...`).
    pub urls: Vec<String>,
    /// Optional TURN username (redacted in `Debug`).
    pub username: Option<String>,
    /// Optional TURN credential (redacted in `Debug`).
    pub credential: Option<String>,
}

impl IceServerConfig {
    /// Converts a generated `papo-api` wire `ICEServer` into a redacted SDK config.
    #[must_use]
    pub(crate) fn from_wire(server: papo_api::wire::ICEServer) -> Self {
        Self {
            urls: server.urls.unwrap_or_default(),
            username: server.username,
            credential: server.credential,
        }
    }
}

impl fmt::Debug for IceServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceServerConfig")
            .field("url_count", &self.urls.len())
            .field("has_username", &self.username.is_some())
            .field("has_credential", &self.credential.is_some())
            .finish()
    }
}

/// Initial capture settings handed to `MediaEngineFactory::start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitialMediaSettings {
    /// Initial microphone mute state (defaults to `true`).
    pub muted: bool,
    /// Initial camera state (defaults to `false`).
    pub camera_on: bool,
    /// Initial camera facing direction.
    pub camera_facing: CameraFacing,
}

impl Default for InitialMediaSettings {
    fn default() -> Self {
        Self {
            muted: true,
            camera_on: false,
            camera_facing: CameraFacing::Front,
        }
    }
}

/// One SFU audio slot-to-publisher mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioRoute {
    /// SFU track identifier (`papo-audio-N`).
    pub track_id: String,
    /// Publisher user identity.
    pub user: Uuid,
}

/// Stable opaque identity for a host video destination bound to `(participant, kind)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VideoDestinationId(u64);

impl VideoDestinationId {
    /// Creates a destination identifier from a caller-chosen non-zero or monotonic value.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Raw value for equality and diagnostics.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Opaque host video destination carried through the SDK without platform or GPU types.
///
/// Platform presenters (14c) implement this trait and downcast via `as_any` inside `papo-call`.
pub trait VideoDestination: Send + Sync + 'static {
    /// Stable identity distinguishing bindings for the same `(participant, kind)`.
    fn id(&self) -> VideoDestinationId;

    /// Revokes frame delivery when the call ends or this destination is unbound/replaced.
    fn revoke(&self) {}

    /// Downcast hook for the injected media implementation.
    fn as_any(&self) -> &(dyn Any + Send + Sync);
}

/// RTP extension and assignment IDs for a single SFU video slot assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoRouteFence {
    /// RFC 8285 one-byte extension ID (1–14).
    pub extension_id: u8,
    /// Nonzero assignment ID, stable until the slot is reused or rebound.
    pub assignment_id: u64,
}

/// One slot entry in the media engine's active video slot plan.
#[derive(Clone)]
pub struct VideoSlotBinding {
    /// SFU track identifier (`papo-video-N`).
    pub track_id: String,
    /// Publisher user identity currently forwarded on this slot.
    pub participant: Uuid,
    /// Media kind (`Camera` or `Screen`).
    pub kind: VideoKind,
    /// Validated wire fence, or `None` for legacy or invalid routes.
    pub fence: Option<VideoRouteFence>,
    /// Bound host destination for `(participant, kind)`, only with a valid fence.
    pub destination: Option<Arc<dyn VideoDestination>>,
}

impl fmt::Debug for VideoSlotBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VideoSlotBinding")
            .field("track_id", &self.track_id)
            .field("participant", &self.participant)
            .field("kind", &self.kind)
            .field("fence", &self.fence)
            .field(
                "destination_id",
                &self.destination.as_ref().map(|dest| dest.id()),
            )
            .finish()
    }
}

/// Error delivering a `MediaReport` to the bounded SDK sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaReportSinkError {
    /// The bounded report queue is full.
    #[error("media report queue full")]
    Full,
    /// The call coordinator closed the receiver.
    #[error("media report sink closed")]
    Closed,
}

/// Bounded report sink handed to `MediaEngineFactory::start`.
#[derive(Clone)]
pub struct MediaReportSink {
    tx: mpsc::Sender<MediaReport>,
}

impl MediaReportSink {
    /// Creates a bounded sink and receiver pair for one session or test harness.
    #[must_use]
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<MediaReport>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { tx }, rx)
    }

    /// Attempts to enqueue a report without blocking the calling thread.
    ///
    /// # Errors
    /// Returns `Full` if the bounded channel is saturated or `Closed` after teardown.
    pub fn try_send(&self, report: MediaReport) -> Result<(), MediaReportSinkError> {
        self.tx.try_send(report).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => MediaReportSinkError::Full,
            mpsc::error::TrySendError::Closed(_) => MediaReportSinkError::Closed,
        })
    }

    /// Asynchronously enqueues a report on the bounded channel.
    ///
    /// # Errors
    /// Returns `Closed` if the receiver has been dropped.
    pub async fn send(&self, report: MediaReport) -> Result<(), MediaReportSinkError> {
        self.tx
            .send(report)
            .await
            .map_err(|_| MediaReportSinkError::Closed)
    }
}

/// Errors sending a command to an active `MediaLease`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaLeaseError {
    /// The media lease has already stopped or closed.
    #[error("media lease closed")]
    Closed,
    /// The media lease's bounded command queue is full.
    #[error("media lease command queue full")]
    Full,
}

/// Errors starting a `MediaLease` through `MediaEngineFactory::start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MediaStartError {
    /// Required audio/media subsystem could not be initialized.
    #[error("media device unavailable")]
    DeviceUnavailable,
    /// Requested codec or media configuration is unsupported.
    #[error("unsupported media codec")]
    UnsupportedCodec,
    /// Startup was cancelled before completion.
    #[error("media startup cancelled")]
    Cancelled,
    /// Unexpected engine startup failure.
    #[error("media startup failed")]
    Internal,
}

/// Active media lease for a single call attempt; commands must never block Tokio workers.
pub trait MediaLease: Send + Sync + 'static {
    /// Delivers a command to the media executor without blocking the caller.
    ///
    /// # Errors
    /// Returns `Closed` after stop or `Full` if the engine's bounded queue is saturated.
    fn send(&self, command: MediaCommand) -> Result<(), MediaLeaseError>;

    /// Requests teardown without blocking. The request is idempotent and must always
    /// be delivered, even if the command queue is full or closed.
    ///
    /// In response, after all workers and devices are released, deliver this attempt's
    /// [`MediaReportKind::Drained`] through the report sink. If the report queue is
    /// full, wait for capacity asynchronously. Report drain even if the command
    /// executor had already exited. The SDK holds the device lease until it receives
    /// the report.
    fn stop(&self);
}

/// Future returned by `MediaEngineFactory::start`.
pub type MediaStartFuture =
    Pin<Box<dyn Future<Output = Result<Box<dyn MediaLease>, MediaStartError>> + Send>>;

/// Platform-supplied media engine factory (injected via `ClientConfig::media_engine`).
pub trait MediaEngineFactory: Send + Sync + 'static {
    /// Prepares devices and peer infrastructure off Tokio workers for one attempt.
    ///
    /// A failed or cancelled start must release every acquired resource.
    fn start(
        &self,
        attempt: CallAttemptId,
        ice_servers: Vec<IceServerConfig>,
        settings: InitialMediaSettings,
        reports: MediaReportSink,
    ) -> MediaStartFuture;
}
