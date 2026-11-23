//! A channel's pin order. Which messages are pinned is decided by the pin list fetch and live
//! pin events (`mutations`); this module ranks them so the newest pin comes first.
//!
//! The backend lists pins oldest pin first and a live pin is newer than every pin a fetch
//! taken before it reported, so a rank is the model revision its evidence was observed at,
//! then its position in that fetch. No pin time is invented: a pin restored from storage keeps
//! the rank it was given, and is only published once this generation has fetched the list.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::wire::Message;
use serde::{Deserialize, Serialize};

use super::model::Model;

/// Orders pins: a higher rank is a newer pin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct PinRank {
    /// Model revision the evidence was taken at: a fetch's ticket, or a live pin event.
    pub revision: u64,
    /// Position in the fetched list, oldest pin first; 0 for a live pin event.
    pub position: u32,
}

impl Model {
    /// Pinned, undeleted messages held by this model, newest pin first. Ties (ranks restored
    /// from an older layout) fall back to message ID order, so the order is stable.
    pub fn pin_order(&self) -> Vec<[u8; 16]> {
        let mut pinned: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.pinned && !entry.deleted && entry.message.is_some())
            .map(|(id, entry)| (entry.pin_rank, *id))
            .collect();
        pinned.sort_unstable_by(|left, right| right.cmp(left));
        pinned.into_iter().map(|(_, id)| id).collect()
    }

    /// Whether this model's pins are known in `generation`: its pin list was fetched in it,
    /// and live pin events of held messages have kept it current since.
    pub fn pins_current(&self, generation: u64) -> bool {
        self.pins_generation != 0 && self.pins_generation == generation
    }
}

/// The published pin list and the payloads it shares between publications.
#[derive(Default)]
pub(super) struct PinPublication {
    /// Each published pin's entry revision and payload, so an unchanged pin keeps its `Arc`.
    published: BTreeMap<[u8; 16], (u64, Arc<Message>)>,
}

impl PinPublication {
    /// The pins to publish in `generation`, newest pin first, or `None` while they are not
    /// current (see `Model::pins_current`). `timeline` returns the timeline's payload for an
    /// entry at a revision, so a pin that is also in the timeline shares its `Arc`.
    pub fn publish(
        &mut self,
        model: &Model,
        generation: u64,
        timeline: impl Fn([u8; 16], u64) -> Option<Arc<Message>>,
    ) -> Option<imbl::Vector<Arc<Message>>> {
        if !model.pins_current(generation) {
            self.published.clear();
            return None;
        }
        let mut published = BTreeMap::new();
        let mut pins = imbl::Vector::new();
        for id in model.pin_order() {
            let Some(entry) = model.entries.get(&id) else {
                continue;
            };
            let Some(message) = &entry.message else {
                continue;
            };
            let payload = self
                .published
                .get(&id)
                .filter(|(revision, _)| *revision == entry.revision)
                .map(|(_, payload)| Arc::clone(payload))
                .or_else(|| timeline(id, entry.revision))
                .unwrap_or_else(|| Arc::new(message.clone()));
            pins.push_back(Arc::clone(&payload));
            published.insert(id, (entry.revision, payload));
        }
        self.published = published;
        Some(pins)
    }
}
