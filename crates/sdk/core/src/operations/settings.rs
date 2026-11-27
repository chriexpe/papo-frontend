//! Owns acknowledged server-side account/channel settings; device preferences belong to clients.
//!
//! The backend replaces the whole `UserConfig` on `PUT /users/settings` (B-020), so a single-field
//! notification toggle (`change_notification_settings`) reads `whoami` inside a per-account lock,
//! verifies the full config is present, and merges only the requested fields.

use papo_api::Uuid;

pub use papo_api::wire::{
    ChannelUserSetting, UpdateChannelUserSettingRequest, UpdateUserSettingsRequest, UserSettings,
};

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::ClientError;

/// Fields of this account's notification settings to change; `None` keeps its saved value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotificationPolicyChange {
    /// Master account notification switch (`notifications.enabled`).
    pub enabled: Option<bool>,
    /// Mention alerts (`notifications.mentions`).
    pub mentions: Option<bool>,
    /// Show message content in notifications (`notifications.messagePreview`).
    pub message_preview: Option<bool>,
    /// Play notification sound (`notifications.sound`).
    pub sound: Option<bool>,
}

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

    /// Saves only the notification fields `change` sets. The backend replaces `UserConfig`
    /// as a whole (B-020), so `whoami` is read back right before the write inside a per-account
    /// lock, every untouched `theme`, `display` and `notifications` field is kept verbatim, and
    /// an incomplete config is refused with `InvalidData` rather than sending defaults.
    /// # Errors
    /// Returns the read-back's failure, `InvalidData` when `whoami` omits any required config
    /// field, or the save's typed rejection, stale generation or uncertain dispatch; never retries.
    pub async fn change_notification_settings(
        &self,
        change: NotificationPolicyChange,
    ) -> Result<OperationReceipt<UserSettings>, OperationError> {
        let me = self.session.account().user();
        // Serialize per-account read-modify-write so concurrent toggles never overwrite each other.
        let _settings = self.session.inner.settings_serial.lock().await;
        let read = self.read(|rest| async move { rest.whoami().await }).await?;
        if read.value.id.as_deref().and_then(|id| id.parse().ok()) != Some(me) {
            return Err(ClientError::Identity.into());
        }
        let request = merged_notifications(change, read.value.settings)?;
        Operations {
            session: self.session.clone(),
            expected_generation: Some(read.generation),
        }
        .update_user_settings(request)
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

fn merged_notifications(
    change: NotificationPolicyChange,
    saved: Option<papo_api::wire::WhoamiResponseSettings>,
) -> Result<UpdateUserSettingsRequest, OperationError> {
    let mut config = saved
        .and_then(|settings| settings.config)
        .ok_or(OperationError::InvalidData)?;
    if config.theme.is_none() {
        return Err(OperationError::InvalidData);
    }
    let display = config.display.as_ref().ok_or(OperationError::InvalidData)?;
    if display.font_size.is_none()
        || display.message_density.is_none()
        || display.show_timestamps.is_none()
        || display.show_avatars.is_none()
    {
        return Err(OperationError::InvalidData);
    }
    let saved_notifications = config
        .notifications
        .as_ref()
        .ok_or(OperationError::InvalidData)?;
    let notifications = papo_api::wire::Notifications {
        enabled: Some(
            change
                .enabled
                .or(saved_notifications.enabled)
                .ok_or(OperationError::InvalidData)?,
        ),
        mentions: Some(
            change
                .mentions
                .or(saved_notifications.mentions)
                .ok_or(OperationError::InvalidData)?,
        ),
        message_preview: Some(
            change
                .message_preview
                .or(saved_notifications.message_preview)
                .ok_or(OperationError::InvalidData)?,
        ),
        sound: Some(
            change
                .sound
                .or(saved_notifications.sound)
                .ok_or(OperationError::InvalidData)?,
        ),
    };
    config.notifications = Some(notifications);
    Ok(UpdateUserSettingsRequest { config })
}

#[cfg(test)]
mod tests {
    //! Verifies B-020 full-config preservation without a network or real clock.

    use super::*;
    use papo_api::wire::{Display, Notifications, UserConfig, WhoamiResponseSettings};

    fn full_saved() -> WhoamiResponseSettings {
        WhoamiResponseSettings {
            version: Some(1),
            config: Some(UserConfig {
                theme: Some("dark".into()),
                notifications: Some(Notifications {
                    enabled: Some(true),
                    mentions: Some(true),
                    message_preview: Some(false),
                    sound: Some(true),
                }),
                display: Some(Display {
                    font_size: Some("medium".into()),
                    message_density: Some("compact".into()),
                    show_timestamps: Some(true),
                    show_avatars: Some(false),
                }),
            }),
        }
    }

    #[test]
    fn one_notification_field_preserves_theme_display_and_other_notification_fields() {
        let request = merged_notifications(
            NotificationPolicyChange {
                sound: Some(false),
                ..NotificationPolicyChange::default()
            },
            Some(full_saved()),
        )
        .unwrap();
        assert_eq!(request.config.theme.as_deref(), Some("dark"));
        let display = request.config.display.unwrap();
        assert_eq!(display.font_size.as_deref(), Some("medium"));
        assert_eq!(display.message_density.as_deref(), Some("compact"));
        assert_eq!(display.show_timestamps, Some(true));
        assert_eq!(display.show_avatars, Some(false));
        let notifications = request.config.notifications.unwrap();
        assert_eq!(notifications.enabled, Some(true));
        assert_eq!(notifications.mentions, Some(true));
        assert_eq!(notifications.message_preview, Some(false));
        assert_eq!(notifications.sound, Some(false));
    }

    #[test]
    fn incomplete_whoami_config_is_refused_without_fabricating_defaults() {
        assert!(matches!(
            merged_notifications(NotificationPolicyChange::default(), None),
            Err(OperationError::InvalidData)
        ));

        let mut no_theme = full_saved();
        no_theme.config.as_mut().unwrap().theme = None;
        assert!(matches!(
            merged_notifications(NotificationPolicyChange::default(), Some(no_theme)),
            Err(OperationError::InvalidData)
        ));

        let mut partial_display = full_saved();
        partial_display
            .config
            .as_mut()
            .unwrap()
            .display
            .as_mut()
            .unwrap()
            .show_avatars = None;
        assert!(matches!(
            merged_notifications(NotificationPolicyChange::default(), Some(partial_display)),
            Err(OperationError::InvalidData)
        ));

        let mut partial_notifications = full_saved();
        partial_notifications
            .config
            .as_mut()
            .unwrap()
            .notifications
            .as_mut()
            .unwrap()
            .mentions = None;
        assert!(matches!(
            merged_notifications(
                NotificationPolicyChange {
                    sound: Some(false),
                    ..NotificationPolicyChange::default()
                },
                Some(partial_notifications),
            ),
            Err(OperationError::InvalidData)
        ));
    }
}
