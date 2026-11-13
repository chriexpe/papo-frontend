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
            *cached = Some(BackendPolicy {
                generation,
                version,
                config,
            });
        }
    })?;
    Ok(())
}
