//! The simulated backend and client driving the coverage property test, and the per-step
//! check comparing the model's published coverage with the oracle and the server.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use papo_api::{
    HistoryOrder, Uuid,
    gateway::{KnownEvent, MessageDeleteEvent, MessageEvent},
    wire::Message,
};

use crate::Generation;

use super::super::{
    HistorySnapshot, chunk_id::ChunkId, merge::Ticket, message_key::MessageKey, model::Model,
    ordering::Point, publication::published_chunk, snapshot_query::published_gaps,
};
use super::oracle::{Fact, Oracle, Slots};

pub(super) const CHANNEL: u128 = 810_000;
pub(super) const SYNTHETIC_ID: u128 = 50_000_000;
pub(super) const INITIAL_MESSAGES: u128 = 12;

pub(super) fn timestamp(offset_us: u128) -> String {
    let micros = i64::try_from(offset_us).unwrap() + 1_767_225_600_000_000;
    chrono::DateTime::from_timestamp_micros(micros)
        .unwrap()
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

/// Message `index` sits at `10 * index` µs; synthetic key `index` sits 5 µs after it.
pub(super) fn message(index: u128) -> Message {
    Message {
        id: Some(Uuid::from_u128(index + 1).to_string()),
        channel_id: Some(Uuid::from_u128(CHANNEL).to_string()),
        created_at: Some(timestamp(index * 10)),
        content: Some(String::new()),
        ..Message::default()
    }
}

pub(super) fn message_point(index: u128) -> Point {
    Point::from(&message(index).cursor().unwrap())
}

pub(super) fn synthetic_point(index: u128) -> Point {
    Point {
        created_at: timestamp(index * 10 + 5),
        id: Uuid::from_u128(SYNTHETIC_ID + index).to_string(),
    }
}

pub(super) fn key(point: &Point) -> MessageKey {
    point.key().unwrap()
}

pub(super) fn live(message: &Message) -> KnownEvent {
    KnownEvent::Message(Box::new(MessageEvent {
        id: message.id.clone().unwrap(),
        channel_id: message.channel_id.clone().unwrap(),
        created_at: message.created_at.clone().unwrap(),
        ..MessageEvent::default()
    }))
}

pub(super) fn delete(message: &Message) -> KnownEvent {
    KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
        id: message.id.clone().unwrap(),
        channel_id: Uuid::from_u128(CHANNEL).to_string(),
    }))
}

/// The backend's `(created_at, id)` cursor semantics over its full message list.
pub(super) fn server_page(
    server: &BTreeMap<MessageKey, Message>,
    cursor: Option<&Point>,
    order: HistoryOrder,
    limit: usize,
) -> (Vec<Message>, bool) {
    let boundary = cursor.map(key);
    let mut messages: Vec<_> = server
        .iter()
        .filter(|(held, _)| match (boundary, order) {
            (None, _) => true,
            (Some(boundary), HistoryOrder::Desc) => **held < boundary,
            (Some(boundary), HistoryOrder::Asc) => **held > boundary,
        })
        .map(|(_, message)| message.clone())
        .collect();
    if order == HistoryOrder::Desc {
        messages.reverse();
    }
    let has_more = messages.len() > limit;
    messages.truncate(limit);
    (messages, has_more)
}

pub(super) fn publish(model: &Model, generation: u64) -> HistorySnapshot {
    let messages = model
        .order
        .values()
        .map(|id| Arc::new(model.entries[id].message.clone().unwrap()))
        .collect();
    let tombstones = model
        .entries
        .iter()
        .filter(|(_, entry)| entry.deleted)
        .map(|(id, _)| Uuid::from_bytes(*id))
        .collect();
    let chunks: Vec<_> = model.chunks.iter().map(published_chunk).collect();
    let empty_complete = model.order.is_empty() && model.empty_generation == generation;
    let gaps = published_gaps(&chunks, generation, empty_complete).unwrap();
    HistorySnapshot {
        version: 1,
        generation: Generation(generation),
        messages,
        tombstones,
        chunks,
        gaps,
        empty_complete,
        server_live_complete: false,
        live_arrival_start: None,
        pins: None,
    }
}

pub(super) fn check(
    model: &Model,
    oracle: &Oracle,
    server: &BTreeMap<MessageKey, Message>,
    universe: Vec<MessageKey>,
    generation: u64,
) {
    let snapshot = publish(model, generation);
    let coverage = snapshot.coverage().unwrap();
    let slots = Slots::new(universe, oracle, generation);
    let held: BTreeSet<_> = model.order.keys().copied().collect();
    // A held message is always inside a run with real boundaries, never only an anchor.
    for point in &held {
        assert!(
            model.chunks.iter().any(|chunk| !chunk.is_anchor_only()
                && key(&chunk.first) <= *point
                && *point <= key(&chunk.last)),
            "held message outside every run with message boundaries"
        );
    }
    // Prefix counts of server messages the client lacks, per slot.
    let mut missing = vec![0usize; slots.covered.len() + 1];
    for slot in 0..slots.covered.len() {
        let lacks = slot % 2 == 1 && {
            let point = slots.universe[slot / 2];
            server.contains_key(&point) && !held.contains(&point)
        };
        missing[slot + 1] = missing[slot] + usize::from(lacks);
    }
    let bounds: Vec<Option<MessageKey>> = std::iter::once(None)
        .chain(slots.universe.iter().copied().map(Some))
        .collect();
    let befores: Vec<_> = bounds.iter().copied().step_by(3).chain([None]).collect();
    for &after in &bounds {
        for &before in &befores {
            let complete = coverage.range(after, before).unwrap().complete;
            let empty = matches!((after, before), (Some(a), Some(b)) if a >= b);
            if empty {
                continue;
            }
            assert_eq!(
                complete,
                slots.proves(after, before),
                "completeness must equal the accepted evidence for {after:?}..{before:?}"
            );
            let between = slots.between(after, before);
            assert!(
                !complete || missing[between.end] == missing[between.start],
                "a complete range lacks a server message"
            );
        }
    }
    check_gaps(&snapshot, &slots, HistoryOrder::Desc);
    check_gaps(&snapshot, &slots, HistoryOrder::Asc);
}

/// Each maximal proven run publishes one gap at each bounded edge, at or right next to it.
pub(super) fn check_gaps(snapshot: &HistorySnapshot, slots: &Slots, order: HistoryOrder) {
    let last_slot = slots.covered.len();
    let mut gap_slots: Vec<usize> = snapshot
        .gaps
        .iter()
        .filter(|gap| gap.order == order)
        .map(|gap| slots.slot_of(MessageKey::from_cursor(&gap.cursor).unwrap()))
        .collect();
    for run in slots.runs() {
        let (bounded, reach) = match order {
            HistoryOrder::Desc => (run.start != 0, run.start.saturating_sub(1)..run.end),
            HistoryOrder::Asc => (run.end != last_slot, run.start..(run.end + 1)),
        };
        if !bounded {
            continue;
        }
        let position = gap_slots
            .iter()
            .position(|slot| reach.contains(slot))
            .expect("a bounded proven edge publishes its gap");
        gap_slots.swap_remove(position);
    }
    assert!(
        gap_slots.is_empty(),
        "every published gap bounds a proven run"
    );
}

pub(super) fn ticket(
    model: &Model,
    cursor: Option<Point>,
    order: HistoryOrder,
    generation: u64,
    limit: usize,
) -> Ticket {
    Ticket {
        channel: *Uuid::from_u128(CHANNEL).as_bytes(),
        generation,
        revision: model.revision,
        cursor,
        order,
        limit,
    }
}

pub(super) struct Simulation {
    pub(super) model: Model,
    pub(super) oracle: Oracle,
    pub(super) server: BTreeMap<MessageKey, Message>,
    pub(super) generation: u64,
    pub(super) next_index: u128,
    /// Every chunk identity observed so far, and those held at the last check: an identity
    /// that disappeared is retired and must never come back.
    pub(super) identities_seen: BTreeSet<ChunkId>,
    pub(super) identities_held: BTreeSet<ChunkId>,
}

impl Simulation {
    pub(super) fn new() -> Self {
        Self {
            model: Model::default(),
            oracle: Oracle::default(),
            server: (0..INITIAL_MESSAGES)
                .map(|index| (key(&message_point(index)), message(index)))
                .collect(),
            generation: 1,
            next_index: INITIAL_MESSAGES,
            identities_seen: BTreeSet::new(),
            identities_held: BTreeSet::new(),
        }
    }

    pub(super) fn page(&mut self, cursor: Option<&Point>, order: HistoryOrder, limit: usize) {
        let (messages, has_more) = server_page(&self.server, cursor, order, limit);
        let returned: Vec<_> = messages
            .iter()
            .map(|message| MessageKey::from_message(message).unwrap())
            .collect();
        let request = ticket(&self.model, cursor.cloned(), order, self.generation, limit);
        self.model
            .merge_page(&request, &messages, has_more)
            .unwrap();
        self.oracle
            .accept_page(cursor.map(key), order, &returned, has_more, self.generation);
    }

    pub(super) fn arrive(&mut self) {
        let message = message(self.next_index);
        self.next_index += 1;
        let point = MessageKey::from_message(&message).unwrap();
        self.server.insert(point, message.clone());
        self.model.event(&live(&message), self.generation).unwrap();
        self.oracle.accept_live(point, self.generation);
    }

    /// A gateway event for a message the server already had, delivered late.
    pub(super) fn deliver_late(&mut self, index: u128) {
        let message = message(index);
        let point = MessageKey::from_message(&message).unwrap();
        assert!(self.server.contains_key(&point));
        self.model.event(&live(&message), self.generation).unwrap();
        self.oracle.accept_live(point, self.generation);
    }

    pub(super) fn delete(&mut self, index: u128) {
        let message = message(index);
        let point = MessageKey::from_message(&message).unwrap();
        if self.server.len() > 1 && self.server.remove(&point).is_some() {
            self.model
                .event(&delete(&message), self.generation)
                .unwrap();
        }
    }

    pub(super) fn evict(&mut self) {
        let before = self.model.chunks.clone();
        let held = !self.model.order.is_empty();
        let evicted = self.model.evict_prefix().unwrap();
        assert!(
            !held || !evicted.is_empty(),
            "retention progresses while messages are held"
        );
        if evicted.is_empty() {
            return;
        }
        // Identities stay with their run while its boundaries move, so the surviving oldest run
        // is found by identity: leading runs before it were discarded whole, and it lost its
        // prefix if its first message moved.
        let survivor = self.model.chunks.first().cloned();
        for chunk in &before {
            if let Some(survivor) = &survivor
                && survivor.id == chunk.id
            {
                assert_eq!(survivor.last, chunk.last, "retention keeps the newest edge");
                if survivor.first != chunk.first {
                    self.oracle.retain_from(key(&survivor.first));
                }
                break;
            }
            let extent = chunk.extent(self.generation).unwrap();
            let run = Fact {
                lower: extent.lower,
                upper: extent.upper,
                live: None,
                empty_from: None,
            };
            self.oracle.discard_run(run, self.generation);
            self.oracle
                .accepted
                .retain(|point| *point > key(&chunk.last));
        }
    }

    pub(super) fn reconnect(&mut self, offline: u128) {
        for _ in 0..offline {
            let message = message(self.next_index);
            self.next_index += 1;
            self.server
                .insert(MessageKey::from_message(&message).unwrap(), message);
        }
        self.oracle.reconnect(self.generation);
        self.generation += 1;
    }

    pub(super) fn universe(&self) -> Vec<MessageKey> {
        let mut universe: Vec<_> = (0..self.next_index)
            .flat_map(|index| [key(&message_point(index)), key(&synthetic_point(index))])
            .collect();
        universe.sort();
        universe
    }

    pub(super) fn check(&mut self) {
        self.check_identities();
        check(
            &self.model,
            &self.oracle,
            &self.server,
            self.universe(),
            self.generation,
        );
    }
}

pub(super) fn cursor_for(selector: u8, index: u128) -> Option<Point> {
    match selector % 3 {
        0 => Some(message_point(index)),
        1 => Some(synthetic_point(index)),
        _ => None,
    }
}
