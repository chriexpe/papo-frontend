//! A channel's pin order. Which messages are pinned is decided by the pin list fetch and live
//! pin events (`mutations`); this module ranks them so the newest pin comes first.
//!
//! The backend lists pins oldest pin first and a live pin is newer than every pin a fetch
//! taken before it reported, so a rank is the model revision its evidence was observed at,
//! then its position in that fetch. No pin time is invented. Ranks live in memory only
//! (`Model::pin_ranks`), so reranking never rewrites an entry, and pins are published only
//! once this generation has fetched the list.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::wire::Message;

use super::model::Model;

/// Orders pins: a higher rank is a newer pin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PinRank {
    /// Model revision the evidence was taken at: a fetch's ticket, or a live pin event.
    pub revision: u64,
    /// Position in the fetched list, oldest pin first; 0 for a live pin event.
    pub position: u32,
}

impl Model {
    /// Pinned, undeleted messages held by this model, newest pin first. Reads only the pins,
    /// never the whole model.
    pub fn pin_order(&self) -> Vec<[u8; 16]> {
        let mut pinned: Vec<_> = self
            .pin_ranks
            .iter()
            .filter(|(id, _)| {
                self.entries
                    .get(*id)
                    .is_some_and(|entry| entry.pinned && !entry.deleted && entry.message.is_some())
            })
            .map(|(id, rank)| (*rank, *id))
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
    /// Each published pin's entry revision, encoded bytes and payload, so an unchanged pin
    /// keeps its `Arc` (a refetch merges every pin again at a new revision).
    published: BTreeMap<[u8; 16], Published>,
}

struct Published {
    revision: u64,
    bytes: Vec<u8>,
    payload: Arc<Message>,
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
            let held = self.published.remove(&id);
            let next = match held {
                Some(held) if held.revision == entry.revision => held,
                held => {
                    // Unencodable payloads are not shared; they are still published.
                    let bytes = serde_json::to_vec(message).unwrap_or_default();
                    let payload = timeline(id, entry.revision)
                        .or_else(|| {
                            held.filter(|held| !bytes.is_empty() && held.bytes == bytes)
                                .map(|held| held.payload)
                        })
                        .unwrap_or_else(|| Arc::new(message.clone()));
                    Published {
                        revision: entry.revision,
                        bytes,
                        payload,
                    }
                }
            };
            pins.push_back(Arc::clone(&next.payload));
            published.insert(id, next);
        }
        self.published = published;
        Some(pins)
    }
}

#[cfg(test)]
mod tests {
    use papo_api::{HistoryOrder, Uuid};

    use super::*;
    use crate::history::merge::Ticket;

    fn message(index: u128) -> Message {
        Message {
            id: Some(Uuid::from_u128(index + 1).to_string()),
            channel_id: Some(Uuid::from_u128(5).to_string()),
            created_at: Some(format!("2026-01-01T00:00:{index:02}.000000Z")),
            content: Some(String::new()),
            ..Message::default()
        }
    }

    fn ticket(model: &Model) -> Ticket {
        Ticket {
            channel: *Uuid::from_u128(5).as_bytes(),
            generation: 1,
            revision: model.revision,
            cursor: None,
            order: HistoryOrder::Desc,
            limit: 100,
        }
    }

    #[test]
    fn fetching_the_same_pins_again_publishes_the_same_payloads() {
        let mut model = Model::default();
        let mut publication = PinPublication::default();
        model
            .pins(&ticket(&model), &[message(1), message(0)])
            .unwrap();
        let first = publication.publish(&model, 1, |_, _| None).unwrap();
        model
            .pins(&ticket(&model), &[message(1), message(0)])
            .unwrap();
        let second = publication.publish(&model, 1, |_, _| None).unwrap();
        assert_eq!(first.len(), 2);
        assert!(
            first
                .iter()
                .zip(second.iter())
                .all(|(a, b)| Arc::ptr_eq(a, b))
        );
        assert!(publication.publish(&model, 2, |_, _| None).is_none());
    }
}
