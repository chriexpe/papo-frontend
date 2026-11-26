//! Revocable account handles and task groups; auth operations and reconnect driving live separately.

use crate::{
    AccountId, AuthService, AuthState, ClientError, ClientHandle, ConnectionState,
    ConnectionStatus, Generated, Generation, ServerAddress,
    generation::Epochs,
    secret::{SecretError, SecretFailure, SecretKey, SecretStoreBoundary, SecretValue},
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
    pub driver_cancel: CancellationToken,
    pub epochs: Epochs,
    pub tasks: TaskTracker,
    pub auth: Arc<crate::observable::StateCell<AuthState>>,
    pub connection: Arc<crate::observable::StateCell<ConnectionStatus>>,
    /// The gateway's latest heartbeat round trip; none before the first.
    pub round_trip: Arc<crate::observable::StateCell<Option<crate::RoundTrip>>>,
    pub events: broadcast::Sender<Generated<Event>>,
    pub resources: Arc<crate::observable::StateCell<Arc<crate::ResourceSnapshot>>>,
    pub activity: Arc<crate::observable::StateCell<crate::ActivitySnapshot>>,
    pub typing_command: watch::Sender<Option<Generated<(papo_api::Uuid, tokio::time::Instant)>>>,
    pub driver_running: AtomicBool,
    pub secret_serial: tokio::sync::Mutex<()>,
    pub auth_serial: tokio::sync::Mutex<()>,
    pub verified_rest: tokio::sync::Mutex<Option<(Generation, papo_api::RestClient)>>,
    /// Held through the revision switch and installation. A failed install requires a full reload.
    pub resource_serial: Arc<tokio::sync::Mutex<()>>,
    pub resource_observation: AtomicU64,
    pub resource_recovery: watch::Sender<Option<Generation>>,
    pub admission: crate::resource_admission::Diagnostics,
    pub operations_serial: tokio::sync::Mutex<()>,
    /// Held by `change_profile` from its read-back to its save, so two changes of
    /// one account's profile never merge over the same read-back.
    pub profile_serial: tokio::sync::Mutex<()>,
    pub history: crate::history::HistoryStore,
    pub reading: crate::reading::Reading,
    pub badges: crate::badges::Badges,
    pub outgoing: crate::outgoing::AccountQueue,
    pub reaction_commands: crate::operations::reactions::ReactionQueue,
    pub detached: crate::search::DetachedStore,
    pub notifications: crate::notifications::Notifications,
    pub expression_changes: watch::Sender<u64>,
    pub emoji_catalog: crate::emoji_catalog::Shared,
    pub calls: Arc<crate::calls::SessionCalls>,
}
impl SessionInner {
    #[cfg(test)]
    pub fn new(
        account: AccountId,
        address: ServerAddress,
        key: SecretKey,
        next: Arc<AtomicU64>,
        root: CancellationToken,
        auth_state: AuthState,
    ) -> Arc<Self> {
        Self::new_with_calls(
            account,
            address,
            key,
            next,
            root,
            auth_state,
            crate::CallConfig::default(),
            Arc::new(crate::calls::ClientCallLease::default()),
        )
    }

    pub fn new_with_calls(
        account: AccountId,
        address: ServerAddress,
        key: SecretKey,
        next: Arc<AtomicU64>,
        root: CancellationToken,
        auth_state: AuthState,
        call_config: crate::CallConfig,
        call_lease: Arc<crate::calls::ClientCallLease>,
    ) -> Arc<Self> {
        let driver_cancel = root.child_token();
        let epochs = Epochs::new(next, root.clone());
        let generation = epochs.current().generation;
        let calls = crate::calls::SessionCalls::new(account, generation, call_config, call_lease);
        Arc::new(Self {
            account,
            address,
            key,
            root,
            driver_cancel,
            epochs,
            tasks: TaskTracker::new(),
            auth: crate::observable::StateCell::new(auth_state),
            connection: crate::observable::StateCell::new(ConnectionStatus {
                generation,
                state: ConnectionState::Offline,
            }),
            round_trip: crate::observable::StateCell::new(None),
            events: broadcast::channel(64).0,
            resources: crate::observable::StateCell::new(Arc::new(crate::ResourceSnapshot::empty(
                generation,
            ))),
            activity: crate::activity::cell(generation),
            typing_command: watch::channel(None).0,
            driver_running: AtomicBool::new(false),
            secret_serial: tokio::sync::Mutex::new(()),
            auth_serial: tokio::sync::Mutex::new(()),
            verified_rest: tokio::sync::Mutex::new(None),
            resource_serial: Arc::new(tokio::sync::Mutex::new(())),
            resource_observation: AtomicU64::new(0),
            resource_recovery: watch::channel(None).0,
            admission: crate::resource_admission::Diagnostics::default(),
            operations_serial: tokio::sync::Mutex::new(()),
            profile_serial: tokio::sync::Mutex::new(()),
            history: crate::history::HistoryStore::default(),
            reading: crate::reading::Reading::new(generation),
            badges: crate::badges::Badges::new(generation),
            outgoing: crate::outgoing::AccountQueue::default(),
            reaction_commands: crate::operations::reactions::ReactionQueue::default(),
            detached: crate::search::DetachedStore::default(),
            notifications: crate::notifications::Notifications::default(),
            expression_changes: watch::channel(0).0,
            emoji_catalog: crate::emoji_catalog::Shared::default(),
            calls,
        })
    }
    pub async fn stop(&self) {
        self.calls
            .stop_and_drain(crate::CallEndReason::Shutdown)
            .await;
        self.root.cancel();
        drop(self.outgoing.files_active.write().await);
        drop(self.outgoing.files_serial.lock().await);
        self.history.close_all().await;
        self.reading.close();
        self.badges.image.close();
        self.outgoing.close_image();
        self.tasks.close();
        self.connection.send_replace(ConnectionStatus {
            generation: self.epochs.current().generation,
            state: ConnectionState::Offline,
        });
        self.auth.send_replace(AuthState::SignedOut);
        self.notifications
            .status
            .send_replace(crate::NotificationStatus::Closed);
        self.auth.close();
        self.connection.close();
        self.round_trip.send_replace(None);
        self.round_trip.close();
        crate::activity::clear(self, self.epochs.current().generation);
        self.activity.close();
        self.resources.close();
        self.tasks.wait().await;
        self.outgoing.close_active_upload_files();
    }
    pub fn publish_resources(&self, snapshot: crate::ResourceSnapshot) -> Result<(), ClientError> {
        self.epochs.apply(snapshot.generation, || {
            self.publish_current_resources(snapshot)
        })?
    }

    /// Caller holds the epoch fence, including when publishing a rejection under R1's lock.
    pub(crate) fn publish_current_resources(
        &self,
        mut snapshot: crate::ResourceSnapshot,
    ) -> Result<(), ClientError> {
        snapshot.version = self
            .resources
            .borrow()
            .version
            .checked_add(1)
            .ok_or(ClientError::Closed)?;
        let published = Arc::new(snapshot);
        let runtime = tokio::runtime::Handle::try_current().ok();
        // A resource watcher may join a newly admitted voice channel immediately.
        self.calls
            .on_resources_updated(&published, runtime.as_ref());
        self.resources.send_replace(published);
        Ok(())
    }
    pub fn status(
        &self,
        generation: Generation,
        state: ConnectionState,
    ) -> Result<(), ClientError> {
        self.epochs.apply(generation, || {
            if state != ConnectionState::Online {
                crate::activity::clear(self, generation);
            }
            let resources = self.resources.borrow().clone();
            let runtime = tokio::runtime::Handle::try_current().ok();
            self.calls
                .on_connection_status(generation, state, &resources, runtime.as_ref());
            self.connection
                .send_replace(ConnectionStatus { generation, state })
        })?;
        Ok(())
    }
    async fn acquire_secret_slot(
        &self,
        cancel: &CancellationToken,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, SecretError> {
        tokio::select! {
            biased;
            () = self.root.cancelled() => Err(SecretError::Failure(SecretFailure::Canceled)),
            () = cancel.cancelled() => Err(SecretError::Failure(SecretFailure::Canceled)),
            guard = self.secret_serial.lock() => Ok(guard),
        }
    }
    pub async fn read_secret(
        &self,
        secrets: &SecretStoreBoundary,
        cancel: &CancellationToken,
    ) -> Result<Option<SecretValue>, SecretError> {
        let _guard = self.acquire_secret_slot(cancel).await?;
        secrets.read(self.key, cancel).await
    }
    pub async fn write_secret(
        &self,
        secrets: &SecretStoreBoundary,
        value: SecretValue,
        cancel: &CancellationToken,
    ) -> Result<(), SecretError> {
        let _guard = self.acquire_secret_slot(cancel).await?;
        secrets.write(self.key, value, cancel).await
    }
    pub async fn delete_secret(
        &self,
        secrets: &SecretStoreBoundary,
        cancel: &CancellationToken,
    ) -> Result<(), SecretError> {
        let _guard = self.acquire_secret_slot(cancel).await?;
        secrets.delete(self.key, cancel).await
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
    /// Subscribes to the gateway's latest heartbeat round trip, stamped with the
    /// generation whose socket measured it: a client shows it only while that
    /// generation is `Online`. Measured every 25 seconds; none before the first
    /// acknowledgement of this session.
    #[must_use]
    pub fn subscribe_round_trip(&self) -> watch::Receiver<Option<crate::RoundTrip>> {
        self.inner.round_trip.subscribe()
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
    /// Returns this session's resource-admission counts: rejected replacements, which
    /// kept their last admitted value, and display-only rows dropped from admitted ones.
    #[must_use]
    pub fn resource_admission(&self) -> crate::AdmissionDiagnostics {
        self.inner.admission.snapshot()
    }
    /// Requests a new connection generation, canceling old socket and REST work.
    /// # Errors
    /// Returns `SignedOut` without a usable account or `Closed` if the session was revoked.
    pub fn reconnect(&self) -> Result<(), ClientError> {
        if !matches!(
            *self.inner.auth.borrow(),
            AuthState::Ready | AuthState::NeedsServer
        ) {
            return Err(ClientError::SignedOut);
        }
        self.inner.epochs.replace(None, |generation| {
            crate::activity::clear(&self.inner, generation);
            let resources = self.inner.resources.borrow().clone();
            self.inner.calls.on_connection_status(
                generation,
                ConnectionState::Connecting,
                &resources,
                Some(&self.client.inner.runtime),
            );
        })?;
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
