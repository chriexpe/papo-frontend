//! Owns channel mutations and role overrides; backend authorization defines their effect.

use super::{Generated, OperationError, OperationReceipt, Operations, Refresh};
use papo_api::Uuid;
pub use papo_api::wire::{
    ChangeChannelPositionRequest, Channel, ChannelPermissionsResponse, CreateChannelRequest,
    UpdateChannelPermissionsRequest, UpdateChannelPermissionsResponse, UpdateChannelRequest,
};

impl Operations {
    /// Deletes at most 100 channels once each, retaining every acknowledgment or failure.
    /// No failed or ambiguous item is retried; later items independently validate the session.
    /// # Errors
    /// Returns `InvalidInput` when the batch exceeds 100 items.
    pub async fn delete_channels_batch(
        &self,
        channels: Vec<Uuid>,
    ) -> Result<Vec<Result<OperationReceipt<()>, OperationError>>, OperationError> {
        if channels.len() > 100 {
            return Err(OperationError::InvalidInput);
        }
        let operations = self.for_batch();
        let mut results = Vec::with_capacity(channels.len());
        for channel in channels {
            results.push(operations.delete_channel(channel).await);
        }
        Ok(results)
    }

    /// Creates a text, voice or category channel using the pinned backend's creation contract.
    /// Parent and access-role fields are submitted unchanged; the backend validates them atomically.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn create_channel(
        &self,
        request: CreateChannelRequest,
    ) -> Result<OperationReceipt<Channel>, OperationError> {
        self.mutate(
            move |rest| async move { rest.create_channel(&request).await },
            Refresh::Channels,
        )
        .await
    }

    /// Replaces channel name and optionally topic; this endpoint cannot change channel type.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn update_channel(
        &self,
        channel: Uuid,
        request: UpdateChannelRequest,
    ) -> Result<OperationReceipt<Channel>, OperationError> {
        self.mutate(
            move |rest| async move { rest.update_channel(channel, &request).await },
            Refresh::Channels,
        )
        .await
    }

    /// Deletes a channel once, leaving rejection and confirmation presentation to the caller.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn delete_channel(
        &self,
        channel: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.delete_channel(channel).await },
            Refresh::Channels,
        )
        .await
    }

    /// Moves a channel using one-based positions. A stale old position is rejected by the backend.
    /// `parent_id` absent keeps the category; an empty string removes it.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn move_channel(
        &self,
        channel: Uuid,
        request: ChangeChannelPositionRequest,
    ) -> Result<OperationReceipt<Channel>, OperationError> {
        self.mutate(
            move |rest| async move { rest.change_channel_position(channel, &request).await },
            Refresh::Channels,
        )
        .await
    }

    /// Reads authoritative role overrides without treating cached permissions as authorization.
    /// # Errors
    /// Returns a typed backend, decoding or current-session error.
    pub async fn channel_permissions(
        &self,
        channel: Uuid,
    ) -> Result<Generated<ChannelPermissionsResponse>, OperationError> {
        self.read(move |rest| async move { rest.channel_permissions(channel).await })
            .await
    }

    /// Replaces the backend's four channel permissions for one role.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn update_channel_permissions(
        &self,
        channel: Uuid,
        role: Uuid,
        request: UpdateChannelPermissionsRequest,
    ) -> Result<OperationReceipt<UpdateChannelPermissionsResponse>, OperationError> {
        self.mutate(
            move |rest| async move {
                rest.update_channel_permissions(channel, role, &request)
                    .await
            },
            Refresh::Channels,
        )
        .await
    }

    /// Removes a role override; the server role itself remains intact.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn remove_channel_role(
        &self,
        channel: Uuid,
        role: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.delete_channel_role(channel, role).await },
            Refresh::Channels,
        )
        .await
    }
}
