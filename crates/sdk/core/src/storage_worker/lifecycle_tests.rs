//! Deterministic per-database lifecycle tests; global drain contracts remain in writer tests.

use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::{Mutex, mpsc},
    task::Poll,
};

use rusqlite::Connection;

use super::{
    tests::{batch, finish, runtime, schema},
    *,
};

fn assert_pending<F: Future>(runtime: &tokio::runtime::Runtime, future: std::pin::Pin<&mut F>) {
    let mut future = future;
    assert!(runtime.block_on(poll_fn(|context| Poll::Ready(
        future.as_mut().poll(context).is_pending()
    ))));
}

#[test]
fn close_fences_clones_and_capacity_waiters_then_drains_accepted_writes() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("closing.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig {
        queue_capacity: 1,
        ..WorkerConfig::default()
    })
    .expect("worker");
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
    let other = worker
        .register(directory.path().join("other.sqlite"), schema)
        .expect("other");
    let peer = writer.clone();
    let first = runtime.block_on(writer.submit(batch())).expect("accepted");
    waiting.recv().expect("initializer blocked");
    let second = runtime.block_on(writer.submit(batch())).expect("queued");
    let mut blocked_write = pin!(peer.submit(batch()));
    assert_pending(&runtime, blocked_write.as_mut());
    let mut blocked_checkpoint = pin!(peer.checkpoint());
    assert_pending(&runtime, blocked_checkpoint.as_mut());
    let mut close = pin!(writer.close());
    assert_pending(&runtime, close.as_mut());
    let mut concurrent_close = pin!(peer.close());
    assert_pending(&runtime, concurrent_close.as_mut());
    assert_eq!(
        runtime.block_on(blocked_write).err(),
        Some(WorkerError::DatabaseClosed)
    );
    assert_eq!(
        runtime.block_on(blocked_checkpoint),
        Err(WorkerError::DatabaseClosed)
    );
    assert_eq!(
        runtime.block_on(peer.commit(batch())),
        Err(WorkerError::DatabaseClosed)
    );
    assert_eq!(
        worker.register(path.clone(), schema).err(),
        Some(WorkerError::AlreadyRegistered)
    );
    let healthy = runtime
        .block_on(other.submit(batch()))
        .expect("other accepts during close");
    release.send(()).expect("release");
    runtime.block_on(async {
        first.committed().await.expect("inflight committed");
        healthy
            .committed()
            .await
            .expect("other commits during close");
        second.committed().await.expect("queued committed");
        close.await.expect("closed");
        concurrent_close.await.expect("same close");
        writer.close().await.expect("repeated close");
    });
    let connection = Connection::open(&path).expect("open after close");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM fixture", [], |row| row
                .get::<_, i64>(0))
            .expect("count"),
        2
    );
    connection.close().expect("close reader");
    assert_eq!(finish(&mut worker).failures, [] as [DatabaseFailure; 0]);
}

#[test]
fn closed_path_can_be_registered_again_without_reopening_old_handles() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("replace.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let old = worker.register(path.clone(), schema).expect("register");
    let other = worker
        .register(directory.path().join("other.sqlite"), schema)
        .expect("other");
    runtime.block_on(old.commit(batch())).expect("commit");
    runtime.block_on(old.close()).expect("close");
    std::fs::remove_file(&path).expect("delete only after close");
    let replacement = worker
        .register(path.clone(), schema)
        .expect("register replacement");
    assert_ne!(old.database_id(), replacement.database_id());
    runtime.block_on(async {
        old.close().await.expect("old remains closed");
        assert_eq!(old.commit(batch()).await, Err(WorkerError::DatabaseClosed));
        assert_eq!(old.checkpoint().await, Err(WorkerError::DatabaseClosed));
        assert!(!path.exists());
        replacement
            .commit(batch())
            .await
            .expect("replacement writes");
        other.commit(batch()).await.expect("other database writes");
        replacement.close().await.expect("replacement closes");
        other.commit(batch()).await.expect("other survives close");
    });
    assert_eq!(finish(&mut worker).failures, [] as [DatabaseFailure; 0]);
}

#[test]
fn closing_unopened_registration_does_not_create_a_database() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("lazy.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let writer = worker
        .register(path.clone(), |_| panic!("must not initialize"))
        .expect("register");
    runtime.block_on(writer.close()).expect("close");
    assert!(!path.exists());
    let registrar = worker.registrar();
    registrar(path, std::sync::Arc::new(schema)).expect("replacement");
    assert_eq!(finish(&mut worker).failures, [] as [DatabaseFailure; 0]);
}

#[test]
fn close_checkpoint_failure_is_typed_stable_and_drops_writer_connection() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("busy.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let writer = worker.register(path.clone(), schema).expect("register");
    runtime
        .block_on(writer.commit(batch()))
        .expect("first commit");
    let reader = Connection::open(&path).expect("reader");
    reader
        .execute_batch("BEGIN; SELECT * FROM fixture;")
        .expect("pin WAL snapshot");
    runtime
        .block_on(writer.commit(batch()))
        .expect("second commit");
    let expected = Err(WorkerError::Sqlite {
        code: Some(rusqlite::ffi::SQLITE_BUSY),
    });
    runtime.block_on(async {
        assert_eq!(writer.close().await, expected);
        assert_eq!(writer.close().await, expected);
        assert_eq!(
            writer.commit(batch()).await,
            Err(WorkerError::DatabaseClosed)
        );
    });
    // A surviving writer would retain WAL sidecars after this last reader closes.
    reader.execute_batch("ROLLBACK").expect("release snapshot");
    reader.close().expect("close reader");
    assert!(!path.with_file_name("busy.sqlite-wal").exists());
    let replacement = worker
        .register(path, schema)
        .expect("register after failed close");
    runtime
        .block_on(replacement.commit(batch()))
        .expect("replacement works");
    assert_eq!(finish(&mut worker).failures.len(), 1);
}

#[test]
fn passive_checkpoint_preserves_active_read_snapshot_and_database_admission() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("checkpoint.sqlite");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).expect("worker");
    let writer = worker.register(path.clone(), schema).expect("register");
    runtime
        .block_on(writer.commit(batch()))
        .expect("first commit");
    let reader = Connection::open(path).expect("reader");
    reader
        .execute_batch("BEGIN; SELECT * FROM fixture;")
        .expect("pin WAL snapshot");
    runtime.block_on(async {
        writer.commit(batch()).await.expect("second commit");
        writer
            .checkpoint()
            .await
            .expect("passive checkpoint does not wait for reader");
        writer.commit(batch()).await.expect("still open");
    });
    assert_eq!(
        reader
            .query_row("SELECT COUNT(*) FROM fixture", [], |row| row
                .get::<_, i64>(0))
            .expect("old snapshot"),
        1
    );
    reader.execute_batch("ROLLBACK").expect("release snapshot");
    reader.close().expect("close reader");
    runtime.block_on(writer.close()).expect("close");
    assert_eq!(finish(&mut worker).failures, [] as [DatabaseFailure; 0]);
}

#[test]
fn close_drains_checkpoint_admitted_before_its_fence() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let mut worker = StorageWorker::shared(WorkerConfig {
        queue_capacity: 1,
        ..WorkerConfig::default()
    })
    .expect("worker");
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = Mutex::new(gate);
    let writer = worker
        .register(
            directory.path().join("queued-checkpoint.sqlite"),
            move |connection| {
                entered.send(()).expect("entered");
                gate.lock().expect("gate").recv().expect("release");
                schema(connection)
            },
        )
        .expect("register");
    let first = runtime.block_on(writer.submit(batch())).expect("accepted");
    waiting.recv().expect("initializer blocked");
    let mut checkpoint = pin!(writer.checkpoint());
    assert_pending(&runtime, checkpoint.as_mut());
    assert_eq!(writer.capacity.available_permits(), 0);
    let mut close = pin!(writer.close());
    assert_pending(&runtime, close.as_mut());
    release.send(()).expect("release");
    runtime.block_on(async {
        first.committed().await.expect("inflight committed");
        checkpoint.await.expect("accepted checkpoint drained");
        close.await.expect("close completed");
    });
    assert_eq!(finish(&mut worker).failures, [] as [DatabaseFailure; 0]);
}

#[test]
fn cancelling_started_close_does_not_restore_admission_or_cancel_drain() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("cancelled.sqlite");
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
    let accepted = runtime.block_on(writer.submit(batch())).expect("accepted");
    waiting.recv().expect("initializer blocked");
    {
        let mut cancelled = pin!(writer.close());
        assert_pending(&runtime, cancelled.as_mut());
    }
    assert_eq!(
        runtime.block_on(writer.commit(batch())),
        Err(WorkerError::DatabaseClosed)
    );
    release.send(()).expect("release");
    runtime.block_on(async {
        accepted.committed().await.expect("accepted write drains");
        writer
            .close()
            .await
            .expect("close completes after cancellation");
    });
    worker
        .register(path, schema)
        .expect("replacement after completed close");
    assert_eq!(finish(&mut worker).failures, [] as [DatabaseFailure; 0]);
}

#[test]
fn shutdown_failure_completes_close_after_dropping_inflight_connection() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("shutdown.sqlite");
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
    let accepted = runtime.block_on(writer.submit(batch())).expect("accepted");
    waiting.recv().expect("initializer blocked");
    let mut close = pin!(writer.close());
    assert_pending(&runtime, close.as_mut());
    worker.begin_shutdown(std::time::Instant::now());
    assert_pending(&runtime, close.as_mut());
    release.send(()).expect("release");
    runtime.block_on(async {
        assert_eq!(
            accepted.committed().await,
            Err(WorkerError::ShutdownDeadline)
        );
        assert_eq!(close.await, Err(WorkerError::ShutdownDeadline));
        assert_eq!(writer.close().await, Err(WorkerError::ShutdownDeadline));
    });
    worker.join().expect("join");
    assert!(!path.with_file_name("shutdown.sqlite-wal").exists());
}

#[test]
fn state_writer_rejects_rebuildable_lifecycle_operations() {
    let directory = tempfile::tempdir().expect("directory");
    let runtime = runtime();
    let (mut worker, writer) = StorageWorker::state(
        directory.path().join("state.sqlite"),
        schema,
        WorkerConfig::default(),
    )
    .expect("state");
    runtime.block_on(async {
        assert_eq!(writer.close().await, Err(WorkerError::InvalidConfiguration));
        assert_eq!(
            writer.checkpoint().await,
            Err(WorkerError::InvalidConfiguration)
        );
        assert!(
            writer
                .commit(batch())
                .await
                .expect("state still writable")
                .durable
        );
    });
    assert_eq!(finish(&mut worker).failures, [] as [DatabaseFailure; 0]);
}
