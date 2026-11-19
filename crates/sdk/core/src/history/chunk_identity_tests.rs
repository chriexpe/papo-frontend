//! Chunk identities against real account-cache SQLite: the two evict→refetch collisions now
//! persist and reload, unions keep one existing identity whatever the UUID order, and
//! anchor-only runs keep distinct identities through materialization. Reuse is covered in
//! `chunk_reuse_tests`.

use std::{collections::BTreeSet, sync::Arc};

use papo_api::{
    HistoryOrder, Uuid,
    gateway::{KnownEvent, MessageEvent},
    wire::Message,
};

use super::{
    HistorySnapshot,
    cache_fixture::{
        CHANNEL, assert_distinct, assert_reloads, cache, ids, merge, message, message_at, messages,
        open, persist, point,
    },
    channel::Channel,
    chunk_id::ChunkId,
    message_key::MessageKey,
    ordering::Point,
    persistence::State,
    retention::Retainer,
};
use crate::{AccountCache, Generation, storage_tests::Fixture};

fn snapshot(state: &State, generation: u64) -> Arc<HistorySnapshot> {
    let channel = Channel::new(
        Uuid::from_u128(CHANNEL),
        tokio::sync::watch::channel(()).0,
        Arc::default(),
    );
    channel.publish(state, Generation(generation)).unwrap();
    channel.subscribe().unwrap().snapshot
}

fn key(index: u128) -> MessageKey {
    point(index).key().unwrap()
}

/// Persists 1..=40 from `initial`, evicts the oldest twenty cold, reloads, and refetches the
/// evicted first message from unheld cursor 10 without bridging to the survivor at 21.
async fn evict_then_refetch_first(
    cache: &Arc<AccountCache>,
    initial: (Vec<Message>, HistoryOrder, bool),
) -> (State, ChunkId) {
    let mut state = open(cache, CHANNEL, 1).await;
    let (page, order, has_more) = initial;
    merge(&mut state.model, &page, None, order, has_more, 1);
    persist(&mut state, cache, 1).await;
    let before = state.model.chunks[0].id;
    let mut retainer = Retainer::new(Arc::clone(cache), Uuid::from_u128(CHANNEL));
    assert!(retainer.evict(None).await.unwrap().is_some());
    let mut state = open(cache, CHANNEL, 1).await;
    assert_eq!(state.model.chunks[0].first, point(21));
    // The surviving run keeps its identity although its first message moved.
    assert_eq!(state.model.chunks[0].id, before);
    merge(
        &mut state.model,
        &messages([1]),
        Some(point(10)),
        HistoryOrder::Desc,
        false,
        1,
    );
    assert_eq!(state.model.chunks.len(), 2);
    assert_distinct(&state.model);
    assert_eq!(state.model.chunks[1].id, before);
    assert!(state.model.chunks[0].id > before);
    (state, before)
}

#[test]
fn refetched_first_message_beside_an_incomplete_newest_edge_persists_and_reloads() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let initial = (messages(1..=40), HistoryOrder::Asc, true);
        let (mut state, _) = evict_then_refetch_first(&cache, initial).await;
        // Formerly rejected before write: both runs emitted the same ascending gap key.
        persist(&mut state, &cache, 1).await;
        let restored = assert_reloads(&state, &cache, 1).await;
        assert_eq!(restored.model.order.len(), 21);
        let snapshot = snapshot(&restored, 1);
        assert_eq!(snapshot.messages.len(), 21);
        let coverage = snapshot.coverage().unwrap();
        assert!(
            !coverage
                .range(Some(key(1)), Some(key(21)))
                .unwrap()
                .complete
        );
        let gaps: Vec<_> = snapshot
            .gaps
            .iter()
            .map(|gap| (gap.order, gap.cursor.clone()))
            .collect();
        assert!(gaps.contains(&(HistoryOrder::Asc, point(1).cursor())));
        assert!(gaps.contains(&(HistoryOrder::Desc, point(21).cursor())));
    });
}

#[test]
fn refetched_first_message_beside_a_complete_newest_edge_persists_reloads_and_joins() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let initial = (messages((1..=40).rev()), HistoryOrder::Desc, false);
        let (mut state, survivor) = evict_then_refetch_first(&cache, initial).await;
        // Formerly committed with one chunk row for two active identities.
        persist(&mut state, &cache, 1).await;
        let data = cache
            .retention_data(*Uuid::from_u128(CHANNEL).as_bytes())
            .await
            .unwrap()
            .unwrap();
        let chunk_rows: BTreeSet<_> = data
            .rows
            .iter()
            .filter(|row| row.table == crate::account_cache::history::HistoryTable::Chunks)
            .map(|row| row.id)
            .collect();
        assert!(chunk_rows.len() >= 2);
        for id in ids(&state.model) {
            assert!(chunk_rows.contains(&id.row_key()));
        }
        let mut restored = assert_reloads(&state, &cache, 1).await;
        assert_eq!(snapshot(&restored, 1).messages.len(), 21);
        // A page bridging both runs joins them under the identity issued first.
        merge(
            &mut restored.model,
            &messages(2..=21),
            Some(point(1)),
            HistoryOrder::Asc,
            true,
            1,
        );
        assert_eq!(ids(&restored.model), vec![survivor]);
        persist(&mut restored, &cache, 1).await;
        let joined = assert_reloads(&restored, &cache, 1).await;
        assert_eq!(joined.model.order.len(), 40);
    });
}

#[test]
fn union_keeps_the_identity_issued_first_whatever_the_uuid_order() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let mut state = open(&cache, CHANNEL, 1).await;
        // UUIDs fall as time rises, so the newest message has the smallest UUID.
        let at = |index: u128| message_at(index, 1_000 - index, false);
        let page = |range: std::ops::RangeInclusive<u128>| -> Vec<Message> {
            range.rev().map(at).collect()
        };
        merge(
            &mut state.model,
            &page(11..=15),
            None,
            HistoryOrder::Desc,
            true,
            1,
        );
        let newest = state.model.chunks[0].id;
        let older_cursor = Point::from(&at(5).cursor().unwrap());
        merge(
            &mut state.model,
            &page(1..=4),
            Some(older_cursor),
            HistoryOrder::Desc,
            true,
            1,
        );
        let oldest = state.model.chunks[0].id;
        assert_ne!(oldest, newest);
        // Bridging page 5..=10 from the newest run's first message joins all three.
        let bridge = Point::from(&at(11).cursor().unwrap());
        merge(
            &mut state.model,
            &page(5..=10),
            Some(bridge),
            HistoryOrder::Desc,
            true,
            1,
        );
        assert_eq!(ids(&state.model), vec![newest]);
        assert_eq!(state.model.chunk_ids.high_water(), 3);
        persist(&mut state, &cache, 1).await;
        let restored = assert_reloads(&state, &cache, 1).await;
        assert_eq!(restored.model.order.len(), 15);
    });
}

fn live(message: &Message) -> KnownEvent {
    KnownEvent::Message(Box::new(MessageEvent {
        id: message.id.clone().unwrap(),
        channel_id: message.channel_id.clone().unwrap(),
        created_at: message.created_at.clone().unwrap(),
        ..MessageEvent::default()
    }))
}

#[test]
fn anchor_only_runs_and_their_materialization_keep_distinct_identities() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let mut state = open(&cache, CHANNEL, 1).await;
        merge(
            &mut state.model,
            &messages([7, 6, 5]),
            None,
            HistoryOrder::Desc,
            true,
            1,
        );
        let run = state.model.chunks[0].id;
        // An empty older page from unheld cursor 3 proves (-inf, 3) as an anchor-only run.
        merge(
            &mut state.model,
            &[],
            Some(point(3)),
            HistoryOrder::Desc,
            false,
            1,
        );
        assert_eq!(state.model.chunks.len(), 2);
        assert!(state.model.chunks[0].is_anchor_only());
        let proof = state.model.chunks[0].id;
        assert_ne!(proof, run);
        persist(&mut state, &cache, 1).await;
        let mut restored = assert_reloads(&state, &cache, 1).await;
        // The anchor's own message arrives late: the cursor becomes a held boundary and
        // the run keeps its identity.
        restored.model.event(&live(&message(3)), 0).unwrap();
        assert!(!restored.model.chunks[0].is_anchor_only());
        assert_eq!(ids(&restored.model), vec![proof, run]);
        // An empty newer proof from unheld cursor 4 reaches the held run and joins it.
        merge(
            &mut restored.model,
            &[],
            Some(point(4)),
            HistoryOrder::Asc,
            false,
            1,
        );
        assert_distinct(&restored.model);
        assert!(ids(&restored.model).contains(&run));
        persist(&mut restored, &cache, 1).await;
        assert_reloads(&restored, &cache, 1).await;
    });
}
