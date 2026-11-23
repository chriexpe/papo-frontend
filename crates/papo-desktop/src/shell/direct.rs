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

use papo_sdk::operations::social::DirectAccess;
use papo_sdk_ui::{DirectMessageId, DirectMessageItem};
use rune::{El, Tx, row, text};

use super::Shell;
use crate::design::{
    COMPOSER_HEIGHT_PT, FOCUS_RING_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT, RADIUS_PILL_PT,
    SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_BODY_STRONG,
};
use crate::direct_list::DirectRows;
use crate::notice::Refusal;
use crate::strings;
use crate::timeline::TimelineKey;

impl Shell {
    /// Takes the newest list of direct conversations, picking the one a
    /// card's Message waits for once it is there. Returns whether the view
    /// changed.
    pub(crate) fn direct_posted(&mut self, rows: DirectRows, tx: &mut Tx) -> bool {
        self.nav.direct_changed(rows.iter());
        self.direct = rows;
        // Back in the list, a conversation is open again; one the picked
        // conversation left asks the backend why.
        let direct = &self.direct;
        self.dm_access
            .retain(|id, _| !direct.iter().any(|row| &row.id == id));
        self.check_picked_dm();
        if self.pick_awaited_dm(tx) {
            return true;
        }
        if self.nav.direct_shown() {
            self.sync_timeline(Some(tx));
        }
        true
    }

    /// A card's Message opened or reopened conversation `id`: it is picked
    /// as soon as the SDK's list holds it (perhaps already).
    pub(crate) fn await_dm(&mut self, id: DirectMessageId, tx: &mut Tx) {
        self.awaited_dm = Some(id);
        self.pick_awaited_dm(tx);
    }

    /// Picks the awaited conversation when the list holds it. Returns
    /// whether it did.
    fn pick_awaited_dm(&mut self, tx: &mut Tx) -> bool {
        let Some(awaited) = &self.awaited_dm else {
            return false;
        };
        let Some(row) = self.direct.iter().find(|row| &row.id == awaited).cloned() else {
            return false;
        };
        self.awaited_dm = None;
        self.pick_dm(row, tx);
        true
    }

    /// The direct conversations, newest activity first.
    pub(crate) fn direct_rows(&self) -> &DirectRows {
        &self.direct
    }

    /// Shows Direct in the server's place.
    pub(crate) fn select_direct(&mut self, tx: &mut Tx) {
        self.worked_outside();
        self.nav.select_direct();
        self.sync_timeline(Some(tx));
    }

    /// Picks direct conversation `row`.
    pub(crate) fn pick_dm(&mut self, row: Arc<DirectMessageItem>, tx: &mut Tx) {
        self.worked_outside();
        self.nav.pick_dm(row);
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
        self.dm_access
            .get(&picked.id)
            .copied()
            .filter(|access| *access != DirectAccess::Open)
    }

    /// Asks the backend about the picked conversation once it left the
    /// list: a refusal and a hide elsewhere look the same there.
    fn check_picked_dm(&mut self) {
        let Some(picked) = self.nav.picked_dm() else {
            return;
        };
        let listed = self.direct.iter().any(|row| row.id == picked.id);
        if listed || self.dm_access.contains_key(&picked.id) {
            return;
        }
        let (id, user) = (picked.id.clone(), picked.user);
        // Asked once: the answer, or "open" until it comes.
        self.dm_access.insert(id.clone(), DirectAccess::Open);
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(id.account) else {
                return;
            };
            match session.operations().direct_access(id.channel, user).await {
                Ok(access) => updates.send(move |shell| {
                    shell.dm_access.insert(id, access);
                    true
                }),
                Err(error) => log::debug!("a conversation's access was not read: {error}"),
            }
        });
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
    palette: &Palette,
) -> El {
    let words = match access {
        DirectAccess::YouBlocked => strings::direct_you_blocked(name),
        _ => strings::direct_cant_message(name),
    };
    let mut pill = row()
        .key(DM_STATE_KEY)
        .w_full()
        .h(COMPOSER_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role("status")
        .child(
            text(words)
                .font(TYPE_BODY)
                .color(palette.text_secondary)
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
                .px(SPACE_MD_PT)
                .py(SPACE_SM_PT)
                .radius(RADIUS_CONTROL_PT)
                .hover(palette.surface_hover)
                .role("button")
                .label(strings::card_unblock())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
                .on_press(move |shell: &mut Shell| shell.set_blocked(key, user, false))
                .child(
                    text(strings::card_unblock())
                        .font(TYPE_BODY_STRONG)
                        .color(palette.link)
                        .a11y_hidden(),
                ),
        );
    }
    pill
}

/// The element key of a DM's state pill.
pub(crate) const DM_STATE_KEY: &str = "dm-state";
