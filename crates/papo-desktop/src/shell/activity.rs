//! What each account shows live beyond a timeline, from the shell's side
//! (`live_watch` posts it): its people's activity (presence marks, who
//! types), the outgoing queue a chip waits on, the account's quick
//! reactions, and its refused reactions,
//! which the notice says where they were made and the queue drops (nothing
//! else removes them: `unresolved_reactions`). The notice pill is shown
//! and timed here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use papo_sdk::{AccountId, OutgoingSnapshot, Uuid};
use papo_sdk_ui::{AccountActivity, unresolved_reactions};
use rune::Tx;

use super::Shell;
use crate::live_watch::Posted;
use crate::notice::{Notice, Refusal};
use crate::timeline::{Reacting, TimelineKey};

/// A hover bar Add reaction stretched into the quick-reaction strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stretched {
    pub(crate) key: TimelineKey,
    pub(crate) id: Uuid,
    /// The emoji ← and → are on.
    pub(crate) chosen: usize,
}

/// One account's live state, as its watch last posted it.
#[derive(Default)]
pub(crate) struct Activity {
    /// Its people's activity; none until posted.
    pub(crate) people: Option<Arc<AccountActivity>>,
    /// Its outgoing queue: a chip whose change is in it waits.
    pub(crate) outgoing: Option<Arc<OutgoingSnapshot>>,
    /// Its quick reactions, most used first; empty until posted.
    pub(crate) quick: Arc<[String]>,
    /// Refused reactions already being dropped, so each is said once.
    dropping: HashSet<Uuid>,
}

impl Shell {
    /// Takes a post of `account`'s reaction watch. Returns whether the view
    /// changed.
    pub(crate) fn activity_posted(
        &mut self,
        account: AccountId,
        posted: Posted,
        tx: &mut Tx,
    ) -> bool {
        match posted {
            Posted::People(people) => {
                self.activity.entry(account).or_default().people = Some(people);
                // Presence and typing show wherever the account's people do:
                // its conversations, DM rows, an open card.
                true
            }
            Posted::Quick(quick) => {
                self.activity.entry(account).or_default().quick = quick.into();
                self.shows_account(account)
            }
            Posted::Outgoing(outgoing) => {
                self.drop_unresolved(account, &outgoing, tx);
                self.activity.entry(account).or_default().outgoing = Some(outgoing);
                // Only the shown conversation's chips wait on a queue.
                self.shows_account(account)
            }
            Posted::Closed => {
                // A conversation of a signed-out account can't stay open.
                self.nav.account_closed(account);
                self.activity.remove(&account).is_some()
            }
        }
    }

    /// Whether the shown conversation is `account`'s.
    fn shows_account(&self, account: AccountId) -> bool {
        self.tabs
            .shown()
            .is_some_and(|open| open.key.account == account)
    }

    /// The account's people's activity; none until posted.
    pub(crate) fn people(&self, account: AccountId) -> Option<&AccountActivity> {
        self.activity.get(&account)?.people.as_deref()
    }

    /// The account's quick reactions, most used first.
    pub(crate) fn quick_reactions(&self, account: AccountId) -> Arc<[String]> {
        self.activity
            .get(&account)
            .map_or_else(|| Arc::from([]), |activity| Arc::clone(&activity.quick))
    }

    /// The account's outgoing queue; empty until posted.
    pub(crate) fn outgoing(&self, account: AccountId) -> Arc<OutgoingSnapshot> {
        self.activity
            .get(&account)
            .and_then(|activity| activity.outgoing.clone())
            .unwrap_or_else(|| Reacting::unknown().outgoing)
    }

    /// What the shown timeline's rows read to draw their reactions and hover
    /// bars.
    pub(crate) fn reacting(&self) -> Reacting {
        let Some(open) = self.tabs.shown() else {
            return Reacting::unknown();
        };
        let account = open.key.account;
        Reacting {
            outgoing: self.outgoing(account),
            quick: self.quick_reactions(account),
            capabilities: super::replies::channel_capabilities(open),
            stretched: self
                .stretched
                .filter(|stretched| stretched.key == open.key)
                .map(|stretched| (stretched.id, stretched.chosen)),
        }
    }

    /// Says `refusal` in the notice pill.
    pub(crate) fn refuse(&mut self, refusal: Refusal, tx: &mut Tx) {
        self.notice = Some(Notice::show(refusal, tx));
    }

    /// ✕ on the notice.
    pub(crate) fn close_notice(&mut self) {
        self.notice = None;
    }

    /// The pointer came onto the notice or left it.
    pub(crate) fn notice_hovered(&mut self, hovered: bool, tx: &mut Tx) {
        if let Some(notice) = &mut self.notice {
            notice.hovered(hovered, tx);
        }
    }

    /// Ends the notice once its time is over. Returns whether the view
    /// changed.
    pub(super) fn poll_notice(&mut self, clock_s: f64) -> bool {
        if self.notice.is_some_and(|notice| notice.gone(clock_s)) {
            self.notice = None;
            return true;
        }
        false
    }

    /// Drops `account`'s refused reactions from its queue, once each, and
    /// says so when one was made in the shown conversation.
    fn drop_unresolved(&mut self, account: AccountId, outgoing: &OutgoingSnapshot, tx: &mut Tx) {
        let shown = self.tabs.shown().map(|open| open.key);
        let activity = self.activity.entry(account).or_default();
        let unresolved: HashMap<Uuid, Uuid> = unresolved_reactions(outgoing)
            .map(|operation| (operation.id, operation.channel_id))
            .collect();
        activity
            .dropping
            .retain(|local| unresolved.contains_key(local));
        let fresh: Vec<(Uuid, Uuid)> = unresolved
            .into_iter()
            .filter(|(local, _)| activity.dropping.insert(*local))
            .collect();
        if fresh.is_empty() {
            return;
        }
        let here = fresh.iter().any(|(_, channel)| {
            shown.is_some_and(|key| key.account == account && key.channel == *channel)
        });
        if here {
            self.refuse(Refusal::Reaction, tx);
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        let locals: Vec<Uuid> = fresh.into_iter().map(|(local, _)| local).collect();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(account) else {
                return;
            };
            for local in locals {
                if let Err(error) = session.outgoing().discard(local).await {
                    log::warn!("a refused reaction was not dropped: {error}");
                }
            }
        });
    }
}
