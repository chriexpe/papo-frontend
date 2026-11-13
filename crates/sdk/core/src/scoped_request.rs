//! Verified, cancellable account REST admission; services retain their own policy and result fences.

use crate::{ClientError, Session, generation::Epoch};

pub(crate) async fn authenticated(
    session: &Session,
    epoch: &Epoch,
) -> Result<papo_api::RestClient, ClientError> {
    let mut verified = session.inner.verified_rest.lock().await;
    session.inner.epochs.validate(epoch.generation)?;
    if let Some((generation, rest)) = &*verified
        && *generation == epoch.generation
    {
        return Ok(rest.clone());
    }
    let secret = session
        .client
        .inner
        .secrets
        .read(session.inner.key, &epoch.cancel)
        .await?
        .ok_or(ClientError::SignedOut)?;
    let rest = session
        .client
        .inner
        .transport(&session.inner.address, Some(secret.expose()))?;
    refresh_policy(session, epoch, &rest).await?;
    *verified = Some((epoch.generation, rest.clone()));
    Ok(rest)
}

pub(crate) async fn refresh_policy(
    session: &Session,
    epoch: &Epoch,
    rest: &papo_api::RestClient,
) -> Result<(), ClientError> {
    let who = session.client.inner.request(rest.whoami()).await?;
    if who.id.as_deref().and_then(|id| id.parse().ok()) != Some(session.account().user()) {
        return Err(ClientError::Identity);
    }
    crate::notifications::backend::remember(&session.inner, epoch.generation, who.settings)
}
