//! Real account-cache SQLite for history persistence tests: deterministic messages, page
//! merges, and open/persist/reload of one channel's history set. Assertions live in the tests.

use std::{collections::BTreeSet, sync::Arc};

use papo_api::{HistoryOrder, Uuid, wire::Message};

use super::{chunk_id::ChunkId, merge::Ticket, model::Model, ordering::Point, persistence::State};
use crate::{
    AccountCache, Generation, ResourceSnapshot,
    generation::Epoch,
    storage_tests::{Fixture, account, address},
};

pub(super) const CHANNEL: u128 = 800_000;
const BASE_US: i64 = 1_767_225_600_000_000;

/// Message `index` with UUID `id`, `index` µs after the base time. Equal-time messages are
/// ordered by UUID; distinct times let a test make UUID order disagree with time order.
pub(super) fn message_at(index: u128, id: u128, same_time: bool) -> Message {
    let offset = if same_time {
        0
    } else {
        i64::try_from(index).unwrap()
    };
    Message {
        id: Some(Uuid::from_u128(id).to_string()),
        channel_id: Some(Uuid::from_u128(CHANNEL).to_string()),
        created_at: Some(
            chrono::DateTime::from_timestamp_micros(BASE_US + offset)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        ),
        content: Some(String::new()),
        ..Message::default()
    }
}

/// The adjudication scenario's messages: one exact timestamp, ordered by UUID `index`.
pub(super) fn message(index: u128) -> Message {
    message_at(index, index, true)
}

pub(super) fn point(index: u128) -> Point {
    Point::from(&message(index).cursor().unwrap())
}

pub(super) fn merge(
    model: &mut Model,
    messages: &[Message],
    cursor: Option<Point>,
    order: HistoryOrder,
    has_more: bool,
    generation: u64,
) {
    let ticket = Ticket {
        channel: *Uuid::from_u128(CHANNEL).as_bytes(),
        generation,
        revision: model.revision,
        cursor,
        order,
        limit: 100,
    };
    model.merge_page(&ticket, messages, has_more).unwrap();
}

pub(super) fn messages(indexes: impl IntoIterator<Item = u128>) -> Vec<Message> {
    indexes.into_iter().map(message).collect()
}

pub(super) fn epoch(generation: u64) -> Epoch {
    Epoch {
        generation: Generation(generation),
        cancel: tokio_util::sync::CancellationToken::new(),
    }
}

/// A cache whose resources make `channels` visible.
pub(super) async fn cache(fixture: &Fixture, channels: &[u128]) -> Arc<AccountCache> {
    let cache = fixture
        .storage()
        .account_caches()
        .open(account(&address(1), 1))
        .await
        .unwrap();
    let mut resources = ResourceSnapshot::empty(Generation(1));
    resources.channels =
        crate::ResourceSnapshot::text_channels(channels.iter().copied().map(Uuid::from_u128));
    cache.replace(&resources).await.unwrap();
    cache
}

/// A new process's view of the channel: authorized and loaded from SQLite.
pub(super) async fn open(cache: &AccountCache, channel: u128, generation: u64) -> State {
    let mut state = State::default();
    state
        .ensure(
            cache,
            *Uuid::from_u128(channel).as_bytes(),
            Generation(generation),
        )
        .await
        .unwrap();
    state
}

pub(super) async fn persist(state: &mut State, cache: &AccountCache, generation: u64) {
    state
        .persist(
            cache,
            *Uuid::from_u128(CHANNEL).as_bytes(),
            &epoch(generation),
        )
        .await
        .unwrap();
}

pub(super) fn ids(model: &Model) -> Vec<ChunkId> {
    model.chunks.iter().map(|chunk| chunk.id).collect()
}

pub(super) fn assert_distinct(model: &Model) {
    let distinct: BTreeSet<_> = ids(model).into_iter().collect();
    assert_eq!(
        distinct.len(),
        model.chunks.len(),
        "two runs share an identity"
    );
}

/// Reload equals the persisted model, ignoring the process-local generations.
pub(super) async fn assert_reloads(state: &State, cache: &AccountCache, generation: u64) -> State {
    let restored = open(cache, CHANNEL, generation).await;
    let mut expected = state.model.chunks.clone();
    for chunk in &mut expected {
        chunk.generation = 0;
        chunk.live_generation = 0;
    }
    assert_eq!(restored.model.chunks, expected);
    assert_eq!(restored.model.order, state.model.order);
    assert_eq!(
        restored.model.chunk_ids.high_water(),
        state.model.chunk_ids.high_water()
    );
    restored
}
