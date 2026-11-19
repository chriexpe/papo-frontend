//! Layout compatibility and guards against real account-cache SQLite: an older layout rebuilds
//! only its own channel's history set, a repeated identity is rejected before any row is
//! written, and stored identities that could be reissued are reported as corrupt. Includes the
//! one interleaving where cold retention retires a set between an open's authorize and load.

use std::sync::Arc;

use papo_api::{HistoryOrder, Uuid};

use super::{
    HistoryError,
    cache_fixture::{CHANNEL, cache, epoch, merge, messages, open, persist, point},
    persisted_layout::{LAYOUT_VERSION, stored_version, structural_layout},
    persistence::{State, Stored, decode},
    retention::Retainer,
};
use crate::{
    AccountCache, CacheError, Generation,
    account_cache::history::{HistoryData, HistoryRow, HistoryTable},
    storage_tests::Fixture,
};

/// A channel whose set was written by a build before layout versioning.
const OUTDATED: u128 = 800_002;

fn bytes(channel: u128) -> [u8; 16] {
    *Uuid::from_u128(channel).as_bytes()
}

/// Current-format history for `CHANNEL`: two runs, as after an evict→refetch.
async fn current_history(cache: &Arc<AccountCache>) -> super::persistence::State {
    let mut state = open(cache, CHANNEL, 1).await;
    merge(
        &mut state.model,
        &messages((21..=40).rev()),
        None,
        HistoryOrder::Desc,
        true,
        1,
    );
    merge(
        &mut state.model,
        &messages([1]),
        Some(point(10)),
        HistoryOrder::Desc,
        false,
        1,
    );
    persist(&mut state, cache, 1).await;
    state
}

/// The poisoned version-1 shape the old identities could commit: two active runs with one
/// message-derived identity, one surviving chunk row.
async fn outdated_history(cache: &AccountCache) -> i64 {
    let state = open(cache, OUTDATED, 1).await;
    let set_id = state.set_id.unwrap();
    let id: Vec<u8> = Uuid::from_u128(1).as_bytes().to_vec();
    let metadata = serde_json::to_vec(&serde_json::json!({
        "revision": 2,
        "empty_generation": 0,
        "active_chunks": [id, id],
    }))
    .unwrap();
    let row = HistoryRow {
        table: HistoryTable::Chunks,
        id: *Uuid::from_u128(1).as_bytes(),
        payload: b"{}".to_vec(),
    };
    cache
        .history_commit(
            bytes(OUTDATED),
            set_id,
            Generation(1),
            metadata,
            vec![row],
            Vec::new(),
        )
        .await
        .unwrap();
    set_id
}

async fn stored(cache: &AccountCache, channel: u128) -> Option<HistoryData> {
    cache.retention_data(bytes(channel)).await.unwrap()
}

/// The resources and the current-format channel are what they were.
async fn assert_account_cache_intact(cache: &AccountCache, current: &super::persistence::State) {
    let resources = cache.load(Generation(1)).await.unwrap();
    assert_eq!(resources.channels.len(), 2);
    let reloaded = open(cache, CHANNEL, 1).await;
    assert_eq!(reloaded.set_id, current.set_id);
    assert_eq!(reloaded.model.order, current.model.order);
}

#[test]
fn an_outdated_layout_rebuilds_only_its_own_channel_on_open() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL, OUTDATED]).await;
        let current = current_history(&cache).await;
        let old_set = outdated_history(&cache).await;
        let data = stored(&cache, OUTDATED).await.unwrap();
        assert!(matches!(decode(data), Ok(Stored::Outdated)));
        // Opening the channel replaces its set instead of failing as corrupt data.
        let rebuilt = open(&cache, OUTDATED, 1).await;
        assert_ne!(rebuilt.set_id, Some(old_set));
        assert!(rebuilt.model.chunks.is_empty() && rebuilt.model.entries.is_empty());
        let fresh = stored(&cache, OUTDATED).await.unwrap();
        assert_eq!(Some(fresh.set_id), rebuilt.set_id);
        assert!(fresh.metadata.is_empty() && fresh.rows.is_empty());
        assert_account_cache_intact(&cache, &current).await;
    });
}

#[test]
fn cold_retention_retires_an_outdated_set_and_leaves_other_channels() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL, OUTDATED]).await;
        let current = current_history(&cache).await;
        outdated_history(&cache).await;
        let mut retainer = Retainer::new(Arc::clone(&cache), Uuid::from_u128(OUTDATED));
        assert!(retainer.evict(None).await.unwrap().is_none());
        assert!(stored(&cache, OUTDATED).await.is_none());
        let rebuilt = open(&cache, OUTDATED, 1).await;
        assert_eq!(rebuilt.model.chunks, Vec::new());
        assert_account_cache_intact(&cache, &current).await;
    });
}

#[test]
fn retiring_a_replaced_set_keeps_its_replacement() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL, OUTDATED]).await;
        let old_set = outdated_history(&cache).await;
        let rebuilt = open(&cache, OUTDATED, 1).await;
        // A maintenance pass that loaded the old set before the open replaced it.
        let clock = cache.retention_version(bytes(OUTDATED));
        cache
            .retention_retire(bytes(OUTDATED), old_set)
            .await
            .unwrap();
        let kept = stored(&cache, OUTDATED).await.unwrap();
        assert_eq!(Some(kept.set_id), rebuilt.set_id);
        // Retiring nothing does not supersede the replacement's in-flight writes.
        assert_eq!(cache.retention_version(bytes(OUTDATED)), clock);
    });
}

#[test]
fn a_repeated_identity_is_rejected_before_any_row_is_written() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let mut state = current_history(&cache).await;
        let before = stored(&cache, CHANNEL).await.unwrap();
        // The newer run has no older gap and the older run no newer one, so their gap rows
        // cannot collide: only the identity guard stands between them and an overwrite.
        assert!(state.model.chunks[0].oldest_complete && !state.model.chunks[0].newest_complete);
        assert!(!state.model.chunks[1].oldest_complete && state.model.chunks[1].newest_complete);
        state.model.chunks[1].id = state.model.chunks[0].id;
        assert!(matches!(
            structural_layout(&state.model),
            Err(HistoryError::Layout)
        ));
        let result = state
            .persist(&cache, bytes(CHANNEL), &super::cache_fixture::epoch(1))
            .await;
        assert!(matches!(result, Err(HistoryError::Layout)));
        let after = stored(&cache, CHANNEL).await.unwrap();
        assert_eq!(after.metadata, before.metadata);
        assert_eq!(
            after
                .rows
                .iter()
                .map(|row| (row.id, &row.payload))
                .collect::<Vec<_>>(),
            before
                .rows
                .iter()
                .map(|row| (row.id, &row.payload))
                .collect::<Vec<_>>(),
        );
        assert!(matches!(
            crate::ClientError::from(HistoryError::Layout),
            crate::ClientError::HistoryLayout
        ));
    });
}

/// Rewrites one metadata field of a stored current-format set.
fn with_metadata(mut data: HistoryData, edit: impl FnOnce(&mut serde_json::Value)) -> HistoryData {
    let mut metadata: serde_json::Value = serde_json::from_slice(&data.metadata).unwrap();
    edit(&mut metadata);
    data.metadata = serde_json::to_vec(&metadata).unwrap();
    data
}

fn corrupt(data: HistoryData) -> bool {
    matches!(
        decode(data),
        Err(HistoryError::Cache(CacheError::InvalidData))
    )
}

#[test]
fn stored_identities_that_repeat_or_could_be_reissued_are_corrupt() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL]).await;
        let mut state = current_history(&cache).await;
        let data = stored(&cache, CHANNEL).await.unwrap();
        assert!(matches!(decode(data.clone()), Ok(Stored::Current(_))));
        // An active identity above the high-water mark would be issued to a later run.
        assert!(corrupt(with_metadata(data.clone(), |metadata| {
            metadata["chunk_high_water"] = 1.into();
        })));
        // One identity listed in place of another: the neighbour links no longer match.
        assert!(corrupt(with_metadata(data, |metadata| {
            let first = metadata["active_chunks"][0].clone();
            metadata["active_chunks"][1] = first;
        })));
        // One run listed twice: its row and links still match, so only the metadata says so.
        merge(
            &mut state.model,
            &messages(2..=21),
            Some(point(1)),
            HistoryOrder::Asc,
            true,
            1,
        );
        assert_eq!(state.model.chunks.len(), 1);
        persist(&mut state, &cache, 1).await;
        let single = stored(&cache, CHANNEL).await.unwrap();
        assert!(matches!(decode(single.clone()), Ok(Stored::Current(_))));
        assert!(corrupt(with_metadata(single, |metadata| {
            let only = metadata["active_chunks"][0].clone();
            metadata["active_chunks"].as_array_mut().unwrap().push(only);
        })));
    });
}

/// `ensure` authorizes and loads in two cache transactions, releasing the shared cache fence in
/// between, while cold retention holds no channel lock (it runs only without a live model). So
/// a retire can land between the two. The fixture runtime is single-threaded: one manual poll
/// of `ensure` takes the free fence and spawns its authorize transaction, which cannot run until
/// this task yields. Retention, awaited next, queues behind that transaction on the fence (FIFO),
/// so the order is exactly authorize → retention load → retire → `ensure`'s load.
#[test]
fn a_set_retired_between_authorize_and_load_supersedes_the_open_and_rebuilds_on_retry() {
    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = cache(&fixture, &[CHANNEL, OUTDATED]).await;
        let current = current_history(&cache).await;
        let old_set = outdated_history(&cache).await;
        let instance = cache.history_instance();
        let clock = cache.retention_version(bytes(OUTDATED));

        let mut opening = State::default();
        let mut ensure = Box::pin(opening.ensure(&cache, bytes(OUTDATED), Generation(1)));
        let first = std::future::poll_fn(|cx| std::task::Poll::Ready(ensure.as_mut().poll(cx)));
        assert!(
            first.await.is_pending(),
            "authorize is spawned, not yet run"
        );
        let mut retainer = Retainer::new(Arc::clone(&cache), Uuid::from_u128(OUTDATED));
        assert!(retainer.evict(None).await.unwrap().is_none());
        assert_eq!(cache.retention_version(bytes(OUTDATED)), clock + 1);
        assert!(
            stored(&cache, OUTDATED).await.is_none(),
            "the authorized set was retired"
        );

        // The open finds its authorized set gone: a lost race, not corrupt data.
        let result = ensure.await;
        assert!(matches!(
            result,
            Err(HistoryError::Cache(CacheError::Superseded))
        ));
        assert!(matches!(
            crate::ClientError::from(result.unwrap_err()),
            crate::ClientError::Cache(CacheError::Superseded)
        ));
        // It failed in its load: it adopted no set and allocated no replacement.
        assert!(stored(&cache, OUTDATED).await.is_none());
        assert_eq!(opening.set_id, None);
        assert!(matches!(
            opening.persist(&cache, bytes(OUTDATED), &epoch(1)).await,
            Err(HistoryError::Unverified)
        ));
        // A writer still holding the retired set and the pre-retire clock is refused.
        let stale = cache
            .history_commit_versioned(
                bytes(OUTDATED),
                old_set,
                (Generation(1), tokio_util::sync::CancellationToken::new()),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Some((clock, 0)),
            )
            .await;
        assert_eq!(stale, Err(CacheError::Superseded));

        // The next authorized open starts a fresh version-2 set and works normally.
        let mut rebuilt = open(&cache, OUTDATED, 1).await;
        assert!(rebuilt.set_id.is_some_and(|set| set != old_set));
        assert_eq!(rebuilt.model.chunks, Vec::new());
        let ticket = super::merge::Ticket {
            channel: bytes(OUTDATED),
            generation: 1,
            revision: rebuilt.model.revision,
            cursor: None,
            order: HistoryOrder::Desc,
            limit: 100,
        };
        let page: Vec<_> = (1..=3)
            .rev()
            .map(|index| {
                let mut message = super::cache_fixture::message(index);
                message.channel_id = Some(Uuid::from_u128(OUTDATED).to_string());
                message
            })
            .collect();
        rebuilt.model.merge_page(&ticket, &page, false).unwrap();
        rebuilt
            .persist(&cache, bytes(OUTDATED), &epoch(1))
            .await
            .unwrap();
        let data = stored(&cache, OUTDATED).await.unwrap();
        assert_eq!(stored_version(&data.metadata).unwrap(), LAYOUT_VERSION);
        let reloaded = open(&cache, OUTDATED, 1).await;
        assert_eq!(reloaded.model.order, rebuilt.model.order);

        // A late, stale retire of the old set neither hides the replacement nor ticks the clock.
        let clock = cache.retention_version(bytes(OUTDATED));
        cache
            .retention_retire(bytes(OUTDATED), old_set)
            .await
            .unwrap();
        assert_eq!(stored(&cache, OUTDATED).await.unwrap().set_id, data.set_id);
        assert_eq!(cache.retention_version(bytes(OUTDATED)), clock);

        // No account-wide invalidation: same cache instance, resources and other channel intact.
        assert_eq!(cache.history_instance(), instance);
        assert_account_cache_intact(&cache, &current).await;
    });
}
