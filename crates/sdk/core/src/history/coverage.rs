//! Proven coverage: which key ranges hold no unknown message, and where the gaps are.
//!
//! A chunk proves its inclusive run `[first, last]`. An anchor extends that proof to an
//! excluded boundary: a page fetched from a cursor the model did not hold proves that the run
//! starts (`Asc`) or ends (`Desc`) right at that cursor, while the cursor itself stays unknown.
//! Oldest completeness proves everything before the run; newest completeness proves everything
//! after it, but only in the connection generation that observed the live edge.
//!
//! Merge (which chunks join), publication (which gaps exist) and the snapshot queries all read
//! coverage through this module, so a published gap and an incomplete range always agree.
//! Recording evidence (pages, anchors, live edges) stays in `chunk_join` and `live_edge`.

use super::message_key::MessageKey;

/// One edge of a proven run, in key order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Edge {
    /// The run extends without limit in this direction.
    Unbounded,
    /// The run includes this key.
    Closed(MessageKey),
    /// The run stops right at this key, which is not itself proven (an anchor).
    Open(MessageKey),
}

/// The keys one chunk, or one connected group of chunks, proves free of unknown messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Extent {
    pub lower: Edge,
    pub upper: Edge,
}

/// The inputs that decide one chunk's extent; both the model and published chunks map onto it.
pub(crate) struct ChunkEvidence {
    pub first: MessageKey,
    pub last: MessageKey,
    pub oldest_complete: bool,
    pub oldest_anchor: Option<MessageKey>,
    /// Newest completeness was observed in the generation being evaluated.
    pub live_edge_current: bool,
    pub newest_anchor: Option<MessageKey>,
}

/// Newest completeness counts as evidence only in the generation that observed it; `0` is a
/// generation that never observed one (restored or unset).
pub(crate) fn live_edge_current(
    newest_complete: bool,
    live_generation: u64,
    generation: u64,
) -> bool {
    newest_complete && live_generation != 0 && live_generation == generation
}

impl ChunkEvidence {
    /// The chunk's single boundary is its own anchor: an empty page from an unheld cursor,
    /// holding no message.
    pub fn is_anchor_only(&self) -> bool {
        self.first == self.last
            && (self.oldest_anchor == Some(self.first) || self.newest_anchor == Some(self.last))
    }
}

impl Extent {
    /// An anchor counts only where it reaches at least to the chunk's own boundary. A chunk
    /// whose single boundary is its anchor holds no message, so neither side proves that key.
    pub fn of(chunk: &ChunkEvidence) -> Self {
        let anchor_only = chunk.is_anchor_only();
        let lower = if chunk.oldest_complete {
            Edge::Unbounded
        } else {
            match chunk.oldest_anchor {
                Some(anchor) if anchor <= chunk.first => Edge::Open(anchor),
                _ if anchor_only => Edge::Open(chunk.first),
                _ => Edge::Closed(chunk.first),
            }
        };
        let upper = if chunk.live_edge_current {
            Edge::Unbounded
        } else {
            match chunk.newest_anchor {
                Some(anchor) if anchor >= chunk.last => Edge::Open(anchor),
                _ if anchor_only => Edge::Open(chunk.last),
                _ => Edge::Closed(chunk.last),
            }
        };
        Self { lower, upper }
    }

    /// No key can lie between the two extents, so together they prove one run.
    pub fn connects(&self, other: &Self) -> bool {
        reaches(self.lower, other.upper) && reaches(other.lower, self.upper)
    }

    /// Widens this extent to cover `other`; callers join only connected extents.
    pub fn join(&mut self, other: &Self) {
        self.lower = lowest(self.lower, other.lower);
        self.upper = highest(self.upper, other.upper);
    }

    /// Every key `inner` proves, this extent proves too.
    pub fn contains(&self, inner: &Self) -> bool {
        lower_rank(self.lower) <= lower_rank(inner.lower)
            && upper_rank(self.upper) >= upper_rank(inner.upper)
    }

    /// No unknown message lies strictly between `after` and `before` (`None` is unbounded).
    pub fn proves(&self, after: Option<MessageKey>, before: Option<MessageKey>) -> bool {
        let lower = match self.lower {
            Edge::Unbounded => true,
            Edge::Closed(edge) | Edge::Open(edge) => after.is_some_and(|after| edge <= after),
        };
        let upper = match self.upper {
            Edge::Unbounded => true,
            Edge::Closed(edge) | Edge::Open(edge) => before.is_some_and(|before| before <= edge),
        };
        lower && upper
    }

    /// The extent proves no key at all, e.g. an anchor-only run whose live edge went stale.
    pub fn is_empty(&self) -> bool {
        match (self.lower, self.upper) {
            (Edge::Unbounded, _) | (_, Edge::Unbounded) => false,
            (Edge::Closed(lower), Edge::Closed(upper)) => lower > upper,
            (Edge::Closed(lower) | Edge::Open(lower), Edge::Closed(upper) | Edge::Open(upper)) => {
                lower >= upper
            }
        }
    }

    /// True when every key this extent proves is at or before `key`.
    fn ends_at_or_before(&self, key: MessageKey) -> bool {
        match self.upper {
            Edge::Unbounded => false,
            Edge::Closed(edge) | Edge::Open(edge) => edge <= key,
        }
    }
}

/// A lower edge reaches an upper edge when no key fits between them. An inclusive edge reaches
/// the same key from either side; two excluded edges at one key leave that key unknown.
fn reaches(lower: Edge, upper: Edge) -> bool {
    match (lower, upper) {
        (Edge::Unbounded, _) | (_, Edge::Unbounded) => true,
        (Edge::Open(lower), Edge::Open(upper)) => lower < upper,
        (Edge::Closed(lower) | Edge::Open(lower), Edge::Closed(upper) | Edge::Open(upper)) => {
            lower <= upper
        }
    }
}

/// The lower edge that proves further down.
pub(crate) fn lowest(a: Edge, b: Edge) -> Edge {
    if lower_rank(b) < lower_rank(a) { b } else { a }
}

/// The upper edge that proves further up.
pub(crate) fn highest(a: Edge, b: Edge) -> Edge {
    if upper_rank(b) > upper_rank(a) { b } else { a }
}

/// Orders lower edges by how far down they prove; an inclusive edge proves more than an
/// excluded one at the same key.
fn lower_rank(edge: Edge) -> (u8, Option<MessageKey>, u8) {
    match edge {
        Edge::Unbounded => (0, None, 0),
        Edge::Closed(key) => (1, Some(key), 0),
        Edge::Open(key) => (1, Some(key), 1),
    }
}

/// Orders upper edges by how far up they prove, mirroring `lower_rank`.
fn upper_rank(edge: Edge) -> (u8, Option<MessageKey>, u8) {
    match edge {
        Edge::Open(key) => (0, Some(key), 0),
        Edge::Closed(key) => (0, Some(key), 1),
        Edge::Unbounded => (1, None, 0),
    }
}

/// Connected chunks grouped into maximal proven runs, ascending and disjoint. Each group names
/// the chunks that supply its oldest and newest edges, which is where its gaps start.
pub(crate) struct Components {
    groups: Vec<Group>,
}

#[derive(Clone, Copy)]
pub(crate) struct Group {
    pub extent: Extent,
    /// Index of the chunk supplying the lower edge.
    pub oldest: usize,
    /// Index of the chunk supplying the upper edge.
    pub newest: usize,
}

impl Components {
    /// Groups extents given in any order; indexes refer to that order. Extents that prove
    /// nothing are left out, so they neither join groups nor produce gaps.
    pub fn build(extents: impl IntoIterator<Item = Extent>) -> Self {
        let mut indexed: Vec<_> = extents
            .into_iter()
            .enumerate()
            .filter(|(_, extent)| !extent.is_empty())
            .collect();
        indexed.sort_by_key(|(_, extent)| lower_rank(extent.lower));
        let mut groups: Vec<Group> = Vec::with_capacity(indexed.len());
        for (index, extent) in indexed {
            if let Some(group) = groups.last_mut()
                && group.extent.connects(&extent)
            {
                if upper_rank(extent.upper) > upper_rank(group.extent.upper) {
                    group.newest = index;
                }
                group.extent.join(&extent);
                continue;
            }
            groups.push(Group {
                extent,
                oldest: index,
                newest: index,
            });
        }
        Self { groups }
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// No unknown message lies strictly between the bounds. Only the group reaching past
    /// `after` can prove the range, so this is a binary search, not a scan.
    pub fn proves(&self, after: Option<MessageKey>, before: Option<MessageKey>) -> bool {
        let candidate = self.groups.partition_point(|group| {
            after.is_some_and(|after| group.extent.ends_at_or_before(after))
        });
        self.groups
            .get(candidate)
            .is_some_and(|group| group.extent.proves(after, before))
    }
}

#[cfg(test)]
mod tests {
    //! Edge algebra cases; generated evidence sequences live in `coverage_tests`.
    use super::*;
    use papo_api::Uuid;

    fn key(index: u128) -> MessageKey {
        MessageKey {
            created_at_us: i64::try_from(index).unwrap(),
            message_id: Uuid::from_u128(index),
        }
    }

    fn closed(first: u128, last: u128) -> Extent {
        Extent {
            lower: Edge::Closed(key(first)),
            upper: Edge::Closed(key(last)),
        }
    }

    #[test]
    fn separately_fetched_messages_do_not_prove_the_space_between() {
        let components = Components::build([closed(1, 1), closed(5, 5)]);
        assert_eq!(components.groups().len(), 2);
        assert!(!components.proves(Some(key(1)), Some(key(5))));
    }

    #[test]
    fn an_anchor_reaches_an_inclusive_edge_but_not_another_anchor_at_the_same_key() {
        let anchored = Extent {
            lower: Edge::Open(key(3)),
            upper: Edge::Closed(key(9)),
        };
        assert!(anchored.connects(&closed(1, 3)));
        let ends_at_anchor = Extent {
            lower: Edge::Closed(key(1)),
            upper: Edge::Open(key(3)),
        };
        assert!(!anchored.connects(&ends_at_anchor));
        let components = Components::build([anchored, ends_at_anchor]);
        assert!(!components.proves(Some(key(2)), Some(key(4))));
        assert!(components.proves(Some(key(3)), Some(key(9))));
    }

    #[test]
    fn unbounded_edges_cover_open_ended_ranges_only_when_proven() {
        let live = Extent {
            lower: Edge::Closed(key(4)),
            upper: Edge::Unbounded,
        };
        let components = Components::build([live]);
        assert!(components.proves(Some(key(4)), None));
        assert!(!components.proves(Some(key(3)), None));
        assert!(!components.proves(None, Some(key(9))));
    }
}
