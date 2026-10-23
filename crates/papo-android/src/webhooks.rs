//! Webhook administration uses the shared API and frozen account credentials.
//! Only the one-shot receipt contains a token; lists and snapshots never do.
use crate::{
    account,
    bindings::{Backend, NativeClient},
};
use papo_core::{
    api::{client::ApiError, models},
    state::ChannelKind,
};
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum WebhookError {
    #[error("account_changed")]
    AccountChanged,
    #[error("permission_denied")]
    Denied,
    #[error("invalid_name")]
    InvalidName,
    #[error("invalid_channel")]
    InvalidChannel,
    #[error("invalid_avatar")]
    InvalidAvatar,
    #[error("not_found")]
    Missing,
    #[error("signed_out")]
    SignedOut,
    #[error("unavailable")]
    Unavailable,
    #[error("creation_unconfirmed")]
    CreationUnconfirmed,
    #[error("rejected")]
    Rejected,
}
impl From<ApiError> for WebhookError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Forbidden(_) => Self::Denied,
            ApiError::Unauthorized | ApiError::ServerLocked => Self::SignedOut,
            ApiError::NotFound => Self::Missing,
            ApiError::Network(_) | ApiError::TooManyRequests(_) => Self::Unavailable,
            _ => Self::Rejected,
        }
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct WebhookView {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub display_name: String,
    pub created_by: String,
    pub created_at: String,
}
impl From<&models::Webhook> for WebhookView {
    fn from(hook: &models::Webhook) -> Self {
        Self {
            id: hook.id.clone(),
            channel_id: hook.channel_id.clone(),
            name: hook.name.clone(),
            display_name: hook.display_name.clone().unwrap_or_default(),
            created_by: hook.created_by.clone(),
            created_at: hook.created_at.to_rfc3339(),
        }
    }
}

// An object avoids generated Kotlin data-class toString() exposing a credential.
// It is not serializable or included in NativeClient/Store. UI consumes it once.
#[derive(uniffi::Object)]
pub struct WebhookReceipt {
    webhook: WebhookView,
    token: Mutex<Option<String>>,
}
#[uniffi::export]
impl WebhookReceipt {
    pub fn webhook(&self) -> WebhookView {
        self.webhook.clone()
    }
    pub fn take_token(&self) -> String {
        self.token
            .lock()
            .ok()
            .and_then(|mut token| token.take())
            .unwrap_or_default()
    }
}

impl NativeClient {
    fn check_webhooks(&self, owner: &str, channel: Option<&str>) -> Result<(), WebhookError> {
        let backend = self.backend.lock().map_err(|_| WebhookError::Unavailable)?;
        let store = match backend.as_ref() {
            Some(Backend::Live(client)) => &client.runtime.store,
            Some(Backend::Demo(client)) => &client.store,
            None => return Err(WebhookError::AccountChanged),
        };
        if owner.is_empty() || owner != store.me {
            return Err(WebhookError::AccountChanged);
        }
        if !store.can_manage_webhooks() {
            return Err(WebhookError::Denied);
        }
        if channel.is_some_and(|id| {
            store
                .channel(id)
                .is_none_or(|c| !matches!(c.kind, ChannelKind::Text | ChannelKind::Voice))
        }) {
            return Err(WebhookError::InvalidChannel);
        }
        Ok(())
    }
}

#[uniffi::export]
impl NativeClient {
    pub fn webhooks(&self, owner: String) -> Result<Vec<WebhookView>, WebhookError> {
        self.check_webhooks(&owner, None)?;
        {
            let backend = self.backend.lock().map_err(|_| WebhookError::Unavailable)?;
            if let Some(Backend::Demo(client)) = backend.as_ref() {
                if client.store.me != owner {
                    return Err(WebhookError::AccountChanged);
                }
                return Ok(client.webhooks.iter().map(WebhookView::from).collect());
            }
        }
        let request = self
            .scoped_api(&owner)
            .map_err(|_| WebhookError::AccountChanged)?;
        let hooks = account::run_api(request.api.webhooks())?;
        self.finish_request(&request, vec![])
            .map_err(|_| WebhookError::AccountChanged)?;
        Ok(hooks
            .iter()
            .filter(|hook| hook.revoked_at.is_none())
            .map(WebhookView::from)
            .collect())
    }

    pub fn create_webhook(
        &self,
        owner: String,
        channel_id: String,
        name: String,
    ) -> Result<Arc<WebhookReceipt>, WebhookError> {
        self.check_webhooks(&owner, Some(&channel_id))?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(WebhookError::InvalidName);
        }
        let created = {
            let mut backend = self.backend.lock().map_err(|_| WebhookError::Unavailable)?;
            if let Some(Backend::Demo(client)) = backend.as_mut() {
                if client.store.me != owner {
                    return Err(WebhookError::AccountChanged);
                }
                let hook = models::Webhook {
                    id: papo_core::cache::new_local_id(),
                    channel_id: channel_id.clone(),
                    name: name.into(),
                    display_name: None,
                    created_by: owner.clone(),
                    created_at: chrono::Utc::now(),
                    revoked_at: None,
                };
                client.webhooks.push(hook.clone());
                Some(models::CreatedWebhook {
                    webhook: hook,
                    token: format!("demo-{}", papo_core::cache::new_local_id()),
                })
            } else {
                None
            }
        };
        let created = if let Some(created) = created {
            created
        } else {
            let request = self
                .scoped_api(&owner)
                .map_err(|_| WebhookError::AccountChanged)?;
            let result = account::run_api(request.api.create_webhook(&channel_id, name));
            let created = match result {
                Ok(created) => created,
                // Never automatically retry a POST whose response may have been lost.
                Err(ApiError::Network(_) | ApiError::Decode(_)) => {
                    return Err(WebhookError::CreationUnconfirmed);
                }
                Err(error) => return Err(error.into()),
            };
            self.finish_request(&request, vec![])
                .map_err(|_| WebhookError::AccountChanged)?;
            created
        };
        Ok(Arc::new(WebhookReceipt {
            webhook: WebhookView::from(&created.webhook),
            token: Mutex::new(Some(created.token)),
        }))
    }

    pub fn update_webhook(
        &self,
        owner: String,
        id: String,
        nickname: String,
        avatar: Option<String>,
        format: Option<String>,
    ) -> Result<WebhookView, WebhookError> {
        self.check_webhooks(&owner, None)?;
        if id.is_empty() {
            return Err(WebhookError::Missing);
        }
        let nickname = nickname.trim().to_owned();
        if nickname.chars().count() > 100 {
            return Err(WebhookError::InvalidName);
        }
        validate_avatar(avatar.as_deref(), format.as_deref())?;
        {
            let mut backend = self.backend.lock().map_err(|_| WebhookError::Unavailable)?;
            if let Some(Backend::Demo(client)) = backend.as_mut() {
                if client.store.me != owner {
                    return Err(WebhookError::AccountChanged);
                }
                let hook = client
                    .webhooks
                    .iter_mut()
                    .find(|hook| hook.id == id)
                    .ok_or(WebhookError::Missing)?;
                hook.display_name = (!nickname.is_empty()).then_some(nickname);
                return Ok(WebhookView::from(&*hook));
            }
        }
        let request = self
            .scoped_api(&owner)
            .map_err(|_| WebhookError::AccountChanged)?;
        let change = models::UpdateWebhookIdentity {
            display_name: Some(nickname),
            avatar,
            avatar_format: format,
        };
        let hook = account::run_api(request.api.update_webhook_identity(&id, &change))?;
        self.finish_request(&request, vec![])
            .map_err(|_| WebhookError::AccountChanged)?;
        Ok(WebhookView::from(&hook))
    }

    pub fn revoke_webhook(&self, owner: String, id: String) -> Result<(), WebhookError> {
        self.check_webhooks(&owner, None)?;
        if id.is_empty() {
            return Err(WebhookError::Missing);
        }
        {
            let mut backend = self.backend.lock().map_err(|_| WebhookError::Unavailable)?;
            if let Some(Backend::Demo(client)) = backend.as_mut() {
                if client.store.me != owner {
                    return Err(WebhookError::AccountChanged);
                }
                if !client.webhooks.iter().any(|hook| hook.id == id) {
                    return Err(WebhookError::Missing);
                }
                client.webhooks.retain(|hook| hook.id != id);
                return Ok(());
            }
        }
        let request = self
            .scoped_api(&owner)
            .map_err(|_| WebhookError::AccountChanged)?;
        account::run_api(request.api.revoke_webhook(&id))?;
        self.finish_request(&request, vec![])
            .map_err(|_| WebhookError::AccountChanged)
    }
}

fn validate_avatar(avatar: Option<&str>, format: Option<&str>) -> Result<(), WebhookError> {
    use base64::Engine;
    let Some(blob) = avatar.filter(|blob| !blob.is_empty()) else {
        return Ok(());
    };
    if blob.len() > (2 * 1024 * 1024usize).div_ceil(3) * 4
        || !matches!(format, Some("PNG" | "JPEG" | "GIF" | "WEBP"))
    {
        return Err(WebhookError::InvalidAvatar);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .map_err(|_| WebhookError::InvalidAvatar)?;
    let info = papo_core::media_prepare::inspect_image_bytes(&bytes)
        .map_err(|_| WebhookError::InvalidAvatar)?;
    if bytes.len() > 2 * 1024 * 1024 || info.width > 512 || info.height > 512 {
        return Err(WebhookError::InvalidAvatar);
    }
    Ok(())
}

#[cfg(test)]
#[path = "webhooks_tests.rs"]
mod tests;
