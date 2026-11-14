//! Server sticker upload/deletion; decoding, cropping and metadata paging belong to media/clients.

pub use papo_api::wire::{CreateEmojiRequest, Emoji};

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::Uuid;

impl Operations {
    /// Deletes at most 100 emojis once each, retaining every item result in input order.
    /// A rejected or ambiguous request is never retried. Reconnect ends admission
    /// for remaining items rather than silently continuing in another generation.
    /// # Errors
    /// Returns `InvalidInput` when the batch exceeds 100 items.
    pub async fn delete_emojis_batch(
        &self,
        items: Vec<Uuid>,
    ) -> Result<Vec<Result<OperationReceipt<()>, OperationError>>, OperationError> {
        if items.len() > 100 {
            return Err(OperationError::InvalidInput);
        }
        let operations = self.for_batch();
        let mut results = Vec::with_capacity(items.len());
        for emoji in items {
            results.push(operations.delete_emoji(emoji).await);
        }
        Ok(results)
    }

    /// Uploads an encoded static/animated GIF, JPEG, PNG or WebP once.
    /// The backend validates the 256 KiB decoded limit, dimensions, magic bytes,
    /// name uniqueness and manage-server permission. No rename endpoint exists.
    /// # Errors
    /// Returns backend rejection, stale account or an ambiguous dispatch outcome.
    pub async fn create_emoji(
        &self,
        request: CreateEmojiRequest,
    ) -> Result<OperationReceipt<Emoji>, OperationError> {
        self.mutate(
            move |rest| async move { rest.create_emoji(&request).await },
            Refresh::None,
        )
        .await
    }

    /// Deletes one server sticker after backend author/owner/manage-server checks.
    /// Client-owned confirmation occurs before calling this method.
    /// # Errors
    /// Returns backend rejection, stale account or an ambiguous dispatch outcome.
    pub async fn delete_emoji(&self, emoji: Uuid) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.delete_emoji(emoji).await },
            Refresh::None,
        )
        .await
    }
}
