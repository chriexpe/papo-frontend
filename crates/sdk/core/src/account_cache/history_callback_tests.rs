//! Shared callback quota and execution-generation rollback; model payload semantics live elsewhere.

use super::*;

#[test]
fn canceled_request_epoch_is_rejected_without_persisting_a_delta() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1).await;
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let result = cache
            .history_commit_current(
                CHANNEL,
                set_id,
                (GENERATION, cancel),
                vec![2],
                vec![row(9)],
                Vec::new(),
            )
            .await;
        assert_eq!(result, Err(CacheError::Superseded));
        assert_eq!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .unwrap()
                .metadata,
            vec![1]
        );
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn request_canceled_inside_actual_callback_rolls_back_before_commit() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1).await;
        let cancel = tokio_util::sync::CancellationToken::new();
        let trigger = cancel.clone();
        let result = cache
            .history_transaction_current(GENERATION, cancel, move |transaction| {
                transaction
                    .execute(
                        "UPDATE history_sets SET metadata=X'02' WHERE id=?1",
                        [set_id],
                    )
                    .map_err(WorkerError::from)?;
                trigger.cancel();
                Ok(())
            })
            .await;
        assert_eq!(result, Err(CacheError::Superseded));
        assert_eq!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .unwrap()
                .metadata,
            vec![1]
        );
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn cache_callback_row_overrun_rolls_back_the_entire_delta() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1).await;
        let result = cache
            .history_transaction(GENERATION, move |transaction| {
                for id in 100_u128..601 {
                    transaction
                        .execute(
                            "INSERT INTO messages (set_id,id,payload) VALUES (?1,?2,X'')",
                            params![set_id, id.to_be_bytes().as_slice()],
                        )
                        .map_err(WorkerError::from)?;
                }
                Ok(())
            })
            .await;
        assert_eq!(result, Err(CacheError::Storage(WorkerError::RowLimit)));
        assert_eq!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .unwrap()
                .rows
                .len(),
            1
        );
        assert_eq!(
            caches.close(account()).await,
            Err(CacheError::Storage(WorkerError::RowLimit))
        );
    });
}

#[test]
fn cache_callback_revalidates_generation_before_commit() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1).await;
        let current = Arc::clone(&cache.generation);
        let result = cache
            .history_transaction(GENERATION, move |transaction| {
                transaction
                    .execute(
                        "UPDATE history_sets SET metadata=X'02' WHERE id=?1",
                        [set_id],
                    )
                    .map_err(WorkerError::from)?;
                current.store(5, Ordering::Release);
                Ok(())
            })
            .await;
        assert_eq!(result, Err(CacheError::Superseded));
        let metadata: Vec<u8> = cache
            .history_transaction(Generation(5), move |transaction| {
                transaction
                    .query_row(
                        "SELECT metadata FROM history_sets WHERE id=?1",
                        [set_id],
                        |row| row.get(0),
                    )
                    .map_err(WorkerError::from)
                    .map_err(CacheError::from)
            })
            .await
            .unwrap();
        assert_eq!(metadata, vec![1]);
        caches.close(account()).await.unwrap();
    });
}
