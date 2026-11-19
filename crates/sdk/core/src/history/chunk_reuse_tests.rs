//! No chunk identity is ever issued twice in a history set: live and late arrivals get fresh
//! ones, retired ones never return after retention, reconnect, reload or a failed persist, and
//! an exhausted allocator fails before a mutation changes anything.

use std::{collections::BTreeSet, sync::Arc};

use papo_api::{
    HistoryOrder, Uuid,
    gateway::{KnownEvent, MessageEvent},
    wire::Message,
};

use super::{
    HistoryError,
    cache_fixture::{
        CHANNEL, assert_distinct, assert_reloads, cache, epoch, ids, merge, message, messages,
        open, persist, point,
    },
    chunk_id::ChunkIds,
    retention::Retainer,
};
use crate::storage_tests::Fixture;

fn live(message: &Message) -> KnownEvent {
    KnownEvent::Message(Box::new(MessageEvent {
        id: message.id.clone().unwrap(),
        channel_id: message.channel_id.clone().unwrap(),
        created_at: message.created_at.clone().unwrap(),
        ..MessageEvent::default()
    }))
}

#[test]
fn live_and_late_arrivals_outside_every_run_get_fresh_identities() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let mut state = open(&cache, CHANNEL, 1).await;
        merge(
            &mut state.model,
            &messages([20, 19]),
            None,
            HistoryOrder::Desc,
            true,
            1,
        );
        merge(
            &mut state.model,
            &messages([10, 9]),
            Some(point(12)),
            HistoryOrder::Desc,
            true,
            1,
        );
        let before = state.model.chunk_ids.high_water();
        let live_run = state.model.chunks[1].id;
        // An arrival at the live edge extends that run under its identity.
        state.model.event(&live(&message(25)), 1).unwrap();
        assert_eq!(state.model.chunks.len(), 2);
        assert_eq!(state.model.chunks[1].id, live_run);
        assert_eq!(state.model.chunk_ids.high_water(), before);
        // A late delivery between the runs, and an arrival beyond the newest run in a
        // connection that has not proven its live edge, each start a run.
        state.model.event(&live(&message(15)), 1).unwrap();
        state.model.event(&live(&message(30)), 2).unwrap();
        assert_eq!(state.model.chunks.len(), 4);
        assert_distinct(&state.model);
        assert_eq!(state.model.chunk_ids.high_water(), before + 2);
        persist(&mut state, &cache, 1).await;
        assert_reloads(&state, &cache, 1).await;
    });
}

#[test]
fn retired_identities_are_never_reissued_after_retention_reconnect_or_reload() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let mut state = open(&cache, CHANNEL, 1).await;
        merge(
            &mut state.model,
            &messages((31..=40).rev()),
            None,
            HistoryOrder::Desc,
            true,
            1,
        );
        merge(
            &mut state.model,
            &messages((11..=20).rev()),
            Some(point(25)),
            HistoryOrder::Desc,
            true,
            1,
        );
        merge(
            &mut state.model,
            &messages((1..=5).rev()),
            Some(point(8)),
            HistoryOrder::Desc,
            false,
            1,
        );
        let issued: BTreeSet<_> = ids(&state.model).into_iter().collect();
        persist(&mut state, &cache, 1).await;
        // One retention turn stops at the oldest run's end: it retires that run's identity.
        let mut retainer = Retainer::new(Arc::clone(&cache), Uuid::from_u128(CHANNEL));
        assert!(retainer.evict(None).await.unwrap().is_some());
        // A restart and a new connection, then the evicted space is fetched again in pieces
        // that do not bridge to the survivors.
        let mut state = open(&cache, CHANNEL, 2).await;
        assert_eq!(state.model.chunks.len(), 2);
        assert!(ids(&state.model).iter().all(|id| issued.contains(id)));
        merge(
            &mut state.model,
            &messages((1..=5).rev()),
            Some(point(8)),
            HistoryOrder::Desc,
            false,
            2,
        );
        merge(
            &mut state.model,
            &messages([28]),
            Some(point(29)),
            HistoryOrder::Desc,
            true,
            2,
        );
        assert_distinct(&state.model);
        let fresh: Vec<_> = ids(&state.model)
            .into_iter()
            .filter(|id| !issued.contains(id))
            .collect();
        assert_eq!(
            fresh.len(),
            2,
            "refetched runs get identities never issued before"
        );
        assert!(fresh.iter().all(|id| id > issued.last().unwrap()));
        persist(&mut state, &cache, 2).await;
        let mut restored = assert_reloads(&state, &cache, 2).await;
        merge(
            &mut restored.model,
            &messages([45]),
            Some(point(44)),
            HistoryOrder::Asc,
            true,
            2,
        );
        let newest = restored.model.chunks.last().unwrap().id;
        assert!(newest > *fresh.iter().max().unwrap());
    });
}

#[test]
fn a_failed_persist_never_reissues_a_committed_identity() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let mut state = open(&cache, CHANNEL, 1).await;
        merge(
            &mut state.model,
            &messages([20, 19]),
            None,
            HistoryOrder::Desc,
            true,
            1,
        );
        persist(&mut state, &cache, 1).await;
        let committed = state.model.chunk_ids.high_water();
        // A run issued in memory, then a newer connection supersedes the write.
        merge(
            &mut state.model,
            &messages([5]),
            Some(point(6)),
            HistoryOrder::Desc,
            true,
            1,
        );
        let uncommitted = state.model.chunks[0].id;
        let mut newer = open(&cache, CHANNEL, 2).await;
        let failed = state
            .persist(&cache, *Uuid::from_u128(CHANNEL).as_bytes(), &epoch(1))
            .await;
        assert!(matches!(failed, Err(HistoryError::Cache(_))));
        state.reset();
        // The stored high-water mark is the last committed one; the next run continues after
        // it. Only the identity that never reached disk may be issued again.
        assert_eq!(newer.model.chunk_ids.high_water(), committed);
        merge(
            &mut newer.model,
            &messages([5]),
            Some(point(6)),
            HistoryOrder::Desc,
            true,
            2,
        );
        assert_distinct(&newer.model);
        assert_eq!(newer.model.chunks[0].id, uncommitted);
        assert!(ChunkIds::resume(committed).issued(newer.model.chunks[1].id));
        persist(&mut newer, &cache, 2).await;
        assert_reloads(&newer, &cache, 2).await;
    });
}

#[test]
fn an_exhausted_allocator_fails_before_the_model_changes() {
    let mut model = super::model::Model {
        chunk_ids: ChunkIds::resume(u64::MAX),
        ..super::model::Model::default()
    };
    let ticket = super::merge::Ticket {
        channel: *Uuid::from_u128(CHANNEL).as_bytes(),
        generation: 1,
        revision: 0,
        cursor: None,
        order: HistoryOrder::Desc,
        limit: 100,
    };
    let page = model.merge_page(&ticket, &messages([2, 1]), false);
    assert!(matches!(page, Err(HistoryError::Layout)));
    let arrival = model.event(&live(&message(3)), 1);
    assert!(matches!(arrival, Err(HistoryError::Layout)));
    assert_eq!(model.revision, 0);
    assert!(model.entries.is_empty() && model.order.is_empty() && model.chunks.is_empty());
}
