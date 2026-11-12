//! Durable account-owned operations and drafts; transport is API work, timeline projection belongs to UI.

mod content;
pub(crate) mod dispatch;
pub(crate) mod files;
mod persistence;
mod receipt;
mod service;
#[cfg(test)]
mod tests;

use crate::{AccountId, ClientError, Message, StorageError, Uuid};
pub use content::{
    Draft, DraftBody, MessageTarget, OperationKind, OutgoingAttachment, OutgoingMessage,
};
pub use service::OutgoingService;
use std::sync::{Arc, atomic::AtomicBool};
use tokio::sync::{Mutex, Notify};

/// Persisted dispatch phase. Only Queued is eligible for automatic dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum OperationState {
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

/// Sanitized durable failure category; no backend diagnostic text or payload is retained here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum OperationFailure {
    /// Authoritative non-transient rejection status.
    Rejected(u16),
    /// The predecessor was discarded or is not a same-channel creation.
    Dependency,
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
    /// Unresolved intent and up to 256 recent confirmed receipts, plus receipts needed by dependencies.
    /// Entries follow durable insertion order. Consumer-held snapshots may retain older bytes.
    pub operations: Vec<Arc<Operation>>,
    /// A dispatcher failure leaves committed intent intact and is observable to consumers.
    pub fault: Option<OutgoingFault>,
}

/// Typed local outgoing failure. Descriptions never expose message, file or server data.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OutgoingError {
    /// Invalid content, filename, size or reaction identity.
    #[error("invalid outgoing input")]
    InvalidInput,
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

pub(crate) struct AccountQueue {
    pub image: Arc<crate::observable::StateCell<Arc<OutgoingSnapshot>>>,
    serial: Mutex<()>,
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
                fault: None,
            })),
            serial: Mutex::new(()),
            initialized: AtomicBool::new(false),
            running: AtomicBool::new(false),
            wake: Notify::new(),
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
    /// Credentials or connection preparation failed before dispatch.
    Unavailable,
}

/// Deterministic process-test boundaries; never indicates transport retry safety on its own.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OutgoingDispatchPhase {
    /// Intent is committed Queued; no claim or request occurred.
    Queued,
    /// Sending has completed its FULL commit, before network execution.
    Claimed,
    /// A valid response arrived, before its durable receipt.
    Response,
    /// Receipt or explicit failure/ambiguity has completed its FULL commit.
    Committed,
}
