//! Cached objective message facts; current resources resolve reply and broadcast authority.

mod parents;
mod rules;

use crate::{HistorySnapshot, ResourceSnapshot, Uuid, authored_by_other};
use papo_api::wire::Message;
use parents::{Parent, ParentEvidence};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
struct Facts {
    message: Arc<Message>,
    direct: bool,
    everyone: bool,
}

/// Objective trigger evidence, including whether all referenced evidence is available.
#[derive(Clone, Copy)]
pub(crate) struct Classification {
    pub triggered: bool,
    pub complete: bool,
}
impl Classification {
    const NONE: Self = Self {
        triggered: false,
        complete: true,
    };
    const TRIGGER: Self = Self {
        triggered: true,
        complete: true,
    };
    const INCOMPLETE: Self = Self {
        triggered: false,
        complete: false,
    };
}

/// Mentions counted over a range, and whether every row's evidence was available.
#[derive(Clone, Copy)]
pub(crate) struct Tally {
    pub triggered: u64,
    pub complete: bool,
}
impl Tally {
    pub const EMPTY: Self = Self {
        triggered: 0,
        complete: true,
    };

    fn add(&mut self, row: Classification) {
        self.triggered += u64::from(row.triggered);
        self.complete &= row.complete;
    }

    fn of(rows: impl IntoIterator<Item = Classification>) -> Self {
        let mut tally = Self::EMPTY;
        for row in rows {
            tally.add(row);
        }
        tally
    }
}

#[derive(Clone, Default)]
struct ChannelFacts {
    #[cfg(test)]
    classified_revisions: usize,
    has_broadcast: bool,
    /// A row of the range could not be classified: it can trigger but has no parseable ID to key
    /// a record, or the range reaches past the history.
    unresolved_rows: bool,
    records: BTreeMap<Uuid, Facts>,
    parents: ParentEvidence,
}

/// Account-owned. A channel with unread rows that count mentions holds facts for exactly the unread
/// rows that can trigger, and no history vector, so it never keeps a trimmed history reachable
/// beyond those rows. A channel with no unread rows, one whose mentions are not counted, one with
/// no resident history, and every direct conversation holds nothing; settings never invalidate a
/// held entry.
#[derive(Default)]
pub(crate) struct ClassificationCache {
    channels: BTreeMap<Uuid, Arc<ChannelFacts>>,
}
impl ClassificationCache {
    /// Compute on detached channel facts, keeping scans outside the shared cache lock.
    /// Concurrent producers may duplicate cache work, but never share mutable facts or results.
    pub fn project(
        shared: &std::sync::Mutex<Self>,
        channel: Uuid,
        snapshot: &HistorySnapshot,
        user: Uuid,
        resources: &ResourceSnapshot,
        range: std::ops::Range<usize>,
    ) -> Tally {
        if range.is_empty() {
            Self::evict(shared, channel);
            return Tally::EMPTY;
        }
        let previous = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .channels
            .get(&channel)
            .cloned();
        let mut local = Self {
            channels: BTreeMap::from([(channel, previous.clone().unwrap_or_default())]),
        };
        let result = local.tally_range(channel, &snapshot.messages, user, resources, range);
        if let Some(facts) = local.channels.remove(&channel) {
            Self::install(shared, channel, previous.as_ref(), facts);
        }
        result
    }

    /// Installs `facts` only while the shared entry is still the one the computation started
    /// from, so an eviction, a retain or a faster producer is never overwritten with older work.
    /// The result of the computation stays valid for its caller either way.
    fn install(
        shared: &std::sync::Mutex<Self>,
        channel: Uuid,
        started_from: Option<&Arc<ChannelFacts>>,
        facts: Arc<ChannelFacts>,
    ) -> bool {
        let mut shared = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let unchanged = match (shared.channels.get(&channel), started_from) {
            (Some(current), Some(started_from)) => Arc::ptr_eq(current, started_from),
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            shared.channels.insert(channel, facts);
        }
        unchanged
    }

    pub fn reply_message(&self, channel: Uuid, reply: Uuid) -> Option<Arc<Message>> {
        self.channels
            .get(&channel)?
            .records
            .get(&reply)
            .map(|facts| Arc::clone(&facts.message))
    }

    pub fn broadcast_channels(&self) -> BTreeSet<Uuid> {
        self.channels
            .iter()
            .filter_map(|(channel, facts)| facts.has_broadcast.then_some(*channel))
            .collect()
    }

    /// Drops a channel's facts so a read or muted channel pins no history. An in-flight
    /// `project` that started from an entry cached before this cannot reinstall it: its install
    /// is compare-checked. One that started with no entry may leave a stale one until the next
    /// report of that channel.
    pub fn evict(shared: &std::sync::Mutex<Self>, channel: Uuid) {
        let removed = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .channels
            .remove(&channel);
        drop(removed);
    }

    pub fn retain(shared: &std::sync::Mutex<Self>, channels: &BTreeSet<Uuid>) {
        let removed = {
            let mut cache = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let removed: Vec<_> = cache
                .channels
                .keys()
                .filter(|channel| !channels.contains(channel))
                .copied()
                .collect();
            removed
                .into_iter()
                .filter_map(|channel| cache.channels.remove(&channel))
                .collect::<Vec<_>>()
        };
        drop(removed);
    }

    #[cfg(test)]
    pub fn retain_channels(&mut self, channels: &BTreeSet<Uuid>) {
        self.channels
            .retain(|channel, _| channels.contains(channel));
    }

    #[cfg(test)]
    pub fn classify(
        &mut self,
        channel: Uuid,
        messages: &imbl::Vector<Arc<Message>>,
        user: Uuid,
        resources: &ResourceSnapshot,
    ) -> Vec<Classification> {
        self.classify_range(channel, messages, user, resources, 0..messages.len())
    }

    /// Counts triggers in `range`. Outside a direct conversation only rows that can trigger
    /// matter, so the cost after preparation follows their number, not the range length.
    pub fn tally_range(
        &mut self,
        channel: Uuid,
        messages: &imbl::Vector<Arc<Message>>,
        user: Uuid,
        resources: &ResourceSnapshot,
        range: std::ops::Range<usize>,
    ) -> Tally {
        if is_direct(resources, channel) {
            return Tally::of(self.classify_range(channel, messages, user, resources, range));
        }
        let cache = self.prepared(channel, messages, user, range);
        let mut tally = Tally::EMPTY;
        tally.complete = !cache.unresolved_rows;
        for facts in cache.records.values() {
            tally.add(rules::classify_candidate(
                &facts.message,
                facts,
                &cache.parents,
                user,
                resources,
            ));
        }
        tally
    }

    /// Brings the channel's facts in line with `range`. Rows whose revision is unchanged keep
    /// their facts, so a refresh costs a scan of the range, not a reclassification.
    fn prepared(
        &mut self,
        channel: Uuid,
        messages: &imbl::Vector<Arc<Message>>,
        user: Uuid,
        range: std::ops::Range<usize>,
    ) -> &ChannelFacts {
        let cache = self.channels.entry(channel).or_default();
        Arc::make_mut(cache).prepare(messages, user, range);
        cache
    }

    pub fn classify_range(
        &mut self,
        channel: Uuid,
        messages: &imbl::Vector<Arc<Message>>,
        user: Uuid,
        resources: &ResourceSnapshot,
        range: std::ops::Range<usize>,
    ) -> Vec<Classification> {
        let mut rows = messages.focus();
        if is_direct(resources, channel) {
            // Every message from someone else triggers, so no facts or parent evidence are needed.
            return range
                .map(|index| match rows.get(index) {
                    None => Classification::INCOMPLETE,
                    Some(message) if authored_by_other(message, user) => Classification::TRIGGER,
                    Some(_) => Classification::NONE,
                })
                .collect();
        }
        let cache = self.prepared(channel, messages, user, range.clone());
        range
            .map(|index| {
                let Some(message) = rows.get(index) else {
                    return Classification::INCOMPLETE;
                };
                if !can_trigger(message) || !authored_by_other(message, user) {
                    return Classification::NONE;
                }
                let stored = message
                    .id
                    .as_deref()
                    .and_then(|id| id.parse().ok())
                    .and_then(|id| cache.records.get(&id));
                match stored {
                    Some(facts) => {
                        rules::classify_candidate(message, facts, &cache.parents, user, resources)
                    }
                    // Not stored: either nothing in the row can trigger, or it has no usable ID.
                    None if Facts::new(message, user).triggers(message) => {
                        Classification::INCOMPLETE
                    }
                    None => Classification::NONE,
                }
            })
            .collect()
    }
}
impl ChannelFacts {
    fn prepare(
        &mut self,
        messages: &imbl::Vector<Arc<Message>>,
        user: Uuid,
        range: std::ops::Range<usize>,
    ) {
        let mut previous = std::mem::take(&mut self.records);
        let mut reply_parents = BTreeSet::new();
        let mut unresolved_rows = range.end > messages.len();
        let mut rows = messages.focus();
        for index in range {
            let Some(message) = rows.get(index) else {
                continue;
            };
            if !can_trigger(message) || !authored_by_other(message, user) {
                continue;
            }
            let id: Option<Uuid> = message.id.as_deref().and_then(|id| id.parse().ok());
            let facts = match id.and_then(|id| previous.remove(&id)) {
                Some(mut facts) if same_revision(&facts.message, message) => {
                    facts.message = Arc::clone(message);
                    facts
                }
                _ => {
                    let facts = Facts::new(message, user);
                    #[cfg(test)]
                    {
                        self.classified_revisions += usize::from(facts.triggers(message));
                    }
                    facts
                }
            };
            if !facts.triggers(message) {
                continue;
            }
            let Some(id) = id else {
                unresolved_rows = true;
                continue;
            };
            if !facts.direct
                && let Some(reply) = message.reply_to.as_deref().and_then(|id| id.parse().ok())
            {
                reply_parents.insert(reply);
            }
            self.records.insert(id, facts);
        }
        self.unresolved_rows = unresolved_rows;
        self.parents.refresh(messages, &reply_parents);
        self.has_broadcast = self.records.values().any(|facts| facts.everyone);
    }
}

fn is_direct(resources: &ResourceSnapshot, channel: Uuid) -> bool {
    resources
        .dms
        .iter()
        .any(|dm| dm.id.parse().ok() == Some(channel))
}

/// A cheap necessary condition for any trigger outside a DM: both mention syntaxes contain `@`
/// and a reply names a parent. Rows failing it need no author or ID parsing and no record. Rows
/// passing it still need `Facts::triggers`: ordinary `@text` or an e-mail address holds no mention.
fn can_trigger(message: &Message) -> bool {
    message.reply_to.is_some()
        || message
            .content
            .as_deref()
            .is_some_and(|content| content.as_bytes().contains(&b'@'))
}

impl Facts {
    fn new(message: &Arc<Message>, user: Uuid) -> Self {
        let content = message.content.as_deref().unwrap_or_default();
        Self {
            message: Arc::clone(message),
            direct: crate::mentions::contains_user_mention(content, user),
            everyone: crate::mentions::contains_everyone_mention(content),
        }
    }

    /// Whether the row can trigger at all: a mention of the account, an `@everyone` whose
    /// authority resources decide later, or a reply whose parent evidence decides later.
    fn triggers(&self, message: &Message) -> bool {
        self.direct || self.everyone || message.reply_to.is_some()
    }
}
fn same_revision(left: &Message, right: &Message) -> bool {
    left.content == right.content
        && left.author_id == right.author_id
        && left.reply_to == right.reply_to
        && left.webhook_id == right.webhook_id
        && left.edited_at == right.edited_at
}

#[cfg(test)]
mod cache_tests;
#[cfg(test)]
mod parent_tests;
#[cfg(test)]
mod tests;
