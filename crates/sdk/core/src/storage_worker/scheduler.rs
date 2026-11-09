//! Fixed-thread round-robin scheduling and failure isolation; SQL/schema policy is elsewhere.

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::Instant,
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{
    DatabaseWriter, Initializer, WorkerConfig, WorkerError,
    connection::{self, OpenConnection},
    execution::Job,
    shutdown::{DatabaseFailure, UnpersistedWrite, WriteDisposition},
};

#[derive(Clone)]
pub(super) struct Specification {
    pub path: PathBuf,
    pub initializer: Initializer,
    pub durable: bool,
}

pub(super) struct Command {
    pub id: u64,
    pub job: Box<dyn Job>,
    pub permit: OwnedSemaphorePermit,
}

pub(super) struct DatabaseQueue {
    pub specification: Specification,
    pub capacity: Arc<Semaphore>,
    pub commands: VecDeque<Command>,
    pub failure: Option<WorkerError>,
}

pub(super) struct State {
    pub accepting: bool,
    pub deadline: Option<Instant>,
    pub finished: bool,
    pub deadline_reached: bool,
    pub databases: Vec<DatabaseQueue>,
    pub inflight: Option<UnpersistedWrite>,
    pub failures: Vec<DatabaseFailure>,
    pub unpersisted: Vec<UnpersistedWrite>,
    pub next_command_id: u64,
    pub cursor: usize,
}

pub(super) struct Shared {
    pub state: Mutex<State>,
    pub wake: Condvar,
    pub config: WorkerConfig,
    pub durable: bool,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

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
            .any(|database| database.specification.path == path)
        {
            return Err(WorkerError::AlreadyRegistered);
        }
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
        });
        Ok(DatabaseWriter {
            shared: Arc::clone(self),
            database_id,
            capacity,
            state: durable,
        })
    }

    pub fn enqueue(
        &self,
        database_id: u64,
        job: Box<dyn Job>,
        permit: OwnedSemaphorePermit,
    ) -> Result<u64, WorkerError> {
        let mut state = self.lock();
        if !state.accepting {
            return Err(WorkerError::AdmissionClosed);
        }
        let index = usize::try_from(database_id).map_err(|_| WorkerError::DatabaseClosed)?;
        if state
            .databases
            .get(index)
            .is_none_or(|database| database.failure.is_some())
        {
            return Err(WorkerError::DatabaseClosed);
        }
        let id = state.next_command_id;
        state.next_command_id = id.checked_add(1).ok_or(WorkerError::WorkerStopped)?;
        state.databases[index]
            .commands
            .push_back(Command { id, job, permit });
        self.wake.notify_all();
        Ok(id)
    }

    pub fn admission_error(&self, database_id: u64) -> WorkerError {
        let state = self.lock();
        if !state.accepting {
            WorkerError::AdmissionClosed
        } else if usize::try_from(database_id)
            .ok()
            .and_then(|index| state.databases.get(index))
            .is_some_and(|database| database.failure.is_some())
        {
            WorkerError::DatabaseClosed
        } else {
            WorkerError::WorkerStopped
        }
    }

    pub fn stop_admission(&self) {
        let mut state = self.lock();
        state.accepting = false;
        for database in &state.databases {
            database.capacity.close();
        }
        self.wake.notify_all();
    }

    pub fn request_stop(&self, deadline: Instant) {
        let mut state = self.lock();
        state.accepting = false;
        for database in &state.databases {
            database.capacity.close();
        }
        state.deadline = Some(
            state
                .deadline
                .map_or(deadline, |previous| previous.min(deadline)),
        );
        self.wake.notify_all();
    }
}

pub(super) fn guarded<T>(
    callback: impl FnOnce() -> Result<T, WorkerError>,
) -> Result<T, WorkerError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback))
        .unwrap_or(Err(WorkerError::CallbackPanicked))
}

pub(super) fn run(shared: &Shared, open: &mut HashMap<u64, OpenConnection>) {
    loop {
        close_idle(shared, open, Instant::now());
        let (database_id, specification, mut command) = match next(shared) {
            Turn::Work(database_id, specification, command) => {
                (database_id, specification, command)
            }
            Turn::Maintenance => continue,
            Turn::Stop => break,
        };
        drop(command.permit);
        let result = guarded(|| {
            if let std::collections::hash_map::Entry::Vacant(entry) = open.entry(database_id) {
                entry.insert(connection::open(&specification)?);
            }
            let connection = open
                .get_mut(&database_id)
                .ok_or(WorkerError::WorkerStopped)?;
            connection.last_used = Instant::now();
            let result = super::execution::run(
                &mut connection.connection,
                command.job.as_mut(),
                shared.config.turn_budget,
                shared.durable,
            );
            connection.last_used = Instant::now();
            result
        });
        let mut state = shared.lock();
        let result = if state
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            state.deadline_reached = true;
            Err(WorkerError::ShutdownDeadline)
        } else {
            result
        };
        state.inflight = None;
        let mut failed_connection = None;
        if let Err(error) = &result
            && *error != WorkerError::TransactionRejected
        {
            state.unpersisted.push(UnpersistedWrite {
                database_id,
                command_id: command.id,
                disposition: if *error == WorkerError::ShutdownDeadline {
                    WriteDisposition::OutcomeUnknown
                } else {
                    WriteDisposition::Failed
                },
            });
            fail(&mut state, database_id, error.clone());
            failed_connection = open.remove(&database_id);
        }
        drop(state);
        command.job.respond(result);
        drop(failed_connection);
        shared.wake.notify_all();
    }
    for (database_id, connection) in open.drain() {
        if let Err(error) = connection::close(connection) {
            fail(&mut shared.lock(), database_id, error);
        }
    }
}

enum Turn {
    Work(u64, Specification, Command),
    Maintenance,
    Stop,
}

fn next(shared: &Shared) -> Turn {
    let mut state = shared.lock();
    loop {
        if state
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            state.deadline_reached = true;
            for index in 0..state.databases.len() {
                while let Some(command) = state.databases[index].commands.pop_front() {
                    state.unpersisted.push(UnpersistedWrite {
                        database_id: index as u64,
                        command_id: command.id,
                        disposition: WriteDisposition::NotStarted,
                    });
                    command.job.respond(Err(WorkerError::ShutdownDeadline));
                }
            }
            return Turn::Stop;
        }
        let count = state.databases.len();
        for offset in 0..count {
            let index = (state.cursor + offset) % count;
            if let Some(command) = state.databases[index].commands.pop_front() {
                state.cursor = (index + 1) % count;
                let database_id = index as u64;
                state.inflight = Some(UnpersistedWrite {
                    database_id,
                    command_id: command.id,
                    disposition: WriteDisposition::OutcomeUnknown,
                });
                return Turn::Work(
                    database_id,
                    state.databases[index].specification.clone(),
                    command,
                );
            }
        }
        if !state.accepting {
            return Turn::Stop;
        }
        let (guard, timeout) = shared
            .wake
            .wait_timeout(state, shared.config.idle_timeout)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state = guard;
        if timeout.timed_out() {
            return Turn::Maintenance;
        }
    }
}

fn fail(state: &mut State, database_id: u64, error: WorkerError) {
    let Some(database) = usize::try_from(database_id)
        .ok()
        .and_then(|index| state.databases.get_mut(index))
    else {
        return;
    };
    let queued_error = if error == WorkerError::ShutdownDeadline {
        WorkerError::ShutdownDeadline
    } else {
        WorkerError::DatabaseClosed
    };
    database.failure = Some(error.clone());
    database.capacity.close();
    state.failures.push(DatabaseFailure { database_id, error });
    while let Some(command) = database.commands.pop_front() {
        state.unpersisted.push(UnpersistedWrite {
            database_id,
            command_id: command.id,
            disposition: WriteDisposition::NotStarted,
        });
        command.job.respond(Err(queued_error.clone()));
    }
}

pub(super) fn close_idle(shared: &Shared, open: &mut HashMap<u64, OpenConnection>, now: Instant) {
    let idle: Vec<_> = open
        .iter()
        .filter(|(_, connection)| {
            !shared.durable
                && now.saturating_duration_since(connection.last_used) >= shared.config.idle_timeout
        })
        .map(|(id, _)| *id)
        .collect();
    for database_id in idle {
        if let Some(connection) = open.remove(&database_id)
            && let Err(error) = connection::close(connection)
        {
            fail(&mut shared.lock(), database_id, error);
        }
    }
}
