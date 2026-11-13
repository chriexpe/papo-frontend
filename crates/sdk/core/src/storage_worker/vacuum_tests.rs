//! Physical reclamation steps every bounded vacuum page; row deletion alone is insufficient.

use super::*;

#[test]
fn one_reclamation_turn_steps_all_sixty_four_free_pages() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.sqlite");
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let writer = worker
        .register(path.clone(), |connection| {
            connection.execute_batch("CREATE TABLE entries(payload BLOB)")?;
            let tx = connection.transaction()?;
            for _ in 0..200 {
                tx.execute("INSERT INTO entries VALUES(zeroblob(3000))", [])?;
            }
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        writer
            .cache_write(|tx| -> Result<(), WorkerError> {
                tx.execute("DELETE FROM entries", [])?;
                Ok(())
            })
            .await
            .unwrap();
        let free_pages = || {
            rusqlite::Connection::open(&path)
                .unwrap()
                .pragma_query_value::<i64, _>(None, "freelist_count", |row| row.get(0))
                .unwrap()
        };
        let before = free_pages();
        assert!(before >= 64);
        writer.reclaim().await.unwrap();
        assert_eq!(before - free_pages(), 64);
        writer.close().await.unwrap();
    });
    worker
        .shutdown(std::time::Instant::now() + std::time::Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}
