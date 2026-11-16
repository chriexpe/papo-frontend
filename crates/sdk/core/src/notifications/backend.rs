//! Versioned backend account notification policy; device overrides remain in FULL state.sqlite.
use crate::{ClientError, Generation, session::SessionInner};
use papo_api::wire::{Notifications, WhoamiResponseSettings};

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
            let changed = cached.as_ref().is_none_or(|current| {
                current.generation != generation
                    || current.config.enabled != config.enabled
                    || current.config.mentions != config.mentions
            });
            *cached = Some(BackendPolicy {
                generation,
                version,
                config,
            });
            if changed {
                session.notifications.policy_changes.send_replace(());
            }
        }
    })?;
    Ok(())
}

/// The backend notification switches cached for `generation`, or the defaults when none are.
pub(crate) fn current(
    session: &SessionInner,
    generation: Generation,
) -> papo_api::wire::Notifications {
    let cached = session
        .notifications
        .backend
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match cached.as_ref() {
        Some(policy) if policy.generation == generation => policy.config.clone(),
        _ => papo_api::wire::Notifications::default(),
    }
}
