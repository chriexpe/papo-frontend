//! Generated coverage evidence: `range(..).complete` must be sound against the server and must
//! equal what the evidence the simulated client accepted proves, with gaps at the same places.
//!
//! The oracle (`oracle`) never consults the coverage code. It keeps each accepted page and live
//! arrival as a fact and decides completeness by brute force over a discrete key universe
//! (every message key plus a synthetic key between each pair), so the server's message list
//! alone can never make a range complete. `simulation` drives the model and the oracle.

mod identity;
mod oracle;
mod simulation;

use papo_api::{HistoryOrder, Uuid};
use proptest::{
    prelude::*,
    test_runner::{Config, RngAlgorithm, TestRng, TestRunner},
};

use crate::Generation;

use super::{message_key::MessageKey, ordering::Point};
use simulation::{
    CHANNEL, INITIAL_MESSAGES, Simulation, cursor_for, key, message, message_point, publish,
    server_page, synthetic_point, ticket,
};

#[test]
fn coverage_is_sound_and_matches_accepted_evidence() {
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: 128,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    let strategy = prop::collection::vec((0u8..10, any::<u8>(), any::<u8>(), 1usize..6), 1..64);
    runner
        .run(&strategy, |operations| {
            let mut simulation = Simulation::new();
            for (operation, selector, index, limit) in operations {
                let index = u128::from(index) % simulation.next_index;
                let cursor = cursor_for(selector, index);
                match operation {
                    0 | 1 => simulation.page(cursor.as_ref(), HistoryOrder::Desc, limit),
                    2 | 3 => simulation.page(cursor.as_ref(), HistoryOrder::Asc, limit),
                    4 => simulation.arrive(),
                    5 => simulation.delete(index),
                    6 => simulation.evict(),
                    7 => simulation.reconnect(u128::from(selector % 3)),
                    8 => simulation.reload(),
                    _ => simulation.page(None, HistoryOrder::Desc, limit),
                }
                simulation.check();
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn separately_fetched_messages_stay_incomplete_until_a_page_covers_the_space_between() {
    let mut simulation = Simulation::new();
    simulation.server.clear();
    let first = message_point(1);
    simulation.server.insert(key(&first), message(1));
    simulation.next_index = 6;
    simulation.page(None, HistoryOrder::Desc, 1);
    simulation.reconnect(0);
    let fifth = message_point(5);
    simulation.server.insert(key(&fifth), message(5));
    simulation.page(None, HistoryOrder::Desc, 1);
    simulation.check();
    let snapshot = publish(&simulation.model, simulation.generation);
    let coverage = snapshot.coverage().unwrap();
    assert!(
        !coverage
            .range(Some(key(&first)), Some(key(&fifth)))
            .unwrap()
            .complete
    );
    assert!(
        snapshot
            .gaps
            .iter()
            .any(|gap| gap.order == HistoryOrder::Asc && gap.cursor == first.cursor())
    );

    simulation.page(Some(&first), HistoryOrder::Asc, 1);
    simulation.check();
    let snapshot = publish(&simulation.model, simulation.generation);
    assert!(
        snapshot
            .coverage()
            .unwrap()
            .range(Some(key(&first)), Some(key(&fifth)))
            .unwrap()
            .complete
    );
}

#[test]
fn an_unheld_cursor_anchors_the_page_without_proving_itself() {
    let mut simulation = Simulation::new();
    let anchor = message_point(4);
    simulation.page(Some(&anchor), HistoryOrder::Asc, 3);
    simulation.check();
    let snapshot = publish(&simulation.model, simulation.generation);
    assert_eq!(snapshot.chunks.len(), 1);
    assert!(snapshot.chunks[0].oldest_anchor == Some(anchor.cursor()));
    let coverage = snapshot.coverage().unwrap();
    let seventh = key(&message_point(7));
    assert!(
        coverage
            .range(Some(key(&anchor)), Some(seventh))
            .unwrap()
            .complete
    );
    // Message 4 itself was never fetched, so a range spanning it stays unknown.
    assert!(
        !coverage
            .range(Some(key(&message_point(3))), Some(seventh))
            .unwrap()
            .complete
    );

    // Filling the published gap below fetches the anchor and joins the runs.
    let gap = snapshot
        .gaps
        .iter()
        .find(|gap| gap.order == HistoryOrder::Desc)
        .unwrap();
    simulation.page(Some(&Point::from(&gap.cursor)), HistoryOrder::Desc, 2);
    simulation.check();
    let snapshot = publish(&simulation.model, simulation.generation);
    assert_eq!(snapshot.chunks.len(), 1);
    assert!(snapshot.chunks[0].oldest_anchor.is_none());
}

#[test]
fn an_anchored_proof_is_restored_unvalidated_and_revalidated_under_chunk_rules() {
    let mut simulation = Simulation::new();
    simulation.generation = 5;
    let anchor = message_point(4);
    simulation.page(Some(&anchor), HistoryOrder::Asc, 3);
    // A restart decodes the cache: the anchor survives, its validation does not.
    let restored = persist_and_restore(std::mem::take(&mut simulation.model), 5);
    assert_eq!(restored.model.chunks.len(), 1);
    let chunk = &restored.model.chunks[0];
    assert_eq!(chunk.oldest_anchor.as_ref(), Some(&anchor));
    assert_eq!((chunk.generation, chunk.live_generation), (0, 0));
    let published = publish(&restored.model, 6);
    assert!(published.chunks[0].generation.is_none());
    let seventh = key(&message_point(7));
    let coverage = published.coverage().unwrap();
    // Interior and anchored proofs persist like any chunk boundary; the cursor itself never.
    assert!(
        coverage
            .range(Some(key(&anchor)), Some(seventh))
            .unwrap()
            .complete
    );
    assert!(
        !coverage
            .range(Some(key(&message_point(3))), Some(seventh))
            .unwrap()
            .complete
    );

    // The same page in the new generation revalidates the run and keeps its anchor.
    simulation.model = restored.model;
    simulation.generation = 6;
    simulation.page(Some(&anchor), HistoryOrder::Asc, 3);
    let chunk = &simulation.model.chunks[0];
    assert_eq!(simulation.model.chunks.len(), 1);
    assert_eq!(chunk.oldest_anchor.as_ref(), Some(&anchor));
    assert_eq!(chunk.generation, 6);
}

/// Persists `model` in a fresh account cache and decodes it back, as a restart would.
fn persist_and_restore(model: super::model::Model, generation: u64) -> super::persistence::State {
    use crate::{
        ResourceSnapshot,
        generation::Epoch,
        storage_tests::{Fixture, account, address},
    };

    let fixture = Fixture::new();
    fixture.runtime().block_on(async {
        let cache = fixture
            .storage()
            .account_caches()
            .open(account(&address(1), 1))
            .await
            .unwrap();
        let channel = Uuid::from_u128(CHANNEL);
        let mut resources = ResourceSnapshot::empty(Generation(generation));
        resources.channels = crate::ResourceSnapshot::text_channels([channel]);
        cache.replace(&resources).await.unwrap();
        let mut state = super::persistence::State::default();
        state
            .ensure(&cache, *channel.as_bytes(), Generation(generation))
            .await
            .unwrap();
        state.model = model;
        let epoch = Epoch {
            generation: Generation(generation),
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        state
            .persist(&cache, *channel.as_bytes(), &epoch)
            .await
            .unwrap();
        let data = cache
            .retention_data(*channel.as_bytes())
            .await
            .unwrap()
            .unwrap();
        super::persistence::decode(data).unwrap().expect_current()
    })
}

#[test]
fn a_late_event_for_an_anchored_cursor_turns_the_anchor_into_a_held_boundary() {
    let mut simulation = Simulation::new();
    let newest = INITIAL_MESSAGES - 1;
    let anchor = message_point(newest);
    // The newest message is not held yet: an empty `Asc` page from it anchors a proof.
    simulation.page(Some(&anchor), HistoryOrder::Asc, 3);
    assert!(simulation.model.chunks[0].is_anchor_only());
    // Its own gateway event arrives late; the message is now held.
    simulation.deliver_late(newest);
    simulation.check();
    assert_eq!(simulation.model.chunks.len(), 1);
    let chunk = &simulation.model.chunks[0];
    assert!(!chunk.is_anchor_only());
    assert!(chunk.oldest_anchor.is_none());

    let snapshot = publish(&simulation.model, simulation.generation);
    let coverage = snapshot.coverage().unwrap();
    assert_eq!(coverage.resolve(key(&anchor)).unwrap().index, Some(0));
    assert!(coverage.range(Some(key(&anchor)), None).unwrap().complete);
    let older = key(&message_point(newest - 1));
    assert!(!coverage.range(Some(older), None).unwrap().complete);
    assert_eq!(snapshot.gaps.len(), 1);
    assert_eq!(snapshot.gaps[0].order, HistoryOrder::Desc);
    assert!(snapshot.gaps[0].cursor == anchor.cursor());

    // A restart keeps the held boundary and starts a new connection generation.
    let restored =
        persist_and_restore(std::mem::take(&mut simulation.model), simulation.generation);
    assert!(!restored.model.chunks[0].is_anchor_only());
    assert_eq!(restored.model.order.len(), 1);
    simulation.model = restored.model;
    simulation.reconnect(0);
    simulation.check();
    // Retention evicts the message together with its run; nothing is left orphaned.
    simulation.evict();
    simulation.check();
    assert!(simulation.model.order.is_empty());
}

#[test]
fn unanchored_chunks_encode_and_decode_without_anchor_fields() {
    let chunk = serde_json::to_value(super::model::Chunk {
        id: super::chunk_id::ChunkIds::default().allocate().unwrap(),
        first: message_point(1),
        last: message_point(2),
        oldest_complete: false,
        newest_complete: false,
        generation: 0,
        live_generation: 0,
        oldest_anchor: None,
        newest_anchor: None,
    })
    .unwrap();
    // Absent anchors are not encoded, and a row without them decodes to unanchored.
    assert!(chunk.get("oldest_anchor").is_none() && chunk.get("newest_anchor").is_none());
    let decoded: super::model::Chunk = serde_json::from_value(chunk).unwrap();
    assert!(decoded.oldest_anchor.is_none() && decoded.newest_anchor.is_none());
}

#[test]
fn empty_pages_on_both_sides_of_an_unheld_cursor_never_prove_the_cursor() {
    let mut simulation = Simulation::new();
    simulation.server.clear();
    let only = message_point(5);
    simulation.server.insert(key(&only), message(5));
    simulation.next_index = 6;
    simulation.page(Some(&only), HistoryOrder::Asc, 1);
    simulation.page(Some(&only), HistoryOrder::Desc, 1);
    simulation.check();
    let snapshot = publish(&simulation.model, simulation.generation);
    let coverage = snapshot.coverage().unwrap();
    assert!(!coverage.range(None, None).unwrap().complete);
    let around = (key(&message_point(4)), key(&synthetic_point(5)));
    assert!(
        !coverage
            .range(Some(around.0), Some(around.1))
            .unwrap()
            .complete
    );
}

#[test]
fn a_page_beyond_a_lagging_gateway_retires_the_live_edge_instead_of_joining_it() {
    let mut simulation = Simulation::new();
    simulation.page(None, HistoryOrder::Desc, 3);
    // Messages reach the server but their gateway events have not arrived yet.
    for index in INITIAL_MESSAGES..INITIAL_MESSAGES + 6 {
        simulation
            .server
            .insert(key(&message_point(index)), message(index));
    }
    simulation.next_index = INITIAL_MESSAGES + 6;
    let (messages, has_more) = server_page(&simulation.server, None, HistoryOrder::Desc, 2);
    let request = ticket(&simulation.model, None, HistoryOrder::Desc, 1, 2);
    simulation
        .model
        .merge_page(&request, &messages, has_more)
        .unwrap();
    let snapshot = publish(&simulation.model, 1);
    let held_newest = key(&message_point(INITIAL_MESSAGES - 1));
    let fetched_oldest = key(&message_point(INITIAL_MESSAGES + 4));
    let coverage = snapshot.coverage().unwrap();
    assert!(
        !coverage
            .range(Some(held_newest), Some(fetched_oldest))
            .unwrap()
            .complete
    );
    assert!(
        snapshot
            .gaps
            .iter()
            .any(|gap| gap.order == HistoryOrder::Desc
                && MessageKey::from_cursor(&gap.cursor).unwrap() == fetched_oldest)
    );
}
