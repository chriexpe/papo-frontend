//! Authenticated account operations and credential rotation; wire requests remain typed API calls.

use crate::{AuthService, AuthState, ClientError, Generated, secret::SecretValue};
use papo_api::wire::{
    ChangePasswordRequest, ConnectedDevicesResponse, DropConnectionRequest, DropConnectionResponse,
    GenericResponse,
};

impl AuthService {
    /// Explicitly rotates the session cookie, verifies identity and persists it before reconnecting.
    /// # Errors
    /// Returns signed-out, backend, identity, superseded or uncertain credential errors.
    pub async fn refresh(&self) -> Result<(), ClientError> {
        self.run(|this, epoch| async move {
            let session = this.session.as_ref().ok_or(ClientError::SignedOut)?;
            let epoch = epoch.as_ref().ok_or(ClientError::SignedOut)?;
            let rest = this.transport(Some(epoch)).await?;
            let result = async {
                this.client.inner.request(rest.refresh()).await?;
                let who = this.client.inner.request(rest.whoami()).await?;
                let user = who
                    .id
                    .as_deref()
                    .and_then(|value| value.parse::<papo_api::Uuid>().ok())
                    .ok_or(ClientError::InvalidAuth)?;
                if user != session.account().user() {
                    return Err(ClientError::Identity);
                }
                this.validate(Some(epoch))?;
                let snapshot = rest.credential_snapshot()?;
                this.client
                    .inner
                    .secrets
                    .write(
                        session.inner.key,
                        SecretValue::new(snapshot.expose_bytes().to_vec()),
                        &epoch.cancel,
                    )
                    .await?;
                this.publish(Some(epoch), AuthState::Ready)
            }
            .await;
            if let Err(error) = &result {
                this.publish(Some(epoch), Self::failure_state(error, true))?;
            }
            result
        })
        .await?;
        self.session
            .as_ref()
            .ok_or(ClientError::SignedOut)?
            .reconnect()
    }
    /// Revokes the remote cookie, deletes the local secret, then revokes all account handles.
    /// Local sign-out is attempted even if the remote request fails; uncertainty is never hidden.
    /// # Errors
    /// Returns backend or credential failures; an uncertain deletion requires read-back on reuse.
    pub async fn logout(&self) -> Result<(), ClientError> {
        let session = self.session.clone().ok_or(ClientError::SignedOut)?;
        session
            .inner
            .notifications
            .push_closing
            .store(true, std::sync::atomic::Ordering::Release);
        let result = self
            .run(|this, epoch| async move {
                let session = this.session.as_ref().ok_or(ClientError::SignedOut)?;
                let push = crate::notifications::push::unregister_known(session).await;
                let remote = match this.transport(epoch.as_ref()).await {
                    Ok(rest) => this.client.inner.request(rest.logout()).await,
                    Err(error) => Err(error),
                };
                this.validate(epoch.as_ref())?;
                this.client
                    .inner
                    .secrets
                    .delete(session.inner.key, &session.inner.root)
                    .await?;
                this.publish(epoch.as_ref(), AuthState::SignedOut)?;
                match push {
                    Ok(crate::PushRegistration::Unknown) | Err(_) => Err(ClientError::Timeout),
                    _ => remote,
                }
            })
            .await;
        let client = self.client.clone();
        let expected = std::sync::Arc::clone(&session.inner);
        self.client
            .dispatch(async move {
                let _guard = client.inner.mutation.lock().await;
                client.inner.pool.stop_if_current(&expected).await;
                Ok(())
            })
            .await?;
        result
    }
    /// Lists connected devices with the generation that requested them.
    /// # Errors
    /// Returns signed-out, revoked, stale or typed REST failures.
    pub async fn connected_devices(
        &self,
    ) -> Result<Generated<ConnectedDevicesResponse>, ClientError> {
        self.run(|this, epoch| async move {
            let epoch = epoch.as_ref().ok_or(ClientError::SignedOut)?;
            let rest = this.transport(Some(epoch)).await?;
            let result = this.client.inner.request(rest.connected_devices()).await?;
            this.generated(epoch, result)
        })
        .await
    }
    /// Drops one or all connected devices, as explicitly specified by the caller.
    /// # Errors
    /// Returns signed-out, revoked, stale or typed REST failures.
    pub async fn drop_connection(
        &self,
        request: DropConnectionRequest,
    ) -> Result<Generated<DropConnectionResponse>, ClientError> {
        self.run(move |this, epoch| async move {
            let epoch = epoch.as_ref().ok_or(ClientError::SignedOut)?;
            let rest = this.transport(Some(epoch)).await?;
            let result = this
                .client
                .inner
                .request(rest.drop_connection(&request))
                .await?;
            this.generated(epoch, result)
        })
        .await
    }
    /// Changes this verified account's password when the backend requires a reset.
    /// # Errors
    /// Returns backend reset-policy rejection, revoked or stale-generation failures.
    pub async fn change_password(
        &self,
        request: ChangePasswordRequest,
    ) -> Result<Generated<GenericResponse>, ClientError> {
        self.run(move |this, epoch| async move {
            let epoch = epoch.as_ref().ok_or(ClientError::SignedOut)?;
            let session = this.session.as_ref().ok_or(ClientError::SignedOut)?;
            let rest = this.transport(Some(epoch)).await?;
            let result = this
                .client
                .inner
                .request(rest.change_password(session.account().user(), &request))
                .await?;
            this.generated(epoch, result)
        })
        .await
    }
}
