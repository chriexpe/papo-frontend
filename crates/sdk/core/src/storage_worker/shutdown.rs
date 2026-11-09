//! Deadline-bounded drain reports and outside-runtime thread joining; runtime teardown is caller-owned.

use std::time::Instant;

use super::{StorageWorker, WorkerError, scheduler::State};

/// Persistence knowledge at the shutdown deadline; unknown never means acknowledged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteDisposition {
    /// The write was still queued and its transaction never started.
    NotStarted,
    /// SQLite or a callback was running; a late physical commit is possible.
    OutcomeUnknown,
    /// The transaction returned an error and was not acknowledged.
    Failed,
}

/// One admitted command without a successful commit acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnpersistedWrite {
    /// Worker-local registered database identifier.
    pub database_id: u64,
    /// Worker-local accepted command identifier.
    pub command_id: u64,
    /// Whether a late physical commit is possible.
    pub disposition: WriteDisposition,
}

/// An isolated database failure; other databases continue to drain normally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseFailure {
    /// Worker-local registered database identifier.
    pub database_id: u64,
    /// Sanitized boundary error.
    pub error: WorkerError,
}

/// Explicit drain outcome. `finished` includes checkpointing and closing all connections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShutdownReport {
    /// The storage thread has closed its connections and ended.
    pub finished: bool,
    /// The deadline was reached before the worker finished.
    pub deadline_reached: bool,
    /// No entry here was acknowledged as successfully committed.
    pub unpersisted: Vec<UnpersistedWrite>,
    /// Database-local failures, including checkpoint failures.
    pub failures: Vec<DatabaseFailure>,
}

impl StorageWorker {
    /// Stop admission, drain accepted writes, checkpoint and close up to `deadline`.
    /// This returns at the deadline even if an OS call/callback is still running.
    /// In-flight work then has an unknown physical outcome and cannot send success.
    /// Retain this worker and the profile lock until `join` completes.
    ///
    /// # Errors
    /// Returns `RuntimeContext` when called from within a Tokio runtime.
    pub fn shutdown(&self, deadline: Instant) -> Result<ShutdownReport, WorkerError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(WorkerError::RuntimeContext);
        }
        self.begin_shutdown(deadline);
        let mut state = self.shared.lock();
        while !state.finished {
            let now = Instant::now();
            if now >= deadline {
                state.deadline_reached = true;
                break;
            }
            let (guard, _) = self
                .shared
                .wake
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = guard;
        }
        Ok(report(&state))
    }

    /// Join a previously stopped writer from outside the runtime. This is not
    /// deadline-bounded: SQLite filesystem calls and trusted callbacks cannot be killed.
    ///
    /// # Errors
    /// Returns `RuntimeContext`, `AdmissionClosed` if still admitting work, or a panic.
    pub fn join(&mut self) -> Result<(), WorkerError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(WorkerError::RuntimeContext);
        }
        if self.shared.lock().accepting {
            return Err(WorkerError::AdmissionClosed);
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| WorkerError::WorkerStopped)?;
        }
        Ok(())
    }
}

fn report(state: &State) -> ShutdownReport {
    let mut unpersisted = state.unpersisted.clone();
    if let Some(inflight) = state.inflight {
        unpersisted.push(inflight);
    }
    for (index, database) in state.databases.iter().enumerate() {
        unpersisted.extend(database.commands.iter().map(|command| UnpersistedWrite {
            database_id: index as u64,
            command_id: command.id,
            disposition: WriteDisposition::NotStarted,
        }));
    }
    unpersisted.sort_by_key(|write| write.command_id);
    unpersisted.dedup_by_key(|write| write.command_id);
    ShutdownReport {
        finished: state.finished,
        deadline_reached: state.deadline_reached,
        unpersisted,
        failures: state.failures.clone(),
    }
}
