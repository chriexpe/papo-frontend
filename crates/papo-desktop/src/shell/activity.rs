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
    /// The spot ← and → are on.
    pub(crate) chosen: crate::quick_strip::StripSpot,
}

/// One account's live state, as its watch last posted it.
#[derive(Default)]
pub(crate) struct Activity {
    /// Its people's activity; none until posted.
    pub(crate) people: Option<Arc<AccountActivity>>,
    /// Its outgoing queue: a chip whose change is in it waits.
    pub(crate) outgoing: Option<Arc<OutgoingSnapshot>>,
    /// Its emoji count, most used first: Frequently used and the quick reactions read it.
    pub(crate) usage: Arc<[(String, u64)]>,
    /// Its favorite GIFs, by GIPHY's ID: their stars are filled.
    pub(crate) favorite_gifs: Arc<HashSet<String>>,
    /// Its quick reactions, most used first, from the count and the catalog.
    pub(crate) quick: Arc<[papo_sdk_ui::operations::ReactionEmoji]>,
    /// Its custom emoji catalog; none until posted.
    pub(crate) emojis: Option<Arc<papo_sdk::EmojiCatalog>>,
    /// That catalog as message content resolves against it.
    pub(crate) custom: papo_sdk_ui::message_content::CustomEmojis,
    /// Its connection; none until posted.
    pub(crate) link: Option<crate::diagnostics::Link>,
    /// Refused reactions already being dropped, so each is said once.
    dropping: HashSet<Uuid>,
}

/// How many quick reactions an account keeps: the message menu's strip.
const QUICK_REACTIONS: usize = 8;

impl Activity {
    /// The quick reactions from the emoji count and the catalog (`quick_reactions`: custom
    /// emoji while the catalog has them, then the defaults).
    fn rank_quick(&mut self) {
        self.quick =
            papo_sdk_ui::quick_reactions(&self.usage, self.emojis.as_deref(), QUICK_REACTIONS)
                .into();
    }
}

impl Shell {
    /// Takes a post of `account`'s live watch. Returns whether the view
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
                // Presence and typing show where the account's people do: its
                // shown conversation, its rows in Direct, an open card.
                self.shows_account(account)
                    || (self.nav.direct_shown()
                        && self.direct.iter().any(|row| row.id.account == account))
                    || self
                        .nav
                        .menu
                        .as_ref()
                        .and_then(|menu| menu.card.as_ref())
                        .is_some_and(|card| card.key.account == account)
            }
            Posted::Link(link) => {
                let came_back = matches!(link, crate::diagnostics::Link::Online { .. })
                    && !matches!(
                        self.activity
                            .get(&account)
                            .and_then(|activity| activity.link.as_ref()),
                        Some(crate::diagnostics::Link::Online { .. })
                    );
                self.activity.entry(account).or_default().link = Some(link);
                // Emoji whose fetch failed while offline are asked for again.
                if came_back && self.emoji_images.reconnected(account) {
                    self.want_emoji();
                    return true;
                }
                // Only Diagnostics shows it.
                self.settings
                    .as_ref()
                    .is_some_and(|panel| panel.page() == crate::settings::PageId::Diagnostics)
            }
            Posted::Emojis(catalog) => {
                let activity = self.activity.entry(account).or_default();
                activity.custom = papo_sdk_ui::message_content::CustomEmojis::new(&catalog);
                activity.emojis = Some(catalog);
                activity.rank_quick();
                self.custom_emojis_changed(account);
                self.shows_account(account)
            }
            Posted::Usage(usage) => {
                let activity = self.activity.entry(account).or_default();
                activity.usage = usage.into();
                activity.rank_quick();
                // A custom emoji may have joined the quick reactions.
                self.want_emoji();
                self.shows_account(account)
            }
            Posted::FavoriteGifs(favorites) => {
                self.favorites_posted(account, favorites);
                self.shows_account(account)
            }
            Posted::Outgoing(outgoing) => {
                self.drop_unresolved(account, &outgoing, tx);
                self.unsent_follow(account, &outgoing);
                self.activity.entry(account).or_default().outgoing = Some(outgoing);
                // Unsent audio drawn before its file was staged is read now.
                if self.unsent_audio_waits(account) {
                    self.want_media();
                }
                if self.uploads_ended(tx) {
                    return true;
                }
                // Only the shown conversation's chips wait on a queue.
                self.shows_account(account)
            }
            Posted::Closed => {
                // A conversation of a signed-out account can't stay open.
                self.nav.account_closed(account);
                if self
                    .awaited_dm
                    .as_ref()
                    .is_some_and(|awaited| awaited.account == account)
                {
                    self.awaited_dm = None;
                }
                self.dm_access.retain(|id, _| id.account != account);
                self.emoji_images.forget(account);
                self.sync_timeline(Some(tx));
                self.activity.remove(&account);
                true
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

    /// What the account's reaction emoji are called and draw as.
    pub(crate) fn reaction_emoji(
        &self,
        account: AccountId,
    ) -> crate::reaction_emoji::ReactionEmojiLook {
        crate::reaction_emoji::ReactionEmojiLook {
            account,
            names: self.custom_emojis(account),
            images: self.emoji_images.looks(),
        }
    }

    /// The account's quick reactions, most used first.
    pub(crate) fn quick_reactions(
        &self,
        account: AccountId,
    ) -> Arc<[papo_sdk_ui::operations::ReactionEmoji]> {
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
            emoji: Some(self.reaction_emoji(account)),
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
        if self
            .notice
            .as_ref()
            .is_some_and(|notice| notice.gone(clock_s))
        {
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
