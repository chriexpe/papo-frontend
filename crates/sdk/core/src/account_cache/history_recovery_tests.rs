//! Crash recovery, revocation and incarnation isolation; shared deterministic fixtures live in the parent.

use super::*;

#[test]
fn corrupt_cache_rebuilds_resources_and_history_together() {
    run(|caches, root| async move {
        let (cache, _) = populated(&caches, 1).await;
        assert!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_some()
        );
        caches.close(account()).await.unwrap();
        std::fs::write(root.account_cache(account()), b"not a sqlite database").unwrap();
        let rebuilt = caches.open(account()).await.unwrap();
        assert_eq!(rebuilt.load(GENERATION).await.unwrap().channels.len(), 0);
        assert!(
            rebuilt
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        let count: i64 = rebuilt
            .history_transaction(GENERATION, |transaction| {
                transaction
                    .query_row("SELECT count(*) FROM history_sets", [], |row| row.get(0))
                    .map_err(WorkerError::from)
                    .map_err(CacheError::from)
            })
            .await
            .unwrap();
        assert_eq!(count, 0);
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn crash_reopen_resumes_partial_purge_and_invalidates_persisted_verification() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1_200).await;
        cache.history_revoke(CHANNEL, GENERATION).await.unwrap();
        let changed = cache
            .history_transaction(GENERATION, super::history_purge::turn)
            .await
            .unwrap();
        assert!(changed > 0 && changed <= 100);
        let remaining: i64 = cache
            .history_transaction(GENERATION, move |transaction| {
                transaction
                    .query_row(
                        "SELECT count(*) FROM messages WHERE set_id=?1",
                        [set_id],
                        |row| row.get(0),
                    )
                    .map_err(WorkerError::from)
                    .map_err(CacheError::from)
            })
            .await
            .unwrap();
        assert!(remaining > 0 && remaining < 1_200);
        caches.close(account()).await.unwrap();
        let reopened = caches.open(account()).await.unwrap();
        assert!(
            reopened
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        let remaining: i64 = reopened
            .history_transaction(GENERATION, |transaction| {
                transaction
                    .query_row("SELECT count(*) FROM history_sets", [], |row| row.get(0))
                    .map_err(WorkerError::from)
                    .map_err(CacheError::from)
            })
            .await
            .unwrap();
        assert_eq!(remaining, 0);
        let fresh = reopened
            .history_authorize(CHANNEL, GENERATION, false)
            .await
            .unwrap();
        assert!(fresh > set_id);
        caches.close(account()).await.unwrap();
        let reopened = caches.open(account()).await.unwrap();
        assert!(
            reopened
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            reopened
                .history_authorize(CHANNEL, GENERATION, false)
                .await
                .unwrap(),
            fresh
        );
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn regrant_and_oversized_reset_never_mix_incarnations_before_purge() {
    run(|caches, _| async move {
        let (cache, old) = populated(&caches, 900).await;
        cache.history_revoke(CHANNEL, GENERATION).await.unwrap();
        let fresh = cache
            .history_authorize(CHANNEL, GENERATION, false)
            .await
            .unwrap();
        assert!(fresh > old);
        assert_eq!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .unwrap()
                .rows,
            []
        );
        cache
            .history_commit(
                CHANNEL,
                fresh,
                GENERATION,
                vec![7],
                vec![row(0)],
                Vec::new(),
            )
            .await
            .unwrap();
        cache.history_purge().await.unwrap();
        let data = cache
            .history_load(CHANNEL, GENERATION)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(data.rows.len(), 1);
        assert_eq!(data.metadata, vec![7]);
        let reset = cache
            .history_authorize(CHANNEL, GENERATION, true)
            .await
            .unwrap();
        assert!(reset > fresh);
        assert_eq!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .unwrap()
                .rows,
            []
        );
        caches.close(account()).await.unwrap();
    });
}

#[test]
fn atomic_manifest_disappearance_blocks_revival_before_cleanup() {
    run(|caches, _| async move {
        let (cache, old) = populated(&caches, 1).await;
        // Simulate the exact commit/crash boundary before post-publication cleanup.
        cache.history_transaction(GENERATION, |transaction| {
            transaction.execute("UPDATE cache_metadata SET channels_revision=2,history_revision=2,next_revision=3", []).map_err(WorkerError::from)?;
            Ok(())
        }).await.unwrap();
        assert!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        cache
            .history_transaction(GENERATION, |transaction| {
                transaction
                    .execute(
                        "INSERT INTO channels (revision,id,position,payload) VALUES (3,?1,0,X'')",
                        [CHANNEL.as_slice()],
                    )
                    .map_err(WorkerError::from)?;
                transaction
                    .execute("UPDATE cache_metadata SET next_revision=4", [])
                    .map_err(WorkerError::from)?;
                Ok(())
            })
            .await
            .unwrap();
        super::history_manifest::stage(&cache.context(), 3, 3, 0, GENERATION)
            .await
            .unwrap();
        cache
            .history_transaction(GENERATION, |transaction| {
                transaction
                    .execute(
                        "UPDATE cache_metadata SET channels_revision=3,history_revision=3",
                        [],
                    )
                    .map_err(WorkerError::from)?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        let fresh = cache
            .history_authorize(CHANNEL, GENERATION, false)
            .await
            .unwrap();
        assert!(fresh > old);
        assert_eq!(
            cache
                .history_load(CHANNEL, GENERATION)
                .await
                .unwrap()
                .unwrap()
                .rows,
            []
        );
        caches.close(account()).await.unwrap();
    });
}
