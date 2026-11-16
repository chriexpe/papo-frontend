//! Reply-parent author evidence for one channel. It owns what is known about the message an
//! unread reply answers and when a negative finding stops being trustworthy; trigger rules
//! stay with the classifier.

use crate::Uuid;
use papo_api::wire::Message;
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

/// The history shape an `Absent` verdict was proven against: the IDs at both ends and the length.
///
/// History is ascending by `(created_at_us, UUID)` and a parent precedes its reply, so a missing
/// parent can only arrive in front of the first row or somewhere before the last one. Appends at
/// the tail cannot supply it. A backfill changes the first row; an insertion anywhere before the
/// last row moves it to a later index (or removes it from the old one), so the row recorded as
/// last no longer sits at its recorded position. Only the tail may therefore grow without
/// discarding the verdict. Changes that cancel out (an insertion and a removal in one
/// publication) can keep a stale verdict, which leaves the reply incomplete: the count stays a
/// lower bound and is never a wrong exact count.
///
/// Holds IDs, not rows, so the evidence pins no message payload.
#[derive(Clone, Default)]
struct AbsenceBasis {
    first: Option<Uuid>,
    last: Option<Uuid>,
    len: usize,
}

fn id_at(messages: &imbl::Vector<Arc<Message>>, index: usize) -> Option<Uuid> {
    messages
        .get(index)
        .and_then(|message| message.id.as_deref()?.parse().ok())
}

impl AbsenceBasis {
    fn of(messages: &imbl::Vector<Arc<Message>>) -> Self {
        Self {
            first: id_at(messages, 0),
            last: messages
                .len()
                .checked_sub(1)
                .and_then(|last| id_at(messages, last)),
            len: messages.len(),
        }
    }

    fn still_holds(&self, messages: &imbl::Vector<Arc<Message>>) -> bool {
        let Some(last_index) = self.len.checked_sub(1) else {
            return messages.is_empty();
        };
        self.first.is_some()
            && self.last.is_some()
            && id_at(messages, 0) == self.first
            && id_at(messages, last_index) == self.last
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
    pub fn refresh(&mut self, messages: &imbl::Vector<Arc<Message>>, wanted: &BTreeSet<Uuid>) {
        if !self.absence_basis.still_holds(messages) {
            self.parents
                .retain(|_, parent| !matches!(parent, Parent::Absent));
        }
        self.absence_basis = AbsenceBasis::of(messages);
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
