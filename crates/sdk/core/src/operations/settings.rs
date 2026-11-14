//! Owns acknowledged server-side account/channel settings; device preferences belong to clients.

use papo_api::Uuid;

pub use papo_api::wire::{
    ChannelUserSetting, UpdateChannelUserSettingRequest, UpdateUserSettingsRequest, UserSettings,
};

use super::{OperationError, OperationReceipt, Operations, Refresh};

impl Operations {
    /// Replaces backend account configuration, without changing local device overrides.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch; never retries.
    pub async fn update_user_settings(
        &self,
        request: UpdateUserSettingsRequest,
    ) -> Result<OperationReceipt<UserSettings>, OperationError> {
        self.mutate(
            move |rest| async move { rest.update_user_settings(&request).await },
            Refresh::Policy,
        )
        .await
    }

    /// Updates this account's notification policy for a backend channel.
    /// The backend validates self-ownership, channel existence and the requested mode.
    /// DM settings have no backend readback: success returns a refresh Unavailable warning.
    /// Device DM suppression remains in the shared `NotificationService` settings.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch; never retries.
    pub async fn update_channel_settings(
        &self,
        channel: Uuid,
        request: UpdateChannelUserSettingRequest,
    ) -> Result<OperationReceipt<ChannelUserSetting>, OperationError> {
        let user = self.session.account().user();
        self.mutate(
            move |rest| async move {
                rest.update_channel_user_setting(channel, user, &request)
                    .await
            },
            Refresh::ChannelSettings(channel),
        )
        .await
    }
}
