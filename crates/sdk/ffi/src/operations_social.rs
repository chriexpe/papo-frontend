//! Direct-conversation and member-blocking REST operations across FFI.
//! History access and permission grants remain separate from DM open/close results.

use papo_sdk::DirectConversation;

use crate::{
    client::FfiClient,
    ids::{FfiAccountId, parse_uuid},
    operation_errors::FfiOperationError,
    operations_channel_records::FfiUserSummaryWire,
    operations_records::{FfiUnitReceipt, closed_operation_error, operation_session},
};

/// Authoritative backend direct conversation wire record.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDirectConversationWire {
    /// DM channel UUID string.
    pub id: String,
    /// Other participant summary.
    pub user: FfiUserSummaryWire,
    /// Creation timestamp string.
    pub created_at: String,
    /// Last message UUID if any.
    pub last_message_id: Option<String>,
    /// Last read message UUID if any.
    pub last_read_message_id: Option<String>,
    /// Last read timestamp string.
    pub last_read_at: Option<String>,
    /// Backend reported unread count.
    pub unread_count: i64,
}

impl From<&DirectConversation> for FfiDirectConversationWire {
    fn from(dm: &DirectConversation) -> Self {
        Self {
            id: dm.id.clone(),
            user: (&dm.user).into(),
            created_at: dm.created_at.clone(),
            last_message_id: dm.last_message.as_ref().and_then(|m| m.id.clone()),
            last_read_message_id: dm.last_read_message.clone(),
            last_read_at: dm.last_read_at.clone(),
            unread_count: dm.unread_count,
        }
    }
}

#[uniffi::export]
impl FfiClient {
    /// Reads visible direct conversations directly from backend.
    ///
    /// # Errors
    /// Returns backend rejection or session errors.
    pub async fn fetch_direct_conversations(
        &self,
        account: FfiAccountId,
    ) -> Result<Vec<FfiDirectConversationWire>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        self.run_scoped(closed_operation_error, async move {
            let g = session.operations().direct_conversations().await?;
            Ok(g.value.dms.iter().map(Into::into).collect())
        })
        .await
    }

    /// Reads and verifies one direct conversation directly from backend.
    ///
    /// # Errors
    /// Returns invalid UUID, backend rejection (including blocks), or session errors.
    pub async fn fetch_direct_conversation(
        &self,
        account: FfiAccountId,
        conversation_id: String,
    ) -> Result<FfiDirectConversationWire, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let conv = parse_uuid(&conversation_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let g = session.operations().direct_conversation(conv).await?;
            Ok(FfiDirectConversationWire::from(&g.value))
        })
        .await
    }

    /// Opens or unhides a direct conversation with `user_id`.
    ///
    /// # Errors
    /// Returns invalid UUID, block rejection, or `Unknown` dispatch failure.
    pub async fn open_direct_conversation(
        &self,
        account: FfiAccountId,
        user_id: String,
    ) -> Result<FfiDirectConversationWire, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let user = parse_uuid(&user_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().open_direct_conversation(user).await?;
            Ok(FfiDirectConversationWire::from(&r.value))
        })
        .await
    }

    /// Hides a direct conversation locally/on server without deleting canonical history.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch failure.
    pub async fn close_direct_conversation(
        &self,
        account: FfiAccountId,
        conversation_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let conv = parse_uuid(&conversation_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().close_direct_conversation(conv).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Reads outbound blocked user list directly from backend.
    ///
    /// # Errors
    /// Returns backend rejection or session errors.
    pub async fn fetch_blocked_users(
        &self,
        account: FfiAccountId,
    ) -> Result<Vec<FfiUserSummaryWire>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        self.run_scoped(closed_operation_error, async move {
            let g = session.operations().blocked_users().await?;
            Ok(g.value.users.iter().map(Into::into).collect())
        })
        .await
    }

    /// Blocks a registered backend user.
    ///
    /// # Errors
    /// Returns invalid UUID, self-block rejection, or `Unknown` dispatch outcome.
    pub async fn block_user(
        &self,
        account: FfiAccountId,
        user_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let user = parse_uuid(&user_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().block_user(user).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Unblocks a user without reopening hidden DMs.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn unblock_user(
        &self,
        account: FfiAccountId,
        user_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let user = parse_uuid(&user_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().unblock_user(user).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }
}
