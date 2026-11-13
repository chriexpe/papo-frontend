//! Synthetic canonical raw publications; production projection and queue behavior stay in the parent harness.

use super::CHANNEL;
use papo_sdk::{
    Generation, HistoryChunk, HistoryGap, HistoryOrder, HistorySnapshot, Message, Uuid,
};
use std::{collections::BTreeMap, sync::Arc};

const AT: &str = "2026-01-01T00:00:00.123456Z";

pub(super) fn message(index: u128) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(index * 100).to_string()),
        channel_id: Some(CHANNEL.to_string()),
        created_at: Some(AT.into()),
        ..Message::default()
    })
}

pub(super) struct Source {
    pub(super) messages: BTreeMap<u128, Arc<Message>>,
    pub(super) version: u64,
    pub(super) generation: u64,
    pub(super) edits: u32,
    pub(super) gap: bool,
}

impl Source {
    pub(super) fn new() -> Self {
        Self {
            messages: (10..=20).map(|id| (id, message(id))).collect(),
            version: 1,
            generation: 1,
            edits: 0,
            gap: false,
        }
    }

    pub(super) fn snapshot(&self) -> Arc<HistorySnapshot> {
        let generation = Generation::for_test(self.generation);
        let messages: Vec<_> = self.messages.values().cloned().collect();
        let groups: Vec<&[Arc<Message>]> = if self.gap && messages.len() > 1 {
            let (older, newer) = messages.split_at(messages.len() / 2);
            vec![older, newer]
        } else {
            vec![&messages]
        };
        let chunks: Vec<_> = groups
            .iter()
            .enumerate()
            .filter_map(|(index, group)| {
                Some(HistoryChunk {
                    first: group.first()?.cursor()?,
                    last: group.last()?.cursor()?,
                    oldest_complete: index == 0,
                    newest_complete: index + 1 == groups.len(),
                    generation: Some(Generation::for_test(if index == 0 && groups.len() > 1 {
                        self.generation.saturating_sub(1)
                    } else {
                        self.generation
                    })),
                    live_generation: (index + 1 == groups.len()).then_some(generation),
                })
            })
            .collect();
        let gaps = if chunks.len() == 2 {
            vec![
                HistoryGap {
                    cursor: chunks[0].last.clone(),
                    order: HistoryOrder::Asc,
                },
                HistoryGap {
                    cursor: chunks[1].first.clone(),
                    order: HistoryOrder::Desc,
                },
            ]
        } else {
            Vec::new()
        };
        Arc::new(HistorySnapshot {
            tombstones: imbl::HashSet::new(),
            version: self.version,
            generation,
            messages: messages.into(),
            chunks,
            gaps,
            empty_complete: self.messages.is_empty(),
        })
    }

    pub(super) fn arrival(&mut self) {
        let id = self.messages.last_key_value().map_or(1, |(id, _)| id + 1);
        self.messages.insert(id, message(id));
    }

    pub(super) fn edit(&mut self, id: u128) {
        if let Some(previous) = self.messages.get_mut(&id) {
            self.edits += 1;
            let mut edited = (**previous).clone();
            edited.edited_at = Some(format!("2026-01-02T00:00:00.{:06}Z", self.edits));
            *previous = Arc::new(edited);
        }
    }
}
