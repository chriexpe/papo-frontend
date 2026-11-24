//! Durable account-owned operations and drafts; transport is API work, timeline projection belongs to UI.

mod attachment_service;
mod content;
pub(crate) mod dispatch;
mod drafts;
mod enqueue;
mod file_io;
pub(crate) mod files;
mod image_edit;
mod pending;
mod persistence;
mod preparing;
mod progress;
mod projection;
mod receipt;
mod request;
mod service;
mod stream;
#[cfg(test)]
mod tests;

use crate::{AccountId, ClientError, Message, StorageError, Uuid};
pub use content::{
    Draft, DraftBody, MessageTarget, OperationKind, OutgoingAttachment, OutgoingAttachmentContent,
    OutgoingMessage, StagedAttachment,
};
pub use content::{
    MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS_PER_MESSAGE, MAX_MESSAGE_ATTACHMENT_BYTES,
};
pub use pending::{PendingAttachment, StagingFailure};
pub use progress::{AttachmentProgress, UploadProgress};
pub use service::OutgoingService;
use std::sync::{Arc, atomic::AtomicBool};
use tokio::sync::{Mutex, Notify};

/// Persisted dispatch phase. Only Queued is eligible for automatic dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum OperationState {
    /// Committed while one of its files is still being copied into staging; becomes Queued when
    /// every file is ready. Never stored as such: the row holds what it becomes if the process
    /// ends first (Failed, [`OperationFailure::AttachmentLost`]), and this state is shown while
    /// the copy runs. Not dispatched; holds its channel's order like any unresolved work.
    Preparing,
    /// Durable intent, with no dispatch yet claimed.
    Queued,
    /// FULL committed before a single attempt; interruption is ambiguous.
    Sending,
    /// A valid authoritative response was durably recorded.
    Sent,
    /// An explicit rejection or invalid local dependency; user resolution required.
    Failed,
    /// May have reached the backend; automatic retry and content/time matching forbidden.
    Unknown,
}

impl OperationState {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            // Never written: the schema refuses it, so a mistake fails instead of persisting.
            Self::Preparing => "Preparing",
            Self::Queued => "Queued",
            Self::Sending => "Sending",
            Self::Sent => "Sent",
            Self::Failed => "Failed",
            Self::Unknown => "Unknown",
        }
    }

    pub(super) fn from_db(value: &str) -> Result<Self, OutgoingError> {
        match value {
            "Queued" => Ok(Self::Queued),
            "Sending" => Ok(Self::Sending),
            "Sent" => Ok(Self::Sent),
            "Failed" => Ok(Self::Failed),
            "Unknown" => Ok(Self::Unknown),
            _ => Err(OutgoingError::InvalidData),
        }
    }
}

pub(super) enum EnqueueSource {
    Kind(OperationKind),
    Draft { revision: u64 },
}

/// Sanitized durable failure category; no backend diagnostic text or payload is retained here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum OperationFailure {
    /// Authoritative non-transient rejection status.
    Rejected(u16),
    /// The predecessor was discarded or is not a same-channel creation.
    Dependency,
    /// The sender cancelled the upload before the whole message reached the server.
    Cancelled,
    /// A file still being copied was lost (the copy failed, or the process ended first). The
    /// message cannot be retried; discard it and attach the file again.
    AttachmentLost,
    /// A file could not be prepared as asked once its copy ended: anonymizing refused it, or it
    /// passed a size limit or the account's queue budget. Discard only.
    AttachmentRefused,
}

/// Caller decision for retry. Unknown requires acknowledging possible duplicate delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Retry a definitely failed operation only.
    DefiniteFailure,
    /// Explicitly accept that retrying an ambiguous operation may duplicate it.
    AcceptPossibleDuplicate,
}

/// Immutable committed operation image. Debug intentionally excludes its message/file payloads.
#[derive(Clone)]
pub struct Operation {
    /// Caller-generated unique local identity, stable across restarts and retries.
    pub id: Uuid,
    /// Account-local channel or DM identifier.
    pub channel_id: Uuid,
    /// Intent; attachment bytes stay immutable until Sent, then are released.
    pub kind: Arc<OperationKind>,
    /// Current FULL-committed state.
    pub state: OperationState,
    /// Claimed attempts, including interrupted claims before socket dispatch.
    pub attempts: u32,
    /// Pending preceding local create, if any; released after this operation is Sent.
    pub depends_on: Option<Uuid>,
    /// Typed reason for Failed.
    pub failure: Option<OperationFailure>,
    /// Backend ID obtained only from a confirmed response or resolved dependency.
    pub message_id: Option<Uuid>,
    /// Confirmed create is accounted for by canonical history (including deletion), independent of send success.
    pub history_projected: bool,
    /// Confirmed create/edit payload, when supplied by the response.
    pub message: Option<Arc<Message>>,
    /// Original Unix microseconds, never used as send ordering or message matching.
    pub created_at_us: i64,
}

/// Account-scoped, coalescing committed image. Watch delivery never blocks dispatch.
#[derive(Clone)]
pub struct OutgoingSnapshot {
    /// Owning account; every operation is isolated by this identity.
    pub account: Option<AccountId>,
    /// Monotonic session publication version; operation changes appear only after FULL commits.
    pub version: u64,
    /// Unresolved intent, confirmed receipts pending projection, and 256 recent projected receipts
    /// plus receipts needed by dependencies. Pending projection counts toward admission.
    /// Entries follow durable insertion order. Consumer-held snapshots may retain older bytes.
    pub operations: Vec<Arc<Operation>>,
    /// Ephemeral in-memory upload progress for the currently sending attachment operation, if any.
    /// Never persisted; cleared on state transitions, discard, or account closure.
    pub upload_progress: Option<UploadProgress>,
    /// A dispatcher failure leaves committed intent intact and is observable to consumers.
    pub fault: Option<OutgoingFault>,
}

impl OutgoingSnapshot {
    /// Returns ephemeral in-memory upload progress for `operation_id`, when actively uploading.
    #[must_use]
    pub fn progress_for(&self, operation_id: Uuid) -> Option<UploadProgress> {
        self.upload_progress
            .filter(|progress| progress.operation_id == operation_id)
    }
}

/// Typed local outgoing failure. Descriptions never expose message, file or server data.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OutgoingError {
    /// Invalid content, filename, size or reaction identity.
    #[error("invalid outgoing input")]
    InvalidInput,
    /// Caller-supplied attachment reader failed before staging completed.
    #[error("attachment source read failed ({0:?})")]
    Source(std::io::ErrorKind),
    /// Identity is missing, belongs to another account or has an invalid dependency.
    #[error("outgoing identity unavailable")]
    NotFound,
    /// The operation identity already exists; enqueue never implicitly retries it.
    #[error("outgoing identity already exists")]
    AlreadyExists,
    /// Only unresolved, non-dispatching work can be retried or discarded.
    #[error("outgoing operation cannot transition")]
    InvalidState,
    /// Unknown retry requires an explicit possible-duplicate acknowledgement.
    #[error("ambiguous retry needs explicit decision")]
    DuplicateRisk,
    /// A newer draft revision replaced the captured value.
    #[error("draft revision superseded")]
    StaleDraft,
    /// The account's queue admission budget is exhausted.
    #[error("outgoing queue budget exceeded")]
    Budget,
    /// Durable state did not decode to its expected schema.
    #[error("invalid outgoing durable state")]
    InvalidData,
    /// An attachment's anonymizing, crop or turn could not be applied; nothing was sent.
    #[error(transparent)]
    Image(#[from] crate::ImageError),
    /// Durable storage could not acknowledge success.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Session, authentication or executor failure.
    #[error(transparent)]
    Session(#[from] ClientError),
}
impl From<rusqlite::Error> for OutgoingError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.into())
    }
}
impl From<crate::storage_worker::WorkerError> for OutgoingError {
    fn from(error: crate::storage_worker::WorkerError) -> Self {
        Self::Storage(error.into())
    }
}

pub(super) struct StagedPins {
    counts: std::sync::Mutex<std::collections::BTreeMap<String, usize>>,
}

impl Default for StagedPins {
    fn default() -> Self {
        Self {
            counts: std::sync::Mutex::new(std::collections::BTreeMap::new()),
        }
    }
}

pub(super) struct StagedPin {
    digest_hex: String,
    pins: std::sync::Weak<StagedPins>,
}

impl Drop for StagedPin {
    fn drop(&mut self) {
        if let Some(pins) = self.pins.upgrade() {
            let mut counts = pins
                .counts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(count) = counts.get_mut(&self.digest_hex) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    counts.remove(&self.digest_hex);
                }
            }
        }
    }
}

pub(crate) struct AccountQueue {
    pub image: Arc<crate::observable::StateCell<Arc<OutgoingSnapshot>>>,
    serial: Mutex<()>,
    pub(crate) files_serial: Arc<Mutex<()>>,
    /// Held shared by every staging copy and exclusively by account shutdown, which so waits for
    /// copies to stop before the staging directory goes. Copies take `files_serial` only to
    /// publish a finished blob, so collection, enqueue and discard never wait for a long copy.
    pub(crate) files_active: Arc<tokio::sync::RwLock<()>>,
    staged_pins: Arc<StagedPins>,
    active_uploads: std::sync::Mutex<Vec<std::sync::Weak<stream::ActiveUploadSlot>>>,
    /// The attachment upload in flight and the token that cancels it; set only by the dispatcher.
    in_flight: std::sync::Mutex<Option<(Uuid, tokio_util::sync::CancellationToken)>>,
    /// Messages committed while a file is still copying, shown as Preparing; see `preparing`.
    preparing: std::sync::Mutex<std::collections::BTreeMap<Uuid, preparing::Preparing>>,
    initialized: AtomicBool,
    running: AtomicBool,
    wake: Notify,
}
impl Default for AccountQueue {
    fn default() -> Self {
        Self {
            image: crate::observable::StateCell::new(Arc::new(OutgoingSnapshot {
                account: None,
                version: 0,
                operations: Vec::new(),
                upload_progress: None,
                fault: None,
            })),
            serial: Mutex::new(()),
            files_serial: Arc::new(Mutex::new(())),
            files_active: Arc::new(tokio::sync::RwLock::new(())),
            staged_pins: Arc::new(StagedPins::default()),
            active_uploads: std::sync::Mutex::new(Vec::new()),
            in_flight: std::sync::Mutex::new(None),
            preparing: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            initialized: AtomicBool::new(false),
            running: AtomicBool::new(false),
            wake: Notify::new(),
        }
    }
}

impl AccountQueue {
    /// Signals the upload of `operation` to stop, if it is the one in flight.
    pub(super) fn cancel_in_flight(&self, operation: Uuid) -> bool {
        let in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match in_flight.as_ref() {
            Some((id, token)) if *id == operation => {
                token.cancel();
                true
            }
            _ => false,
        }
    }

    /// Atomically clears ephemeral upload progress and closes the outgoing observable.
    pub(crate) fn close_image(&self) {
        self.image.close_with(|current| {
            if current.upload_progress.is_none() {
                return false;
            }
            let mut next = (**current).clone();
            next.upload_progress = None;
            next.version = next.version.saturating_add(1);
            *current = Arc::new(next);
            true
        });
    }

    pub(super) fn pin_staged(&self, digest_hex: String) -> Arc<StagedPin> {
        let mut counts = self
            .staged_pins
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = counts.entry(digest_hex.clone()).or_insert(0);
        *entry = entry.saturating_add(1);
        Arc::new(StagedPin {
            digest_hex,
            pins: Arc::downgrade(&self.staged_pins),
        })
    }

    pub(super) fn pinned_names(&self) -> Vec<String> {
        self.staged_pins
            .counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    fn register_upload_slot(&self, slot: &Arc<stream::ActiveUploadSlot>) {
        let mut slots = self
            .active_uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots.retain(|weak| weak.strong_count() > 0);
        slots.push(Arc::downgrade(slot));
    }

    /// Closes any staged upload file handles still held by an idle HTTP transport stream.
    /// Call after cancelling `root` and awaiting `tasks` so no blocking chunk read is active.
    pub(crate) fn close_active_upload_files(&self) {
        let slots = {
            let mut slots = self
                .active_uploads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *slots)
        };
        for weak in slots {
            if let Some(slot) = weak.upgrade() {
                slot.close();
            }
        }
    }
}

/// Deterministic test-only process interruption boundary; never a production host integration.
#[cfg(feature = "test-support")]
pub trait OutgoingDispatchProbe: Send + Sync {
    /// Pauses a named real dispatch boundary without blocking a Tokio worker.
    fn reached(
        &self,
        id: Uuid,
        phase: OutgoingDispatchPhase,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>>;
}

/// Sanitized account dispatcher health, independent of an operation's durable outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutgoingFault {
    /// Durable read/write/decoding could not complete; Sending is never replayed.
    Storage,
    /// Credentials, connection preparation or canonical receipt projection is unavailable.
    Unavailable,
}

/// Deterministic process-test boundaries; never indicates transport retry safety on its own.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OutgoingDispatchPhase {
    /// First attachment chunk copied on a blocking file task; ID is the operation or draft channel.
    #[cfg(feature = "test-support")]
    Staging,
    /// Intent is committed Queued; no claim or request occurred.
    Queued,
    /// Sending has completed its FULL commit, before network execution.
    Claimed,
    /// A valid response arrived, before its durable receipt.
    Response,
    /// Receipt or explicit failure/ambiguity has completed its FULL commit.
    Committed,
}
