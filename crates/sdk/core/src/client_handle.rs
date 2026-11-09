//! Asynchronous client entry points and committed registry notifications; UI projections live above them.

use crate::{
    AccountId, AuthService, ClientError, SavedAccount, SavedServer, ServerAddress, ServerId,
    Session, UnixMicros, client::ClientInner, removal::RemovalJob,
};
use std::{future::Future, sync::Arc};
use tokio::sync::watch;

/// A coherent non-secret durable registry image, not a view-shaped UI list.
#[derive(Clone, Default)]
pub struct RegistrySnapshot {
    /// Monotonically increasing publication version within this Client.
    pub version: u64,
    /// Visible servers in durable order.
    pub servers: Vec<SavedServer>,
    /// Visible account metadata, grouped in server order.
    pub accounts: Vec<SavedAccount>,
    /// Hidden removals and their typed retry failures.
    pub removals: Vec<RemovalJob>,
}

/// Cloneable service access, without ownership of runtime shutdown.
#[derive(Clone)]
pub struct ClientHandle {
    pub(crate) inner: Arc<ClientInner>,
}
impl ClientHandle {
    /// True after work admission closes.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.pool.root.is_cancelled()
    }
    /// Atomic baseline plus a bounded/coalescing committed registry notification stream.
    #[must_use]
    pub fn subscribe_registry(&self) -> watch::Receiver<Arc<RegistrySnapshot>> {
        self.inner.registry_rx.clone()
    }
    pub(crate) async fn dispatch<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, ClientError>> + Send + 'static,
    ) -> Result<T, ClientError> {
        let token = self.inner.tasks.token();
        if self.is_closed() {
            return Err(ClientError::Closed);
        }
        let cancel = self.inner.pool.root.clone();
        self.inner.runtime.spawn(async move {
            let _token = token;
            tokio::select! { biased; () = cancel.cancelled() => Err(ClientError::Closed), result = work => result }
        }).await.map_err(|_| ClientError::Executor)?
    }
    /// Validates and probes `/server`, then acknowledges only after a FULL registry commit.
    /// # Errors
    /// Returns validation, probe, duplicate/removal identity, or storage errors.
    pub async fn add_server(
        &self,
        address: &str,
        added_at: UnixMicros,
    ) -> Result<ServerId, ClientError> {
        let address = ServerAddress::parse(address)?;
        let this = self.clone();
        self.dispatch(async move {
            let rest = this.inner.transport(&address, None)?;
            this.inner.request(rest.server()).await?;
            let _guard = this.inner.mutation.lock().await;
            let id = address.id();
            this.inner.registry.add_server(address, added_at).await?;
            this.inner.publish_registry().await?;
            Ok(id)
        })
        .await
    }
    /// Reorders exactly the visible server set in one FULL commit.
    /// # Errors
    /// Returns invalid-order, closed, or storage errors.
    pub async fn reorder_servers(&self, order: Vec<ServerId>) -> Result<(), ClientError> {
        let this = self.clone();
        self.dispatch(async move {
            let _guard = this.inner.mutation.lock().await;
            this.inner.registry.reorder_servers(order).await?;
            this.inner.publish_registry().await
        })
        .await
    }
    /// Hides a server atomically and runs its recoverable removal jobs.
    /// # Errors
    /// Durable failures are returned; external cleanup failures are published in the registry.
    pub async fn remove_server(&self, id: ServerId) -> Result<(), ClientError> {
        let this = self.clone();
        self.dispatch(async move {
            let _guard = this.inner.mutation.lock().await;
            this.inner.removals.begin_server(id).await?;
            // Publishing waits for lookups holding the old snapshot to finish registration.
            this.inner.publish_registry().await?;
            this.inner.pool.retire_server(id);
            this.inner.removals.resume().await?;
            this.inner.publish_registry().await
        })
        .await
    }
    /// Hides an account, revokes handles and runs recoverable cleanup.
    /// # Errors
    /// Returns closed, missing identity, or durable failures; cleanup failures remain observable.
    pub async fn remove_account(&self, id: AccountId) -> Result<(), ClientError> {
        let this = self.clone();
        self.dispatch(async move {
            let _guard = this.inner.mutation.lock().await;
            this.inner.removals.begin_account(id).await?;
            this.inner.publish_registry().await?;
            this.inner.removals.resume().await?;
            this.inner.publish_registry().await
        })
        .await
    }
    /// Retries pending idempotent removals once and publishes remaining typed failures.
    /// # Errors
    /// Returns closed or durable journal failures.
    pub async fn retry_removals(&self) -> Result<(), ClientError> {
        let this = self.clone();
        self.dispatch(async move {
            let _guard = this.inner.mutation.lock().await;
            this.inner.removals.resume().await?;
            this.inner.publish_registry().await
        })
        .await
    }
    /// Selects this server's active account without merging account sessions or caches.
    /// # Errors
    /// Returns invalid identity, closed, or durable commit failures.
    pub async fn set_active_account(
        &self,
        server: ServerId,
        account: Option<AccountId>,
    ) -> Result<(), ClientError> {
        let this = self.clone();
        self.dispatch(async move {
            let _guard = this.inner.mutation.lock().await;
            this.inner
                .registry
                .set_active_account(server, account)
                .await?;
            this.inner.publish_registry().await
        })
        .await
    }
    /// Starts a server-scoped sign-in attempt. Successful login returns an account-scoped Session.
    /// # Errors
    /// Returns `NotFound` for an unsaved/hidden server and `Closed` after shutdown.
    pub fn auth(&self, server: ServerId) -> Result<AuthService, ClientError> {
        if self.is_closed() {
            return Err(ClientError::Closed);
        }
        let snapshot = self.inner.registry_rx.borrow();
        let saved = snapshot
            .servers
            .iter()
            .find(|saved| saved.address.id() == server)
            .ok_or(ClientError::NotFound)?;
        Ok(AuthService::for_server(self.clone(), saved.address.clone()))
    }
    /// Gets a restored or newly authenticated account session, never another account's handle.
    /// # Errors
    /// Returns missing/revoked account or closed client errors.
    pub fn session(&self, account: AccountId) -> Result<Session, ClientError> {
        if self.is_closed() {
            return Err(ClientError::Closed);
        }
        let inner = self
            .inner
            .pool
            .get(account)
            .filter(|session| !session.root.is_cancelled())
            .ok_or(ClientError::NotFound)?;
        Ok(Session {
            inner,
            client: self.clone(),
        })
    }
    pub(crate) async fn restore_accounts(&self) -> Result<(), ClientError> {
        let snapshot = Arc::clone(&self.inner.registry_rx.borrow());
        for account in &snapshot.accounts {
            let Some(server) = snapshot
                .servers
                .iter()
                .find(|server| server.address.id() == account.id.server())
            else {
                return Err(ClientError::NotFound);
            };
            crate::auth::restore(self, server.address.clone(), account.clone()).await?;
        }
        Ok(())
    }
}
impl ClientInner {
    pub async fn publish_registry(&self) -> Result<(), ClientError> {
        let servers = self.registry.servers().await?;
        let mut accounts = Vec::new();
        for server in &servers {
            accounts.extend(self.registry.accounts(server.address.id()).await?);
        }
        let removals = self.removals.pending().await?;
        let sender = self
            .registry_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sender = sender.as_ref().ok_or(ClientError::Closed)?;
        let version = sender
            .borrow()
            .version
            .checked_add(1)
            .ok_or(ClientError::Closed)?;
        sender.send_replace(Arc::new(RegistrySnapshot {
            version,
            servers,
            accounts,
            removals,
        }));
        Ok(())
    }
    pub fn transport(
        &self,
        address: &ServerAddress,
        credentials: Option<&[u8]>,
    ) -> Result<papo_api::RestClient, ClientError> {
        let snapshot = credentials
            .map(papo_api::CredentialSnapshot::import)
            .transpose()?;
        Ok(match (self.config.insecure_loopback, snapshot.as_ref()) {
            (true, Some(snapshot)) => {
                papo_api::RestClient::loopback_from_snapshot(address.as_str(), snapshot)?
            }
            (true, None) => papo_api::RestClient::loopback(address.as_str())?,
            (false, Some(snapshot)) => {
                papo_api::RestClient::from_snapshot(address.as_str(), snapshot)?
            }
            (false, None) => papo_api::RestClient::new(address.as_str())?,
        })
    }
    pub async fn request<T>(
        &self,
        request: impl Future<Output = Result<T, papo_api::Error>>,
    ) -> Result<T, ClientError> {
        Ok(tokio::time::timeout(self.config.request_timeout, request)
            .await
            .map_err(|_| ClientError::Timeout)??)
    }
}
