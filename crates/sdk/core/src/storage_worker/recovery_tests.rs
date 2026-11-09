//! Recovery and budget tests with explicit clocks and channel-controlled execution.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use rusqlite::Connection;

use super::*;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn schema(connection: &mut Connection) -> Result<(), WorkerError> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS fixture(value INTEGER)")?;
    Ok(())
}

fn batch() -> SqlBatch {
    SqlBatch::new(vec![SqlMutation::new(
        "INSERT INTO fixture VALUES (1)",
        vec![],
    )])
    .expect("batch")
}

fn finish(worker: &mut StorageWorker) {
    worker
        .shutdown(Instant::now() + Duration::from_secs(30))
        .expect("shutdown");
    worker.join().expect("join");
}

#[test]
fn unknown_shared_version_is_rebuilt_through_caller_initializer() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("future-cache.sqlite");
    let connection = Connection::open(&path).expect("cache");
    connection
        .pragma_update(None, "user_version", 9)
        .expect("future version");
    connection.close().expect("close");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let writer = worker
        .register(path.clone(), move |connection| {
            observed.fetch_add(1, Ordering::SeqCst);
            let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
            if version > 1 {
                return Err(WorkerError::NewerSchema {
                    found: version,
                    supported: 1,
                });
            }
            schema(connection)?;
            connection.pragma_update(None, "user_version", 1)?;
            Ok(())
        })
        .expect("register");
    runtime
        .block_on(writer.commit(batch()))
        .expect("rebuilt commit");
    finish(&mut worker);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let connection = Connection::open(path).expect("rebuilt cache");
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .expect("version"),
        1
    );
}

#[test]
fn corrupt_shared_file_is_recreated_without_a_worker_owned_schema() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("corrupt.sqlite");
    std::fs::write(&path, b"invalid sqlite fixture").expect("invalid file");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let writer = worker.register(path.clone(), schema).expect("register");
    runtime
        .block_on(writer.commit(batch()))
        .expect("rebuilt commit");
    finish(&mut worker);
    let connection = Connection::open(path).expect("cache");
    assert_eq!(
        connection
            .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
            .expect("integrity"),
        "ok"
    );
    assert_eq!(
        connection
            .query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, i32>(0))
            .expect("vacuum"),
        2
    );
}

#[test]
fn idle_connections_close_at_the_configured_deadline_without_sleeping() {
    let directory = tempfile::tempdir().expect("directory");
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let writer = worker
        .register(directory.path().join("idle.sqlite"), schema)
        .expect("register");
    let specification = worker.shared.lock().databases[0].specification.clone();
    let mut connection = connection::open(&specification).expect("open");
    let base = Instant::now();
    connection.last_used = base;
    let mut open = HashMap::from([(writer.database_id(), connection)]);
    scheduler::close_idle(&worker.shared, &mut open, base + Duration::from_secs(599));
    assert_eq!(open.len(), 1);
    scheduler::close_idle(&worker.shared, &mut open, base + Duration::from_secs(600));
    assert!(open.is_empty());
    finish(&mut worker);
}

#[test]
fn zero_execution_budget_rolls_back_and_never_acknowledges() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("budget.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig {
        turn_budget: Duration::ZERO,
        ..WorkerConfig::default()
    })
    .expect("worker");
    let writer = worker.register(path.clone(), schema).expect("register");
    assert_eq!(
        runtime.block_on(writer.commit(batch())),
        Err(WorkerError::TurnDeadline)
    );
    finish(&mut worker);
    let connection = Connection::open(path).expect("database");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM fixture", [], |row| row
                .get::<_, i64>(0))
            .expect("count"),
        0
    );
}

#[test]
fn dropping_accepted_receipt_does_not_cancel_persistence() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("receipt.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let writer = worker
        .register(path.clone(), move |connection| {
            entered.send(()).expect("entered");
            gate.lock().expect("gate").recv().expect("release");
            schema(connection)
        })
        .expect("register");
    let receipt = runtime.block_on(writer.submit(batch())).expect("accepted");
    waiting.recv().expect("initializer blocked");
    drop(receipt);
    release.send(()).expect("release");
    finish(&mut worker);
    let connection = Connection::open(path).expect("database");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM fixture", [], |row| row
                .get::<_, i64>(0))
            .expect("count"),
        1
    );
}

#[test]
fn trusted_state_transaction_keeps_more_than_500_rows_atomic() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("large-state.sqlite");
    let runtime = runtime();
    let (mut worker, writer) = StorageWorker::state(
        path.clone(),
        schema,
        WorkerConfig {
            turn_budget: Duration::ZERO,
            ..WorkerConfig::default()
        },
    )
    .expect("worker");
    let rows: usize = runtime.block_on(writer.write::<_, WorkerError, _>(|transaction| {
        Ok(transaction.execute("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<601) INSERT INTO fixture SELECT x FROM n", [])?)
    })).expect("atomic FULL transaction");
    assert_eq!(rows, 601);
    finish(&mut worker);
    let connection = Connection::open(path).expect("state");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM fixture", [], |row| row
                .get::<_, i64>(0))
            .expect("count"),
        601
    );
}

#[test]
fn trusted_state_sqlite_failure_closes_database_but_preserves_typed_error() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let (mut worker, writer) = StorageWorker::state(
        directory.path().join("failure.sqlite"),
        schema,
        WorkerConfig::default(),
    )
    .expect("worker");
    let result = runtime.block_on(writer.write::<_, WorkerError, _>(|transaction| {
        transaction.execute("INSERT INTO absent_fixture VALUES (1)", [])?;
        Ok(())
    }));
    assert!(matches!(result, Err(WorkerError::Sqlite { .. })));
    assert_eq!(
        runtime.block_on(writer.commit(batch())),
        Err(WorkerError::DatabaseClosed)
    );
    finish(&mut worker);
}

#[test]
fn begin_shutdown_installs_acknowledgement_deadline_before_any_wait() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let writer = worker
        .register(
            directory.path().join("begin-shutdown.sqlite"),
            move |connection| {
                entered.send(()).expect("entered");
                gate.lock().expect("gate").recv().expect("release");
                schema(connection)
            },
        )
        .expect("register");
    let receipt = runtime.block_on(writer.submit(batch())).expect("accepted");
    waiting.recv().expect("initializer blocked");
    let deadline = Instant::now();
    worker.begin_shutdown(deadline);
    release.send(()).expect("release");
    worker.join().expect("join without blocking shutdown");
    assert_eq!(
        runtime.block_on(receipt.committed()),
        Err(WorkerError::ShutdownDeadline)
    );
    let report = worker.shutdown(deadline).expect("final report");
    assert!(report.finished && report.deadline_reached);
    assert_eq!(report.unpersisted.len(), 1);
}
