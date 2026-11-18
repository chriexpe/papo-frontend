//! Cached local page serving, chunk target selection, and freshness checks.
//! Structural chunk merging lives in `coverage`; snapshot range queries live in `queries`.

use std::ops::Bound::{Excluded, Included};

use papo_api::{HistoryOrder, wire::Message};

use super::{
    HistoryError, HistoryGap,
    model::{Chunk, Model},
    ordering::{Key, Point},
};

pub(super) struct CachedPage {
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub gap: Option<HistoryGap>,
    pub fresh: bool,
}

impl Model {
    pub fn page(
        &self,
        cursor: Option<&Point>,
        order: HistoryOrder,
        limit: usize,
        generation: u64,
    ) -> Result<CachedPage, HistoryError> {
        let boundary = cursor.map(Point::key).transpose()?;
        let Some(chunk) = self.page_target(boundary, order, generation) else {
            return Ok(self.empty_cached_page(cursor, boundary, order, generation));
        };
        // A cursor in a hole must not jump directly to the next cached run.
        let first = chunk.first.key()?;
        let last = chunk.last.key()?;
        let lower = match (order, boundary) {
            (HistoryOrder::Asc, Some(key)) => Excluded(key),
            _ => Included(first),
        };
        let upper = match (order, boundary) {
            (HistoryOrder::Desc, Some(key)) => Excluded(key),
            _ => Included(last),
        };
        let range = self.order.range((lower, upper));
        let payload = |(_, id): (&Key, &[u8; 16])| self.entries.get(id)?.message.as_ref();
        let candidates: Vec<_> = match order {
            HistoryOrder::Desc => range.rev().filter_map(payload).take(limit + 1).collect(),
            HistoryOrder::Asc => range.filter_map(payload).take(limit + 1).collect(),
        };
        let more_cached = candidates.len() > limit;
        let complete = match order {
            HistoryOrder::Desc => {
                chunk.oldest_complete
                    || (self.chunks.first().is_some_and(|c| c.id == chunk.id)
                        && self.empty_before.as_ref().is_some_and(|before| {
                            before.generation != 0 && before.point.key().is_ok_and(|bk| first <= bk)
                        }))
            }
            HistoryOrder::Asc => {
                chunk.newest_complete
                    || (self.chunks.last().is_some_and(|c| c.id == chunk.id)
                        && self.empty_after.as_ref().is_some_and(|after| {
                            after.generation == generation
                                && after.point.key().is_ok_and(|ak| last >= ak)
                        }))
            }
        };
        let gap = (!more_cached && !complete).then(|| HistoryGap {
            cursor: match order {
                HistoryOrder::Desc => chunk.first.cursor(),
                HistoryOrder::Asc => chunk.last.cursor(),
            },
            order,
        });
        let fresh = self.cached_page_fresh(
            chunk,
            boundary,
            order,
            generation,
            limit,
            &candidates,
            more_cached,
        )?;
        Ok(CachedPage {
            messages: candidates.into_iter().take(limit).cloned().collect(),
            has_more: more_cached || !complete,
            gap,
            fresh,
        })
    }

    fn page_target(
        &self,
        boundary: Option<Key>,
        order: HistoryOrder,
        generation: u64,
    ) -> Option<&Chunk> {
        match boundary {
            Some(key) => self
                .chunks
                .iter()
                .find(|chunk| match order {
                    HistoryOrder::Asc => {
                        chunk.first.key().is_ok_and(|first| first <= key)
                            && chunk.last.key().is_ok_and(|last| key < last)
                    }
                    HistoryOrder::Desc => {
                        chunk.first.key().is_ok_and(|first| first < key)
                            && chunk.last.key().is_ok_and(|last| key <= last)
                    }
                })
                .or_else(|| {
                    self.chunks.iter().find(|chunk| match order {
                        HistoryOrder::Asc => {
                            chunk.first.key().is_ok_and(|first| key < first)
                                && (chunk.oldest_complete
                                    || chunk
                                        .oldest_anchor
                                        .as_ref()
                                        .is_some_and(|a| a.key().is_ok_and(|ak| ak <= key)))
                        }
                        HistoryOrder::Desc => {
                            chunk.last.key().is_ok_and(|last| key > last)
                                && ((chunk.newest_complete && chunk.live_generation == generation)
                                    || chunk
                                        .newest_anchor
                                        .as_ref()
                                        .is_some_and(|b| b.key().is_ok_and(|bk| key <= bk)))
                        }
                    })
                })
                .or_else(|| {
                    self.chunks
                        .iter()
                        .find(|chunk| chunk.contains(key).unwrap_or(false))
                }),
            None if order == HistoryOrder::Desc => self.chunks.last(),
            None => self.chunks.first(),
        }
    }

    fn empty_cached_page(
        &self,
        cursor: Option<&Point>,
        boundary: Option<Key>,
        order: HistoryOrder,
        generation: u64,
    ) -> CachedPage {
        let empty_proven = (self.chunks.is_empty() && self.empty_generation == generation)
            || match (order, boundary) {
                (HistoryOrder::Desc, Some(key)) => {
                    self.empty_before.as_ref().is_some_and(|anchor| {
                        anchor.generation == generation
                            && anchor.point.key().is_ok_and(|ak| key <= ak)
                    })
                }
                (HistoryOrder::Asc, Some(key)) => self.empty_after.as_ref().is_some_and(|anchor| {
                    anchor.generation == generation && anchor.point.key().is_ok_and(|ak| key >= ak)
                }),
                _ => false,
            };
        CachedPage {
            messages: Vec::new(),
            has_more: !empty_proven,
            gap: (!empty_proven)
                .then(|| {
                    cursor.map(|cursor| HistoryGap {
                        cursor: cursor.cursor(),
                        order,
                    })
                })
                .flatten(),
            fresh: empty_proven,
        }
    }

    fn cached_page_fresh(
        &self,
        chunk: &Chunk,
        boundary: Option<Key>,
        order: HistoryOrder,
        generation: u64,
        limit: usize,
        candidates: &[&Message],
        more_cached: bool,
    ) -> Result<bool, HistoryError> {
        let payloads_fresh = candidates.iter().take(limit).all(|message| {
            message
                .id
                .as_deref()
                .and_then(|id| Self::id(id).ok())
                .and_then(|id| self.entries.get(&id))
                .is_some_and(|entry| entry.validated_generation == generation)
        });
        let start_proven = match boundary {
            Some(key) => {
                chunk.contains(key)?
                    || match order {
                        HistoryOrder::Asc => chunk.generation == generation,
                        HistoryOrder::Desc => {
                            (chunk.newest_complete && chunk.live_generation == generation)
                                || chunk.generation == generation
                        }
                    }
            }
            None => match order {
                HistoryOrder::Desc => chunk.newest_complete && chunk.live_generation == generation,
                HistoryOrder::Asc => chunk.oldest_complete && chunk.generation == generation,
            },
        };
        let terminal_proven = more_cached
            || match order {
                HistoryOrder::Desc => chunk.oldest_complete && chunk.generation == generation,
                HistoryOrder::Asc => chunk.newest_complete && chunk.live_generation == generation,
            };
        Ok(payloads_fresh && start_proven && terminal_proven)
    }
}
