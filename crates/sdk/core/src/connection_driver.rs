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
    /// Where each acknowledged heartbeat's round trip is published.
    round_trip: Arc<crate::observable::StateCell<Option<crate::RoundTrip>>>,
    resources: crate::reconcile::ResourceConnection,
    typing: tokio::sync::watch::Receiver<Option<Generated<(papo_api::Uuid, tokio::time::Instant)>>>,
    voice: crate::calls::VoiceCommandReceiver,
    voice_closed: bool,
    calls: Arc<crate::calls::SessionCalls>,
    generation: crate::Generation,
    prefer_read: bool,
}
impl AccountSocket {
    async fn send_voice_item(
        &mut self,
        item: Option<crate::calls::QueuedVoiceCommand>,
    ) -> Result<(), ClientError> {
        let Some(item) = item else {
            self.voice_closed = true;
            return Ok(());
        };
        if self.calls.should_send(&item, self.generation) {
            self.gateway.send(&item.command).await?;
        }
        Ok(())
    }

    /// Publishes the round trip the event read completed, if it completed one, and
    /// passes the event on unchanged.
    fn measured(&mut self, event: Option<Event>) -> Option<Event> {
        publish_round_trip(
            &self.round_trip,
            self.generation,
            self.gateway.take_round_trip(),
        );
        event
    }

    async fn send_typing_if_fresh(&mut self) -> Result<(), ClientError> {
        let command = self.typing.borrow_and_update().clone();
        if let Some(command) = command
            && command.generation == self.generation
            && command.value.1.elapsed() < std::time::Duration::from_secs(10)
        {
            self.gateway
                .send(&papo_api::gateway::Command::Typing {
                    channel_id: command.value.0.to_string(),
                })
                .await?;
        }
        Ok(())
    }
}
/// Publishes `measured`, a round trip the socket just completed, if any.
fn publish_round_trip(
    cell: &crate::observable::StateCell<Option<crate::RoundTrip>>,
    generation: crate::Generation,
    measured: Option<std::time::Duration>,
) {
    if let Some(round_trip) = measured {
        cell.send_replace(Some(crate::RoundTrip {
            generation,
            round_trip,
        }));
    }
}
impl Socket for AccountSocket {
    fn next(&mut self) -> IoFuture<'_, Option<Event>> {
        Box::pin(async {
            loop {
                if self.prefer_read {
                    tokio::select! {
                        biased;
                        result = self.gateway.next() => {
                            self.prefer_read = false;
                            return Ok(self.measured(result?));
                        }
                        item = self.voice.recv(), if !self.voice_closed => {
                            self.send_voice_item(item).await?;
                            self.prefer_read = true;
                        }
                        changed = self.typing.changed() => {
                            if changed.is_err() { return Err(ClientError::Closed); }
                            self.send_typing_if_fresh().await?;
                        }
                    }
                } else {
                    tokio::select! {
                        biased;
                        item = self.voice.recv(), if !self.voice_closed => {
                            self.send_voice_item(item).await?;
                            self.prefer_read = true;
                        }
                        result = self.gateway.next() => {
                            self.prefer_read = false;
                            return Ok(self.measured(result?));
                        }
                        changed = self.typing.changed() => {
                            if changed.is_err() { return Err(ClientError::Closed); }
                            self.send_typing_if_fresh().await?;
                        }
                    }
                }
            }
        })
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
            let secret = session
                .read_secret(&self.0.inner.secrets, &epoch.cancel)
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
            crate::notifications::backend::remember(session, epoch.generation, who.settings)?;
            *session.verified_rest.lock().await = Some((epoch.generation, rest.clone()));
            let cache = self.0.inner.caches.for_reconcile(session.account).await?;
            session.epochs.validate(epoch.generation)?;
            let gateway =
                tokio::time::timeout(self.0.inner.config.request_timeout, Gateway::connect(&rest))
                    .await
                    .map_err(|_| ClientError::Timeout)??;
            session.epochs.validate(epoch.generation)?;
            let voice = session.calls.attach_socket(epoch.generation);
            Ok(Box::new(AccountSocket {
                gateway,
                round_trip: Arc::clone(&session.round_trip),
                typing: session.typing_command.subscribe(),
                voice,
                voice_closed: false,
                calls: Arc::clone(&session.calls),
                generation: epoch.generation,
                prefer_read: true,
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
    let expiry_session = Arc::clone(&session);
    let expiry_token = session.tasks.token();
    client.inner.runtime.spawn(async move {
        let _token = expiry_token;
        crate::activity::expire_loop(expiry_session).await;
    });
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
    session: &Arc<SessionInner>,
    connector: &dyn Connector,
) {
    let mut attempt = 0_u32;
    loop {
        if session.root.is_cancelled() || session.driver_cancel.is_cancelled() {
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
            () = session.driver_cancel.cancelled() => return,
            () = epoch.cancel.cancelled() => continue,
            result = connector.connect(session, &epoch) => result,
        };
        let failure = match connected {
            Ok(mut socket) => {
                let reconciles = socket.resources().is_some();
                if !reconciles
                    && session
                        .status(epoch.generation, ConnectionState::Online)
                        .is_err()
                {
                    continue;
                }
                let _ = session.epochs.apply(epoch.generation, || {
                    session.auth.send_replace(AuthState::Ready)
                });
                if !reconciles {
                    attempt = 0;
                }
                let mut recovery = session.resource_recovery.subscribe();
                let failure = tokio::select! {
                    biased;
                    () = session.driver_cancel.cancelled() => return,
                    _ = recovery.wait_for(|requested| *requested == Some(epoch.generation)) => Some(ClientError::ReconcileRequired),
                    result = pump(session, &epoch, socket.as_mut()) => result,
                };
                // A reconciling socket proves the connection only once reconcile reaches
                // Online. A reply rejected before that keeps growing the delay, so a
                // persistently malformed backend never retries at the initial interval.
                if reconciles && reached_online(session, epoch.generation) {
                    attempt = 0;
                }
                failure
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
                () = session.driver_cancel.cancelled() => return,
                () = epoch.cancel.cancelled() => continue,
            }
        }
        if !wait_backoff(config, session, epoch.generation, &mut attempt).await {
            return;
        }
    }
}

/// Between the driver's `Connecting` and its next status, only reconcile writes one
/// (session stop also does, but then the driver exits). So after the pump returns, the
/// status still shows whether this generation got as far as Online.
fn reached_online(session: &SessionInner, generation: crate::Generation) -> bool {
    *session.connection.borrow()
        == crate::ConnectionStatus {
            generation,
            state: ConnectionState::Online,
        }
}

async fn wait_backoff(
    config: &crate::ClientConfig,
    session: &SessionInner,
    failed: crate::Generation,
    attempt: &mut u32,
) -> bool {
    let next = match session.epochs.replace(Some(failed), |_| {}) {
        Ok(next) => next,
        Err(ClientError::Superseded) => return true,
        Err(_) => return false,
    };
    if session
        .history
        .generation_changed(next.generation)
        .await
        .is_err()
    {
        return true;
    }
    let retry_at =
        tokio::time::Instant::now() + config.backoff.delay(*attempt, config.jitter.as_ref());
    *attempt = attempt.saturating_add(1);
    if session
        .status(next.generation, ConnectionState::Backoff { retry_at })
        .is_err()
    {
        return true;
    }
    tokio::select! {
        biased;
        () = session.root.cancelled() => false,
        () = session.driver_cancel.cancelled() => false,
        () = next.cancel.cancelled() => true,
        () = tokio::time::sleep_until(retry_at) => true,
    }
}

#[cfg(test)]
mod tests;

/// Whether a reconcile failure means the account cache itself cannot be trusted. Only
/// storage-level faults do; a history layout invariant (`HistoryLayout`) is rejected before
/// any write, so it leaves the cache intact and never discards it.
fn invalidates_account_cache(
    session: &SessionInner,
    generation: crate::Generation,
    error: &ClientError,
) -> bool {
    // A failed resource commit may already have switched SQLite's visible revision.
    // Its replacement must reload that revision even when downstream installation hit I/O.
    if *session.resource_recovery.borrow() == Some(generation) {
        return false;
    }
    matches!(
        error,
        ClientError::Cache(
            crate::CacheError::Storage(_)
                | crate::CacheError::InvalidData
                | crate::CacheError::TaskStopped
        )
    )
}

/// Serves exactly one socket; returning a failure never reconnects inside the API layer.
async fn pump(
    session: &Arc<SessionInner>,
    epoch: &Epoch,
    socket: &mut dyn Socket,
) -> Option<ClientError> {
    if let Some(resources) = socket.resources() {
        let result = crate::reconcile::run(session, epoch, socket, &resources).await;
        if result
            .as_ref()
            .is_err_and(|error| invalidates_account_cache(session, epoch.generation, error))
            && session.epochs.validate(epoch.generation).is_ok()
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
                crate::calls::receive_gateway_event(
                    session,
                    epoch.generation,
                    &event,
                    &tokio::runtime::Handle::current(),
                );
                if !crate::calls::is_voice_only_event(&event) {
                    let _ = session.epochs.apply(epoch.generation, || {
                        session.events.send(Generated {
                            generation: epoch.generation,
                            value: event,
                        })
                    });
                }
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
