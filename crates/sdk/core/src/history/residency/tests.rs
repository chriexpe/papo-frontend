//! Deterministic idle count/byte eviction and capability pinning; cache hydration is integration-tested.

use super::*;
use papo_api::Uuid;

#[tokio::test]
async fn count_and_byte_limits_evict_only_idle_models_and_keep_recent_channels() {
    let store = HistoryStore::default();
    for id in 0..500 {
        let channel = Arc::new(Channel::new(
            Uuid::from_u128(id),
            store.changes.clone(),
            Arc::clone(&store.counters),
        ));
        channel
            .last_access
            .store(u64::try_from(id).unwrap(), Ordering::Relaxed);
        channel.state.lock().await.encoded_bytes = 100;
        store.channels.lock().await.insert(channel.id, channel);
    }
    let protected = store
        .channels
        .lock()
        .await
        .get(&Uuid::nil())
        .cloned()
        .unwrap();
    store
        .trim(HistoryBudget {
            idle_channels: 8,
            idle_bytes: 300,
        })
        .await;
    let report = store.diagnostics().await;
    assert_eq!(
        (
            report.pinned_channels,
            report.idle_channels,
            report.idle_bytes
        ),
        (1, 3, 300)
    );
    assert_eq!(report.work.evictions, 496);
    assert!(
        store
            .channels
            .lock()
            .await
            .contains_key(&Uuid::from_u128(499))
    );
    drop(protected);
    store
        .trim(HistoryBudget {
            idle_channels: 0,
            idle_bytes: 0,
        })
        .await;
    assert!(store.channels.lock().await.is_empty());
}

#[tokio::test]
async fn raw_receiver_pins_the_model_after_its_last_handle_is_dropped() {
    let store = HistoryStore::default();
    let channel = Arc::new(Channel::new(
        Uuid::nil(),
        store.changes.clone(),
        Arc::clone(&store.counters),
    ));
    let state = channel.state.lock().await;
    channel.publish(&state, crate::Generation(1)).unwrap();
    drop(state);
    let subscription = channel.subscribe().unwrap();
    store.channels.lock().await.insert(channel.id, channel);
    store
        .trim(HistoryBudget {
            idle_channels: 0,
            idle_bytes: 0,
        })
        .await;
    assert_eq!(store.diagnostics().await.pinned_channels, 1);
    drop(subscription);
    store
        .trim(HistoryBudget {
            idle_channels: 0,
            idle_bytes: 0,
        })
        .await;
    assert!(store.channels.lock().await.is_empty());
}
