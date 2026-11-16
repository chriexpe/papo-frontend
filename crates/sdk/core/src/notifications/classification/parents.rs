//! Reply-parent author evidence for one channel. It owns what is known about the message an
//! unread reply answers and when a negative finding stops being trustworthy; trigger rules
//! stay with the classifier.

use crate::{HistoryGap, Uuid};
use papo_api::{HistoryOrder, wire::Message};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// What the retained history says about one reply parent.
#[derive(Clone, Copy)]
pub(super) enum Parent {
    /// The parent is retained and names this author.
    Author(Uuid),
    /// The parent is retained but names no valid author, so it cannot be ruled out.
    UnknownAuthor,
    /// No retained message has this ID. Only valid for the history it was searched in.
    Absent,
}

/// The history shape an `Absent` verdict was proven against.
///
/// History is ascending by `(created_at_us, UUID)` and a parent precedes its reply, so a missing
/// parent can only arrive through a backfill before the first row or a fill of a known gap.
/// Live appends, edits, reactions and deletions cannot supply it. A parent that arrives any other
/// way (for example a late row with a skewed clock) leaves the reply incomplete, which keeps the
/// count a lower bound: the failure is never a wrong exact count.
///
/// The basis is compared when the evidence is next prepared, not when the gaps change. It leaves
/// out the newest-edge gap: the last gap, when it fills upward, sits at the cached tail and moves
/// with every live append, and what it would supply is newer than every cached row.
#[derive(Clone, Default)]
struct AbsenceBasis {
    first: Option<Arc<Message>>,
    gaps: Vec<HistoryGap>,
}

/// Gaps that can still hold an older row: all but a newest-edge gap that fills upward.
fn interior(gaps: &[HistoryGap]) -> &[HistoryGap] {
    match gaps.split_last() {
        Some((newest, rest)) if newest.order == HistoryOrder::Asc => rest,
        _ => gaps,
    }
}

impl AbsenceBasis {
    fn of(messages: &imbl::Vector<Arc<Message>>, gaps: &[HistoryGap]) -> Self {
        Self {
            first: messages.front().cloned(),
            gaps: interior(gaps).to_vec(),
        }
    }

    fn still_holds(&self, messages: &imbl::Vector<Arc<Message>>, gaps: &[HistoryGap]) -> bool {
        let same_first = match (&self.first, messages.front()) {
            (Some(known), Some(current)) => Arc::ptr_eq(known, current),
            (None, None) => true,
            _ => false,
        };
        let gaps = interior(gaps);
        same_first
            && self.gaps.len() == gaps.len()
            && self.gaps.iter().zip(gaps).all(|(known, current)| {
                known.cursor == current.cursor && known.order == current.order
            })
    }
}

/// Parent evidence keyed by message ID. A message's author never changes, so a found parent
/// stays valid across snapshots; only `Absent` depends on the history it was searched in.
#[derive(Clone, Default)]
pub(super) struct ParentEvidence {
    parents: BTreeMap<Uuid, Parent>,
    absence_basis: AbsenceBasis,
    #[cfg(test)]
    pub rows_examined: usize,
}

impl ParentEvidence {
    pub fn get(&self, parent: &Uuid) -> Option<Parent> {
        self.parents.get(parent).copied()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.parents.len()
    }

    /// Keeps exactly the evidence for `wanted`, searching `messages` once for parents that have
    /// none. The scan runs from the newest row and stops as soon as every searched parent is
    /// found, so history is walked in full only for a parent that is not retained.
    pub fn refresh(
        &mut self,
        messages: &imbl::Vector<Arc<Message>>,
        gaps: &[HistoryGap],
        wanted: &BTreeSet<Uuid>,
    ) {
        if !self.absence_basis.still_holds(messages, gaps) {
            self.parents
                .retain(|_, parent| !matches!(parent, Parent::Absent));
            self.absence_basis = AbsenceBasis::of(messages, gaps);
        }
        self.parents.retain(|id, _| wanted.contains(id));
        let mut searching: BTreeSet<Uuid> = wanted
            .iter()
            .filter(|id| !self.parents.contains_key(id))
            .copied()
            .collect();
        if searching.is_empty() {
            return;
        }
        // Newest first: a reply usually answers a recent message, so the search ends early.
        for message in messages.iter().rev() {
            #[cfg(test)]
            {
                self.rows_examined += 1;
            }
            let Some(id) = message.id.as_deref().and_then(|id| id.parse().ok()) else {
                continue;
            };
            if !searching.remove(&id) {
                continue;
            }
            let parent = match message.author_id.as_deref().and_then(|id| id.parse().ok()) {
                Some(author) => Parent::Author(author),
                None => Parent::UnknownAuthor,
            };
            self.parents.insert(id, parent);
            if searching.is_empty() {
                return;
            }
        }
        for id in searching {
            self.parents.insert(id, Parent::Absent);
        }
    }
}
