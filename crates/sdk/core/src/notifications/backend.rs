//! Versioned backend account notification policy; device overrides remain in FULL state.sqlite.
use crate::{ClientError, Generation, session::SessionInner};
use papo_api::wire::{Notifications, WhoamiResponseSettings};

/// Effective account-scoped notification settings reported by the server (`whoami`).
/// Device overrides remain separate in [`super::NotificationSettings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "mirrors backend UserConfig.notifications booleans"
)]
pub struct AccountNotificationPolicy {
    /// Connection generation that admitted this policy.
    pub generation: Generation,
    /// Master account notification switch (`notifications.enabled`).
    pub enabled: bool,
    /// Mention alerts (`notifications.mentions`).
    pub mentions: bool,
    /// Show message content in notifications (`notifications.messagePreview`).
    pub message_preview: bool,
    /// Play notification sound (`notifications.sound`).
    pub sound: bool,
}

pub(crate) struct BackendPolicy {
    pub generation: Generation,
    version: i64,
    pub config: Notifications,
}
pub(crate) fn remember(
    session: &SessionInner,
    generation: Generation,
    settings: Option<WhoamiResponseSettings>,
) -> Result<(), ClientError> {
    let version = settings
        .as_ref()
        .and_then(|settings| settings.version)
        .unwrap_or(0);
    let config = settings
        .and_then(|settings| settings.config)
        .and_then(|config| config.notifications)
        .unwrap_or_default();
    session.epochs.apply(generation, || {
        let mut cached = session
            .notifications
            .backend
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cached
            .as_ref()
            .is_none_or(|current| current.generation != generation || version >= current.version)
        {
            let [enabled, mentions, message_preview, sound] = effective_config(&config);
            let changed = cached.as_ref().is_none_or(|current| {
                current.generation != generation
                    || effective_config(&current.config)
                        != [enabled, mentions, message_preview, sound]
            });
            *cached = Some(BackendPolicy {
                generation,
                version,
                config,
            });
            drop(cached);
            if changed {
                session.notifications.account_policy.send_replace(Some(
                    AccountNotificationPolicy {
                        generation,
                        enabled,
                        mentions,
                        message_preview,
                        sound,
                    },
                ));
                session.notifications.policy_changes.send_replace(());
            }
        }
    })?;
    Ok(())
}

fn effective_config(config: &Notifications) -> [bool; 4] {
    [
        config.enabled != Some(false),
        config.mentions != Some(false),
        config.message_preview != Some(false),
        config.sound != Some(false),
    ]
}
