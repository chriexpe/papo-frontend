//! Acknowledged webhook administration. One-time credentials stay only in transient memory.
use crate::api::models::{CreatedWebhook, UpdateWebhookIdentity, Webhook};

#[derive(Clone, Debug)]
pub enum WebhookAction {
    Load,
    Create { channel: String, name: String },
    Identity { id: String, display_name: String },
    Revoke { id: String },
}
#[derive(Debug)]
pub enum WebhookOutcome {
    Listed(Vec<Webhook>),
    Created(CreatedWebhook),
    Updated(Webhook),
    Revoked(String),
}
#[derive(Default, Debug)]
pub struct Webhooks {
    pub items: Vec<Webhook>,
    pub created: Option<CreatedWebhook>,
    pub error: Option<String>,
    pub loaded: bool,
    request: Option<u64>,
}
impl Webhooks {
    pub fn loading(&self) -> bool {
        self.request.is_some()
    }
    pub fn begin(&mut self) -> Option<u64> {
        if self.loading() {
            return None;
        }
        let id = crate::next_operation_id();
        self.request = Some(id);
        self.error = None;
        Some(id)
    }
    pub fn complete(&mut self, id: u64, result: Result<WebhookOutcome, String>) {
        if self.request != Some(id) {
            return;
        }
        self.request = None;
        match result {
            Ok(WebhookOutcome::Listed(items)) => {
                self.items = items;
                self.loaded = true;
            }
            Ok(WebhookOutcome::Created(created)) => {
                self.items.push(created.webhook.clone());
                self.created = Some(created);
            }
            Ok(WebhookOutcome::Updated(hook)) => {
                self.items.retain(|h| h.id != hook.id);
                self.items.push(hook);
            }
            Ok(WebhookOutcome::Revoked(id)) => {
                self.items.retain(|h| h.id != id);
                if self.created.as_ref().is_some_and(|c| c.webhook.id == id) {
                    self.created = None;
                }
            }
            Err(error) => self.error = Some(error),
        }
        self.items.retain(|h| h.revoked_at.is_none());
    }
}

pub async fn execute(
    api: &crate::api::client::Api,
    owner: &str,
    action: WebhookAction,
) -> Result<WebhookOutcome, crate::api::client::ApiError> {
    use crate::api::client::ApiError;
    // Read fresh authorization before every operation, including queued mutations.
    let who = api.whoami().await?;
    if who.id != owner {
        return Err(ApiError::Unauthorized);
    }
    let server = api.server().await?;
    let roles = api.roles().await?;
    let permitted = server
        .as_ref()
        .is_some_and(|s| s.owner_id.as_deref() == Some(owner))
        || roles.iter().any(|r| {
            r.permissions.manage_webhooks && who.roles.iter().any(|assigned| assigned.id == r.id)
        });
    if !permitted {
        return Err(ApiError::Forbidden("webhooks".into()));
    }
    match action {
        WebhookAction::Load => Ok(WebhookOutcome::Listed(api.webhooks().await?)),
        WebhookAction::Create { channel, name } => {
            if name.trim().is_empty() || name.chars().count() > 80 {
                return Err(ApiError::Decode("invalid_webhook_name".into()));
            }
            if !api
                .channels()
                .await?
                .iter()
                .any(|c| c.id == channel && matches!(c.kind.as_str(), "text" | "voice"))
            {
                return Err(ApiError::Decode("invalid_webhook_channel".into()));
            }
            Ok(WebhookOutcome::Created(
                api.create_webhook(&channel, name.trim()).await?,
            ))
        }
        WebhookAction::Identity { id, display_name } => {
            if display_name.chars().count() > 100 {
                return Err(ApiError::Decode("invalid_webhook_name".into()));
            }
            Ok(WebhookOutcome::Updated(
                api.update_webhook_identity(
                    &id,
                    &UpdateWebhookIdentity {
                        display_name: Some(display_name),
                        ..Default::default()
                    },
                )
                .await?,
            ))
        }
        WebhookAction::Revoke { id } => {
            api.revoke_webhook(&id).await?;
            Ok(WebhookOutcome::Revoked(id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parity_webhook_receipts_are_transient_and_old_results_cannot_replace_current_state() {
        let mut hooks = Webhooks::default();
        let request = hooks.begin().unwrap();
        assert!(hooks.begin().is_none());
        let raw = serde_json::json!({"id":"hook","channel_id":"chat","name":"Builds","created_by":"me",
            "created_at":"2026-10-23T12:00:00Z","token":"one-time-secret"});
        let created: CreatedWebhook = serde_json::from_value(raw).unwrap();
        assert!(!format!("{created:?}").contains("one-time-secret"));
        hooks.complete(request, Ok(WebhookOutcome::Created(created)));
        assert_eq!(hooks.items.len(), 1);
        let current = hooks.begin().unwrap();
        hooks.complete(request, Ok(WebhookOutcome::Listed(vec![])));
        assert_eq!(hooks.items.len(), 1);
        hooks.complete(current, Err("unconfirmed".into()));
        assert_eq!(hooks.items.len(), 1);
        let revoke = hooks.begin().unwrap();
        hooks.complete(revoke, Ok(WebhookOutcome::Revoked("hook".into())));
        assert!(hooks.items.is_empty());
        assert!(hooks.created.is_none());
    }
}
