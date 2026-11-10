//! Per-database registration and terminal close state; scheduling and SQL stay elsewhere.

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::Arc,
};

use tokio::sync::{Semaphore, watch};

use super::{
    DatabaseWriter, Initializer, WorkerError,
    connection::{self, OpenConnection},
    scheduler::{DatabaseQueue, Shared, Specification, fail, guarded},
};

type CloseResult = Option<Result<(), WorkerError>>;

pub(super) struct DatabaseLifecycle {
    pub closing: bool,
    completion: watch::Sender<CloseResult>,
}

impl DatabaseLifecycle {
    fn new() -> Self {
        Self {
            closing: false,
            completion: watch::channel(None).0,
        }
    }

    pub fn finished(&self) -> bool {
        self.closing && self.completion.borrow().is_some()
    }

    pub fn pending(&self) -> bool {
        self.closing && self.completion.borrow().is_none()
    }
}

impl Shared {
    pub fn register(
        self: &Arc<Self>,
        path: PathBuf,
        initializer: Initializer,
        durable: bool,
    ) -> Result<DatabaseWriter, WorkerError> {
        let mut state = self.lock();
        if !state.accepting {
            return Err(WorkerError::AdmissionClosed);
        }
        if durable != self.durable || (durable && !state.databases.is_empty()) {
            return Err(WorkerError::InvalidConfiguration);
        }
        if state
            .databases
            .iter()
            .any(|database| database.specification.path == path && !database.lifecycle.finished())
        {
            return Err(WorkerError::AlreadyRegistered);
        }
        // Retain tombstones so an old handle can never address a replacement registration.
        let database_id =
            u64::try_from(state.databases.len()).map_err(|_| WorkerError::InvalidConfiguration)?;
        let capacity = Arc::new(Semaphore::new(self.config.queue_capacity));
        state.databases.push(DatabaseQueue {
            specification: Specification {
                path,
                initializer,
                durable,
            },
            capacity: Arc::clone(&capacity),
            commands: VecDeque::new(),
            failure: None,
            lifecycle: DatabaseLifecycle::new(),
        });
        Ok(DatabaseWriter {
            shared: Arc::clone(self),
            database_id,
            capacity,
            state: durable,
        })
    }

    pub fn request_close(
        &self,
        database_id: u64,
    ) -> Result<watch::Receiver<CloseResult>, WorkerError> {
        if self.durable {
            return Err(WorkerError::InvalidConfiguration);
        }
        let mut state = self.lock();
        let database = usize::try_from(database_id)
            .ok()
            .and_then(|index| state.databases.get_mut(index))
            .ok_or(WorkerError::DatabaseClosed)?;
        database.lifecycle.closing = true;
        database.capacity.close();
        let completion = database.lifecycle.completion.subscribe();
        self.wake.notify_all();
        Ok(completion)
    }
}

/// Publish only after the worker has dropped the database's connection.
pub(super) fn complete(shared: &Shared, database_id: u64, result: Result<(), WorkerError>) {
    let state = shared.lock();
    let Some(database) = usize::try_from(database_id)
        .ok()
        .and_then(|index| state.databases.get(index))
    else {
        return;
    };
    if database.lifecycle.completion.borrow().is_some() {
        return;
    }
    let result = if let Some(error) = &database.failure {
        Err(error.clone())
    } else if database.lifecycle.closing && state.deadline_reached {
        Err(WorkerError::ShutdownDeadline)
    } else {
        result
    };
    database.lifecycle.completion.send_replace(Some(result));
}

pub(super) fn close_database(
    shared: &Shared,
    open: &mut HashMap<u64, OpenConnection>,
    database_id: u64,
) {
    let result = guarded(|| open.remove(&database_id).map_or(Ok(()), connection::close));
    if let Err(error) = &result {
        fail(&mut shared.lock(), database_id, error.clone());
    }
    complete(shared, database_id, result);
}

pub(super) fn close_remaining(shared: &Shared, open: &mut HashMap<u64, OpenConnection>) {
    let count = shared.lock().databases.len();
    for index in 0..count {
        close_database(shared, open, index as u64);
    }
}
