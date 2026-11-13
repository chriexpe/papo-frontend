//! Startup session restoration after removal recovery; identity verification precedes Ready or gateway use.

use crate::{
    AuthState, ClientError, ClientHandle, SavedAccount, ServerAddress, secret::SecretKey,
    session::SessionInner,
};
use std::sync::Arc;

pub(crate) async fn restore(
    client: &ClientHandle,
    address: ServerAddress,
    account: SavedAccount,
) -> Result<(), ClientError> {
    client
        .inner
        .secrets
        .activate(account.id, account.credential_generation)?;
    let inner = SessionInner::new(
        account.id,
        address,
        SecretKey::new(account.id, account.credential_generation),
        Arc::clone(&client.inner.pool.next),
        client.inner.pool.root.child_token(),
        AuthState::NeedsServer,
    );
    client.inner.pool.insert(Arc::clone(&inner)).await;
    let result = async {
        let Some(secret) = client.inner.secrets.read(inner.key, &inner.root).await? else {
            return Ok(false);
        };
        let rest = client
            .inner
            .transport(&inner.address, Some(secret.expose()))?;
        let who = client.inner.request(rest.whoami()).await?;
        let user = who
            .id
            .as_deref()
            .and_then(|value| value.parse::<papo_api::Uuid>().ok())
            .ok_or(ClientError::InvalidAuth)?;
        if user != account.id.user() {
            return Err(ClientError::Identity);
        }
        Ok(true)
    }
    .await;
    match result {
        Ok(true) => {
            inner.auth.send_replace(AuthState::Ready);
            crate::connection_driver::start(client.clone(), inner);
        }
        Ok(false) => {
            inner.auth.send_replace(AuthState::SignedOut);
        }
        Err(error) => {
            let state = super::AuthService::failure_state(&error, true);
            inner.auth.send_replace(state);
            if state == AuthState::NeedsServer {
                crate::connection_driver::start(client.clone(), inner);
            }
        }
    }
    Ok(())
}
