//! What a server page proves, recorded as a chunk, and the joining of chunks whose coverage
//! connects. Message merging and deletion by omission stay in `merge`; interpreting coverage
//! (edges, extents, gaps) stays in `coverage`.

use papo_api::HistoryOrder;

use super::{
    HistoryError,
    coverage::{Edge, Extent, highest, lowest},
    merge::Ticket,
    message_key::MessageKey,
    model::{Chunk, Model},
    ordering::Point,
};

impl Model {
    /// Records what an empty page proves: the channel is empty (no cursor), the held run
    /// containing the cursor is complete on that side, or, for a cursor the model does not
    /// hold, an anchor-only run proving everything beyond it.
    pub(super) fn finish_empty(
        &mut self,
        ticket: &Ticket,
        has_more: bool,
    ) -> Result<(), HistoryError> {
        if has_more {
            return Err(HistoryError::InvalidPage);
        }
        let Some(cursor) = &ticket.cursor else {
            self.chunks.clear();
            self.empty_generation = ticket.generation;
            self.server_live_generation = ticket.generation;
            if let (Some((_, first_id)), Some((_, last_id))) =
                (self.order.first_key_value(), self.order.last_key_value())
            {
                let first = self.entries[first_id]
                    .point
                    .clone()
                    .ok_or(HistoryError::InvalidPage)?;
                let last = self.entries[last_id]
                    .point
                    .clone()
                    .ok_or(HistoryError::InvalidPage)?;
                self.chunks.push(Chunk {
                    id: *first_id,
                    first,
                    last,
                    oldest_complete: true,
                    newest_complete: true,
                    generation: ticket.generation,
                    live_generation: ticket.generation,
                    oldest_anchor: None,
                    newest_anchor: None,
                });
            }
            return Ok(());
        };
        let key = cursor.key()?;
        let ascending = ticket.order == HistoryOrder::Asc;
        let mut held = false;
        for chunk in &mut self.chunks {
            if chunk.contains(key)? {
                held = true;
                // An anchor-only chunk does not hold its cursor: a proof from the other side
                // would join both sides across that unknown key, so it is declined.
                let same_side = if ascending {
                    chunk.oldest_anchor.as_ref() == Some(cursor)
                } else {
                    chunk.newest_anchor.as_ref() == Some(cursor)
                };
                if chunk.is_anchor_only() && !same_side {
                    continue;
                }
                match ticket.order {
                    HistoryOrder::Desc => chunk.oldest_complete = true,
                    HistoryOrder::Asc => {
                        chunk.newest_complete = true;
                        chunk.live_generation = ticket.generation;
                        self.server_live_generation = ticket.generation;
                    }
                }
            }
        }
        if held {
            return Ok(());
        }
        // An empty page from a cursor the model does not hold still proves everything on its
        // side of that cursor; the cursor itself stays unknown.
        if ascending {
            self.server_live_generation = ticket.generation;
        }
        let proof = Chunk {
            id: key.id_bytes(),
            first: cursor.clone(),
            last: cursor.clone(),
            oldest_complete: !ascending,
            newest_complete: ascending,
            generation: ticket.generation,
            live_generation: if ascending { ticket.generation } else { 0 },
            oldest_anchor: ascending.then(|| cursor.clone()),
            newest_anchor: (!ascending).then(|| cursor.clone()),
        };
        self.insert_chunk(proof, ticket, &page_extent(ticket, None, false)?)
    }

    /// Records what a non-empty page proves as a run anchored at its cursor and joins it.
    pub(super) fn merge_coverage(
        &mut self,
        ticket: &Ticket,
        points: &[Point],
        has_more: bool,
    ) -> Result<(), HistoryError> {
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return Err(HistoryError::InvalidPage);
        };
        let (first, last) = match ticket.order {
            HistoryOrder::Desc => (last.clone(), first.clone()),
            HistoryOrder::Asc => (first.clone(), last.clone()),
        };
        let ascending = ticket.order == HistoryOrder::Asc;
        let newest_complete = if ascending {
            !has_more
        } else {
            ticket.cursor.is_none()
        };
        if newest_complete {
            self.server_live_generation = ticket.generation;
        }
        // The page's cursor is recorded as an anchor on the side it was fetched from. When the
        // model holds that cursor, joining with its chunk replaces the anchor with a real edge.
        let page = Chunk {
            id: first.key()?.id_bytes(),
            first,
            last,
            oldest_complete: if ascending {
                ticket.cursor.is_none()
            } else {
                !has_more
            },
            newest_complete,
            generation: ticket.generation,
            live_generation: if newest_complete {
                ticket.generation
            } else {
                0
            },
            oldest_anchor: ticket.cursor.clone().filter(|_| ascending),
            newest_anchor: ticket.cursor.clone().filter(|_| !ascending),
        };
        let far = points.last().map(Point::key).transpose()?;
        self.insert_chunk(page, ticket, &page_extent(ticket, far, has_more)?)
    }

    /// Adds a newly proven run and joins every chunk whose coverage connects with it.
    /// `proved` is what the server response itself proves; a joined chunk inside it counts as
    /// revalidated in this generation, any other keeps its older validation generation.
    fn insert_chunk(
        &mut self,
        added: Chunk,
        ticket: &Ticket,
        proved: &Extent,
    ) -> Result<(), HistoryError> {
        let generation = ticket.generation;
        if !added.is_anchor_only() {
            self.retire_contradicted_live_edges(&added, generation)?;
        }
        let mut extent = added.extent(generation)?;
        let mut members = vec![added];
        let mut validated = generation;
        let mut rest = std::mem::take(&mut self.chunks);
        // Joining can widen the run, so repeat until no remaining chunk connects.
        loop {
            let mut joined_any = false;
            let mut retained = Vec::with_capacity(rest.len());
            for chunk in rest {
                let other = chunk.extent(generation)?;
                if !extent.connects(&other) {
                    retained.push(chunk);
                    continue;
                }
                if !proved.contains(&chunk.bounds()?) {
                    validated = validated.min(chunk.generation);
                }
                extent.join(&other);
                members.push(chunk);
                joined_any = true;
            }
            rest = retained;
            if !joined_any {
                break;
            }
        }
        let mut joined = Chunk::union(members)?;
        joined.generation = validated;
        rest.push(joined);
        rest.sort_by_key(|chunk| chunk.first.key().ok());
        self.chunks = rest;
        Ok(())
    }

    /// Messages fetched beyond a live edge, not reached by that run's own boundaries, did not
    /// arrive live (the gateway lags the page): the edge is no longer evidence, as when a live
    /// arrival lands outside every run.
    fn retire_contradicted_live_edges(
        &mut self,
        added: &Chunk,
        generation: u64,
    ) -> Result<(), HistoryError> {
        let added_bounds = added.bounds()?;
        for chunk in &mut self.chunks {
            let live = chunk.extent(generation)?.upper == Edge::Unbounded;
            if live
                && !chunk.bounds()?.connects(&added_bounds)
                && chunk.last.key()? < added.first.key()?
            {
                chunk.newest_complete = false;
                chunk.live_generation = 0;
            }
        }
        Ok(())
    }
}

/// What one server response proves on its own: everything between its cursor (excluded) and
/// its far end, or without limit when the server reported no more messages.
fn page_extent(
    ticket: &Ticket,
    far: Option<MessageKey>,
    has_more: bool,
) -> Result<Extent, HistoryError> {
    let cursor = match &ticket.cursor {
        Some(cursor) => Edge::Open(cursor.key()?),
        None => Edge::Unbounded,
    };
    let far = match far {
        Some(far) if has_more => Edge::Closed(far),
        _ => Edge::Unbounded,
    };
    Ok(match ticket.order {
        HistoryOrder::Asc => Extent {
            lower: cursor,
            upper: far,
        },
        HistoryOrder::Desc => Extent {
            lower: far,
            upper: cursor,
        },
    })
}

impl Chunk {
    /// One chunk proving what connected `members` prove together. Message boundaries come
    /// from members that have them. An anchor survives where it reaches past every member's
    /// boundary, independent of any generation's live edge; an anchor-only member's anchor
    /// bounds only its own side (an empty `Asc` page never proves anything below its cursor).
    /// The caller sets the validation generation.
    pub fn union(members: Vec<Self>) -> Result<Self, HistoryError> {
        let mut lower: Option<Edge> = None;
        let mut upper: Option<Edge> = None;
        for member in &members {
            let bounds = member.bounds()?;
            let anchor_only = member.is_anchor_only();
            if !anchor_only || member.oldest_anchor.is_some() {
                lower = Some(lower.map_or(bounds.lower, |edge| lowest(edge, bounds.lower)));
            }
            if !anchor_only || member.newest_anchor.is_some() {
                upper = Some(upper.map_or(bounds.upper, |edge| highest(edge, bounds.upper)));
            }
        }
        let mut first: Option<(MessageKey, Point)> = None;
        let mut last: Option<(MessageKey, Point)> = None;
        let mut anchors = Vec::new();
        let mut id = [u8::MAX; 16];
        let (mut oldest_complete, mut newest_complete, mut live_generation) = (false, false, 0);
        for member in members {
            id = id.min(member.id);
            oldest_complete |= member.oldest_complete;
            newest_complete |= member.newest_complete;
            live_generation = live_generation.max(member.live_generation);
            if !member.is_anchor_only() {
                let member_first = member.first.key()?;
                if first.as_ref().is_none_or(|(key, _)| member_first < *key) {
                    first = Some((member_first, member.first.clone()));
                }
                let member_last = member.last.key()?;
                if last.as_ref().is_none_or(|(key, _)| member_last > *key) {
                    last = Some((member_last, member.last.clone()));
                }
            }
            anchors.extend(member.oldest_anchor);
            anchors.extend(member.newest_anchor);
        }
        let anchor_at = |key: MessageKey| -> Result<Option<Point>, HistoryError> {
            for anchor in &anchors {
                if anchor.key()? == key {
                    return Ok(Some(anchor.clone()));
                }
            }
            Ok(None)
        };
        let oldest_anchor = match lower {
            Some(Edge::Open(key)) => anchor_at(key)?,
            _ => None,
        };
        let newest_anchor = match upper {
            Some(Edge::Open(key)) => anchor_at(key)?,
            _ => None,
        };
        // A run made only of anchors keeps the anchor as its boundary.
        let first = first
            .map(|(_, point)| point)
            .or_else(|| oldest_anchor.clone())
            .or_else(|| newest_anchor.clone())
            .or_else(|| anchors.first().cloned())
            .ok_or(HistoryError::InvalidPage)?;
        let last = last.map_or_else(|| first.clone(), |(_, point)| point);
        Ok(Self {
            id,
            first,
            last,
            oldest_complete,
            newest_complete,
            generation: 0,
            live_generation,
            oldest_anchor,
            newest_anchor,
        })
    }
}
