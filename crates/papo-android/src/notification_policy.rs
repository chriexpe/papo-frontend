//! Apply the existing account/channel endpoint sequentially. Each ACK is
//! projected independently; a partial batch never claims atomic success.
use crate::{
    account,
    bindings::{Backend, CoreError, NativeClient},
};
use papo_core::{
    api::{client::ApiError, net::Update},
    state::ChannelKind,
};
use std::collections::HashSet;

#[derive(Clone, Debug, uniffi::Record)]
pub struct NotificationPolicyFailure {
    pub channel_id: String,
    pub reason: String,
}
#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct NotificationPolicyResult {
    pub applied: Vec<String>,
    pub failed: Vec<NotificationPolicyFailure>,
}
#[uniffi::export]
impl NativeClient {
    pub fn notification_policy(
        &self,
        owner: String,
        ids: Vec<String>,
        setting: String,
    ) -> Result<NotificationPolicyResult, CoreError> {
        if !matches!(setting.as_str(), "all" | "only_mentions" | "off") {
            return Err("Invalid notification policy".to_owned().into());
        }
        let mut result = NotificationPolicyResult::default();
        let mut seen = HashSet::new();
        for id in ids.into_iter().filter(|id| seen.insert(id.clone())) {
            {
                let mut backend = self
                    .backend
                    .lock()
                    .map_err(|_| "Client lock failed".to_owned())?;
                let store = match backend.as_ref().ok_or("Client is closed".to_owned())? {
                    Backend::Live(c) => &c.runtime.store,
                    Backend::Demo(c) => &c.store,
                };
                if store.me != owner {
                    return Err("Account changed".to_owned().into());
                }
                let Some(_) = store.channels.iter().find(|c| {
                    c.id == id && matches!(c.kind, ChannelKind::Text | ChannelKind::Voice)
                }) else {
                    result.failed.push(NotificationPolicyFailure {
                        channel_id: id,
                        reason: "missing".into(),
                    });
                    continue;
                };
                if let Some(Backend::Demo(c)) = backend.as_mut() {
                    c.store.apply(Update::ChannelNotificationSetting {
                        channel_id: id.clone(),
                        setting: setting.clone(),
                    });
                    result.applied.push(id);
                    continue;
                }
            }
            let request = self.scoped_api(&owner)?;
            let response = account::run_api(async {
                let first = request
                    .api
                    .set_channel_notifications(&id, &owner, &setting)
                    .await;
                // A 429 precedes mutation; repeat once through the shared
                // governor, which waits for the backend's Retry-After deadline.
                if matches!(first, Err(ApiError::TooManyRequests(_))) {
                    request
                        .api
                        .set_channel_notifications(&id, &owner, &setting)
                        .await
                } else {
                    first
                }
            });
            match response {
                Ok(_) => {
                    self.finish_request(
                        &request,
                        vec![Update::ChannelNotificationSetting {
                            channel_id: id.clone(),
                            setting: setting.clone(),
                        }],
                    )?;
                    result.applied.push(id);
                }
                Err(ApiError::Unauthorized) => return Err("Sign in again".to_owned().into()),
                Err(error) => {
                    self.finish_request(&request, vec![])?;
                    let reason = match error {
                        ApiError::NotFound => "missing",
                        ApiError::Forbidden(_) => "denied",
                        ApiError::TooManyRequests(_) => "rate_limited",
                        _ => "unconfirmed",
                    };
                    result.failed.push(NotificationPolicyFailure {
                        channel_id: id,
                        reason: reason.into(),
                    });
                }
            }
        }
        Ok(result)
    }
}
