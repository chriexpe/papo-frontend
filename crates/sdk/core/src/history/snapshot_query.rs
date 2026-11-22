//! The coverage queries on a published `HistorySnapshot`, the lookup of a bare message ID, and
//! the gaps it publishes.
//!
//! Every consumer that needs "which message does this key (or ID) denote" or "is anything
//! unknown between these keys" asks here. Every binary search over snapshot messages lives in this
//! module. Interval rules, unread counting and arrival tracking stay with their owners.

use std::ops::Range;

use papo_api::{HistoryOrder, Uuid, wire::Message};

use crate::Generation;

use super::{
    HistoryGap,
    coverage::{ChunkEvidence, Components, Edge, Extent, live_edge_current},
    message_key::{InvalidMessageKey, MessageKey},
    subscription::{HistoryChunk, HistorySnapshot},
};

/// Legacy server read marks carry millisecond precision; a mark matches its message within
/// this distance in either direction.
const LEGACY_MARK_TOLERANCE_US: i64 = 1_000;

/// What a key denotes in one snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedKey {
    /// The held message's exact key when the key matched one, otherwise the key unchanged.
    pub key: MessageKey,
    /// Index into `HistorySnapshot::messages` of the matched message.
    pub index: Option<usize>,
    /// The key names a message this cache incarnation knows was deleted.
    pub deleted: bool,
}

/// What a bare message ID denotes in one snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocatedMessage {
    /// Held at `index` into `HistorySnapshot::messages`, with its exact key.
    Held {
        /// The held message's exact key.
        key: MessageKey,
        /// Index into `HistorySnapshot::messages`.
        index: usize,
    },
    /// This cache incarnation knows the message was deleted.
    Deleted,
    /// Not held: never fetched, evicted, or deleted before this cache incarnation. There is no
    /// GET-by-ID route, so nothing more can be learned about it.
    Unknown,
}

/// The held messages strictly between two keys, and whether none can be missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRange {
    /// Indexes into `HistorySnapshot::messages`.
    pub messages: Range<usize>,
    /// Coverage proves that no unknown message lies strictly between the bounds. A complete
    /// range is knowledge, never exposure: it does not mean anyone saw these messages.
    pub complete: bool,
}

/// One snapshot's coverage, parsed once for repeated queries.
pub struct HistoryCoverage<'a> {
    snapshot: &'a HistorySnapshot,
    components: Components,
}

impl HistorySnapshot {
    /// Prepares the coverage queries for this snapshot; parsing is linear in its chunks.
    /// # Errors
    /// Returns `InvalidMessageKey` if a chunk boundary is not a valid message key.
    pub fn coverage(&self) -> Result<HistoryCoverage<'_>, InvalidMessageKey> {
        let extents = self
            .chunks
            .iter()
            .map(|chunk| chunk_extent(chunk, self.generation.get()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(HistoryCoverage {
            snapshot: self,
            components: Components::build(extents),
        })
    }

    /// Locates a message known only by its ID (a saved reading position keeps no timestamp).
    /// An ID carries no position, so this is linear in the held messages, newest first; a caller
    /// holding a full key asks `coverage()?.resolve(key)` instead. Locating a message proves
    /// nothing about coverage around it.
    /// # Errors
    /// Returns `InvalidMessageKey` if the matching message lacks a valid key.
    pub fn locate(&self, id: Uuid) -> Result<LocatedMessage, InvalidMessageKey> {
        let held = self.messages.iter().enumerate().rev().find(|(_, message)| {
            message
                .id
                .as_deref()
                .is_some_and(|held| held.parse::<Uuid>().is_ok_and(|held| held == id))
        });
        if let Some((index, message)) = held {
            return Ok(LocatedMessage::Held {
                key: MessageKey::from_message(message)?,
                index,
            });
        }
        if self.tombstones.contains(&id) {
            return Ok(LocatedMessage::Deleted);
        }
        Ok(LocatedMessage::Unknown)
    }
}

/// Answers [`HistorySnapshot::locate`] for many IDs of one snapshot after a single pass,
/// instead of a scan per ID. The answers are `locate`'s exactly.
pub struct MessageLocator<'a> {
    snapshot: &'a HistorySnapshot,
    held: std::collections::HashMap<Uuid, usize>,
}

impl HistorySnapshot {
    /// Indexes the held messages by ID once, for repeated [`MessageLocator::locate`] calls.
    #[must_use]
    pub fn locator(&self) -> MessageLocator<'_> {
        let mut held = std::collections::HashMap::with_capacity(self.messages.len());
        // Oldest first, so a repeated ID ends on its newest copy, as `locate` finds it.
        for (index, message) in self.messages.iter().enumerate() {
            if let Some(id) = message
                .id
                .as_deref()
                .and_then(|held| held.parse::<Uuid>().ok())
            {
                held.insert(id, index);
            }
        }
        MessageLocator {
            snapshot: self,
            held,
        }
    }
}

impl MessageLocator<'_> {
    /// What `id` denotes in the indexed snapshot, as [`HistorySnapshot::locate`] says it.
    /// # Errors
    /// Returns `InvalidMessageKey` if the matching message lacks a valid key.
    pub fn locate(&self, id: Uuid) -> Result<LocatedMessage, InvalidMessageKey> {
        if let Some((index, message)) = self
            .held
            .get(&id)
            .and_then(|&index| Some((index, self.snapshot.messages.get(index)?)))
        {
            return Ok(LocatedMessage::Held {
                key: MessageKey::from_message(message)?,
                index,
            });
        }
        if self.snapshot.tombstones.contains(&id) {
            return Ok(LocatedMessage::Deleted);
        }
        Ok(LocatedMessage::Unknown)
    }
}

impl HistoryCoverage<'_> {
    /// Resolves `key` to the held message it denotes: an exact key, or a legacy millisecond
    /// read mark naming a held message within one millisecond. A deleted anchor is reported,
    /// never matched to another message, so resolution cannot reopen read content.
    /// # Errors
    /// Returns `InvalidMessageKey` if a compared message lacks a valid key.
    pub fn resolve(&self, key: MessageKey) -> Result<ResolvedKey, InvalidMessageKey> {
        let messages = &self.snapshot.messages;
        let index = partition(messages, |held| held < key)?;
        if let Some(message) = messages.get(index)
            && MessageKey::from_message(message)? == key
        {
            return Ok(ResolvedKey {
                key,
                index: Some(index),
                deleted: false,
            });
        }
        let deleted = self.snapshot.tombstones.contains(&key.message_id);
        if !deleted && key.created_at_us % LEGACY_MARK_TOLERANCE_US == 0 {
            let earliest = key.created_at_us.saturating_sub(LEGACY_MARK_TOLERANCE_US);
            let latest = key.created_at_us.saturating_add(LEGACY_MARK_TOLERANCE_US);
            let mut index = partition(messages, |held| held.created_at_us < earliest)?;
            while let Some(message) = messages.get(index) {
                let candidate = MessageKey::from_message(message)?;
                if candidate.created_at_us > latest {
                    break;
                }
                if candidate.message_id == key.message_id {
                    return Ok(ResolvedKey {
                        key: candidate,
                        index: Some(index),
                        deleted: false,
                    });
                }
                index += 1;
            }
        }
        Ok(ResolvedKey {
            key,
            index: None,
            deleted,
        })
    }

    /// The held messages strictly between `after` and `before` (`None` is unbounded), and
    /// whether coverage proves that no unknown message lies between them.
    /// # Errors
    /// Returns `InvalidMessageKey` if a compared message lacks a valid key.
    pub fn range(
        &self,
        after: Option<MessageKey>,
        before: Option<MessageKey>,
    ) -> Result<HistoryRange, InvalidMessageKey> {
        let messages = &self.snapshot.messages;
        let start = match after {
            Some(after) => partition(messages, |held| held <= after)?,
            None => 0,
        };
        if let (Some(after), Some(before)) = (after, before)
            && after >= before
        {
            return Ok(HistoryRange {
                messages: start..start,
                complete: true,
            });
        }
        let end = match before {
            Some(before) => partition(messages, |held| held < before)?,
            None => messages.len(),
        };
        Ok(HistoryRange {
            messages: start..end,
            complete: self.is_complete(after, before),
        })
    }

    /// `range(after, before).complete` without locating the messages, for callers that ask
    /// many times per report and never read the indexes.
    pub(crate) fn is_complete(
        &self,
        after: Option<MessageKey>,
        before: Option<MessageKey>,
    ) -> bool {
        if let (Some(after), Some(before)) = (after, before)
            && after >= before
        {
            return true;
        }
        self.snapshot.empty_complete || self.components.proves(after, before)
    }
}

/// The gaps a snapshot publishes: one at each edge of each proven run that coverage does not
/// extend without limit. `range(..).complete` is false exactly across these.
pub(super) fn published_gaps(
    chunks: &[HistoryChunk],
    generation: u64,
    empty_complete: bool,
) -> Result<Vec<HistoryGap>, InvalidMessageKey> {
    if empty_complete {
        return Ok(Vec::new());
    }
    let extents = chunks
        .iter()
        .map(|chunk| chunk_extent(chunk, generation))
        .collect::<Result<Vec<_>, _>>()?;
    let mut gaps = Vec::new();
    for group in Components::build(extents).groups() {
        if group.extent.lower != Edge::Unbounded {
            gaps.push(HistoryGap {
                cursor: chunks[group.oldest].first.clone(),
                order: HistoryOrder::Desc,
            });
        }
        if group.extent.upper != Edge::Unbounded {
            gaps.push(HistoryGap {
                cursor: chunks[group.newest].last.clone(),
                order: HistoryOrder::Asc,
            });
        }
    }
    Ok(gaps)
}

fn chunk_extent(chunk: &HistoryChunk, generation: u64) -> Result<Extent, InvalidMessageKey> {
    Ok(Extent::of(&ChunkEvidence {
        first: MessageKey::from_cursor(&chunk.first)?,
        last: MessageKey::from_cursor(&chunk.last)?,
        oldest_complete: chunk.oldest_complete,
        oldest_anchor: chunk
            .oldest_anchor
            .as_ref()
            .map(MessageKey::from_cursor)
            .transpose()?,
        live_edge_current: live_edge_current(
            chunk.newest_complete,
            chunk.live_generation.map_or(0, Generation::get),
            generation,
        ),
        newest_anchor: chunk
            .newest_anchor
            .as_ref()
            .map(MessageKey::from_cursor)
            .transpose()?,
    }))
}

/// First index whose key fails `before`; messages are ascending by key.
fn partition(
    messages: &imbl::Vector<std::sync::Arc<Message>>,
    mut before: impl FnMut(MessageKey) -> bool,
) -> Result<usize, InvalidMessageKey> {
    let (mut low, mut high) = (0, messages.len());
    while low < high {
        let middle = low + (high - low) / 2;
        if before(MessageKey::from_message(&messages[middle])?) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Ok(low)
}
