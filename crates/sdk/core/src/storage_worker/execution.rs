//! Atomic SQL batches and trusted state jobs; admission and thread scheduling live elsewhere.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use rusqlite::{
    Connection, Transaction, TransactionBehavior,
    hooks::{AuthAction, AuthContext, Authorization},
    params_from_iter,
    types::Value,
};
use tokio::sync::oneshot;

use super::WorkerError;

/// One parameterized DML statement. Schema, PRAGMA, ATTACH and transaction control
/// are denied by SQLite's authorizer; SQL and parameters are never logged.
pub struct SqlMutation {
    sql: &'static str,
    parameters: Vec<Value>,
}

impl SqlMutation {
    /// Construct a parameterized statement; validation occurs inside its transaction.
    #[must_use]
    pub fn new(sql: &'static str, parameters: Vec<Value>) -> Self {
        Self { sql, parameters }
    }
}

/// One atomic transaction, with no more than 500 statements or affected rows.
/// Submit multiple batches for large rebuilds; each gets a separate fair turn.
pub struct SqlBatch {
    mutations: Vec<SqlMutation>,
}

impl SqlBatch {
    /// Construct a batch; triggers and multi-row DML also count toward the row limit.
    ///
    /// # Errors
    /// Returns `RowLimit` when more than 500 statements are supplied.
    pub fn new(mutations: Vec<SqlMutation>) -> Result<Self, WorkerError> {
        if mutations.len() > 500 {
            return Err(WorkerError::RowLimit);
        }
        Ok(Self { mutations })
    }
}

/// Evidence that one transaction has committed, not merely been admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    /// Actual rows changed, including trigger and foreign-key effects.
    pub affected_rows: u64,
    /// True only for the state writer configured with synchronous FULL.
    pub durable: bool,
}

pub(super) trait Job: Send {
    fn execute(&mut self, transaction: &Transaction<'_>) -> Result<(), WorkerError>;
    fn respond(self: Box<Self>, result: Result<CommitInfo, WorkerError>);
}

pub(super) struct BatchJob {
    pub batch: SqlBatch,
    pub sender: Option<oneshot::Sender<Result<CommitInfo, WorkerError>>>,
    pub rows: u64,
}

impl Job for BatchJob {
    fn execute(&mut self, transaction: &Transaction<'_>) -> Result<(), WorkerError> {
        let start_rows = transaction.total_changes();
        for mutation in &self.batch.mutations {
            transaction.execute(mutation.sql, params_from_iter(&mutation.parameters))?;
            self.rows = transaction.total_changes().saturating_sub(start_rows);
            if self.rows > 500 {
                return Err(WorkerError::RowLimit);
            }
        }
        Ok(())
    }
    fn respond(mut self: Box<Self>, result: Result<CommitInfo, WorkerError>) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(result);
        }
    }
}

pub(super) struct StateJob<T, E, F> {
    pub callback: Option<F>,
    pub output: Option<Result<T, E>>,
    pub sender: Option<oneshot::Sender<Result<T, E>>>,
}

impl<T, E, F> Job for StateJob<T, E, F>
where
    T: Send + 'static,
    E: std::error::Error + From<WorkerError> + Send + 'static,
    F: FnOnce(&Transaction<'_>) -> Result<T, E> + Send + 'static,
{
    fn execute(&mut self, transaction: &Transaction<'_>) -> Result<(), WorkerError> {
        let callback = self.callback.take().ok_or(WorkerError::WorkerStopped)?;
        self.output = Some(callback(transaction));
        if let Some(Err(error)) = self.output.as_ref() {
            return Err(database_error(error).unwrap_or(WorkerError::TransactionRejected));
        }
        Ok(())
    }
    fn respond(mut self: Box<Self>, result: Result<CommitInfo, WorkerError>) {
        let output = match result {
            Ok(_) | Err(WorkerError::TransactionRejected) => self
                .output
                .take()
                .unwrap_or_else(|| Err(E::from(WorkerError::WorkerStopped))),
            Err(WorkerError::ShutdownDeadline) => Err(E::from(WorkerError::ShutdownDeadline)),
            Err(error) => match self.output.take() {
                Some(Err(original)) => Err(original),
                _ => Err(E::from(error)),
            },
        };
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(output);
        }
    }
}

fn database_error(error: &(dyn std::error::Error + 'static)) -> Option<WorkerError> {
    let mut source = Some(error);
    for _ in 0..32 {
        let current = source?;
        if let Some(error) = current.downcast_ref::<rusqlite::Error>() {
            return Some(WorkerError::Sqlite {
                code: error.sqlite_error().map(|error| error.extended_code),
            });
        }
        if let Some(error @ WorkerError::Sqlite { .. }) = current.downcast_ref::<WorkerError>() {
            return Some(error.clone());
        }
        source = current.source();
    }
    None
}

/// Execute one atomic bounded turn. Rust callbacks, fsync and filesystem calls cannot
/// be forcibly preempted; the scheduler suppresses acknowledgements after shutdown.
pub(super) fn run(
    connection: &mut Connection,
    job: &mut dyn Job,
    budget: Duration,
    durable: bool,
) -> Result<CommitInfo, WorkerError> {
    let start = Instant::now();
    let internal = Arc::new(AtomicBool::new(true));
    let gate = Arc::clone(&internal);
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        authorize(context, gate.load(Ordering::Relaxed))
    }))?;
    let expired = Arc::new(AtomicBool::new(false));
    let progress_expired = Arc::clone(&expired);
    let progress_internal = Arc::clone(&internal);
    connection.progress_handler(
        100,
        Some(move || {
            let timed_out =
                !durable && !progress_internal.load(Ordering::Relaxed) && start.elapsed() >= budget;
            if timed_out {
                progress_expired.store(true, Ordering::Relaxed);
            }
            timed_out
        }),
    )?;
    let result = transaction(connection, job, &internal, start, budget, durable);
    internal.store(true, Ordering::Relaxed);
    connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
    connection.progress_handler(0, None::<fn() -> bool>)?;
    if expired.load(Ordering::Relaxed) {
        return Err(WorkerError::TurnDeadline);
    }
    result
}

fn transaction(
    connection: &mut Connection,
    job: &mut dyn Job,
    internal: &AtomicBool,
    start: Instant,
    budget: Duration,
    durable: bool,
) -> Result<CommitInfo, WorkerError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let before = transaction.total_changes();
    internal.store(false, Ordering::Relaxed);
    let result = job.execute(&transaction);
    internal.store(true, Ordering::Relaxed);
    result?;
    let affected_rows = transaction.total_changes().saturating_sub(before);
    if !durable && affected_rows > 500 {
        return Err(WorkerError::RowLimit);
    }
    if !durable && start.elapsed() >= budget {
        return Err(WorkerError::TurnDeadline);
    }
    transaction.commit()?;
    Ok(CommitInfo {
        affected_rows,
        durable,
    })
}

fn authorize(context: AuthContext<'_>, internal: bool) -> Authorization {
    let permitted = match context.action {
        AuthAction::Transaction { .. } => internal,
        AuthAction::Read { .. } | AuthAction::Select | AuthAction::Recursive => true,
        AuthAction::Insert { table_name }
        | AuthAction::Delete { table_name }
        | AuthAction::Update { table_name, .. } => !table_name.starts_with("sqlite_"),
        AuthAction::Function { function_name } => {
            !function_name.eq_ignore_ascii_case("load_extension")
        }
        _ => false,
    };
    if permitted && context.database_name.is_none_or(|name| name == "main") {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
