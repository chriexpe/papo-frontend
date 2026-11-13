//! Profile storage ownership and external-runtime lifecycle; Client and sessions live above it.

use crate::removal::{RemovalHooks, RemovalJobs};
use crate::secret::SecretStoreBoundary;
use crate::storage_worker::{ReadPool, StorageWorker, WorkerConfig, WorkerError};
use crate::{DataRoot, Registry, StorageError};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

/// Storage limits shared across the profile, never multiplied by account count.
#[derive(Clone, Debug)]
pub struct StorageConfig {
    /// Bounded command queues and shared-writer turn/idle limits.
    pub writer: WorkerConfig,
    /// Maximum simultaneous read connections; default four.
    pub read_connections: usize,
    /// Maximum time to drain accepted writes; default three seconds.
    pub drain_timeout: Duration,
    /// Timeout supplied to Tokio shutdown after drain; default three seconds.
    pub runtime_stop_timeout: Duration,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            writer: WorkerConfig::default(),
            read_connections: 4,
            drain_timeout: Duration::from_secs(3),
            runtime_stop_timeout: Duration::from_secs(3),
        }
    }
}

/// Storage owned by `Client::open`. This owns no runtime.
/// Open and shutdown run on an external thread; every async mutation runs on the
/// supplied runtime. A successful open has already attempted all pending removals.
pub struct Storage {
    root: DataRoot,
    registry: Registry,
    removals: RemovalJobs,
    state: StorageWorker,
    shared: StorageWorker,
    cancel: CancellationToken,
    config: StorageConfig,
}

impl Storage {
    /// Locks the profile, migrates on its state writer, and resumes removal jobs.
    /// Persistent external cleanup failures remain available from `removals().pending()`;
    /// entries stay hidden. No session or cache is opened here.
    ///
    /// # Errors
    /// Returns `RuntimeContext`, `ProfileInUse`, `NewerSchema`, migration, thread,
    /// or journal failures. A newer schema is refused before persistent pragmas.
    pub fn open(
        root: DataRoot,
        runtime: &Runtime,
        secrets: Arc<SecretStoreBoundary>,
        hooks: Arc<dyn RemovalHooks>,
        config: StorageConfig,
    ) -> Result<Self, StorageError> {
        Self::open_with_removals(root, runtime, secrets, hooks, config, true)
    }

    /// Client installs cache/media cleanup capabilities before its single startup removal attempt.
    pub(crate) fn open_deferred(
        root: DataRoot,
        runtime: &Runtime,
        secrets: Arc<SecretStoreBoundary>,
        hooks: Arc<dyn RemovalHooks>,
        config: StorageConfig,
    ) -> Result<Self, StorageError> {
        Self::open_with_removals(root, runtime, secrets, hooks, config, false)
    }

    fn open_with_removals(
        root: DataRoot,
        runtime: &Runtime,
        secrets: Arc<SecretStoreBoundary>,
        hooks: Arc<dyn RemovalHooks>,
        config: StorageConfig,
        resume: bool,
    ) -> Result<Self, StorageError> {
        outside_runtime()?;
        let lease = root.acquire()?;
        crate::private_fs::file(&root.state())?;
        let thread_lease = Arc::clone(&lease);
        let (state, writer) = StorageWorker::state(
            root.state(),
            move |connection| {
                let _lease = &thread_lease;
                crate::state_schema::migrate(connection).map_err(|error| match error {
                    StorageError::NewerSchema { found } => WorkerError::NewerSchema {
                        found: u32::try_from(found).unwrap_or(u32::MAX),
                        supported: u32::try_from(crate::state_schema::VERSION).unwrap_or(u32::MAX),
                    },
                    StorageError::Sqlite(error) => WorkerError::from(error),
                    _ => WorkerError::Initialization,
                })
            },
            config.writer.clone(),
        )
        .map_err(|error| match error {
            WorkerError::NewerSchema { found, .. } => StorageError::NewerSchema {
                found: i64::from(found),
            },
            error => error.into(),
        })?;
        let shared = StorageWorker::shared(config.writer.clone())?;
        let readers = Arc::new(ReadPool::with_profile_lease(
            config.read_connections,
            Arc::clone(&lease),
        )?);
        let registry = Registry {
            writer,
            readers,
            path: root.state(),
            lease,
        };
        let cancel = CancellationToken::new();
        let removals = RemovalJobs {
            registry: registry.clone(),
            root: root.clone(),
            secrets,
            hooks,
            cancel: cancel.child_token(),
            serial: Arc::new(tokio::sync::Mutex::new(())),
        };
        let storage = Self {
            root,
            registry,
            removals,
            state,
            shared,
            cancel,
            config,
        };
        if resume {
            runtime.block_on(storage.removals.resume())?;
        }
        Ok(storage)
    }

    /// Returns non-secret durable registry access; it does not authenticate accounts.
    #[must_use]
    pub fn registry(&self) -> Registry {
        self.registry.clone()
    }

    /// Returns the recoverable removal runner and its persistent failure journal.
    #[must_use]
    pub fn removals(&self) -> RemovalJobs {
        self.removals.clone()
    }

    /// Returns the client-selected data layout.
    #[must_use]
    pub fn root(&self) -> &DataRoot {
        &self.root
    }

    /// Lazily registers a rebuildable database after its owner supplies schema/identity checks.
    /// The worker retains the profile lock until the connection and accepted writes close.
    ///
    /// # Errors
    /// Returns closed admission or duplicate-path errors from the shared writer.
    pub fn register_rebuildable(
        &self,
        path: std::path::PathBuf,
        initializer: impl Fn(&mut rusqlite::Connection) -> Result<(), WorkerError>
        + Send
        + Sync
        + 'static,
    ) -> Result<crate::storage_worker::DatabaseWriter, StorageError> {
        let lease = Arc::clone(&self.registry.lease);
        Ok(self.shared.register(path, move |connection| {
            let _lease = &lease;
            initializer(connection)
        })?)
    }

    /// Return a lazy account-cache factory sharing the existing NORMAL writer and read pool.
    /// Construct this once per Client and clone it; no account database is opened here.
    #[must_use]
    pub fn account_caches(&self) -> crate::account_cache::AccountCaches {
        let register = self.shared.registrar();
        let lease = Arc::clone(&self.registry.lease);
        let registrar = Arc::new(
            move |path, initializer: crate::storage_worker::Initializer| {
                let lease = Arc::clone(&lease);
                register(
                    path,
                    Arc::new(move |connection| {
                        let _lease = &lease;
                        initializer(connection)
                    }),
                )
            },
        );
        crate::account_cache::AccountCaches::new(self.root.clone(), registrar, self.readers())
    }

    /// Read pool shared across account databases; each batch releases its transaction.
    #[must_use]
    pub fn readers(&self) -> Arc<ReadPool> {
        Arc::clone(&self.registry.readers)
    }

    /// Cancellation root for storage-owned credential calls and removal jobs.
    #[must_use]
    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.child_token()
    }

    /// Cancel admission/tasks, drain accepted writes, stop Tokio, then join storage.
    /// The runtime remains in the caller's Option when called from inside Tokio, so
    /// rejecting an invalid call never drops a runtime on one of its own workers.
    /// Reports every accepted write that missed the drain deadline, never as success.
    ///
    /// # Errors
    /// Returns `RuntimeContext` before changing anything, or a worker lifecycle failure.
    pub fn shutdown(
        &mut self,
        runtime: &mut Option<Runtime>,
    ) -> Result<StorageShutdown, StorageError> {
        outside_runtime()?;
        self.cancel.cancel();
        let deadline = Instant::now() + self.config.drain_timeout;
        self.state.begin_shutdown(deadline);
        self.shared.begin_shutdown(deadline);
        self.registry.readers.stop_admission();
        let reads_drained = self.registry.readers.drain(deadline)?;
        let state = self.state.shutdown(deadline)?;
        let shared = self.shared.shutdown(deadline)?;
        if let Some(runtime) = runtime.take() {
            runtime.shutdown_timeout(self.config.runtime_stop_timeout);
        }
        self.state.join()?;
        self.shared.join()?;
        Ok(StorageShutdown {
            reads_drained,
            state,
            shared,
        })
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.state.stop_admission();
        self.shared.stop_admission();
        self.registry.readers.stop_admission();
    }
}

/// Drain outcomes for both fixed writer threads, including unacknowledged work.
#[derive(Debug)]
pub struct StorageShutdown {
    /// False if a blocking read remained active at the drain deadline.
    pub reads_drained: bool,
    /// Durable state writer drain report.
    pub state: crate::storage_worker::ShutdownReport,
    /// Shared rebuildable writer drain report.
    pub shared: crate::storage_worker::ShutdownReport,
}

fn outside_runtime() -> Result<(), StorageError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        Err(StorageError::RuntimeContext)
    } else {
        Ok(())
    }
}
