//! Owns direct-conversation and blocking mutations; history and permission proofs remain separate.

pub use papo_api::{
    Uuid,
    wire::{
        BlockListResponse, DirectConversation, DirectConversationList, OpenDirectMessageRequest,
    },
};

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::Generated;

impl Operations {
    /// Reads the authenticated account's visible DM list, without granting access to its history.
    /// # Errors
    /// Returns backend rejection or a stale/revoked account generation.
    pub async fn direct_conversations(
        &self,
    ) -> Result<Generated<DirectConversationList>, OperationError> {
        self.read(|rest| async move { rest.direct_conversations().await })
            .await
    }

    /// Verifies one conversation through the backend, including both directions of blocking.
    /// # Errors
    /// Returns backend rejection or a stale/revoked account generation.
    pub async fn direct_conversation(
        &self,
        conversation: Uuid,
    ) -> Result<Generated<DirectConversation>, OperationError> {
        self.read(move |rest| async move { rest.direct_conversation(conversation).await })
            .await
    }

    /// Opens or unhides a conversation with a real backend user ID.
    /// External webhook authors have no person ID and must not use a bridge account here.
    /// A DM result grants neither timeline coverage nor a read acknowledgement.
    /// # Errors
    /// Returns backend rejection (including reverse blocking), stale generation, or uncertainty.
    pub async fn open_direct_conversation(
        &self,
        user: Uuid,
    ) -> Result<OperationReceipt<DirectConversation>, OperationError> {
        let request = OpenDirectMessageRequest {
            user_id: user.to_string(),
        };
        self.mutate(
            move |rest| async move { rest.open_direct_conversation(&request).await },
            Refresh::Social,
        )
        .await
    }

    /// Hides a conversation without deleting canonical history or read state.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch.
    pub async fn close_direct_conversation(
        &self,
        conversation: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.close_direct_conversation(conversation).await },
            Refresh::Social,
        )
        .await
    }

    /// Reads only this user's outbound blocks; absence does not prove reverse-block absence.
    /// # Errors
    /// Returns backend rejection or a stale/revoked account generation.
    pub async fn blocked_users(&self) -> Result<Generated<BlockListResponse>, OperationError> {
        self.read(|rest| async move { rest.blocked_users().await })
            .await
    }

    /// Blocks a real backend user. The server rejects self-blocking and unknown identities.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch.
    pub async fn block_user(&self, user: Uuid) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.block_user(user).await },
            Refresh::Social,
        )
        .await
    }

    /// Removes an outbound block without reopening the conversation.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch.
    pub async fn unblock_user(&self, user: Uuid) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.unblock_user(user).await },
            Refresh::Social,
        )
        .await
    }
}
