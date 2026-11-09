//! Channel-gated read-pool tests; no network, sleeps or elapsed-time assertions.

use std::{
    future::{Future, poll_fn},
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::Poll,
    time::{Duration, Instant},
};

use super::*;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn fixture(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path).expect("fixture");
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .expect("WAL");
    connection
        .execute_batch("CREATE TABLE fixture(value INTEGER); INSERT INTO fixture VALUES (1)")
        .expect("schema");
    connection
}

#[test]
fn reads_are_bounded_and_canceled_callers_do_not_release_active_permits() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("read.sqlite");
    let _connection = fixture(&path);
    let runtime = runtime();
    let pool = ReadPool::new(1).expect("pool");
    let first_pool = pool.clone();
    let first_path = path.clone();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, gate) = mpsc::sync_channel(1);
    let first = runtime.spawn(async move {
        first_pool
            .query(first_path, move |transaction| {
                let value: i64 =
                    transaction.query_row("SELECT value FROM fixture", [], |row| row.get(0))?;
                let _ = entered.send(());
                gate.recv().expect("release");
                Ok(value)
            })
            .await
    });
    runtime.block_on(waiting).expect("first query entered");
    first.abort();
    runtime.block_on(async {
        assert!(first.await.expect_err("caller canceled").is_cancelled());
    });
    let ran = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&ran);
    let mut second = pin!(pool.query(path, move |_| {
        observed.store(true, Ordering::SeqCst);
        Ok(())
    }));
    assert!(runtime.block_on(poll_fn(|context| Poll::Ready(
        second.as_mut().poll(context).is_pending()
    ))));
    assert!(!ran.load(Ordering::SeqCst));
    release.send(()).expect("release");
    runtime
        .block_on(second)
        .expect("second completes after first closes");
    assert!(ran.load(Ordering::SeqCst));
    assert!(pool.drain(Instant::now()).expect("drained"));
}

#[test]
fn drain_reports_active_queries_and_stops_new_admission() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("drain.sqlite");
    let _connection = fixture(&path);
    let runtime = runtime();
    let pool = ReadPool::default();
    let active_pool = pool.clone();
    let active_path = path.clone();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, gate) = mpsc::sync_channel(1);
    let active = runtime.spawn(async move {
        active_pool
            .query(active_path, move |_| {
                let _ = entered.send(());
                gate.recv().expect("release");
                Ok(())
            })
            .await
    });
    runtime.block_on(waiting).expect("query entered");
    assert!(!pool.drain(Instant::now()).expect("deadline report"));
    assert_eq!(
        runtime.block_on(pool.query(path, |_| Ok(()))),
        Err(WorkerError::AdmissionClosed)
    );
    release.send(()).expect("release");
    runtime.block_on(active).expect("task").expect("query");
    assert!(
        pool.drain(Instant::now() + Duration::from_secs(30))
            .expect("drained")
    );
}

#[test]
fn query_only_connection_rejects_writes_and_pragma_escape() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("readonly.sqlite");
    let connection = fixture(&path);
    let runtime = runtime();
    let pool = ReadPool::default();
    let write = runtime.block_on(pool.query(path.clone(), |transaction| {
        transaction.execute("DELETE FROM fixture", [])
    }));
    assert!(matches!(write, Err(WorkerError::Sqlite { .. })));
    let escape = runtime.block_on(pool.query(path, |transaction| {
        transaction.execute_batch("PRAGMA query_only=OFF")
    }));
    assert!(matches!(escape, Err(WorkerError::Sqlite { .. })));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM fixture", [], |row| row
                .get::<_, i64>(0))
            .expect("rows"),
        1
    );
}

#[test]
fn completed_query_does_not_pin_wal_snapshot() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("wal.sqlite");
    let connection = fixture(&path);
    let runtime = runtime();
    let pool = ReadPool::default();
    let rows: i64 = runtime
        .block_on(pool.query(path, |transaction| {
            transaction.query_row("SELECT COUNT(*) FROM fixture", [], |row| row.get(0))
        }))
        .expect("query");
    assert_eq!(rows, 1);
    connection
        .execute("INSERT INTO fixture VALUES (2)", [])
        .expect("write");
    let checkpoint: (i64, i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("checkpoint");
    assert_eq!(checkpoint, (0, 0, 0));
}

#[test]
fn canceled_read_retains_profile_lease_until_blocking_connection_closes() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("lease-read.sqlite");
    let _connection = fixture(&path);
    let lease = Arc::new(std::fs::File::create(directory.path().join("lease")).expect("lease"));
    let weak = Arc::downgrade(&lease);
    let pool = ReadPool::with_profile_lease(1, lease).expect("leased pool");
    let runtime = runtime();
    let active_pool = pool.clone();
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, gate) = mpsc::sync_channel(1);
    let active = runtime.spawn(async move {
        active_pool
            .query(path, move |_| {
                let _ = entered.send(());
                gate.recv().expect("release");
                Ok(())
            })
            .await
    });
    runtime.block_on(waiting).expect("query entered");
    active.abort();
    runtime.block_on(async {
        assert!(active.await.expect_err("canceled").is_cancelled());
    });
    drop(pool);
    assert!(weak.upgrade().is_some());
    release.send(()).expect("release");
    drop(runtime);
    assert!(weak.upgrade().is_none());
}
