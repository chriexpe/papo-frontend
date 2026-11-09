//! Bounded SQLite writers and read execution; schema, identity and runtime ownership
//! belong to their callers. Only FULL state commits constitute durable acknowledgements.

mod admission;
mod connection;
mod execution;
mod read_pool;
#[cfg(test)]
mod recovery_tests;
mod scheduler;
mod shutdown;
mod startup;
#[cfg(test)]
mod tests;

use std::{path::PathBuf, sync::Arc, thread::JoinHandle, time::Duration};

use rusqlite::Connection;

pub use admission::{CommitReceipt, DatabaseWriter};
pub use execution::{CommitInfo, SqlBatch, SqlMutation};
pub use read_pool::ReadPool;
pub use shutdown::{DatabaseFailure, ShutdownReport, UnpersistedWrite, WriteDisposition};

/// Sanitized storage-boundary failures; never contains SQL, parameter data or paths.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WorkerError {
    /// Work was not admitted because shutdown has started.
    #[error("storage admission is closed")]
    AdmissionClosed,
    /// This database was closed after an earlier failure.
    #[error("database writer is closed")]
    DatabaseClosed,
    /// A SQLite failure, retaining its extended numeric code only.
    #[error("sqlite storage failure (code {code:?})")]
    Sqlite {
        /// SQLite extended result code when supplied by the engine.
        code: Option<i32>,
    },
    /// A caller-owned initializer failed without a more specific category.
    #[error("database initialization failed")]
    Initialization,
    /// The caller's version check refused a newer file.
    #[error("database schema is newer than supported")]
    NewerSchema {
        /// Schema version found in the file.
        found: u32,
        /// Highest schema version understood by the caller.
        supported: u32,
    },
    /// SQLite's integrity check did not succeed.
    #[error("database integrity check failed")]
    IntegrityCheckFailed,
    /// A transactional batch exceeded 500 affected rows.
    #[error("transaction row budget exceeded")]
    RowLimit,
    /// SQL execution exceeded the per-turn time budget.
    #[error("transaction execution budget exceeded")]
    TurnDeadline,
    /// An admitted write did not finish before shutdown's deadline.
    #[error("write persistence was not acknowledged before shutdown deadline")]
    ShutdownDeadline,
    /// A trusted state transaction returned its own typed error.
    #[error("state transaction rejected")]
    TransactionRejected,
    /// A worker callback panicked; its database is closed.
    #[error("storage callback panicked")]
    CallbackPanicked,
    /// A worker ended without returning a commit result.
    #[error("storage worker stopped unexpectedly")]
    WorkerStopped,
    /// Blocking shutdown/join was called from a Tokio runtime context.
    #[error("blocking storage lifecycle call inside runtime")]
    RuntimeContext,
    /// Invalid capacity or time configuration.
    #[error("invalid storage worker configuration")]
    InvalidConfiguration,
    /// A database path is already registered on this worker.
    #[error("database is already registered")]
    AlreadyRegistered,
    /// A filesystem or thread creation operation failed.
    #[error("storage system operation failed")]
    System,
}

impl From<rusqlite::Error> for WorkerError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite {
            code: error.sqlite_error().map(|error| error.extended_code),
        }
    }
}

/// Shared-writer turn limits; trusted atomic state callbacks are exempt.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// Queued commands per database, excluding its currently executing command.
    pub queue_capacity: usize,
    /// Close shared connections after this idle duration; default ten minutes.
    pub idle_timeout: Duration,
    /// SQL execution budget, at most 20 ms; fsync/OS calls cannot be preempted.
    pub turn_budget: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 256,
            idle_timeout: Duration::from_secs(600),
            turn_budget: Duration::from_millis(20),
        }
    }
}

/// Caller-owned schema/identity hook, executed only on the writer thread.
pub type Initializer = Arc<dyn Fn(&mut Connection) -> Result<(), WorkerError> + Send + Sync>;

/// One dedicated writer thread. Keep one state worker and one shared worker per profile.
/// Explicitly shut down and join outside the runtime before releasing the profile lock.
pub struct StorageWorker {
    shared: Arc<scheduler::Shared>,
    thread: Option<JoinHandle<()>>,
}

impl StorageWorker {
    /// Open the FULL state database on its dedicated thread and run its migration hook.
    ///
    /// # Errors
    /// Returns initialization, SQLite, configuration or thread creation failures.
    /// Must be called outside a Tokio runtime, because opening waits for the thread.
    pub fn state(
        path: PathBuf,
        initializer: impl Fn(&mut Connection) -> Result<(), WorkerError> + Send + Sync + 'static,
        config: WorkerConfig,
    ) -> Result<(Self, DatabaseWriter), WorkerError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(WorkerError::RuntimeContext);
        }
        startup::start(config, Some((path, Arc::new(initializer)))).and_then(|(worker, writer)| {
            writer
                .map(|writer| (worker, writer))
                .ok_or(WorkerError::WorkerStopped)
        })
    }

    /// Start the NORMAL shared writer without opening any database.
    ///
    /// # Errors
    /// Returns invalid configuration or thread creation failures.
    pub fn shared(config: WorkerConfig) -> Result<Self, WorkerError> {
        startup::start(config, None).map(|(worker, _)| worker)
    }

    /// Register a rebuildable database without opening it or inventing a schema.
    /// The hook must verify account identity before accessing account-owned rows.
    /// Existing corrupt files and hooks returning `NewerSchema` are rebuilt once.
    ///
    /// # Errors
    /// Returns closed admission, duplicate registration, or wrong-worker errors.
    pub fn register(
        &self,
        path: PathBuf,
        initializer: impl Fn(&mut Connection) -> Result<(), WorkerError> + Send + Sync + 'static,
    ) -> Result<DatabaseWriter, WorkerError> {
        self.shared.register(path, Arc::new(initializer), false)
    }

    /// Atomically stop admission and install the final acknowledgement deadline without
    /// waiting. Call this on both writers before waiting for reads or either writer.
    /// Safe in a runtime context: no SQLite or thread joining occurs here.
    pub fn begin_shutdown(&self, deadline: std::time::Instant) {
        self.shared.request_stop(deadline);
    }

    /// Stop accepting commands immediately; already-admitted commands remain queued.
    pub fn stop_admission(&self) {
        self.shared.stop_admission();
    }
}

impl Drop for StorageWorker {
    fn drop(&mut self) {
        self.shared.request_stop(std::time::Instant::now());
    }
}
