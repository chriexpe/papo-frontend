//! Deterministic prefix/gap/pin/tombstone preservation and reload after physical row eviction.

use super::super::{
    model::{Chunk, Entry},
    ordering::Point,
};
use super::*;
use crate::{
    Generation, ResourceSnapshot, Uuid,
    storage_tests::{Fixture, account, address},
};

fn point(index: u128) -> Point {
    Point {
        created_at: "2026-01-01T00:00:00.123456Z".into(),
        id: Uuid::from_u128(index).to_string(),
    }
}
fn model(count: u128) -> Model {
    let mut model = Model {
        revision: 1,
        ..Model::default()
    };
    for index in 1..=count {
        let point = point(index);
        let id = *Uuid::from_u128(index).as_bytes();
        let mut entry = Entry::empty(1);
        entry.point = Some(point.clone());
        entry.message = Some(crate::Message {
            id: Some(point.id.clone()),
            channel_id: Some(Uuid::from_u128(800_000).to_string()),
            created_at: Some(point.created_at.clone()),
            ..crate::Message::default()
        });
        entry.timeline = true;
        model.entries.insert(id, entry);
        model.dirty.insert(id);
    }
    model.chunks.push(Chunk {
        id: *Uuid::from_u128(1).as_bytes(),
        first: point(1),
        last: point(count),
        oldest_complete: true,
        newest_complete: true,
        generation: 1,
        live_generation: 1,
    });
    model.rebuild_order().unwrap();
    model
}

#[test]
fn prefix_retirement_is_bounded_and_does_not_cross_an_existing_gap() {
    let mut model = model(100);
    model.chunks[0].last = point(5);
    model.chunks[0].newest_complete = false;
    model.chunks.push(Chunk {
        id: *Uuid::from_u128(11).as_bytes(),
        first: point(11),
        last: point(100),
        oldest_complete: false,
        newest_complete: true,
        generation: 1,
        live_generation: 1,
    });
    for index in 6..=10 {
        model.entries.remove(Uuid::from_u128(index).as_bytes());
    }
    model.rebuild_order().unwrap();
    assert_eq!(model.evict_prefix().unwrap().len(), 5);
    assert_eq!(model.order.len(), 90);
    assert_eq!(model.chunks.len(), 1);
    assert_eq!(model.chunks[0].first.id, point(11).id);
    assert!(!model.chunks[0].oldest_complete);
    assert_eq!(model.empty_generation, 0);
}

#[test]
fn physical_prefix_eviction_reloads_with_detached_pin_real_tombstone_and_exact_gap() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let generation = Generation(77);
        let cache = fixture
            .storage()
            .account_caches()
            .open(account(&address(1), 1))
            .await
            .unwrap();
        let channel = Uuid::from_u128(800_000);
        let mut resources = ResourceSnapshot::empty(generation);
        resources.channels = vec![crate::Channel {
            id: Some(channel.to_string()),
            ..crate::Channel::default()
        }]
        .into();
        cache.replace(&resources).await.unwrap();
        let mut state = State::default();
        state
            .ensure(&cache, *channel.as_bytes(), generation)
            .await
            .unwrap();
        state.model = model(100);
        state
            .model
            .entries
            .get_mut(Uuid::from_u128(1).as_bytes())
            .unwrap()
            .pinned = true;
        let deleted = state
            .model
            .entries
            .get_mut(Uuid::from_u128(2).as_bytes())
            .unwrap();
        deleted.deleted = true;
        deleted.timeline = false;
        deleted.message = None;
        state
            .model
            .record_deletions(1, vec![*Uuid::from_u128(2).as_bytes()]);
        state.model.rebuild_order().unwrap();
        let epoch = Epoch {
            generation,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        state
            .persist(&cache, *channel.as_bytes(), &epoch)
            .await
            .unwrap();
        let epoch = Epoch {
            generation: Generation(0),
            cancel: epoch.cancel,
        };
        assert!(
            state
                .evict(&cache, *channel.as_bytes(), &epoch)
                .await
                .unwrap()
        );
        assert!(state.last_write_rows <= 80);
        let loaded = cache
            .retention_data(*channel.as_bytes())
            .await
            .unwrap()
            .unwrap();
        let restored = super::super::persistence::decode(loaded).unwrap();
        let pin = &restored.model.entries[Uuid::from_u128(1).as_bytes()];
        assert!(pin.pinned && !pin.timeline && pin.message.is_some());
        assert!(restored.model.entries[Uuid::from_u128(2).as_bytes()].deleted);
        assert!(!restored.model.chunks[0].oldest_complete);
        assert_eq!(restored.model.order.len(), 79);
        // A later burst with no live session must load the opaque cache without granting authority.
        let mut cold = Retainer::new(Arc::clone(&cache), channel);
        assert!(cold.evict(None).await.unwrap().is_some());
        let data = cache
            .retention_data(*channel.as_bytes())
            .await
            .unwrap()
            .unwrap();
        let cold_restored = super::super::persistence::decode(data).unwrap();
        assert_eq!(cold_restored.model.order.len(), 59);
        assert!(cold_restored.model.entries[Uuid::from_u128(1).as_bytes()].pinned);
        assert!(cold_restored.model.entries[Uuid::from_u128(2).as_bytes()].deleted);
        assert!(!cold_restored.model.chunks[0].oldest_complete);
        let cursor = restored.model.chunks[0].first.clone();
        let page = restored
            .model
            .page(Some(&cursor), papo_api::HistoryOrder::Desc, 30, 77)
            .unwrap();
        assert_eq!(page.messages.len(), 0);
        assert!(page.has_more);
        assert_eq!(page.gap.unwrap().cursor.id, cursor.id);
    });
}
