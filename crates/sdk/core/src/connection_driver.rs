//! Generation-fenced reconnect task; transport connects once and resource reconciliation owns its journal.

use crate::{
    AuthState, ClientError, ClientHandle, ConnectionState, Generated, generation::Epoch,
    session::SessionInner,
};
use papo_api::gateway::{Event, Gateway};
use std::{future::Future, pin::Pin, sync::Arc};

type IoFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ClientError>> + Send + 'a>>;
pub(crate) trait Socket: Send {
    fn next(&mut self) -> IoFuture<'_, Option<Event>>;
    fn resources(&self) -> Option<crate::reconcile::ResourceConnection> {
        None
    }
}
struct AccountSocket {
    gateway: Gateway,
    resources: crate::reconcile::ResourceConnection,
}
impl Socket for AccountSocket {
    fn next(&mut self) -> IoFuture<'_, Option<Event>> {
        Box::pin(async { Ok(self.gateway.next().await?) })
    }
    fn resources(&self) -> Option<crate::reconcile::ResourceConnection> {
        Some(self.resources.clone())
    }
}
impl Socket for Gateway {
    fn next(&mut self) -> IoFuture<'_, Option<Event>> {
        Box::pin(async { Ok(Gateway::next(self).await?) })
    }
}
pub(crate) trait Connector: Send + Sync {
    fn connect<'a>(
        &'a self,
        session: &'a SessionInner,
        epoch: &'a Epoch,
    ) -> IoFuture<'a, Box<dyn Socket>>;
}
struct ApiConnector(ClientHandle);
impl Connector for ApiConnector {
    fn connect<'a>(
        &'a self,
        session: &'a SessionInner,
        epoch: &'a Epoch,
    ) -> IoFuture<'a, Box<dyn Socket>> {
        Box::pin(async move {
            let secret = self
                .0
                .inner
                .secrets
                .read(session.key, &epoch.cancel)
                .await?
                .ok_or(ClientError::SignedOut)?;
            session.epochs.validate(epoch.generation)?;
            let rest = self
                .0
                .inner
                .transport(&session.address, Some(secret.expose()))?;
            let who = self.0.inner.request(rest.whoami()).await?;
            session.epochs.validate(epoch.generation)?;
            if who.id.as_deref().and_then(|id| id.parse().ok()) != Some(session.account.user()) {
                return Err(ClientError::Identity);
            }
            let cache = self.0.inner.caches.open(session.account).await?;
            session.epochs.validate(epoch.generation)?;
            let gateway =
                tokio::time::timeout(self.0.inner.config.request_timeout, Gateway::connect(&rest))
                    .await
                    .map_err(|_| ClientError::Timeout)??;
            session.epochs.validate(epoch.generation)?;
            Ok(Box::new(AccountSocket {
                gateway,
                resources: crate::reconcile::ResourceConnection {
                    client: self.0.clone(),
                    rest,
                    cache,
                },
            }) as Box<dyn Socket>)
        })
    }
}

pub(crate) fn start(client: ClientHandle, session: Arc<SessionInner>) {
    crate::outgoing::dispatch::start(crate::Session {
        inner: Arc::clone(&session),
        client: client.clone(),
    });
    if session
        .driver_running
        .swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        return;
    }
    let token = session.tasks.token();
    let runtime = client.inner.runtime.clone();
    runtime.spawn(async move {
        let _token = token;
        drive(
            &client.inner.config,
            &session,
            &ApiConnector(client.clone()),
        )
        .await;
        session
            .driver_running
            .store(false, std::sync::atomic::Ordering::Release);
    });
}

pub(crate) async fn drive(
    config: &crate::ClientConfig,
    session: &SessionInner,
    connector: &dyn Connector,
) {
    let mut attempt = 0_u32;
    loop {
        if session.root.is_cancelled() {
            return;
        }
        let epoch = session.epochs.current();
        if session
            .history
            .generation_changed(epoch.generation)
            .await
            .is_err()
        {
            continue;
        }
        if session
            .status(epoch.generation, ConnectionState::Connecting)
            .is_err()
        {
            continue;
        }
        let connected = tokio::select! {
            biased;
            () = session.root.cancelled() => return,
            () = epoch.cancel.cancelled() => continue,
            result = connector.connect(session, &epoch) => result,
        };
        let failure = match connected {
            Ok(mut socket) => {
                if socket.resources().is_none()
                    && session
                        .status(epoch.generation, ConnectionState::Online)
                        .is_err()
                {
                    continue;
                }
                let _ = session.epochs.apply(epoch.generation, || {
                    session.auth.send_replace(AuthState::Ready)
                });
                attempt = 0;
                pump(session, &epoch, socket.as_mut()).await
            }
            Err(error) => Some(error),
        };
        let Some(error) = failure else {
            continue;
        };
        if session.epochs.validate(epoch.generation).is_err() {
            continue;
        }
        let auth_state = crate::AuthService::failure_state(&error, true);
        if session
            .epochs
            .apply(epoch.generation, || session.auth.send_replace(auth_state))
            .is_err()
        {
            continue;
        }
        if matches!(error, ClientError::Identity | ClientError::SignedOut)
            || auth_state == AuthState::Expired
        {
            let _ = session.status(epoch.generation, ConnectionState::Offline);
            tokio::select! {
                biased;
                () = session.root.cancelled() => return,
                () = epoch.cancel.cancelled() => continue,
            }
        }
        let Ok(next) = session.epochs.advance() else {
            return;
        };
        if session
            .history
            .generation_changed(next.generation)
            .await
            .is_err()
        {
            continue;
        }
        let retry_at =
            tokio::time::Instant::now() + config.backoff.delay(attempt, config.jitter.as_ref());
        attempt = attempt.saturating_add(1);
        if session
            .status(next.generation, ConnectionState::Backoff { retry_at })
            .is_err()
        {
            continue;
        }
        tokio::select! {
            biased;
            () = session.root.cancelled() => return,
            () = next.cancel.cancelled() => {},
            () = tokio::time::sleep_until(retry_at) => {},
        }
    }
}

#[cfg(test)]
mod tests;

/// Serves exactly one socket; returning a failure never reconnects inside the API layer.
async fn pump(
    session: &SessionInner,
    epoch: &Epoch,
    socket: &mut dyn Socket,
) -> Option<ClientError> {
    if let Some(resources) = socket.resources() {
        let result = crate::reconcile::run(session, epoch, socket, &resources).await;
        if matches!(
            &result,
            Err(ClientError::Cache(
                crate::CacheError::Storage(_)
                    | crate::CacheError::InvalidData
                    | crate::CacheError::TaskStopped
            ))
        ) && session.epochs.validate(epoch.generation).is_ok()
            && let Err(error) = resources
                .client
                .inner
                .caches
                .invalidate(session.account)
                .await
        {
            return Some(error.into());
        }
        return result.err();
    }
    loop {
        let next = tokio::select! {
            biased;
            () = session.root.cancelled() => return None,
            () = epoch.cancel.cancelled() => return None,
            result = socket.next() => result,
        };
        match next {
            Ok(Some(event)) => {
                let closed = matches!(event, Event::Closed(_));
                let _ = session.epochs.apply(epoch.generation, || {
                    session.events.send(Generated {
                        generation: epoch.generation,
                        value: event,
                    })
                });
                if closed {
                    return Some(ClientError::Gateway(papo_api::gateway::Error::Transport));
                }
            }
            Ok(None) => {
                return Some(ClientError::Gateway(papo_api::gateway::Error::Transport));
            }
            Err(error) => return Some(error),
        }
    }
}
