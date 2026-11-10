//! Visibility manifests and keyset batches larger than one storage turn; fixtures live in the parent.

use super::*;

#[test]
fn large_visibility_manifests_stage_and_revoke_in_bounded_turns() {
    run(|caches, _| async move {
        let cache = caches.open(account()).await.unwrap();
        let mut resources = ResourceSnapshot::empty(GENERATION);
        resources.channels = (1..=600)
            .map(|id| Channel {
                id: Some(Uuid::from_u128(id).to_string()),
                ..Channel::default()
            })
            .collect::<Vec<_>>()
            .into();
        cache
            .update(&resources, &[CacheResource::Channels])
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
            .update(&resources, &[CacheResource::Channels])
            .await
            .unwrap();
        for (channel, expected) in [(1_u128, sets[0]), (600, sets[599])] {
            cache
                .history_validate(channel.to_be_bytes(), expected, GENERATION)
                .await
                .unwrap();
        }
        resources.channels = Vec::new().into();
        cache
            .update(&resources, &[CacheResource::Channels])
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
