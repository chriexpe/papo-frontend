//! Bounded blocking read connections; no read transaction survives its query callback.

use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use rusqlite::{
    Connection, OpenFlags, Transaction,
    hooks::{AuthAction, AuthContext, Authorization},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::WorkerError;
use crate::data_root::ProfileLease;

/// Shared bound on active read connections; cancellation retains the permit until
/// the blocking query actually finishes. Connections never retain a WAL snapshot.
#[derive(Clone)]
pub struct ReadPool {
    inner: Arc<ReadFence>,
}

struct ReadFence {
    capacity: Arc<Semaphore>,
    active: Mutex<usize>,
    finished: Condvar,
    _profile_lease: Option<Arc<ProfileLease>>,
}

struct ActiveRead {
    fence: Arc<ReadFence>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for ActiveRead {
    fn drop(&mut self) {
        let mut active = self
            .fence
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.saturating_sub(1);
        self.fence.finished.notify_all();
    }
}

impl Default for ReadPool {
    fn default() -> Self {
        Self::with_capacity(4, None)
    }
}

impl ReadPool {
    fn with_capacity(max_connections: usize, profile_lease: Option<Arc<ProfileLease>>) -> Self {
        Self {
            inner: Arc::new(ReadFence {
                capacity: Arc::new(Semaphore::new(max_connections)),
                active: Mutex::new(0),
                finished: Condvar::new(),
                _profile_lease: profile_lease,
            }),
        }
    }

    /// Construct a pool with a nonzero connection bound (default is four).
    ///
    /// # Errors
    /// Returns invalid configuration for zero or excessive capacity.
    pub fn new(max_connections: usize) -> Result<Self, WorkerError> {
        Self::checked(max_connections, None)
    }

    /// Construct a bounded read pool retaining the profile lock until all pool handles
    /// and active blocking queries are dropped, including canceled query futures.
    ///
    /// # Errors
    /// Returns invalid configuration for zero or excessive capacity.
    pub(crate) fn with_profile_lease(
        max_connections: usize,
        lease: Arc<ProfileLease>,
    ) -> Result<Self, WorkerError> {
        Self::checked(max_connections, Some(lease))
    }

    fn checked(
        max_connections: usize,
        lease: Option<Arc<ProfileLease>>,
    ) -> Result<Self, WorkerError> {
        if max_connections == 0 || max_connections > Semaphore::MAX_PERMITS {
            return Err(WorkerError::InvalidConfiguration);
        }
        Ok(Self::with_capacity(max_connections, lease))
    }

    /// Run one owned query batch in one query-only read transaction off the runtime.
    /// Callbacks must be finite and return owned data; they cannot retain a connection.
    ///
    /// # Errors
    /// Returns admission closure, SQLite errors, or a panicked blocking callback.
    pub async fn query<T, F>(&self, path: PathBuf, query: F) -> Result<T, WorkerError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T> + Send + 'static,
    {
        let permit = Arc::clone(&self.inner.capacity)
            .acquire_owned()
            .await
            .map_err(|_| WorkerError::AdmissionClosed)?;
        let active_read = {
            let mut active = self
                .inner
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.inner.capacity.is_closed() {
                return Err(WorkerError::AdmissionClosed);
            }
            *active += 1;
            ActiveRead {
                fence: Arc::clone(&self.inner),
                _permit: permit,
            }
        };
        tokio::task::spawn_blocking(move || {
            let _active_read = active_read;
            let mut connection = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            connection.busy_timeout(Duration::ZERO)?;
            connection.pragma_update(None, "query_only", "ON")?;
            connection.authorizer(Some(|context: AuthContext<'_>| {
                let allowed = match context.action {
                    AuthAction::Read { .. }
                    | AuthAction::Select
                    | AuthAction::Recursive
                    | AuthAction::Transaction { .. } => true,
                    AuthAction::Function { function_name } => {
                        !function_name.eq_ignore_ascii_case("load_extension")
                    }
                    _ => false,
                };
                if allowed {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }))?;
            let transaction = connection.transaction()?;
            let result = query(&transaction)?;
            transaction.commit()?;
            Ok::<T, WorkerError>(result)
        })
        .await
        .map_err(|_| WorkerError::CallbackPanicked)?
    }

    /// Reject new reads; admitted blocking reads retain permits until actually finished.
    pub fn stop_admission(&self) {
        let _active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.inner.capacity.close();
    }

    /// Stop admission and wait outside the runtime for all blocking reads to close.
    /// Returns false at the deadline if a query is still active. Keep profile ownership
    /// until a later drain returns true; dropping a query future cannot release its fence.
    ///
    /// # Errors
    /// Returns `RuntimeContext` when called inside a Tokio runtime.
    pub fn drain(&self, deadline: Instant) -> Result<bool, WorkerError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(WorkerError::RuntimeContext);
        }
        self.stop_admission();
        let mut active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *active != 0 {
            let now = Instant::now();
            if now >= deadline {
                return Ok(false);
            }
            let (guard, _) = self
                .inner
                .finished
                .wait_timeout(active, deadline.saturating_duration_since(now))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active = guard;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
