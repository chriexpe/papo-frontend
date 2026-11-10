//! One runtime and profile owner; durable locking, recovery and shutdown remain Storage's responsibility.

use crate::{
    Backoff, ClientError, ClientHandle, DataRoot, Jitter, Registry, RegistrySnapshot,
    RuntimeJitter, Storage, StorageConfig, StorageShutdown,
    removal::RemovalJobs,
    secret::{SecretStore, SecretStoreBoundary},
    session_pool::SessionPool,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    runtime::{Builder, Runtime},
    sync::watch,
};
use tokio_util::task::TaskTracker;

/// Limits for the one runtime and every account's bounded asynchronous calls.
#[derive(Clone)]
pub struct ClientConfig {
    /// Worker count shared by every session; default two.
    pub worker_threads: usize,
    /// Deadline per credential host call; default thirty seconds.
    pub credential_timeout: Duration,
    /// Deadline per REST request or gateway handshake; default thirty seconds.
    pub request_timeout: Duration,
    /// Durable/shared storage configuration.
    pub storage: StorageConfig,
    /// Capped retry delay configuration.
    pub backoff: Backoff,
    /// Injectable jitter; tests use deterministic implementations.
    pub jitter: Arc<dyn Jitter>,
    /// Permit Secure cookies over literal HTTP loopback only, for integration proofs.
    pub insecure_loopback: bool,
}
impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            worker_threads: 2,
            credential_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(30),
            storage: StorageConfig::default(),
            backoff: Backoff::default(),
            jitter: Arc::new(RuntimeJitter::default()),
            insecure_loopback: false,
        }
    }
}

pub(crate) struct ClientInner {
    pub runtime: tokio::runtime::Handle,
    pub registry: Registry,
    pub caches: crate::AccountCaches,
    pub removals: RemovalJobs,
    pub secrets: Arc<SecretStoreBoundary>,
    pub pool: Arc<SessionPool>,
    pub tasks: TaskTracker,
    pub mutation: tokio::sync::Mutex<()>,
    pub registry_tx: Mutex<Option<watch::Sender<Arc<RegistrySnapshot>>>>,
    pub registry_rx: watch::Receiver<Arc<RegistrySnapshot>>,
    pub config: ClientConfig,
}

/// External-thread owner of one Tokio runtime and session 4's storage owner.
/// Call `shutdown` explicitly to receive the persistence report and join every thread.
/// Drop outside Tokio also joins; Drop inside Tokio hands cleanup to an external reaper.
pub struct Client {
    pub(crate) handle: ClientHandle,
    storage: Option<Storage>,
    runtime: Option<Runtime>,
}
impl Client {
    /// Locks the profile and resumes removals before restoring any account session.
    /// Never reads legacy-client files. Credentials are supplied only by the host capability.
    /// # Errors
    /// Returns `RuntimeContext` before creating a runtime, `ProfileInUse` for a second
    /// owner, configuration errors, or typed durable storage failures.
    pub fn open(
        root: DataRoot,
        store: Arc<dyn SecretStore>,
        config: ClientConfig,
    ) -> Result<Self, ClientError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(ClientError::RuntimeContext);
        }
        config.backoff.validate()?;
        if config.worker_threads == 0
            || config.credential_timeout.is_zero()
            || config.request_timeout.is_zero()
        {
            return Err(ClientError::Configuration);
        }
        let runtime = Builder::new_multi_thread()
            .worker_threads(config.worker_threads)
            .enable_all()
            .thread_name("papo-sdk")
            .build()
            .map_err(|_| ClientError::Executor)?;
        let pool = SessionPool::new();
        let secrets = Arc::new(SecretStoreBoundary::new(store, config.credential_timeout));
        let storage = Storage::open(
            root,
            &runtime,
            Arc::clone(&secrets),
            pool.clone(),
            config.storage.clone(),
        )?;
        let caches = storage.account_caches();
        pool.set_caches(caches.clone());
        let (tx, rx) = watch::channel(Arc::new(RegistrySnapshot::default()));
        let handle = ClientHandle {
            inner: Arc::new(ClientInner {
                runtime: runtime.handle().clone(),
                registry: storage.registry(),
                caches,
                removals: storage.removals(),
                secrets,
                pool,
                tasks: TaskTracker::new(),
                mutation: tokio::sync::Mutex::new(()),
                registry_tx: Mutex::new(Some(tx)),
                registry_rx: rx,
                config,
            }),
        };
        let mut client = Self {
            handle,
            storage: Some(storage),
            runtime: Some(runtime),
        };
        let startup = client
            .runtime
            .as_ref()
            .ok_or(ClientError::Closed)?
            .block_on(async {
                client.handle.inner.publish_registry().await?;
                client.handle.restore_accounts().await
            });
        if let Err(error) = startup {
            let _ = client.shutdown();
            return Err(error);
        }
        Ok(client)
    }
    /// Returns async services; they schedule work only on this client's runtime.
    #[must_use]
    pub fn handle(&self) -> ClientHandle {
        self.handle.clone()
    }
    /// Drives an async service/example from an external thread on the one owned runtime.
    /// # Errors
    /// Returns `RuntimeContext` inside Tokio or `Closed` after shutdown.
    pub fn block_on<F: std::future::Future>(&self, future: F) -> Result<F::Output, ClientError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(ClientError::RuntimeContext);
        }
        Ok(self
            .runtime
            .as_ref()
            .ok_or(ClientError::Closed)?
            .block_on(future))
    }
    /// Cancels admission and session work, then delegates drain/runtime stop/thread join to Storage.
    /// # Errors
    /// Rejects calls inside any Tokio runtime without changing ownership; otherwise
    /// returns storage lifecycle failures. Accepted-but-unpersisted writes are in the report.
    pub fn shutdown(&mut self) -> Result<StorageShutdown, ClientError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(ClientError::RuntimeContext);
        }
        self.handle.inner.pool.root.cancel();
        self.handle.inner.tasks.close();
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.block_on(async {
                self.handle.inner.pool.stop_all().await;
                self.handle.inner.tasks.wait().await;
            });
        }
        self.handle
            .inner
            .registry_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(self
            .storage
            .as_mut()
            .ok_or(ClientError::Closed)?
            .shutdown(&mut self.runtime)?)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        if self.runtime.is_none() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            // A tracked task may own Client: joining here would wait for that same task.
            self.handle.inner.pool.root.cancel();
            self.handle.inner.tasks.close();
            self.handle
                .inner
                .registry_tx
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let exit = Arc::new(Mutex::new(Some((self.runtime.take(), self.storage.take()))));
            let pending = Arc::clone(&exit);
            let handle = self.handle.clone();
            let reaper = std::thread::Builder::new()
                .name("papo-exit".into())
                .spawn(move || {
                    if let Some((mut runtime, mut storage)) = pending
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                    {
                        if let Some(runtime) = runtime.as_ref() {
                            runtime.block_on(async {
                                handle.inner.pool.stop_all().await;
                                handle.inner.tasks.wait().await;
                            });
                        }
                        if let Some(storage) = storage.as_mut() {
                            let _ = storage.shutdown(&mut runtime);
                        }
                    }
                });
            if reaper.is_err()
                && let Some((runtime, _storage)) = exit
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
            {
                // Resource exhaustion cannot justify dropping Runtime on its own worker.
                if let Some(runtime) = runtime {
                    runtime.shutdown_background();
                }
            }
        } else {
            let _ = self.shutdown();
        }
    }
}
