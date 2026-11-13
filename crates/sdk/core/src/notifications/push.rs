//! Authenticated push token ownership transitions; Firebase transport/configuration and native permissions stay in clients.

use super::{NotificationError, NotificationService};
use crate::{AccountId, ClientError, ServerId, Session};
use papo_api::wire::PushDeviceRequest;
use std::collections::BTreeMap;

/// Host-supplied token metadata. Intentionally has no Debug/Serialize implementation in the SDK.
#[derive(Clone)]
pub struct PushDevice {
    /// Provider token; never persisted in SDK files or diagnostics.
    pub token: String,
    /// Actual backend platform string (android/ios/web).
    pub platform: String,
    /// Optional host-provided non-secret label.
    pub device_name: Option<String>,
    /// Backend-supported provider, normally fcm.
    pub provider: String,
}
impl PushDevice {
    fn request(&self) -> Result<PushDeviceRequest, NotificationError> {
        if self.token.is_empty()
            || self.token.len() > 4096
            || self
                .device_name
                .as_ref()
                .is_some_and(|name| name.len() > 256)
        {
            return Err(NotificationError::InvalidData);
        }
        Ok(PushDeviceRequest {
            token: self.token.clone(),
            platform: Some(self.platform.clone()),
            device_name: self.device_name.clone(),
            provider: Some(self.provider.clone()),
        })
    }
}
/// Remote ownership outcome, never a claim of real FCM delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PushRegistration {
    /// Backend acknowledged the active registration.
    Registered,
    /// Backend acknowledged removal (or owner-scoped 404).
    Removed,
    /// A mutation may have committed remotely; host must resolve explicitly with current ownership.
    Unknown,
}
pub(crate) struct PushOwner {
    account: AccountId,
    device: PushDevice,
    uncertain: bool,
    previous: Option<AccountId>,
}
#[derive(Default)]
pub(crate) struct PushState {
    pub serial: tokio::sync::Mutex<BTreeMap<ServerId, PushOwner>>,
}
impl NotificationService {
    /// Serializes backend-global token ownership and rotates this client's prior token first.
    /// Different accounts on one backend cannot independently own the same physical device token.
    /// # Errors
    /// Returns invalid metadata, stale account or definite backend failure. Ambiguous writes return Unknown.
    pub async fn register_push(
        &self,
        device: PushDevice,
    ) -> Result<PushRegistration, NotificationError> {
        device.request()?;
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move { Ok(register(&session, device).await) })
            .await?
    }
    /// Deletes the authenticated owner registration before permission disable/signout/token rotation.
    /// # Errors
    /// Returns account/transport failures; ambiguous network outcomes are observable as Unknown.
    pub async fn unregister_push(&self) -> Result<PushRegistration, NotificationError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move { Ok(unregister(&session).await) })
            .await?
    }
    /// Removes a host-supplied token after restart or an uncertain registration.
    /// The authenticated owner-scoped DELETE is authoritative; no local owner-map assumption is used.
    /// # Errors
    /// Returns invalid metadata, account or definite backend failure; uncertain writes return Unknown.
    pub async fn unregister_push_device(
        &self,
        device: PushDevice,
    ) -> Result<PushRegistration, NotificationError> {
        device.request()?;
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move {
                let mut owners = session.client.inner.push.serial.lock().await;
                let epoch = session.inner.epochs.current();
                let rest = crate::scoped_request::authenticated(&session, &epoch).await?;
                let response = session
                    .client
                    .inner
                    .request(rest.delete_push_device(
                        &device.request().map_err(|_| ClientError::Configuration)?,
                    ))
                    .await;
                let result = if removed(&response) {
                    PushRegistration::Removed
                } else if !definite(&response) {
                    PushRegistration::Unknown
                } else {
                    response?;
                    PushRegistration::Removed
                };
                if result == PushRegistration::Removed
                    && owners
                        .get(&session.account().server())
                        .is_some_and(|owner| owner.device.token == device.token)
                {
                    retire_owner(&mut owners, session.account().server(), session.account());
                }
                session.inner.epochs.validate(epoch.generation)?;
                Ok(result)
            })
            .await
            .map_err(Into::into)
    }
}
async fn register(
    session: &Session,
    device: PushDevice,
) -> Result<PushRegistration, NotificationError> {
    let mut owners = session.client.inner.push.serial.lock().await;
    let epoch = session.inner.epochs.current();
    session.inner.epochs.validate(epoch.generation)?;
    if session
        .inner
        .notifications
        .push_closing
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(ClientError::Closed.into());
    }
    let rest = crate::scoped_request::authenticated(session, &epoch).await?;
    let server = session.account().server();
    if let Some(old) = owners.get(&server) {
        if old.uncertain {
            return Ok(PushRegistration::Unknown);
        }
        if old.device.token != device.token {
            let old_session = session.client.session(old.account)?;
            let old_epoch = old_session.inner.epochs.current();
            let old_rest = crate::scoped_request::authenticated(&old_session, &old_epoch).await?;
            let response = old_session
                .client
                .inner
                .request(old_rest.delete_push_device(&old.device.request()?))
                .await;
            if !removed(&response) {
                if definite(&response) {
                    response?;
                } else {
                    if let Some(old) = owners.get_mut(&server) {
                        old.uncertain = true;
                    }
                    return Ok(PushRegistration::Unknown);
                }
            }
        }
    }
    if owners
        .get(&server)
        .is_some_and(|old| old.device.token != device.token)
    {
        owners.remove(&server);
    }
    session.inner.epochs.validate(epoch.generation)?;
    if session
        .inner
        .notifications
        .push_closing
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(ClientError::Closed.into());
    }
    let previous = owners
        .get(&server)
        .filter(|owner| owner.account != session.account())
        .map(|owner| owner.account);
    // Do not cancel an admitted mutation on an epoch change; retain serialization until its outcome.
    let response = session
        .client
        .inner
        .request(rest.register_push_device(&device.request()?))
        .await;
    let uncertain = !definite(&response);
    if uncertain || response.is_ok() {
        owners.insert(
            server,
            PushOwner {
                account: session.account(),
                device,
                uncertain,
                previous: uncertain.then_some(previous).flatten(),
            },
        );
    }
    if uncertain {
        return Ok(PushRegistration::Unknown);
    }
    response?;
    session.inner.epochs.validate(epoch.generation)?;
    Ok(PushRegistration::Registered)
}
pub(crate) async fn unregister(session: &Session) -> Result<PushRegistration, NotificationError> {
    let mut owners = session.client.inner.push.serial.lock().await;
    let server = session.account().server();
    let Some(owner) = owners.get(&server) else {
        return Ok(PushRegistration::Unknown);
    };
    if owner.account != session.account() && owner.previous != Some(session.account()) {
        return Ok(if owner.uncertain {
            PushRegistration::Unknown
        } else {
            PushRegistration::Removed
        });
    }
    let epoch = session.inner.epochs.current();
    let rest = crate::scoped_request::authenticated(session, &epoch).await?;
    let response = session
        .client
        .inner
        .request(rest.delete_push_device(&owner.device.request()?))
        .await;
    if removed(&response) {
        retire_owner(&mut owners, server, session.account());
        return Ok(PushRegistration::Removed);
    }
    if !definite(&response) {
        if let Some(owner) = owners.get_mut(&server) {
            owner.uncertain = true;
        }
        return Ok(PushRegistration::Unknown);
    }
    response?;
    Ok(PushRegistration::Removed)
}
fn removed(response: &Result<(), ClientError>) -> bool {
    matches!(
        response,
        Ok(())
            | Err(ClientError::Api(
                papo_api::Error::Http { status: 404 }
                    | papo_api::Error::Problem { status: 404, .. }
            ))
    )
}
fn definite(response: &Result<(), ClientError>) -> bool {
    matches!(response, Ok(()))
        || matches!(
            response,
            Err(ClientError::Api(
                papo_api::Error::Http { status: 400..=499 }
                    | papo_api::Error::Problem {
                        status: 400..=499,
                        ..
                    }
            ))
        ) && !matches!(
            response,
            Err(ClientError::Api(
                papo_api::Error::Http { status: 408 }
                    | papo_api::Error::Problem { status: 408, .. }
            ))
        )
}

pub(crate) async fn unregister_known(
    session: &Session,
) -> Result<PushRegistration, NotificationError> {
    if !session
        .client
        .inner
        .push
        .serial
        .lock()
        .await
        .contains_key(&session.account().server())
    {
        return Ok(PushRegistration::Removed);
    }
    unregister(session).await
}

fn retire_owner(owners: &mut BTreeMap<ServerId, PushOwner>, server: ServerId, account: AccountId) {
    let Some(owner) = owners.get_mut(&server) else {
        return;
    };
    if owner.account == account {
        if let Some(previous) = owner.previous.take() {
            owner.account = previous;
            owner.uncertain = true;
        } else {
            owners.remove(&server);
        }
    } else if owner.previous == Some(account) {
        owner.previous = None;
        owner.uncertain = true;
    }
}
