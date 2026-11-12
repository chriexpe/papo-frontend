//! Revocable account handles and task groups; auth operations and reconnect driving live separately.

use crate::{
    AccountId, AuthService, AuthState, ClientError, ClientHandle, ConnectionState,
    ConnectionStatus, Generated, Generation, ServerAddress, generation::Epochs, secret::SecretKey,
};
use papo_api::gateway::Event;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64},
};
use tokio::sync::{broadcast, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(crate) struct SessionInner {
    pub account: AccountId,
    pub address: ServerAddress,
    pub key: SecretKey,
    pub root: CancellationToken,
    pub epochs: Epochs,
    pub tasks: TaskTracker,
    pub auth: Arc<crate::observable::StateCell<AuthState>>,
    pub connection: Arc<crate::observable::StateCell<ConnectionStatus>>,
    pub events: broadcast::Sender<Generated<Event>>,
    pub resources: Arc<crate::observable::StateCell<Arc<crate::ResourceSnapshot>>>,
    pub driver_running: AtomicBool,
    pub auth_serial: tokio::sync::Mutex<()>,
    pub resource_serial: tokio::sync::Mutex<()>,
    pub history: crate::history::HistoryStore,
    pub reading: crate::reading::Reading,
    pub outgoing: crate::outgoing::AccountQueue,
}
impl SessionInner {
    pub fn new(
        account: AccountId,
        address: ServerAddress,
        key: SecretKey,
        next: Arc<AtomicU64>,
        root: CancellationToken,
        auth_state: AuthState,
    ) -> Arc<Self> {
        let epochs = Epochs::new(next, root.clone());
        let generation = epochs.current().generation;
        Arc::new(Self {
            account,
            address,
            key,
            root,
            epochs,
            tasks: TaskTracker::new(),
            auth: crate::observable::StateCell::new(auth_state),
            connection: crate::observable::StateCell::new(ConnectionStatus {
                generation,
                state: ConnectionState::Offline,
            }),
            events: broadcast::channel(64).0,
            resources: crate::observable::StateCell::new(Arc::new(crate::ResourceSnapshot::empty(
                generation,
            ))),
            driver_running: AtomicBool::new(false),
            auth_serial: tokio::sync::Mutex::new(()),
            resource_serial: tokio::sync::Mutex::new(()),
            history: crate::history::HistoryStore::default(),
            reading: crate::reading::Reading::new(generation),
            outgoing: crate::outgoing::AccountQueue::default(),
        })
    }
    pub async fn stop(&self) {
        self.root.cancel();
        self.history.close_all().await;
        self.reading.image.close();
        self.outgoing.image.close();
        self.tasks.close();
        self.connection.send_replace(ConnectionStatus {
            generation: self.epochs.current().generation,
            state: ConnectionState::Offline,
        });
        self.auth.send_replace(AuthState::SignedOut);
        self.auth.close();
        self.connection.close();
        self.resources.close();
        self.tasks.wait().await;
    }
    pub fn publish_resources(
        &self,
        mut snapshot: crate::ResourceSnapshot,
    ) -> Result<(), ClientError> {
        self.epochs.apply(snapshot.generation, || {
            snapshot.version = self
                .resources
                .borrow()
                .version
                .checked_add(1)
                .ok_or(ClientError::Closed)?;
            self.resources.send_replace(Arc::new(snapshot));
            Ok(())
        })?
    }
    pub fn status(
        &self,
        generation: Generation,
        state: ConnectionState,
    ) -> Result<(), ClientError> {
        self.epochs.apply(generation, || {
            self.connection
                .send_replace(ConnectionStatus { generation, state })
        })?;
        Ok(())
    }
}

/// One signed-in account. Replacement/removal revokes every clone of this handle.
#[derive(Clone)]
pub struct Session {
    pub(crate) inner: Arc<SessionInner>,
    pub(crate) client: ClientHandle,
}
impl Session {
    /// Returns this session's verified server/user identity.
    #[must_use]
    pub fn account(&self) -> AccountId {
        self.inner.account
    }
    /// Returns true after removal, replacement, logout or client shutdown.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.root.is_cancelled()
    }
    /// Returns the current request/socket epoch.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.inner.epochs.current().generation
    }
    /// Gets account-scoped typed authentication operations.
    #[must_use]
    pub fn auth(&self) -> AuthService {
        AuthService::for_session(self.clone())
    }
    /// Subscribes to generation-stamped connection state. Intermediate states may coalesce.
    #[must_use]
    pub fn subscribe_connection(&self) -> watch::Receiver<ConnectionStatus> {
        self.inner.connection.subscribe()
    }
    /// Subscribes to bounded live traffic. Old queued generations are discarded at delivery.
    #[must_use]
    pub fn subscribe_events(&self) -> SessionEvents {
        SessionEvents {
            inner: Arc::clone(&self.inner),
            receiver: self.inner.events.subscribe(),
        }
    }
    /// Subscribes atomically to this account's generation-stamped resource image.
    /// The bounded watch coalesces intermediate publications and closes on revocation.
    #[must_use]
    pub fn subscribe_resources(&self) -> watch::Receiver<Arc<crate::ResourceSnapshot>> {
        self.inner.resources.subscribe()
    }
    /// Requests a new connection generation, canceling old socket and REST work.
    /// # Errors
    /// Returns `Closed` if this handle was revoked.
    pub fn reconnect(&self) -> Result<(), ClientError> {
        if !matches!(
            *self.inner.auth.borrow(),
            AuthState::Ready | AuthState::NeedsServer
        ) {
            return Err(ClientError::SignedOut);
        }
        self.inner.epochs.advance()?;
        crate::connection_driver::start(self.client.clone(), Arc::clone(&self.inner));
        Ok(())
    }
    /// Rejects an explicitly stamped result after its requesting generation ended.
    /// # Errors
    /// Returns `Closed` or `Superseded`; no stale value is exposed.
    pub fn current<T>(&self, result: Generated<T>) -> Result<T, ClientError> {
        self.inner.epochs.validate(result.generation)?;
        Ok(result.value)
    }
}

/// A bounded account subscription that closes on account revocation.
pub struct SessionEvents {
    inner: Arc<SessionInner>,
    receiver: broadcast::Receiver<Generated<Event>>,
}
impl SessionEvents {
    /// Receives only the current generation; lag is explicit rather than unbounded buffering.
    /// # Errors
    /// Returns `Closed` on revocation or `Lagged` if the consumer missed live events.
    pub async fn next(&mut self) -> Result<Generated<Event>, SessionEventError> {
        loop {
            let event = tokio::select! {
                biased;
                () = self.inner.root.cancelled() => return Err(SessionEventError::Closed),
                event = self.receiver.recv() => event.map_err(|error| match error {
                    broadcast::error::RecvError::Closed => SessionEventError::Closed,
                    broadcast::error::RecvError::Lagged(count) => SessionEventError::Lagged(count),
                })?,
            };
            if self.inner.epochs.validate(event.generation).is_ok() {
                return Ok(event);
            }
        }
    }
}

/// Typed live-subscription termination or overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SessionEventError {
    /// This account handle was revoked.
    #[error("session subscription closed")]
    Closed,
    /// The bounded subscriber missed this many events; reconnect/reconcile is needed.
    #[error("session subscription lagged")]
    Lagged(u64),
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
