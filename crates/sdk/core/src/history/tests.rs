//! Deterministic behavior and generated interleavings for history; sockets are integration tests.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::{
    HistoryOrder, Uuid,
    gateway::{KnownEvent, MessageDeleteEvent, MessageEditEvent, MessageEvent},
    wire::Message,
};
use proptest::{
    prelude::*,
    test_runner::{Config, RngAlgorithm, TestRng, TestRunner},
};

use super::{
    merge::Ticket,
    model::Model,
    ordering::{Key, Point},
};

const CHANNEL: u128 = 800_000;
const AT: &str = "2026-01-01T00:00:00.123456Z";

fn message(index: u128) -> Message {
    Message {
        id: Some(Uuid::from_u128(index + 1).to_string()),
        channel_id: Some(Uuid::from_u128(CHANNEL).to_string()),
        created_at: Some(AT.into()),
        content: Some(String::new()),
        ..Message::default()
    }
}

fn point(message: &Message) -> Point {
    Point::from(&message.cursor().unwrap())
}

fn ticket(
    model: &Model,
    cursor: Option<Point>,
    order: HistoryOrder,
    generation: u64,
    limit: usize,
) -> Ticket {
    Ticket {
        channel: *Uuid::from_u128(CHANNEL).as_bytes(),
        cursor,
        order,
        generation,
        revision: model.revision,
        limit,
    }
}

fn page(
    server: &BTreeMap<Key, Message>,
    cursor: Option<&Point>,
    direction: HistoryOrder,
    limit: usize,
) -> (Vec<Message>, bool) {
    let boundary = cursor.map(|point| point.key().unwrap());
    let mut messages: Vec<_> = server
        .iter()
        .filter(|(key, _)| {
            boundary.is_none_or(|boundary| match direction {
                HistoryOrder::Desc => **key < boundary,
                HistoryOrder::Asc => **key > boundary,
            })
        })
        .map(|(_, message)| message.clone())
        .collect();
    if direction == HistoryOrder::Desc {
        messages.reverse();
    }
    let has_more = messages.len() > limit;
    messages.truncate(limit);
    (messages, has_more)
}

fn live(message: &Message) -> KnownEvent {
    KnownEvent::Message(Box::new(MessageEvent {
        id: message.id.clone().unwrap(),
        channel_id: message.channel_id.clone().unwrap(),
        created_at: message.created_at.clone().unwrap(),
        ..MessageEvent::default()
    }))
}

fn invariants(model: &Model, server: &BTreeMap<Key, Message>, generation: u64) {
    for pair in model.chunks.windows(2) {
        assert!(pair[0].last.key().unwrap() < pair[1].first.key().unwrap());
    }
    let ids: BTreeSet<_> = model.order.values().copied().collect();
    assert_eq!(ids.len(), model.order.len());
    for (key, id) in &model.order {
        assert_eq!(
            model
                .chunks
                .iter()
                .filter(|chunk| chunk.contains(*key).unwrap())
                .count(),
            1
        );
        let entry = &model.entries[id];
        assert!(entry.timeline && !entry.deleted);
        if entry.validated_generation == generation {
            let authoritative = server.get(key).unwrap();
            assert_eq!(
                entry.message.as_ref().unwrap().edited_at,
                authoritative.edited_at
            );
        }
    }
    for chunk in &model.chunks {
        let first = chunk.first.key().unwrap();
        let last = chunk.last.key().unwrap();
        if chunk.generation == generation {
            let cached: Vec<_> = model
                .order
                .range(first..=last)
                .map(|(key, _)| *key)
                .collect();
            let expected: Vec<_> = server.range(first..=last).map(|(key, _)| *key).collect();
            assert_eq!(cached, expected);
        }
        for (cursor, direction, complete) in [
            (&chunk.first, HistoryOrder::Desc, chunk.oldest_complete),
            (&chunk.last, HistoryOrder::Asc, chunk.newest_complete),
        ] {
            let cached = model
                .page(Some(cursor), direction, 100, generation)
                .unwrap();
            if !complete {
                let gap = cached.gap.unwrap();
                assert_eq!(gap.cursor.created_at, cursor.created_at);
                assert_eq!(gap.cursor.id, cursor.id);
                assert_eq!(gap.order, direction);
                assert!(
                    page(server, Some(&Point::from(&gap.cursor)), direction, 4)
                        .0
                        .len()
                        <= 4
                );
            }
        }
    }
}

#[test]
fn late_page_cannot_resurrect_a_delete_or_replace_a_newer_edit() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    let before = message(2);
    model
        .event(
            &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: message(1).id.unwrap(),
                channel_id: Uuid::from_u128(CHANNEL).to_string(),
            })),
            1,
        )
        .unwrap();
    model
        .event(
            &KnownEvent::MessageEdit(Box::new(MessageEditEvent {
                id: before.id.clone().unwrap(),
                channel_id: Uuid::from_u128(CHANNEL).to_string(),
                content: String::new(),
                edited_at: "2026-01-01T00:01:00.000002Z".into(),
            })),
            1,
        )
        .unwrap();
    model
        .merge_page(&request, &[before, message(1), message(0)], false)
        .unwrap();
    let deleted_id = *Uuid::from_u128(2).as_bytes();
    assert!(model.entries[&deleted_id].deleted);
    assert!(!model.order.values().any(|id| *id == deleted_id));
    assert_eq!(
        model.entries[Uuid::from_u128(3).as_bytes()]
            .message
            .as_ref()
            .unwrap()
            .edited_at
            .as_deref(),
        Some("2026-01-01T00:01:00.000002Z")
    );
}

#[test]
fn detached_pin_is_not_a_cursor_or_overlap_evidence() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    model.pins(&request, &[message(0)]).unwrap();
    model
        .merge_page(&request, &[message(9), message(8)], true)
        .unwrap();
    assert!(
        !model
            .order
            .values()
            .any(|id| *id == *Uuid::from_u128(1).as_bytes())
    );
    assert_eq!(model.chunks.len(), 1);
    assert_eq!(model.chunks[0].first.id, message(8).id.unwrap());
}

#[test]
fn live_without_current_generation_edge_leaves_a_fillable_gap() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    model
        .merge_page(&request, &[message(2), message(1)], true)
        .unwrap();
    model.event(&live(&message(3)), 1).unwrap();
    assert_eq!(model.chunks.len(), 1);
    model.event(&live(&message(7)), 2).unwrap();
    assert_eq!(model.chunks.len(), 2);
    assert!(!model.chunks[1].oldest_complete);
    let cached = model
        .page(Some(&point(&message(7))), HistoryOrder::Desc, 10, 2)
        .unwrap();
    assert_eq!(cached.messages.len(), 0);
    assert!(cached.gap.is_some());
}

#[test]
fn malformed_or_nonadvancing_page_leaves_state_unchanged() {
    let mut model = Model::default();
    let request = ticket(&model, Some(point(&message(3))), HistoryOrder::Desc, 1, 100);
    assert!(model.merge_page(&request, &[message(4)], true).is_err());
    assert_eq!(model.revision, 0);
    assert!(model.entries.is_empty());
}

#[test]
fn empty_newest_page_proves_the_first_live_append_has_no_gap() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    model.merge_page(&request, &[], false).unwrap();
    model.event(&live(&message(0)), 1).unwrap();
    assert!(model.chunks[0].oldest_complete);
    assert!(model.chunks[0].newest_complete);
}

#[test]
fn random_pages_live_mutations_reconnects_and_evictions_preserve_proof() {
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: 128,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    let strategy = prop::collection::vec((0u8..8, 0u8..64, 1usize..16), 1..160);
    runner
        .run(&strategy, |operations| {
            let mut model = Model::default();
            let mut server: BTreeMap<_, _> = (0..64)
                .map(message)
                .map(|message| (point(&message).key().unwrap(), message))
                .collect();
            let mut generation = 1;
            let mut next_id = 64;
            let mut edit_clock_us = 1;
            for (operation, index, limit) in operations {
                match operation {
                    0 | 1 => {
                        let cursor = (index != 0).then(|| point(&message(u128::from(index))));
                        let direction = if operation == 0 {
                            HistoryOrder::Desc
                        } else {
                            HistoryOrder::Asc
                        };
                        let (messages, has_more) = page(&server, cursor.as_ref(), direction, limit);
                        let request = ticket(&model, cursor, direction, generation, limit);
                        model.merge_page(&request, &messages, has_more).unwrap();
                    }
                    2 => {
                        let message = message(next_id);
                        next_id += 1;
                        server.insert(point(&message).key().unwrap(), message.clone());
                        model.event(&live(&message), generation).unwrap();
                    }
                    3 => {
                        let key = point(&message(u128::from(index))).key().unwrap();
                        if let Some(message) = server.get_mut(&key) {
                            let edited_at = format!("2026-01-01T00:01:00.{edit_clock_us:06}Z");
                            edit_clock_us += 1;
                            message.edited_at = Some(edited_at.clone());
                            model
                                .event(
                                    &KnownEvent::MessageEdit(Box::new(MessageEditEvent {
                                        id: message.id.clone().unwrap(),
                                        channel_id: Uuid::from_u128(CHANNEL).to_string(),
                                        content: String::new(),
                                        edited_at,
                                    })),
                                    generation,
                                )
                                .unwrap();
                        }
                    }
                    4 => {
                        let message = message(u128::from(index));
                        server.remove(&point(&message).key().unwrap());
                        model
                            .event(
                                &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                                    id: message.id.unwrap(),
                                    channel_id: Uuid::from_u128(CHANNEL).to_string(),
                                })),
                                generation,
                            )
                            .unwrap();
                    }
                    5 | 6 => {
                        generation += 1;
                        let offline_count = if operation == 5 { 2 } else { 24 };
                        for _ in 0..offline_count {
                            let message = message(next_id);
                            next_id += 1;
                            server.insert(point(&message).key().unwrap(), message);
                        }
                        let mut cursor = None;
                        for _ in 0..4 {
                            let (messages, has_more) =
                                page(&server, cursor.as_ref(), HistoryOrder::Desc, 4);
                            let request = ticket(&model, cursor, HistoryOrder::Desc, generation, 4);
                            cursor = messages.last().map(point);
                            model.merge_page(&request, &messages, has_more).unwrap();
                            if !has_more {
                                break;
                            }
                        }
                    }
                    _ => {
                        model = Model::default();
                    }
                }
                invariants(&model, &server, generation);
            }
            Ok(())
        })
        .unwrap();
}

#[path = "race_tests.rs"]
mod races;
