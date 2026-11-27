//! Direct messages from the shell's side (`docs/design.md` §10): taking the
//! SDK's list as `direct_list` posts it, showing Direct in the server's
//! place, picking a conversation, and the timeline a picked one shows.
//!
//! A picked conversation that is not open says why in its text pill's
//! place: this account blocks them, or the backend refuses it.
//!
//! Owns these transitions and the state pill; the list and its order are
//! the SDK's, where the person is lives in `navigation`, and the list's
//! drawing in `direct_pane`.

use std::sync::Arc;

use papo_sdk::Uuid;
use papo_sdk::operations::social::DirectAccess;
use papo_sdk_ui::{DirectMessageId, DirectMessageItem};
use rune::{El, Tx, row, text};

use super::Shell;
use crate::design::{
    COMPOSER_HEIGHT, FOCUS_RING_PT, Look, RADIUS_CONTROL, RADIUS_PILL, SPACE_LG, SPACE_MD,
    SPACE_SM, Surfaces, TYPE_BODY, TYPE_BODY_STRONG,
};
use crate::direct_list::DirectRows;
use crate::notice::Refusal;
use crate::strings;
use crate::timeline::TimelineKey;

/// What the shell knows from the backend about a conversation that left
/// the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Asked {
    /// Asked; no answer yet.
    Pending,
    /// The backend's answer.
    Answered(DirectAccess),
    /// The read failed: asked again at the next pick.
    Failed,
}

impl Shell {
    /// Takes the newest list of direct conversations, picking the one a
    /// card's Message waits for once it is there. Returns whether the view
    /// changed.
    pub(crate) fn direct_posted(&mut self, rows: DirectRows, tx: &mut Tx) -> bool {
        self.nav.direct_changed(rows.iter());
        // Remembered for an unblock, which reopens what a block hid.
        self.dm_peers
            .extend(rows.iter().map(|row| (row.id.account, row.user)));
        self.direct = rows;
        self.drop_unshown_dms();
        self.check_picked_dm();
        if self.pick_awaited_dm(tx) {
            return true;
        }
        if self.nav.direct_shown() {
            self.sync_timeline(Some(tx));
        }
        self.try_open_pending_notification(tx);
        true
    }

    /// A card's Message opened or reopened conversation `id`: it is picked
    /// as soon as the SDK's list holds it (perhaps already).
    pub(crate) fn await_dm(&mut self, id: DirectMessageId, tx: &mut Tx) {
        self.awaited_dm = Some(id);
        self.pick_awaited_dm(tx);
    }

    /// Picks the awaited conversation when the list holds it, the
    /// keyboard in its text field: it was opened to write in (a card's
    /// Message, a New message pick). Returns whether it did.
    fn pick_awaited_dm(&mut self, tx: &mut Tx) -> bool {
        let Some(awaited) = &self.awaited_dm else {
            return false;
        };
        let Some(row) = self.direct.iter().find(|row| &row.id == awaited).cloned() else {
            return false;
        };
        self.awaited_dm = None;
        self.pick_dm(row, tx);
        tx.focus(crate::composer::COMPOSER_KEY, true);
        true
    }

    /// Forgets the picked and the awaited conversation of an account the
    /// rail no longer shows (an account switch, a sign-out): its title,
    /// typing and composer would be that account's.
    pub(super) fn drop_unshown_dms(&mut self) {
        let shown: Vec<_> = self
            .nav
            .servers()
            .servers
            .iter()
            .filter_map(|server| server.account.as_ref().map(|account| account.id))
            .collect();
        if self
            .awaited_dm
            .as_ref()
            .is_some_and(|awaited| !shown.contains(&awaited.account))
        {
            self.awaited_dm = None;
        }
        self.dm_access.retain(|id, _| shown.contains(&id.account));
        let unshown = self
            .nav
            .picked_dm_row()
            .is_some_and(|picked| !shown.contains(&picked.id.account));
        if unshown {
            self.nav.unpick_dm();
        }
    }

    /// The direct conversations, newest activity first.
    pub(crate) fn direct_rows(&self) -> &DirectRows {
        &self.direct
    }

    /// Switches Direct between the conversations and the people the
    /// accounts block.
    pub(crate) fn toggle_direct_blocked(&mut self) {
        self.worked_outside();
        self.nav.toggle_direct_blocked();
    }

    /// Shows Direct in the server's place.
    pub(crate) fn select_direct(&mut self, tx: &mut Tx) {
        self.worked_outside();
        self.awaited_dm = None;
        self.nav.select_direct();
        // The list builds only the rows on screen: the picked one, the
        // list's Tab stop, is brought there.
        if let Some(picked) = self.nav.picked_dm() {
            tx.scroll_into_view(crate::direct_pane::dm_row_key(picked));
        }
        self.sync_timeline(Some(tx));
    }

    /// Picks direct conversation `row`.
    pub(crate) fn pick_dm(&mut self, row: Arc<DirectMessageItem>, tx: &mut Tx) {
        self.worked_outside();
        self.awaited_dm = None;
        // Each pick asks the backend afresh (`check_picked_dm`).
        self.dm_access.remove(&row.id);
        let (key, user) = (
            TimelineKey {
                account: row.id.account,
                channel: row.id.channel,
            },
            row.user,
        );
        self.nav.pick_dm(row);
        self.check_picked_dm();
        // An open profile pane shows the newly picked person.
        if self.layout.dm_profile() {
            self.load_profile(key, user);
        }
        self.sync_timeline(Some(tx));
    }

    /// Closes direct conversation `row` at once, no question: the SDK's
    /// list drops it once the backend has; messaging them again reopens it.
    /// A picked one is no longer picked.
    pub(crate) fn close_dm(&mut self, row: &DirectMessageItem, tx: &mut Tx) {
        self.worked_outside();
        if self
            .nav
            .picked_dm()
            .is_some_and(|picked| picked.id == row.id)
        {
            self.nav.unpick_dm();
            self.sync_timeline(Some(tx));
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        let (account, channel) = (row.id.account, row.id.channel);
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let closed = match client.session(account) {
                Ok(session) => session
                    .operations()
                    .close_direct_conversation(channel)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = closed {
                log::warn!("a direct conversation was not closed: {error}");
                updates.send_tx(|shell, tx| {
                    shell.refuse(Refusal::CloseDm, tx);
                    true
                });
            }
        });
    }

    /// What the picked conversation allows, when it is not open: this
    /// account blocks them (its own block list, at once), or the backend
    /// refuses it (asked once it left the list). None while open or not
    /// yet known.
    /// Whether a direct conversation's state pill stands where the field
    /// would: nothing can be written or attached there.
    pub(super) fn composer_hidden(&self) -> bool {
        self.picked_dm_access().is_some()
    }

    pub(super) fn picked_dm_access(&self) -> Option<DirectAccess> {
        let picked = self.nav.picked_dm()?;
        let key = self.wanted_direct_timeline()?;
        let blocked = self
            .shown_timeline(key)
            .and_then(|open| open.resources.as_deref())
            .is_some_and(|resources| resources.blocks.contains(picked.user));
        if blocked {
            return Some(DirectAccess::YouBlocked);
        }
        // Only a refusal says so: "you blocked" is read live above, and
        // an open or unknown conversation needs no pill.
        match self.dm_access.get(&picked.id) {
            Some(Asked::Answered(DirectAccess::Refused)) => Some(DirectAccess::Refused),
            _ => None,
        }
    }

    /// Asks the backend about the picked conversation, once per pick: the
    /// list does not hear of a block the other person made, nor tells a
    /// refusal from a hide elsewhere, so it can't say whether a
    /// conversation is open.
    fn check_picked_dm(&mut self) {
        let Some(picked) = self.nav.picked_dm() else {
            return;
        };
        if self.dm_access.contains_key(&picked.id) {
            return;
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        let id = picked.id.clone();
        self.dm_access.insert(id.clone(), Asked::Pending);
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let answer = match client.session(id.account) {
                Ok(session) => match session.operations().direct_conversation(id.channel).await {
                    Ok(_) => Ok(DirectAccess::Open),
                    Err(error) => DirectAccess::proven_by(&error).ok_or_else(|| error.to_string()),
                },
                Err(error) => Err(error.to_string()),
            };
            updates.send_tx(move |shell, tx| shell.dm_access_answered(id, answer, tx));
        });
    }

    /// The backend's answer about conversation `id`, kept until it is
    /// picked again, unblocked or its account goes; an answer no longer
    /// asked for is dropped. A refusal shows the state pill, even while a
    /// stale list still holds the conversation; a conversation the account
    /// is no longer in closes; open (a hide elsewhere reads as open) needs
    /// no pill, and "you blocked" is read from the live block list. A
    /// failed read waits for the next pick. Returns whether the view
    /// changed.
    pub(super) fn dm_access_answered(
        &mut self,
        id: DirectMessageId,
        answer: Result<DirectAccess, String>,
        tx: &mut Tx,
    ) -> bool {
        if !matches!(self.dm_access.get(&id), Some(Asked::Pending)) {
            return false;
        }
        match answer {
            Ok(DirectAccess::Gone) => {
                self.dm_access.remove(&id);
                if self.nav.picked_dm().is_some_and(|picked| picked.id == id) {
                    self.nav.unpick_dm();
                    self.sync_timeline(Some(tx));
                }
                true
            }
            Ok(access) => {
                self.dm_access.insert(id, Asked::Answered(access));
                access == DirectAccess::Refused
            }
            Err(error) => {
                log::debug!("a conversation's access was not read: {error}");
                self.dm_access.insert(id, Asked::Failed);
                false
            }
        }
    }

    /// This account blocked or unblocked `user`: their conversations are
    /// asked about again, since the backend refuses a conversation blocked
    /// in either direction.
    pub(super) fn blocks_changed(&mut self, user: Uuid) {
        let picked = self
            .nav
            .picked_dm()
            .filter(|picked| picked.user == user)
            .map(|picked| picked.id.clone());
        if let Some(id) = picked {
            self.dm_access.remove(&id);
            self.check_picked_dm();
        }
    }

    /// The timeline Direct shows: the picked conversation's.
    pub(super) fn wanted_direct_timeline(&self) -> Option<TimelineKey> {
        let picked = self.nav.picked_dm()?;
        Some(TimelineKey {
            account: picked.id.account,
            channel: picked.id.channel,
        })
    }
}

/// A DM's state pill, in the text pill's place and at its height: "You
/// blocked Lia." with Unblock, or "You can't message Lia." without saying
/// who blocked whom.
pub(super) fn state_pill(
    access: DirectAccess,
    name: &str,
    picked: &DirectMessageItem,
    look: &Look,
) -> El {
    let words = match access {
        DirectAccess::YouBlocked => strings::direct_you_blocked(name),
        _ => strings::direct_cant_message(name),
    };
    let mut pill = row()
        // In the text pill's place, under its key: the notice pill hangs
        // above whichever shows.
        .key(crate::composer::COMPOSER_PILL_KEY)
        .w_full()
        .h(look.space(COMPOSER_HEIGHT))
        .px(look.space(SPACE_LG))
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("status")
        .child(
            text(words)
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_secondary)
                .grow(1.0)
                .min_w(0.0)
                .max_lines(1),
        );
    if access == DirectAccess::YouBlocked {
        let key = TimelineKey {
            account: picked.id.account,
            channel: picked.id.channel,
        };
        let user = picked.user;
        pill = pill.child(
            row()
                .key("dm-unblock")
                .px(look.space(SPACE_MD))
                .py(look.space(SPACE_SM))
                .radius(look.radius(RADIUS_CONTROL))
                .hover(look.on_pill(look.palette.surface_hover))
                .role("button")
                .label(strings::card_unblock())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press(move |shell: &mut Shell| shell.set_blocked(key.account, user, false))
                .child(
                    text(strings::card_unblock())
                        .font(look.font(TYPE_BODY_STRONG))
                        .color(look.palette.link)
                        .a11y_hidden(),
                ),
        );
    }
    pill
}
