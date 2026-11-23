//! Who reacted to a timeline's messages, as far as it has been read: the
//! SDK's `reaction_users` pages, oldest reaction first, folded by
//! `Reactors` (`papo_sdk_ui`). A message's list is loaded when the pointer
//! first rests on one of its chips or its reactions list opens, and is
//! dropped once its chips change: the pages no longer match the totals.
//!
//! Owns the folded lists and the requests on their way; the fetching is
//! the shell's, the drawing `reactions` and the menu's reactions list.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use papo_sdk::Uuid;
use papo_sdk::operations::interactions::{ReactorCursor, ReactorPage};
use papo_sdk_ui::{ReactionChip, Reactors};

/// One message's list.
pub(crate) struct ReactorList {
    /// The chips it was built from; other chips mean another list.
    signature: u64,
    pub(crate) reactors: Reactors,
    /// Where the next page starts; none before the first and once all are
    /// read.
    next: Option<ReactorCursor>,
    /// The page on its way, by its number.
    pub(crate) loading: Option<u64>,
    /// The last page could not be read.
    pub(crate) failed: bool,
    /// A first page arrived.
    pub(crate) started: bool,
}

/// What a page is read for.
#[derive(Clone, Copy)]
pub(crate) enum Want<'a> {
    /// A chip's hint: the first page is enough.
    Hint,
    /// The reactions list on this tab (everyone's when none).
    People(Option<&'a papo_sdk_ui::operations::ReactionEmoji>),
}

/// What to fetch for a message: the page after `cursor` (the first when
/// none), as request number `ticket`.
pub(crate) struct PageRequest {
    pub(crate) id: Uuid,
    pub(crate) cursor: Option<ReactorCursor>,
    pub(crate) ticket: u64,
}

/// A timeline's lists, by message.
#[derive(Default)]
pub(crate) struct ReactorLists {
    lists: HashMap<Uuid, ReactorList>,
    /// People whose profiles were asked for, so each is asked once.
    asked: HashSet<Uuid>,
    tickets: u64,
}

impl ReactorLists {
    /// Message `id`'s list, when it matches its `chips`.
    pub(crate) fn get(&self, id: Uuid, chips: &[ReactionChip]) -> Option<&ReactorList> {
        self.lists
            .get(&id)
            .filter(|list| list.signature == signature(chips))
    }

    /// The next page message `id`'s list needs, if one is needed and none
    /// is on its way: for a hint, only the first; for the list, until it
    /// shows `filter`'s people (everyone's when none). Asked for the list,
    /// a list whose last page failed tries again. A list that no longer
    /// matches `chips` starts again.
    pub(crate) fn wanted(
        &mut self,
        id: Uuid,
        account_user: Uuid,
        chips: &[ReactionChip],
        want: Want<'_>,
    ) -> Option<PageRequest> {
        let signature = signature(chips);
        let list = self
            .lists
            .entry(id)
            .and_modify(|list| {
                if list.signature != signature {
                    *list = ReactorList::new(signature, account_user, chips);
                }
            })
            .or_insert_with(|| ReactorList::new(signature, account_user, chips));
        let needed = match want {
            Want::Hint => !list.started,
            Want::People(filter) => {
                list.failed = false;
                !list.started || list.reactors.wants_more(filter, chips)
            }
        };
        if !needed || list.loading.is_some() || list.failed {
            return None;
        }
        self.tickets += 1;
        list.loading = Some(self.tickets);
        Some(PageRequest {
            id,
            cursor: list.next.clone(),
            ticket: self.tickets,
        })
    }

    /// Page request `ticket` for message `id` answered. Returns whether the
    /// list took it (a list started again since drops it).
    pub(crate) fn arrived(&mut self, id: Uuid, ticket: u64, page: Option<ReactorPage>) -> bool {
        let Some(list) = self
            .lists
            .get_mut(&id)
            .filter(|list| list.loading == Some(ticket))
        else {
            return false;
        };
        list.loading = None;
        match page {
            Some(page) => {
                list.reactors.push(&page);
                list.next = page.next;
                list.started = true;
            }
            None => list.failed = true,
        }
        true
    }

    /// Of `people`, the ones whose profiles have not been asked for yet,
    /// now marked asked.
    pub(crate) fn unasked(&mut self, people: impl IntoIterator<Item = Uuid>) -> Vec<Uuid> {
        people
            .into_iter()
            .filter(|person| self.asked.insert(*person))
            .collect()
    }
}

impl ReactorList {
    fn new(signature: u64, account_user: Uuid, chips: &[ReactionChip]) -> ReactorList {
        ReactorList {
            signature,
            reactors: Reactors::new(account_user, chips),
            next: None,
            loading: None,
            failed: false,
            started: false,
        }
    }
}

/// The chips' identity: their emoji, counts and which are yours.
fn signature(chips: &[ReactionChip]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for chip in chips {
        (&chip.emoji, chip.count, chip.mine).hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use papo_sdk_ui::operations::ReactionEmoji;

    use super::*;

    fn chip(count: u64) -> ReactionChip {
        ReactionChip {
            emoji: ReactionEmoji::Unicode("🧀".into()),
            count,
            mine: false,
            waiting: false,
        }
    }

    fn page(more: bool) -> ReactorPage {
        ReactorPage::for_test(Vec::new(), more)
    }

    #[test]
    fn one_page_at_a_time_until_none_is_needed() {
        let mut lists = ReactorLists::default();
        let id = Uuid::from_u128(1);
        let me = Uuid::from_u128(2);
        let chips = [chip(4)];
        let first = lists
            .wanted(id, me, &chips, Want::People(None))
            .expect("the first page");
        assert!(first.cursor.is_none());
        assert!(
            lists.wanted(id, me, &chips, Want::People(None)).is_none(),
            "one on its way"
        );
        assert!(lists.arrived(id, first.ticket, Some(page(true))));
        assert!(
            lists.wanted(id, me, &chips, Want::People(None)).is_some(),
            "more is needed"
        );
    }

    #[test]
    fn changed_chips_start_the_list_again_and_drop_late_pages() {
        let mut lists = ReactorLists::default();
        let id = Uuid::from_u128(1);
        let me = Uuid::from_u128(2);
        let old = lists
            .wanted(id, me, &[chip(4)], Want::People(None))
            .unwrap();
        let new = lists
            .wanted(id, me, &[chip(5)], Want::People(None))
            .unwrap();
        assert!(!lists.arrived(id, old.ticket, Some(page(false))));
        assert!(lists.arrived(id, new.ticket, Some(page(false))));
        assert!(lists.get(id, &[chip(5)]).is_some());
        assert!(lists.get(id, &[chip(4)]).is_none());
    }

    #[test]
    fn each_person_is_asked_for_once() {
        let mut lists = ReactorLists::default();
        let lia = Uuid::from_u128(7);
        assert_eq!(lists.unasked([lia]), [lia]);
        assert_eq!(lists.unasked([lia]), Vec::<Uuid>::new());
    }

    #[test]
    fn a_hint_reads_the_first_page_only() {
        let mut lists = ReactorLists::default();
        let id = Uuid::from_u128(1);
        let me = Uuid::from_u128(2);
        let chips = [chip(400)];
        let first = lists.wanted(id, me, &chips, Want::Hint).unwrap();
        assert!(lists.arrived(id, first.ticket, Some(page(true))));
        assert!(lists.wanted(id, me, &chips, Want::Hint).is_none());
        assert!(lists.wanted(id, me, &chips, Want::People(None)).is_some());
    }

    #[test]
    fn the_list_tries_a_failed_page_again() {
        let mut lists = ReactorLists::default();
        let id = Uuid::from_u128(1);
        let me = Uuid::from_u128(2);
        let chips = [chip(4)];
        let first = lists.wanted(id, me, &chips, Want::Hint).unwrap();
        assert!(lists.arrived(id, first.ticket, None));
        assert!(
            lists.wanted(id, me, &chips, Want::Hint).is_none(),
            "a hint does not retry"
        );
        assert!(lists.wanted(id, me, &chips, Want::People(None)).is_some());
    }
}
