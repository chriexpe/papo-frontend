//! Thread construction and initial state opening; scheduling and schemas are separate.

use super::{
    DatabaseWriter, Initializer, StorageWorker, WorkerConfig, WorkerError, connection,
    scheduler::{Shared, State, guarded, run},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};
use tokio::sync::Semaphore;

pub(super) fn start(
    config: WorkerConfig,
    initial: Option<(PathBuf, Initializer)>,
) -> Result<(StorageWorker, Option<DatabaseWriter>), WorkerError> {
    if config.queue_capacity == 0
        || config.queue_capacity > Semaphore::MAX_PERMITS
        || config.turn_budget > std::time::Duration::from_millis(20)
        || config.idle_timeout.is_zero()
    {
        return Err(WorkerError::InvalidConfiguration);
    }
    let durable = initial.is_some();
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            accepting: true,
            deadline: None,
            finished: false,
            deadline_reached: false,
            databases: Vec::new(),
            inflight: None,
            failures: Vec::new(),
            unpersisted: Vec::new(),
            next_command_id: 1,
            cursor: 0,
        }),
        wake: Condvar::new(),
        config,
        durable,
    });
    let writer = initial
        .map(|(path, initializer)| shared.register(path, initializer, true))
        .transpose()?;
    let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
    let thread_shared = Arc::clone(&shared);
    let thread = thread::Builder::new()
        .name(
            if durable {
                "papo-state-writer"
            } else {
                "papo-shared-writer"
            }
            .into(),
        )
        .spawn(move || {
            let mut open = HashMap::new();
            let startup = if durable {
                let specification = thread_shared.lock().databases[0].specification.clone();
                guarded(|| connection::open(&specification)).map(|connection| {
                    open.insert(0, connection);
                })
            } else {
                Ok(())
            };
            let failed = startup.is_err();
            let _ = ready_sender.send(startup);
            if !failed {
                run(&thread_shared, &mut open);
            }
            let mut state = thread_shared.lock();
            state.finished = true;
            thread_shared.wake.notify_all();
        })
        .map_err(|_| WorkerError::System)?;
    let worker = StorageWorker {
        shared,
        thread: Some(thread),
    };
    if durable {
        ready_receiver
            .recv()
            .map_err(|_| WorkerError::WorkerStopped)??;
    }
    Ok((worker, writer))
}
