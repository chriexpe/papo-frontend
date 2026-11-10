//! Async admission and typed commit receipts; SQLite execution belongs to the worker.

use std::sync::Arc;

use rusqlite::Transaction;
use tokio::sync::{Semaphore, oneshot};

use super::{
    WorkerError,
    execution::{CommitInfo, QueuedJob, SqlBatch, StateJob},
    scheduler::Shared,
};

/// Cloneable admission handle for one database; cloning does not create a thread.
#[derive(Clone)]
pub struct DatabaseWriter {
    pub(super) shared: Arc<Shared>,
    pub(super) database_id: u64,
    pub(super) capacity: Arc<Semaphore>,
    pub(super) state: bool,
}

/// An accepted write. Dropping this receipt never cancels the admitted transaction.
/// A successful result is sent only after SQLite's commit returns successfully.
pub struct CommitReceipt<T> {
    /// Worker-local monotonically increasing write identifier.
    pub command_id: u64,
    receiver: oneshot::Receiver<Result<T, WorkerError>>,
}

impl<T> CommitReceipt<T> {
    /// Wait for the actual commit outcome, never just queue acceptance.
    ///
    /// # Errors
    /// Returns the write error or unexpected worker termination.
    pub async fn committed(self) -> Result<T, WorkerError> {
        self.receiver
            .await
            .map_err(|_| WorkerError::WorkerStopped)?
    }
}

impl DatabaseWriter {
    /// Worker-local database identifier used in shutdown and failure reports.
    #[must_use]
    pub fn database_id(&self) -> u64 {
        self.database_id
    }

    /// Admit one bounded atomic batch, awaiting queue capacity without dropping work.
    /// Cancellation before this method returns means the command was not accepted.
    ///
    /// # Errors
    /// Returns closed admission/database errors. SQLite errors arrive on the receipt.
    pub async fn submit(&self, batch: SqlBatch) -> Result<CommitReceipt<CommitInfo>, WorkerError> {
        let (sender, receiver) = oneshot::channel();
        let command_id = self
            .admit(QueuedJob::Write(Box::new(super::execution::BatchJob {
                batch,
                sender: Some(sender),
                rows: 0,
            })))
            .await?;
        Ok(CommitReceipt {
            command_id,
            receiver,
        })
    }

    /// Admit and await one atomic batch's explicit commit result.
    ///
    /// # Errors
    /// Returns admission, SQLite, turn-budget or shutdown failures.
    pub async fn commit(&self, batch: SqlBatch) -> Result<CommitInfo, WorkerError> {
        self.submit(batch).await?.committed().await
    }

    /// Run a trusted, short multi-table state transaction, returning only after FULL commit.
    /// This escape hatch is state-only. Callbacks must not block or issue transaction,
    /// schema or PRAGMA statements. Shared-writer row/time quotas do not split state transactions;
    /// arbitrary Rust callback code cannot be preempted. A dropped future does not cancel
    /// a transaction that has already been admitted.
    ///
    /// # Errors
    /// Preserves callback errors; boundary failures convert through `E::from`.
    pub(crate) async fn write<T, E, F>(&self, callback: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: std::error::Error + From<WorkerError> + Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, E> + Send + 'static,
    {
        if !self.state {
            return Err(E::from(WorkerError::InvalidConfiguration));
        }
        let (sender, receiver) = oneshot::channel();
        self.admit(QueuedJob::Write(Box::new(StateJob {
            callback: Some(callback),
            output: None,
            sender: Some(sender),
        })))
        .await
        .map_err(E::from)?;
        receiver
            .await
            .map_err(|_| E::from(WorkerError::WorkerStopped))?
    }

    /// Run a trusted cache transaction on the shared writer. The existing execution
    /// boundary rolls back callbacks exceeding 500 changed rows or the turn deadline.
    /// Callbacks must not block or issue transaction, schema or PRAGMA statements.
    pub(crate) async fn cache_write<T, E, F>(&self, callback: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: std::error::Error + From<WorkerError> + Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, E> + Send + 'static,
    {
        if self.state {
            return Err(E::from(WorkerError::InvalidConfiguration));
        }
        let (sender, receiver) = oneshot::channel();
        self.admit(QueuedJob::Write(Box::new(StateJob {
            callback: Some(callback),
            output: None,
            sender: Some(sender),
        })))
        .await
        .map_err(E::from)?;
        receiver
            .await
            .map_err(|_| E::from(WorkerError::WorkerStopped))?
    }

    /// Permanently stop admission on all clones, drain accepted commands, then checkpoint
    /// and drop this NORMAL database's connection on the shared writer thread.
    /// Concurrent/repeated callers observe the same terminal result. Once first polled,
    /// cancelling this future does not cancel close or reopen admission. Completion,
    /// including an error, guarantees the writer connection was dropped; callers must
    /// separately fence their reads before deleting files. A later registration of the
    /// same path gets a new database ID; old handles remain closed.
    ///
    /// # Errors
    /// Returns `InvalidConfiguration` for the FULL state writer, or a typed database,
    /// checkpoint, or shutdown failure after the connection has been dropped.
    pub async fn close(&self) -> Result<(), WorkerError> {
        let mut completion = self.shared.request_close(self.database_id)?;
        completion
            .wait_for(Option::is_some)
            .await
            .map_err(|_| WorkerError::WorkerStopped)?
            .clone()
            .ok_or(WorkerError::WorkerStopped)?
    }

    /// Queue a fixed passive WAL checkpoint after previously accepted commands.
    /// Shares bounded queue capacity and round-robin turns with batches; does not wait
    /// for active readers or promise FULL durability. Cancellation before admission
    /// cancels the request; after admission it does not cancel the checkpoint.
    ///
    /// # Errors
    /// Returns `InvalidConfiguration` for the state writer, admission/database errors,
    /// or typed SQLite and shutdown failures.
    pub async fn checkpoint(&self) -> Result<(), WorkerError> {
        if self.state {
            return Err(WorkerError::InvalidConfiguration);
        }
        let (sender, receiver) = oneshot::channel();
        self.admit(QueuedJob::Checkpoint(sender)).await?;
        receiver.await.map_err(|_| WorkerError::WorkerStopped)?
    }

    async fn admit(&self, job: QueuedJob) -> Result<u64, WorkerError> {
        let permit = Arc::clone(&self.capacity)
            .acquire_owned()
            .await
            .map_err(|_| self.shared.admission_error(self.database_id))?;
        self.shared.enqueue(self.database_id, job, permit)
    }
}
