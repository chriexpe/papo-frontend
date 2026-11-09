//! Deterministic writer contract tests; read-pool tests live beside their implementation.

use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::{Arc, Mutex, mpsc},
    task::Poll,
    time::{Duration, Instant},
};

use rusqlite::{Connection, hooks::Action, types::Value};

use super::*;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn schema(connection: &mut Connection) -> Result<(), WorkerError> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS fixture (value INTEGER NOT NULL)")?;
    Ok(())
}

fn batch() -> SqlBatch {
    SqlBatch::new(vec![SqlMutation::new(
        "INSERT INTO fixture VALUES (?)",
        vec![Value::Integer(1)],
    )])
    .expect("batch")
}

fn finish(worker: &mut StorageWorker) -> ShutdownReport {
    let report = worker
        .shutdown(Instant::now() + Duration::from_secs(30))
        .expect("shutdown");
    worker.join().expect("join");
    report
}

fn blocked(
    entered: mpsc::SyncSender<()>,
    release: mpsc::Receiver<()>,
) -> impl Fn(&mut Connection) -> Result<(), WorkerError> + Send + Sync {
    let release = Mutex::new(release);
    move |connection| {
        entered.send(()).expect("entered");
        release.lock().expect("gate").recv().expect("release");
        schema(connection)
    }
}

#[test]
fn shared_writer_serves_ready_databases_round_robin() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let events = Arc::new(Mutex::new(Vec::new()));
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let a_events = Arc::clone(&events);
    let a = worker
        .register(directory.path().join("a.sqlite"), move |connection| {
            entered.send(()).expect("entered");
            gate.lock().expect("gate").recv().expect("release");
            schema(connection)?;
            let events = Arc::clone(&a_events);
            connection.update_hook(Some(move |_: Action, _: &str, _: &str, _: i64| {
                events.lock().expect("events").push('a');
            }))?;
            Ok(())
        })
        .expect("register");
    let b_events = Arc::clone(&events);
    let b = worker
        .register(directory.path().join("b.sqlite"), move |connection| {
            schema(connection)?;
            let events = Arc::clone(&b_events);
            connection.update_hook(Some(move |_: Action, _: &str, _: &str, _: i64| {
                events.lock().expect("events").push('b');
            }))?;
            Ok(())
        })
        .expect("register");
    let first = runtime.block_on(a.submit(batch())).expect("accept first");
    waiting.recv().expect("initializer blocked");
    let receipts = runtime.block_on(async {
        vec![
            a.submit(batch()).await,
            a.submit(batch()).await,
            b.submit(batch()).await,
            b.submit(batch()).await,
        ]
    });
    release.send(()).expect("release");
    runtime.block_on(async {
        first.committed().await.expect("commit");
        for receipt in receipts {
            receipt
                .expect("accepted")
                .committed()
                .await
                .expect("commit");
        }
    });
    assert_eq!(*events.lock().expect("events"), ['a', 'b', 'a', 'b', 'a']);
    assert_eq!(
        finish(&mut worker).unpersisted,
        [] as [crate::storage_worker::UnpersistedWrite; 0]
    );
}

#[test]
fn full_database_queue_awaits_capacity_without_dropping_accepted_work() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig {
        queue_capacity: 1,
        ..WorkerConfig::default()
    })
    .expect("worker");
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let writer = worker
        .register(
            directory.path().join("queue.sqlite"),
            blocked(entered, gate),
        )
        .expect("register");
    let first = runtime
        .block_on(writer.submit(batch()))
        .expect("accept first");
    waiting.recv().expect("initializer blocked");
    let second = runtime
        .block_on(writer.submit(batch()))
        .expect("accept second");
    let mut third = pin!(writer.submit(batch()));
    assert!(runtime.block_on(poll_fn(|context| Poll::Ready(
        third.as_mut().poll(context).is_pending()
    ))));
    release.send(()).expect("release");
    let third = runtime
        .block_on(third)
        .expect("accept third after capacity");
    runtime.block_on(async {
        first.committed().await.expect("first committed");
        second.committed().await.expect("second committed");
        third.committed().await.expect("third committed");
    });
    assert_eq!(
        finish(&mut worker).unpersisted,
        [] as [crate::storage_worker::UnpersistedWrite; 0]
    );
}

#[test]
fn database_failure_rejects_its_queue_but_other_databases_continue() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let bad = worker
        .register(directory.path().join("bad.sqlite"), move |_| {
            entered.send(()).expect("entered");
            gate.lock().expect("gate").recv().expect("release");
            Err(WorkerError::Initialization)
        })
        .expect("register bad");
    let good = worker
        .register(directory.path().join("good.sqlite"), schema)
        .expect("register good");
    let first = runtime
        .block_on(bad.submit(batch()))
        .expect("first accepted");
    waiting.recv().expect("bad initializer blocked");
    let queued = runtime
        .block_on(bad.submit(batch()))
        .expect("queued accepted");
    let healthy = runtime
        .block_on(good.submit(batch()))
        .expect("healthy accepted");
    release.send(()).expect("release");
    runtime.block_on(async {
        assert_eq!(first.committed().await, Err(WorkerError::Initialization));
        assert_eq!(queued.committed().await, Err(WorkerError::DatabaseClosed));
        assert!(!healthy.committed().await.expect("healthy commit").durable);
        assert_eq!(bad.commit(batch()).await, Err(WorkerError::DatabaseClosed));
    });
    assert_eq!(finish(&mut worker).failures.len(), 1);
}

#[test]
fn deadline_reports_inflight_and_queued_without_late_success() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let writer = worker
        .register(
            directory.path().join("deadline.sqlite"),
            blocked(entered, gate),
        )
        .expect("register");
    let first = runtime
        .block_on(writer.submit(batch()))
        .expect("first accepted");
    waiting.recv().expect("initializer blocked");
    let second = runtime
        .block_on(writer.submit(batch()))
        .expect("second accepted");
    let report = worker.shutdown(Instant::now()).expect("deadline report");
    assert!(!report.finished);
    assert_eq!(
        report.unpersisted,
        vec![
            UnpersistedWrite {
                database_id: writer.database_id(),
                command_id: first.command_id,
                disposition: WriteDisposition::OutcomeUnknown
            },
            UnpersistedWrite {
                database_id: writer.database_id(),
                command_id: second.command_id,
                disposition: WriteDisposition::NotStarted
            },
        ]
    );
    assert_eq!(
        runtime.block_on(writer.submit(batch())).err(),
        Some(WorkerError::AdmissionClosed)
    );
    release.send(()).expect("release");
    worker.join().expect("join");
    runtime.block_on(async {
        assert_eq!(first.committed().await, Err(WorkerError::ShutdownDeadline));
        assert!(matches!(
            second.committed().await,
            Err(WorkerError::DatabaseClosed | WorkerError::ShutdownDeadline)
        ));
    });
}

#[test]
fn state_commit_is_full_and_domain_rejection_does_not_close_database() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let (mut worker, writer) = StorageWorker::state(
        directory.path().join("state.sqlite"),
        schema,
        WorkerConfig::default(),
    )
    .expect("state worker");
    runtime.block_on(async {
        let result: Result<(), WorkerError> = writer
            .write(|_| Err(WorkerError::TransactionRejected))
            .await;
        assert_eq!(result, Err(WorkerError::TransactionRejected));
        assert!(writer.commit(batch()).await.expect("FULL commit").durable);
        let rows: i64 = writer
            .write::<_, WorkerError, _>(|transaction| {
                Ok(transaction.query_row("SELECT COUNT(*) FROM fixture", [], |row| row.get(0))?)
            })
            .await
            .expect("typed state transaction");
        assert_eq!(rows, 1);
    });
    finish(&mut worker);
}

#[test]
fn newer_state_schema_is_refused_before_persistent_pragmas() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("future.sqlite");
    let connection = Connection::open(&path).expect("create future file");
    connection
        .pragma_update(None, "user_version", 9)
        .expect("future version");
    connection.close().expect("close");
    let before = std::fs::read(&path).expect("read before");
    let result = StorageWorker::state(
        path.clone(),
        |_| {
            Err(WorkerError::NewerSchema {
                found: 9,
                supported: 1,
            })
        },
        WorkerConfig::default(),
    );
    assert!(matches!(
        result,
        Err(WorkerError::NewerSchema {
            found: 9,
            supported: 1
        })
    ));
    assert_eq!(std::fs::read(path).expect("read after"), before);
}

#[test]
fn oversized_statement_rolls_back_without_acknowledgement() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("rows.sqlite");
    let runtime = runtime();
    let (mut worker, writer) = StorageWorker::state(path.clone(), |connection| {
        schema(connection)?;
        connection.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<501) INSERT INTO fixture SELECT x FROM n")?;
        Ok(())
    }, WorkerConfig::default()).expect("state worker");
    let update =
        SqlBatch::new(vec![SqlMutation::new("UPDATE fixture SET value=0", vec![])]).expect("batch");
    assert_eq!(
        runtime.block_on(writer.commit(update)),
        Err(WorkerError::RowLimit)
    );
    finish(&mut worker);
    let connection = Connection::open(path).expect("open");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM fixture WHERE value=0", [], |row| row
                .get::<_, i64>(
                0
            ))
            .expect("count"),
        0
    );
}

#[test]
fn shared_registration_is_lazy_and_can_rebuild_unknown_schema() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("cache.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let writer = worker.register(path.clone(), schema).expect("register");
    assert!(!path.exists());
    runtime
        .block_on(writer.commit(batch()))
        .expect("lazy commit");
    assert!(path.exists());
    finish(&mut worker);
}
