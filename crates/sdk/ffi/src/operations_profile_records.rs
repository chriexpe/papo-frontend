//! Password reset link security wrapper, user settings, and custom emoji records across FFI.
//! Self-profile and emoji execution methods live in `operations_profile.rs`.

use std::sync::Arc;

use papo_sdk::{
    Display, Notifications, UserConfig,
    operations::{emojis::Emoji, profile::PasswordResetLink},
};

use crate::{
    ids::{FfiAccountId, FfiBackendMode},
    operation_errors::FfiRefreshStatus,
};

/// Opaque single-use password reset link bound to its issuing account and target user.
/// Deliberately omits `Debug` so formatting never logs reset URLs.
#[derive(uniffi::Object)]
pub struct FfiPasswordResetLink {
    /// Source account.
    account: FfiAccountId,
    /// Target user UUID string.
    target_user_id: String,
    /// Exact expiry timestamp string.
    expires_at: String,
    /// Secret-bearing single-use URL.
    url: String,
}

impl From<PasswordResetLink> for FfiPasswordResetLink {
    fn from(link: PasswordResetLink) -> Self {
        Self {
            account: FfiAccountId::from_sdk(link.account()),
            target_user_id: link.target().to_string(),
            expires_at: link.expires_at().to_owned(),
            url: link.expose_url().to_owned(),
        }
    }
}

#[uniffi::export]
impl FfiPasswordResetLink {
    /// Account that requested or generated the link.
    #[must_use]
    pub fn account(&self) -> FfiAccountId {
        self.account.clone()
    }

    /// Target backend user UUID string.
    #[must_use]
    pub fn target_user_id(&self) -> String {
        self.target_user_id.clone()
    }

    /// Exact backend expiry timestamp string.
    #[must_use]
    pub fn expires_at(&self) -> String {
        self.expires_at.clone()
    }

    /// Explicitly exposes the reset URL only for a user-confirmed copy or navigation action.
    #[must_use]
    pub fn expose_url(&self) -> String {
        self.url.clone()
    }
}

/// Password reset preparation outcome (`Prepared` for self-reset or expiring owner `Link`).
#[derive(Clone, uniffi::Enum)]
pub enum FfiPasswordResetPreparation {
    /// Requesting account may proceed to `change_password`.
    Prepared,
    /// Owner-authorized single-use reset link for target user.
    Link {
        /// Opaque non-`Debug` reset link wrapper.
        link: Arc<FfiPasswordResetLink>,
    },
}

/// Receipt acknowledging `prepare_password_reset`.
#[derive(Clone, uniffi::Record)]
pub struct FfiPasswordResetReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Resulting preparation status or link object.
    pub preparation: FfiPasswordResetPreparation,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

/// Server-synced account notification sub-config.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiUserNotificationsConfig {
    /// Enable server notifications.
    pub enabled: Option<bool>,
    /// Include message preview text.
    pub message_preview: Option<bool>,
    /// Enable notification sound.
    pub sound: Option<bool>,
    /// Mention trigger preference.
    pub mentions: Option<bool>,
}

/// Server-synced display sub-config.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiUserDisplayConfig {
    /// Font size string preference.
    pub font_size: Option<String>,
    /// Message density string preference.
    pub message_density: Option<String>,
    /// Show timestamps toggle.
    pub show_timestamps: Option<bool>,
    /// Show avatars toggle.
    pub show_avatars: Option<bool>,
}

/// Server-synced account settings document.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiUserConfig {
    /// Optional theme token.
    pub theme: Option<String>,
    /// Notification preferences.
    pub notifications: Option<FfiUserNotificationsConfig>,
    /// Display preferences.
    pub display: Option<FfiUserDisplayConfig>,
}

impl From<&UserConfig> for FfiUserConfig {
    fn from(c: &UserConfig) -> Self {
        Self {
            theme: c.theme.clone(),
            notifications: c
                .notifications
                .as_ref()
                .map(|n| FfiUserNotificationsConfig {
                    enabled: n.enabled,
                    message_preview: n.message_preview,
                    sound: n.sound,
                    mentions: n.mentions,
                }),
            display: c.display.as_ref().map(|d| FfiUserDisplayConfig {
                font_size: d.font_size.clone(),
                message_density: d.message_density.clone(),
                show_timestamps: d.show_timestamps,
                show_avatars: d.show_avatars,
            }),
        }
    }
}

impl From<FfiUserConfig> for UserConfig {
    fn from(c: FfiUserConfig) -> Self {
        Self {
            theme: c.theme,
            notifications: c.notifications.map(|n| Notifications {
                enabled: n.enabled,
                message_preview: n.message_preview,
                sound: n.sound,
                mentions: n.mentions,
            }),
            display: c.display.map(|d| Display {
                font_size: d.font_size,
                message_density: d.message_density,
                show_timestamps: d.show_timestamps,
                show_avatars: d.show_avatars,
            }),
        }
    }
}

/// Receipt acknowledging updated account configuration.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUserSettingsReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Authoritative user config version.
    pub version: Option<i64>,
    /// Returned server configuration.
    pub config: Option<FfiUserConfig>,
    /// Update timestamp string.
    pub updated_at: Option<String>,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

/// Receipt acknowledging channel notification setting update.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelSettingReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Channel UUID string.
    pub channel_id: Option<String>,
    /// Saved notification mode.
    pub mode: Option<FfiBackendMode>,
    /// Update timestamp string.
    pub updated_at: Option<String>,
    /// Post-mutation refresh status (DMs intentionally return `Warning(Unavailable)`).
    pub refresh: FfiRefreshStatus,
}

/// Custom server emoji record returned by `create_emoji`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmojiWire {
    /// Emoji UUID string.
    pub id: Option<String>,
    /// Shortcode name.
    pub name: Option<String>,
    /// Format string.
    pub format: Option<String>,
    /// Creator UUID string.
    pub created_by: Option<String>,
    /// Exact creation timestamp string.
    pub created_at: Option<String>,
}

impl From<&Emoji> for FfiEmojiWire {
    fn from(e: &Emoji) -> Self {
        Self {
            id: e.id.clone(),
            name: e.name.clone(),
            format: e.format.clone(),
            created_by: e.created_by.clone(),
            created_at: e.created_at.clone(),
        }
    }
}

/// Receipt acknowledging custom emoji upload.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmojiReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Returned emoji record.
    pub emoji: FfiEmojiWire,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}
