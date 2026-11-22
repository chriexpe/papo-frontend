//! Message pin/unpin operations, grouped reactor pagination, and coalesced reaction taps.
//! Human sends/edits/deletes use durable `enqueue_operation` in `outgoing`.

use std::sync::Arc;

use papo_sdk::{
    UnixMicros,
    operations::{
        interactions::{MessageReactionGroup, PinnedMessage, ReactorCursor},
        reactions::{ReactionAdmission, ReactionEmoji, ReactionIntent},
    },
};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid},
    operation_errors::{FfiOperationError, FfiReactionError, FfiRefreshStatus},
    operations_records::{FfiUnitReceipt, closed_operation_error, operation_session},
    outgoing_payload::FfiMessageTarget,
};

/// Pinned message reference returned by `pin_message`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPinnedMessageWire {
    /// Channel UUID string.
    pub channel_id: Option<String>,
    /// Message UUID string.
    pub message_id: Option<String>,
    /// Pinning user UUID string.
    pub pinned_by: Option<String>,
    /// Pin timestamp string.
    pub pinned_at: Option<String>,
}

impl From<&PinnedMessage> for FfiPinnedMessageWire {
    fn from(p: &PinnedMessage) -> Self {
        Self {
            channel_id: p.channel_id.clone(),
            message_id: p.message_id.clone(),
            pinned_by: p.pinned_by.clone(),
            pinned_at: p.pinned_at.clone(),
        }
    }
}

/// Receipt acknowledging message pin.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPinReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Connection generation dispatching pin.
    pub generation: u64,
    /// Returned pin record.
    pub pinned: FfiPinnedMessageWire,
    /// Post-pin refresh status.
    pub refresh: FfiRefreshStatus,
}

/// Opaque grouped reaction pagination cursor scoped to `(account, generation, channel, message)`.
#[derive(Clone, uniffi::Object)]
pub struct FfiReactorCursor {
    /// Underlying SDK reactor cursor.
    pub(crate) inner: ReactorCursor,
}

/// Individual reactor user within one page.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReactionUserWire {
    /// Reaction row ID.
    pub id: Option<String>,
    /// User UUID string.
    pub user_id: Option<String>,
    /// Reaction timestamp string.
    pub created_at: Option<String>,
}

/// Grouped reactions scoped to the current page (never inferred as total message reaction count).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReactionGroupPageItem {
    /// Custom emoji UUID if custom.
    pub emoji_id: Option<String>,
    /// Unicode emoji string if unicode.
    pub unicode: Option<String>,
    /// Page-scoped reaction count.
    pub count: Option<i64>,
    /// Reactor user entries in this group on this page.
    pub users: Vec<FfiReactionUserWire>,
}

impl From<&MessageReactionGroup> for FfiReactionGroupPageItem {
    fn from(g: &MessageReactionGroup) -> Self {
        Self {
            emoji_id: g.emoji_id.clone(),
            unicode: g.unicode.clone(),
            count: g.count,
            users: g
                .users
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|u| FfiReactionUserWire {
                    id: u.id.clone(),
                    user_id: u.user_id.clone(),
                    created_at: u.created_at.clone(),
                })
                .collect(),
        }
    }
}

/// One grouped page of reaction users returned by `reaction_users`.
#[derive(Clone, uniffi::Record)]
pub struct FfiReactorPage {
    /// Connection epoch authorizing this page.
    pub generation: u64,
    /// Page-scoped reaction groups.
    pub groups: Vec<FfiReactionGroupPageItem>,
    /// Continuation cursor when backend reports more rows.
    pub next_cursor: Option<Arc<FfiReactorCursor>>,
}

/// Canonical reaction emoji identifier (`Custom` UUID or `Unicode` up to 16 scalar values).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiReactionEmoji {
    /// Custom server emoji UUID string.
    Custom {
        /// Emoji UUID string.
        emoji_id: String,
    },
    /// Unicode emoji sequence.
    Unicode {
        /// Unicode sequence.
        unicode: String,
    },
}

/// Desired reaction state passed to `set_reaction`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReactionIntent {
    /// Target channel/DM UUID string.
    pub channel_id: String,
    /// Target message (remote or local durable create).
    pub message: FfiMessageTarget,
    /// Canonical custom or unicode reaction identity.
    pub emoji: FfiReactionEmoji,
    /// Desired membership (`true` to add, `false` to remove).
    pub add: bool,
    /// Caller-generated unique local operation UUID string.
    pub local_id: String,
    /// Caller timestamp in Unix microseconds.
    pub at_us: i64,
}

/// FULL-acknowledged durable reaction admission from coalesced lane.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiReactionAdmission {
    /// This tap's desired state was durably enqueued under `operation_id`.
    Queued {
        /// Enqueued durable operation UUID string.
        operation_id: String,
    },
    /// A newer pending tap superseded this call cleanly under `operation_id`.
    CoalescedInto {
        /// Superseding durable operation UUID string.
        operation_id: String,
    },
}

/// Closed client fallback for `set_reaction`.
fn closed_reaction_error() -> FfiReactionError {
    FfiReactionError::Closed
}

#[uniffi::export]
impl FfiClient {
    /// Pins a backend message inside `channel_id`.
    ///
    /// # Errors
    /// Returns invalid UUID, permission denial, or `Unknown` dispatch outcome.
    pub async fn pin_message(
        &self,
        account: FfiAccountId,
        channel_id: String,
        message_id: String,
    ) -> Result<FfiPinReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let message = parse_uuid(&message_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().pin_message(channel, message).await?;
            Ok(FfiPinReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                pinned: (&r.value).into(),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Unpins a backend message without advancing read state.
    ///
    /// # Errors
    /// Returns invalid UUID, permission denial, or `Unknown` dispatch outcome.
    pub async fn unpin_message(
        &self,
        account: FfiAccountId,
        channel_id: String,
        message_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let message = parse_uuid(&message_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().unpin_message(channel, message).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Paginates descending grouped reaction users for one message.
    ///
    /// # Errors
    /// Returns stale/foreign cursor error, invalid UUIDs, or backend errors.
    pub async fn reaction_users(
        &self,
        account: FfiAccountId,
        channel_id: String,
        message_id: String,
        cursor: Option<Arc<FfiReactorCursor>>,
    ) -> Result<FfiReactorPage, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let message = parse_uuid(&message_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let sdk_cursor = cursor.map(|c| c.inner.clone());
        self.run_scoped(closed_operation_error, async move {
            let g = session
                .operations()
                .reaction_users(channel, message, sdk_cursor)
                .await?;
            Ok(FfiReactorPage {
                generation: g.generation.get(),
                groups: g.value.groups.iter().map(Into::into).collect(),
                next_cursor: g
                    .value
                    .next
                    .map(|c| Arc::new(FfiReactorCursor { inner: c })),
            })
        })
        .await
    }

    /// Coalesces rapid reaction taps before admitting durable outgoing queue items (`synchronous=FULL`).
    ///
    /// # Errors
    /// Returns [`FfiReactionError`] for invalid emoji, queue capacity, closed session, or durable failure.
    pub async fn set_reaction(
        &self,
        account: FfiAccountId,
        intent: FfiReactionIntent,
    ) -> Result<FfiReactionAdmission, FfiReactionError> {
        let session = self.session_by_id(&account).map_err(|err| match err {
            FfiClientError::InvalidArgument => FfiReactionError::InvalidInput,
            FfiClientError::Superseded => FfiReactionError::Superseded,
            _ => FfiReactionError::Closed,
        })?;
        let channel = parse_uuid(&intent.channel_id).map_err(|_| FfiReactionError::InvalidInput)?;
        let local = parse_uuid(&intent.local_id).map_err(|_| FfiReactionError::InvalidInput)?;
        let message = intent
            .message
            .into_sdk()
            .map_err(|_| FfiReactionError::InvalidInput)?;
        let emoji = match intent.emoji {
            FfiReactionEmoji::Custom { emoji_id } => ReactionEmoji::Custom(
                parse_uuid(&emoji_id).map_err(|_| FfiReactionError::InvalidInput)?,
            ),
            FfiReactionEmoji::Unicode { unicode } => ReactionEmoji::Unicode(unicode),
        };
        let sdk_intent = ReactionIntent {
            channel,
            message,
            emoji,
            add: intent.add,
            local,
            at: UnixMicros(intent.at_us),
        };
        self.run_scoped(closed_reaction_error, async move {
            let adm = session.reactions().set(sdk_intent).await?;
            Ok(match adm {
                ReactionAdmission::Queued(id) => FfiReactionAdmission::Queued {
                    operation_id: id.to_string(),
                },
                ReactionAdmission::CoalescedInto(id) => FfiReactionAdmission::CoalescedInto {
                    operation_id: id.to_string(),
                },
                _ => FfiReactionAdmission::Queued {
                    operation_id: local.to_string(),
                },
            })
        })
        .await
    }
}
