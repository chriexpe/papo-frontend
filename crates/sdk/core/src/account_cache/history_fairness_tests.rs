//! Realistic history bursts on the shared writer; deterministic gates prove cross-account fairness.

use super::*;
use crate::storage_worker::{SqlBatch, SqlMutation};
use rusqlite::{hooks::Action, types::Value};
use std::sync::{Mutex as BlockingMutex, mpsc};

fn initialize_history(connection: &mut Connection, account: AccountId) -> Result<(), WorkerError> {
    schema::initialize(connection, account)?;
    connection.execute(
        "INSERT INTO history_sets (channel_id) VALUES (?1)",
        [CHANNEL.as_slice()],
    )?;
    Ok(())
}

/// A 100-message page: independent message and chunk-item upserts plus one metadata row.
fn page_batch(page: u128, count: u128) -> SqlBatch {
    let mut mutations = Vec::new();
    for offset in 0..count {
        let id = (page * 100 + offset).to_be_bytes().to_vec();
        for sql in [
            "INSERT INTO messages (set_id,id,payload) VALUES (1,?1,X'') ON CONFLICT(set_id,id) DO UPDATE SET payload=excluded.payload",
            "INSERT INTO chunk_items (set_id,id,payload) VALUES (1,?1,X'') ON CONFLICT(set_id,id) DO UPDATE SET payload=excluded.payload",
        ] {
            mutations.push(SqlMutation::new(sql, vec![Value::Blob(id.clone())]));
        }
    }
    mutations.push(SqlMutation::new(
        "UPDATE history_sets SET metadata=?1 WHERE id=1",
        vec![Value::Blob(page.to_be_bytes().to_vec())],
    ));
    SqlBatch::new(mutations).unwrap()
}

#[test]
fn ten_thousand_message_history_burst_yields_to_ready_account_after_one_turn() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let events = Arc::new(BlockingMutex::new(Vec::new()));
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = BlockingMutex::new(gate);
    let first_events = Arc::clone(&events);
    let first = worker
        .register(
            directory.path().join("history-a.sqlite"),
            move |connection| {
                entered.send(()).unwrap();
                gate.lock().unwrap().recv().unwrap();
                initialize_history(connection, account())?;
                let events = Arc::clone(&first_events);
                connection.update_hook(Some(move |_: Action, _: &str, table: &str, _: i64| {
                    if table == "history_sets" {
                        events.lock().unwrap().push('a');
                    }
                }))?;
                Ok(())
            },
        )
        .unwrap();
    let second_events = Arc::clone(&events);
    let other = AccountId::new(account().server(), Uuid::from_u128(2));
    let second = worker
        .register(
            directory.path().join("history-b.sqlite"),
            move |connection| {
                initialize_history(connection, other)?;
                let events = Arc::clone(&second_events);
                connection.update_hook(Some(move |_: Action, _: &str, table: &str, _: i64| {
                    if table == "history_sets" {
                        events.lock().unwrap().push('b');
                    }
                }))?;
                Ok(())
            },
        )
        .unwrap();

    let initial = runtime.block_on(first.submit(page_batch(0, 100))).unwrap();
    waiting.recv().unwrap();
    // The initializer gate holds the actual writer thread while all page commands
    // are admitted. Account B arrives last, behind 99 already queued A pages.
    let (pages, peer) = runtime.block_on(async {
        let mut pages = Vec::new();
        for page in 1..100 {
            pages.push(first.submit(page_batch(page, 100)).await.unwrap());
        }
        let peer = second.submit(page_batch(0, 1)).await.unwrap();
        (pages, peer)
    });
    release.send(()).unwrap();
    runtime.block_on(async {
        assert_eq!(initial.committed().await.unwrap().affected_rows, 201);
        assert_eq!(peer.committed().await.unwrap().affected_rows, 3);
        for page in pages {
            let committed = page.committed().await.unwrap();
            assert_eq!(committed.affected_rows, 201);
            assert!(!committed.durable);
        }
    });
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 101);
    assert_eq!(events[0..2], ['a', 'b']);
    assert!(events[2..].iter().all(|account| *account == 'a'));
    drop(events);

    runtime.block_on(async {
        first.close().await.unwrap();
        second.close().await.unwrap();
    });
    let connection = Connection::open(directory.path().join("history-a.sqlite")).unwrap();
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap(),
        10_000
    );
    assert_eq!(
        connection
            .query_row::<i64, _, _>("SELECT count(*) FROM chunk_items", [], |row| row.get(0))
            .unwrap(),
        10_000
    );
    let report = worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    assert_eq!(report.unpersisted.len(), 0);
    worker.join().unwrap();
}
