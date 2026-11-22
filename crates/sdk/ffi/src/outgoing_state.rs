//! Durable outgoing operation states, failure diagnostics, queue snapshots, and listeners.
//! Attachment/draft inputs live in `outgoing_payload.rs`; queue commands in `outgoing.rs`.

use papo_sdk::{
    Operation, OperationFailure, OperationKind, OperationState, OutgoingFault, RetryDecision,
    UploadProgress,
};

use crate::{
    expressions::FfiGif,
    ids::FfiAccountId,
    messages::FfiMessage,
    outgoing_payload::{
        FfiEmbedInput, FfiMessageTarget, FfiOutgoingAttachmentView, FfiOutgoingMessageInput,
    },
};

/// Durable operation state. Only `Failed` (or `Unknown` with `AcceptPossibleDuplicate`) is retriable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiOperationState {
    /// Queued locally, awaiting dispatch.
    Queued,
    /// Currently claimed and sending (ambiguous if interrupted).
    Sending,
    /// Confirmed sent with server receipt.
    Sent,
    /// Definite non-transient failure or broken local dependency.
    Failed,
    /// Ambiguous outcome; must ask user before `AcceptPossibleDuplicate` retry.
    Unknown,
}

impl From<OperationState> for FfiOperationState {
    fn from(v: OperationState) -> Self {
        match v {
            OperationState::Queued => Self::Queued,
            OperationState::Sending => Self::Sending,
            OperationState::Sent => Self::Sent,
            OperationState::Failed => Self::Failed,
            // Unknown and future states map conservatively to Unknown.
            _ => Self::Unknown,
        }
    }
}

/// Typed durable failure cause on a `Failed` operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiOperationFailure {
    /// Backend rejected non-transiently with HTTP status.
    Rejected {
        /// Backend HTTP status code.
        status: u16,
    },
    /// Required local predecessor operation was discarded or failed.
    Dependency,
}

impl From<OperationFailure> for FfiOperationFailure {
    fn from(v: OperationFailure) -> Self {
        match v {
            OperationFailure::Rejected(status) => Self::Rejected { status },
            // Dependency and future variants map to Dependency.
            _ => Self::Dependency,
        }
    }
}

/// Explicit retry confirmation required by SDK before re-sending ambiguous operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRetryDecision {
    /// Retry only when operation is definitively `Failed`.
    DefiniteFailure,
    /// Retry an `Unknown` operation acknowledging duplicate delivery is possible.
    AcceptPossibleDuplicate,
}

impl From<FfiRetryDecision> for RetryDecision {
    fn from(v: FfiRetryDecision) -> Self {
        match v {
            FfiRetryDecision::DefiniteFailure => Self::DefiniteFailure,
            FfiRetryDecision::AcceptPossibleDuplicate => Self::AcceptPossibleDuplicate,
        }
    }
}

/// Kind union describing a committed outgoing queue entry without raw byte buffers.
#[derive(Clone, uniffi::Enum)]
pub enum FfiOperationKindView {
    /// Multipart message creation.
    Message {
        /// Canonical message text.
        content: String,
        /// Optional reply target.
        reply_to: Option<FfiMessageTarget>,
        /// Custom embed inputs.
        embeds: Vec<FfiEmbedInput>,
        /// Committed attachment references/releases.
        attachments: Vec<FfiOutgoingAttachmentView>,
        /// Attached GIF metadata if any.
        gif: Option<FfiGif>,
    },
    /// Message edit operation.
    Edit {
        /// Remote or local target message.
        target: FfiMessageTarget,
        /// Replacement canonical text.
        content: String,
        /// Optional replacement embeds.
        embeds: Option<Vec<FfiEmbedInput>>,
    },
    /// Message delete operation.
    Delete {
        /// Remote or local target message.
        target: FfiMessageTarget,
    },
    /// Durable reaction membership change.
    Reaction {
        /// Remote or local target message.
        target: FfiMessageTarget,
        /// Custom emoji UUID string if custom.
        emoji_id: Option<String>,
        /// Unicode sequence if unicode.
        unicode: Option<String>,
        /// True for add, false for remove.
        add: bool,
    },
}

impl From<&OperationKind> for FfiOperationKindView {
    fn from(kind: &OperationKind) -> Self {
        match kind {
            OperationKind::Message(msg) => Self::Message {
                content: msg.content.clone(),
                reply_to: msg.reply_to.as_ref().map(Into::into),
                embeds: msg.embeds.iter().map(Into::into).collect(),
                attachments: msg.attachments.iter().map(Into::into).collect(),
                gif: msg.gif.as_ref().map(Into::into),
            },
            OperationKind::Edit {
                target,
                content,
                embeds,
            } => Self::Edit {
                target: target.into(),
                content: content.clone(),
                embeds: embeds
                    .as_ref()
                    .map(|list| list.iter().map(Into::into).collect()),
            },
            OperationKind::Delete { target } => Self::Delete {
                target: target.into(),
            },
            OperationKind::Reaction {
                target,
                emoji_id,
                unicode,
                add,
            } => Self::Reaction {
                target: target.into(),
                emoji_id: emoji_id.map(|u| u.to_string()),
                unicode: unicode.clone(),
                add: *add,
            },
        }
    }
}

/// Outgoing mutation kind submitted to `enqueue`.
#[derive(Clone, uniffi::Enum)]
pub enum FfiOperationKindInput {
    /// Send a multipart message.
    Message {
        /// Message payload.
        message: FfiOutgoingMessageInput,
    },
    /// Edit an existing or local message.
    Edit {
        /// Target message.
        target: FfiMessageTarget,
        /// Replacement canonical text.
        content: String,
        /// Optional replacement embeds.
        embeds: Option<Vec<FfiEmbedInput>>,
    },
    /// Delete an existing or local message.
    Delete {
        /// Target message.
        target: FfiMessageTarget,
    },
    /// Add or remove reaction durably.
    Reaction {
        /// Target message.
        target: FfiMessageTarget,
        /// Custom emoji UUID string.
        emoji_id: Option<String>,
        /// Unicode emoji sequence.
        unicode: Option<String>,
        /// Desired membership.
        add: bool,
    },
}

/// Committed durable operation record projected to UI/Kotlin without local file paths.
#[derive(Clone, uniffi::Record)]
pub struct FfiOperation {
    /// Caller-generated local operation UUID.
    pub id: String,
    /// Target channel or DM UUID.
    pub channel_id: String,
    /// Operation intent.
    pub kind: FfiOperationKindView,
    /// Current durable phase.
    pub state: FfiOperationState,
    /// Attempt counter.
    pub attempts: u32,
    /// Pending local dependency operation UUID if any.
    pub depends_on: Option<String>,
    /// Failure cause when `state == Failed`.
    pub failure: Option<FfiOperationFailure>,
    /// Backend message UUID once confirmed.
    pub message_id: Option<String>,
    /// True once projected into canonical history.
    pub history_projected: bool,
    /// Server-returned message payload when available.
    pub message: Option<FfiMessage>,
    /// Creation Unix microseconds.
    pub created_at_us: i64,
}

impl From<&Operation> for FfiOperation {
    fn from(op: &Operation) -> Self {
        Self {
            id: op.id.to_string(),
            channel_id: op.channel_id.to_string(),
            kind: op.kind.as_ref().into(),
            state: op.state.into(),
            attempts: op.attempts,
            depends_on: op.depends_on.map(|id| id.to_string()),
            failure: op.failure.map(Into::into),
            message_id: op.message_id.map(|id| id.to_string()),
            history_projected: op.history_projected,
            message: op.message.as_deref().map(Into::into),
            created_at_us: op.created_at_us,
        }
    }
}

/// Ephemeral HTTP attachment upload byte counter for an active `Sending` operation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUploadProgress {
    /// Owning account.
    pub account: FfiAccountId,
    /// Active operation UUID string.
    pub operation_id: String,
    /// Attachment payload bytes consumed by HTTP transport.
    pub bytes_uploaded: u64,
    /// Total attachment bytes across this operation.
    pub total_bytes: u64,
}

impl From<UploadProgress> for FfiUploadProgress {
    fn from(v: UploadProgress) -> Self {
        Self {
            account: FfiAccountId::from_sdk(v.account),
            operation_id: v.operation_id.to_string(),
            bytes_uploaded: v.bytes_uploaded,
            total_bytes: v.total_bytes,
        }
    }
}

/// Sanitized dispatcher health fault unrelated to individual item retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiOutgoingFault {
    /// Storage read/write fault.
    Storage,
    /// Session credentials or transport unavailable.
    Unavailable,
}

/// Coalesced outgoing queue image across all pending and recent operations for an account.
#[derive(Clone, uniffi::Record)]
pub struct FfiOutgoingSnapshot {
    /// Owning account if attached to a live session.
    pub account: Option<FfiAccountId>,
    /// Monotonic outgoing publication version.
    pub version: u64,
    /// Durable operation entries in insertion order.
    pub operations: Vec<FfiOperation>,
    /// Active upload progress for currently sending attachment operation if any.
    pub upload_progress: Option<FfiUploadProgress>,
    /// Dispatcher health fault if any.
    pub fault: Option<FfiOutgoingFault>,
}

/// Listener for outgoing queue snapshots and progress updates.
#[uniffi::export(with_foreign)]
pub trait FfiOutgoingListener: Send + Sync {
    /// Initial atomic queue snapshot.
    fn on_snapshot(&self, snapshot: FfiOutgoingSnapshot);
    /// Subsequent updates on enqueue, progress, receipt, or retry.
    fn on_update(&self, snapshot: FfiOutgoingSnapshot);
    /// Terminal notification on account revocation.
    fn on_closed(&self);
}

/// Maps SDK `OutgoingSnapshot` into FFI snapshot.
pub(crate) fn map_outgoing_snapshot(snap: &papo_sdk::OutgoingSnapshot) -> FfiOutgoingSnapshot {
    FfiOutgoingSnapshot {
        account: snap.account.map(FfiAccountId::from_sdk),
        version: snap.version,
        operations: snap
            .operations
            .iter()
            .map(|op| op.as_ref().into())
            .collect(),
        upload_progress: snap.upload_progress.map(Into::into),
        fault: snap.fault.map(|f| match f {
            OutgoingFault::Storage => FfiOutgoingFault::Storage,
            // Unavailable and future faults map to Unavailable.
            _ => FfiOutgoingFault::Unavailable,
        }),
    }
}
