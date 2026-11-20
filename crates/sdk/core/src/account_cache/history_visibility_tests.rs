//! Visibility manifests and keyset batches larger than one storage turn; fixtures live in the parent.

use super::*;

#[test]
fn large_visibility_manifests_stage_and_revoke_in_bounded_turns() {
    run(|caches, _| async move {
        let cache = caches.open(account()).await.unwrap();
        let mut resources = ResourceSnapshot::empty(GENERATION);
        for id in 1..=500 {
            let channel = crate::account_cache::resource_fixtures::channel(Uuid::from_u128(id));
            resources.channels.set(channel.id, channel);
        }
        let peer = Uuid::from_u128(601);
        for id in 501..=600 {
            let dm = crate::DirectRecord {
                id: Uuid::from_u128(id),
                peer_id: peer,
                created_at: String::new(),
                last_read_message: None,
                last_read_at: None,
                last_message: None,
                unread_count: 0,
            };
            resources.dms.set(dm.id, dm);
        }
        cache
            .update(&resources, &[CacheResource::Channels, CacheResource::Dms])
            .await
            .unwrap();
        let mut sets = Vec::new();
        for id in 1_u128..=600 {
            sets.push(
                cache
                    .history_authorize(id.to_be_bytes(), GENERATION, false)
                    .await
                    .unwrap(),
            );
        }
        cache
            .update(&resources, &[CacheResource::Channels, CacheResource::Dms])
            .await
            .unwrap();
        for (channel, expected) in [(1_u128, sets[0]), (600, sets[599])] {
            cache
                .history_validate(channel.to_be_bytes(), expected, GENERATION)
                .await
                .unwrap();
        }
        // A new process may start its generation counter at the same value. Every
        // persisted verification claim must be cleared before the cache becomes usable.
        caches.close(account()).await.unwrap();
        let cache = caches.open(account()).await.unwrap();
        assert!(
            cache
                .history_validate(600_u128.to_be_bytes(), sets[599], GENERATION)
                .await
                .is_err()
        );
        let claims: i64 = cache
            .readers
            .query(cache.path.clone(), |tx| {
                tx.query_row(
                    "SELECT COUNT(*) FROM history_channels WHERE verified_generation IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(claims, 0);
        resources.channels = crate::ResourceTable::default();
        resources.dms = crate::ResourceTable::default();
        cache
            .update(&resources, &[CacheResource::Channels, CacheResource::Dms])
            .await
            .unwrap();
        assert!(
            cache
                .history_load(600_u128.to_be_bytes(), GENERATION)
                .await
                .unwrap()
                .is_none()
        );
        let count: i64 = cache
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
fn keyset_reads_return_all_rows_across_bounded_batches() {
    run(|caches, _| async move {
        let (cache, set_id) = populated(&caches, 1_200).await;
        let data = cache
            .history_load(CHANNEL, GENERATION)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(data.set_id, set_id);
        assert_eq!(data.rows.len(), 1_200);
        assert_eq!(data.rows.first().unwrap().id, 0_u128.to_be_bytes());
        assert_eq!(data.rows.last().unwrap().id, 1_199_u128.to_be_bytes());
        caches.close(account()).await.unwrap();
    });
}
