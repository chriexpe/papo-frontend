//! Owns ban state and moderator deletion; server channel and DM authorization remain authoritative.

use super::{OperationError, OperationReceipt, Operations, Refresh};
pub use papo_api::wire::GenericResponse;
use papo_api::{Uuid, wire::BanUserRequest};

impl Operations {
    /// Changes at most 100 users' ban states, keeping one result per item in input order.
    /// Ordinary rejections do not discard earlier acknowledgments or prevent later attempts.
    /// # Errors
    /// Returns `InvalidInput` when the batch exceeds 100 items.
    pub async fn set_banned_batch(
        &self,
        changes: Vec<(Uuid, bool)>,
    ) -> Result<Vec<Result<OperationReceipt<GenericResponse>, OperationError>>, OperationError>
    {
        if changes.len() > 100 {
            return Err(OperationError::InvalidInput);
        }
        let operations = self.for_batch();
        let mut results = Vec::with_capacity(changes.len());
        for (user, banned) in changes {
            results.push(operations.set_banned(user, banned).await);
        }
        Ok(results)
    }

    /// Sets ban state using the authoritative path identity. Backend owner protection still applies.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn set_banned(
        &self,
        user: Uuid,
        banned: bool,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let request = BanUserRequest {
            user_id: None,
            ban_state: banned,
        };
        self.mutate(
            move |rest| async move { rest.ban_user(user, &request).await },
            Refresh::Member(user),
        )
        .await
    }

    /// Deletes one message through the ordinary authenticated endpoint.
    /// Channel-owner privileges never substitute for the backend's DM or external-author rules.
    /// The supplied channel scopes local canonical deletion after acknowledgment.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn moderate_delete_message(
        &self,
        channel: Uuid,
        message: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.delete_message(message).await },
            Refresh::DeletedMessage { channel, message },
        )
        .await
    }
}
