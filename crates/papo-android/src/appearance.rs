//! Acknowledged, owner-scoped appearance operations; Android owns presentation wording.
use crate::{
    account,
    bindings::{Backend, CoreError, NativeClient},
};
use papo_core::api::{
    client::{ApiError, DisplayChange, DisplayFontSize, MessageDensity},
    models::UserSettings,
    net::Update,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceView {
    pub font_size: String,
    pub message_density: String,
}
impl From<&UserSettings> for AppearanceView {
    fn from(settings: &UserSettings) -> Self {
        Self {
            font_size: settings.config.display.font_size.clone(),
            message_density: settings.config.display.message_density.clone(),
        }
    }
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum AppearanceChange {
    FontSize { value: String },
    MessageDensity { value: String },
    ImportLegacyCompact,
}
impl AppearanceChange {
    fn core(&self) -> Result<DisplayChange, AppearanceError> {
        match self {
            Self::FontSize { value } => DisplayFontSize::parse(value).map(DisplayChange::FontSize),
            Self::MessageDensity { value } => {
                MessageDensity::parse(value).map(DisplayChange::MessageDensity)
            }
            Self::ImportLegacyCompact => Some(DisplayChange::ImportLegacyCompact),
        }
        .ok_or(AppearanceError::InvalidChoice)
    }
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AppearanceError {
    #[error("invalid_choice")]
    InvalidChoice,
    #[error("account_changed")]
    AccountChanged,
    #[error("network")]
    Network,
    #[error("rate_limited")]
    RateLimited,
    #[error("unavailable")]
    Unavailable,
    #[error("rejected")]
    Rejected,
    #[error("incompatible")]
    Incompatible,
}
impl From<CoreError> for AppearanceError {
    fn from(_: CoreError) -> Self {
        Self::AccountChanged
    }
}
impl From<ApiError> for AppearanceError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Network(_) => Self::Network,
            ApiError::TooManyRequests(_) => Self::RateLimited,
            ApiError::Unauthorized | ApiError::ServerLocked | ApiError::NotFound => {
                Self::Unavailable
            }
            ApiError::Decode(_) => Self::Incompatible,
            _ => Self::Rejected,
        }
    }
}

#[uniffi::export]
impl NativeClient {
    pub fn account_appearance(
        &self,
        owner: String,
        change: Option<AppearanceChange>,
    ) -> Result<AppearanceView, AppearanceError> {
        let change = change.as_ref().map(AppearanceChange::core).transpose()?;
        {
            let mut backend = self
                .backend
                .lock()
                .map_err(|_| AppearanceError::Unavailable)?;
            if let Some(Backend::Demo(demo)) = backend.as_mut() {
                if owner.is_empty() || demo.store.me != owner {
                    return Err(AppearanceError::AccountChanged);
                }
                let mut settings = demo
                    .store
                    .user_settings
                    .clone()
                    .ok_or(AppearanceError::Unavailable)?;
                if let Some(change) = change {
                    match change {
                        DisplayChange::FontSize(value) => {
                            settings.config.display.font_size = value.wire().into()
                        }
                        DisplayChange::MessageDensity(value) => {
                            settings.config.display.message_density = value.wire().into()
                        }
                        DisplayChange::ImportLegacyCompact
                            if settings.config.display.message_density == "normal" =>
                        {
                            settings.config.display.message_density = "compact".into()
                        }
                        DisplayChange::ImportLegacyCompact => {}
                    }
                }
                let result = AppearanceView::from(&settings);
                demo.store.apply(Update::UserSettings(Box::new(settings)));
                return Ok(result);
            }
        }
        let request = self.scoped_api(&owner)?;
        let settings = account::run_api(request.api.account_display_settings(&owner, change))?;
        let result = AppearanceView::from(&settings);
        self.finish_request(&request, vec![Update::UserSettings(Box::new(settings))])?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_uses_acknowledged_values_and_rejects_stale_owners_and_invalid_choices() {
        let client = NativeClient::demo();
        client
            .account_appearance("me".into(), Some(AppearanceChange::ImportLegacyCompact))
            .unwrap();
        assert_eq!(
            client.poll().unwrap().appearance.unwrap().message_density,
            "compact"
        );
        client
            .account_appearance(
                "me".into(),
                Some(AppearanceChange::FontSize {
                    value: "huge".into(),
                }),
            )
            .unwrap();
        assert_eq!(client.poll().unwrap().appearance.unwrap().font_size, "huge");
        assert!(matches!(
            client.account_appearance("other".into(), None),
            Err(AppearanceError::AccountChanged)
        ));
        assert!(matches!(
            client.account_appearance(
                "me".into(),
                Some(AppearanceChange::FontSize {
                    value: "invalid".into()
                })
            ),
            Err(AppearanceError::InvalidChoice)
        ));
        assert_eq!(
            client
                .account_appearance("me".into(), None)
                .unwrap()
                .font_size,
            "huge"
        );
    }
}
