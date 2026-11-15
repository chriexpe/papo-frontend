//! The coverage oracle: accepted evidence as plain facts and a brute-force discrete key line.
//! It never consults `coverage`; the simulated client and server live in `simulation`.

use std::collections::BTreeSet;

use papo_api::{HistoryOrder, Uuid};

use super::super::{coverage::Edge, message_key::MessageKey};

/// One accepted piece of evidence. An unbounded upper edge holds only in `live`'s generation.
#[derive(Clone, Copy, Debug)]
pub(super) struct Fact {
    pub(super) lower: Edge,
    pub(super) upper: Edge,
    pub(super) live: Option<u64>,
    /// An empty page from this cursor in this direction produced the fact.
    pub(super) empty_from: Option<(MessageKey, HistoryOrder)>,
}

/// No key fits between the two facts.
pub(super) fn touches(a: &Fact, b: &Fact) -> bool {
    pub(super) fn reaches(lower: Edge, upper: Edge) -> bool {
        match (lower, upper) {
            (Edge::Unbounded, _) | (_, Edge::Unbounded) => true,
            (Edge::Open(lower), Edge::Open(upper)) => lower < upper,
            (Edge::Closed(lower) | Edge::Open(lower), Edge::Closed(upper) | Edge::Open(upper)) => {
                lower <= upper
            }
        }
    }
    reaches(a.lower, b.upper) && reaches(b.lower, a.upper)
}

pub(super) fn admits_from_below(lower: Edge, point: MessageKey) -> bool {
    match lower {
        Edge::Unbounded => true,
        Edge::Closed(edge) => edge <= point,
        Edge::Open(edge) => edge < point,
    }
}

pub(super) fn admits_from_above(upper: Edge, point: MessageKey) -> bool {
    match upper {
        Edge::Unbounded => true,
        Edge::Closed(edge) => point <= edge,
        Edge::Open(edge) => point < edge,
    }
}

#[derive(Default)]
pub(super) struct Oracle {
    pub(super) facts: Vec<Fact>,
    /// Message keys the client accepted and has not discarded, deleted or not.
    pub(super) accepted: BTreeSet<MessageKey>,
}

impl Oracle {
    pub(super) fn accept_page(
        &mut self,
        cursor: Option<MessageKey>,
        order: HistoryOrder,
        returned: &[MessageKey],
        has_more: bool,
        generation: u64,
    ) {
        self.accepted.extend(returned.iter().copied());
        let empty_from = cursor
            .filter(|_| returned.is_empty())
            .map(|cursor| (cursor, order));
        if let Some((cursor, _)) = empty_from {
            // The client declines an empty page on the other side of an unproven cursor that
            // another empty page already anchors: it would prove the cursor itself.
            let opposite = match order {
                HistoryOrder::Asc => HistoryOrder::Desc,
                HistoryOrder::Desc => HistoryOrder::Asc,
            };
            if self
                .facts
                .iter()
                .any(|fact| fact.empty_from == Some((cursor, opposite)))
                && !self.covers_point(cursor, generation)
            {
                return;
            }
        }
        let cursor_edge = cursor.map_or(Edge::Unbounded, Edge::Open);
        let far = match returned.last() {
            Some(far) if has_more => Edge::Closed(*far),
            _ => Edge::Unbounded,
        };
        let fact = match order {
            HistoryOrder::Asc => Fact {
                lower: cursor_edge,
                upper: far,
                live: (far == Edge::Unbounded).then_some(generation),
                empty_from,
            },
            HistoryOrder::Desc => Fact {
                lower: far,
                upper: cursor_edge,
                live: (cursor_edge == Edge::Unbounded).then_some(generation),
                empty_from,
            },
        };
        self.facts.push(fact);
    }

    /// The gateway delivers every later message in this generation.
    pub(super) fn accept_live(&mut self, point: MessageKey, generation: u64) {
        self.accepted.insert(point);
        self.facts.push(Fact {
            lower: Edge::Closed(point),
            upper: Edge::Unbounded,
            live: Some(generation),
            empty_from: None,
        });
    }

    /// A live edge from an ended generation still proves up to the newest message accepted
    /// inside it, never beyond.
    pub(super) fn reconnect(&mut self, ended: u64) {
        for fact in &mut self.facts {
            if fact.live != Some(ended) {
                continue;
            }
            let newest = self
                .accepted
                .iter()
                .rev()
                .find(|point| admits_from_below(fact.lower, **point))
                .copied();
            fact.upper = match (newest, fact.lower) {
                (Some(newest), _) => Edge::Closed(newest),
                (None, Edge::Open(anchor) | Edge::Closed(anchor)) => Edge::Closed(anchor),
                (None, Edge::Unbounded) => Edge::Open(MessageKey {
                    created_at_us: i64::MIN,
                    message_id: Uuid::nil(),
                }),
            };
            fact.live = None;
        }
    }

    /// Retention moved the oldest run's first message up to `first`: evidence below it goes.
    pub(super) fn retain_from(&mut self, first: MessageKey) {
        self.accepted.retain(|point| *point >= first);
        for fact in &mut self.facts {
            let below = match fact.lower {
                Edge::Unbounded => true,
                Edge::Closed(lower) | Edge::Open(lower) => lower < first,
            };
            if below {
                fact.lower = Edge::Closed(first);
            }
        }
    }

    /// Retention discarded a whole run: every fact inside `run`, or joined to one that is.
    pub(super) fn discard_run(&mut self, run: Fact, generation: u64) {
        let valid: Vec<_> = self.valid(generation).collect();
        let mut discarded: Vec<bool> = valid.iter().map(|fact| touches(fact, &run)).collect();
        loop {
            let mut grew = false;
            for index in 0..valid.len() {
                if discarded[index] {
                    continue;
                }
                let joined = (0..valid.len())
                    .any(|other| discarded[other] && touches(&valid[index], &valid[other]));
                if joined {
                    discarded[index] = true;
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        let mut index = 0;
        self.facts.retain(|_| {
            index += 1;
            !discarded[index - 1]
        });
    }

    pub(super) fn covers_point(&self, point: MessageKey, generation: u64) -> bool {
        self.valid(generation).any(|fact| {
            admits_from_below(fact.lower, point) && admits_from_above(fact.upper, point)
        })
    }

    /// Covers the open region strictly between two adjacent universe keys.
    pub(super) fn covers_region(
        &self,
        below: Option<MessageKey>,
        above: Option<MessageKey>,
        generation: u64,
    ) -> bool {
        self.valid(generation).any(|fact| {
            let lower = match (fact.lower, below) {
                (Edge::Unbounded, _) => true,
                (_, None) => false,
                (Edge::Closed(edge) | Edge::Open(edge), Some(below)) => edge <= below,
            };
            let upper = match (fact.upper, above) {
                (Edge::Unbounded, _) => true,
                (_, None) => false,
                (Edge::Closed(edge) | Edge::Open(edge), Some(above)) => above <= edge,
            };
            lower && upper
        })
    }

    pub(super) fn valid(&self, generation: u64) -> impl Iterator<Item = Fact> + '_ {
        self.facts.iter().map(move |fact| match fact.live {
            Some(live) if live != generation => Fact {
                upper: Edge::Open(MessageKey {
                    created_at_us: i64::MIN,
                    message_id: Uuid::nil(),
                }),
                ..*fact
            },
            _ => *fact,
        })
    }
}

/// Discrete view of the key line: slot `2i` is the open region below universe key `i`, slot
/// `2i + 1` is key `i`, and the final slot is the region above the newest key.
pub(super) struct Slots {
    pub(super) universe: Vec<MessageKey>,
    pub(super) covered: Vec<bool>,
}

impl Slots {
    pub(super) fn new(universe: Vec<MessageKey>, oracle: &Oracle, generation: u64) -> Self {
        let mut covered = Vec::with_capacity(universe.len() * 2 + 1);
        for (index, point) in universe.iter().enumerate() {
            let below = index.checked_sub(1).map(|below| universe[below]);
            covered.push(oracle.covers_region(below, Some(*point), generation));
            covered.push(oracle.covers_point(*point, generation));
        }
        covered.push(oracle.covers_region(universe.last().copied(), None, generation));
        Self { universe, covered }
    }

    pub(super) fn slot_of(&self, point: MessageKey) -> usize {
        self.universe.binary_search(&point).unwrap() * 2 + 1
    }

    /// Slots strictly between two bounds.
    pub(super) fn between(
        &self,
        after: Option<MessageKey>,
        before: Option<MessageKey>,
    ) -> std::ops::Range<usize> {
        let start = after.map_or(0, |after| self.slot_of(after) + 1);
        let end = before.map_or(self.covered.len(), |before| self.slot_of(before));
        start..end.max(start)
    }

    pub(super) fn proves(&self, after: Option<MessageKey>, before: Option<MessageKey>) -> bool {
        self.covered[self.between(after, before)]
            .iter()
            .all(|covered| *covered)
    }

    /// Maximal covered stretches as slot ranges.
    pub(super) fn runs(&self) -> Vec<std::ops::Range<usize>> {
        let mut runs = Vec::new();
        let mut start = None;
        for (slot, covered) in self.covered.iter().enumerate() {
            match (start, covered) {
                (None, true) => start = Some(slot),
                (Some(first), false) => {
                    runs.push(first..slot);
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(first) = start {
            runs.push(first..self.covered.len());
        }
        runs
    }
}
