//! Opening a message in the conversation from a header panel (pins,
//! search), the way ⌛ opens its block (`docs/design.md` §10): in place
//! when the shown window holds it, else in a window opened around its
//! exact place, then flashed with `highlight.selected`. A search hit may
//! be in another channel of the same server: the channel shows at the
//! message, never at the reader's own place first.
//!
//! Owns the opening and which one counts (the newest; a panel closing
//! cancels it). Which message a panel marks, and its shrinking, are the
//! panels'.

use std::time::Duration;

use papo_sdk::{HistoryCursor, Uuid};
use rune::Tx;

use super::Shell;
use crate::design::JUMP_HIGHLIGHT_S;
use crate::timeline::anchor::Anchor;
use crate::timeline::rows::message_key;
use crate::timeline::{TimelineKey, watch};

impl Shell {
    /// Shows message `id` of timeline `key`, whose exact place is `cursor`.
    /// From the keyboard (`keyboard`), the message takes the keyboard once
    /// it is in place and the panel gives way meanwhile (`worked_outside`).
    pub(super) fn open_message(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        cursor: Option<HistoryCursor>,
        keyboard: bool,
        tx: &mut Tx,
    ) {
        self.openings += 1;
        let opening = self.openings;
        let shown = self.tabs.shown();
        if shown.is_some_and(|open| open.key == key && open.holds(id)) {
            self.jump_to_original(key, id, tx);
            if keyboard {
                tx.focus(message_key(id), true);
                self.worked_outside();
            }
            return;
        }
        // There is no fetch by ID: the window opens around the exact cursor.
        let Some(cursor) = cursor else {
            return;
        };
        let offset_pt = shown.map_or(0.0, |open| open.tracker.jump_offset_pt());
        if keyboard {
            self.worked_outside();
        }
        let offsets = std::sync::Arc::clone(&self.local_time.offsets);
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(key.account) else {
                return;
            };
            let config = watch::config(offsets);
            match papo_sdk_ui::Timeline::focused(session, key.channel, cursor, config).await {
                Ok(window) => updates.send_tx(move |shell, tx| {
                    shell.message_opened(key, id, opening, window, offset_pt, tx)
                }),
                Err(error) => log::warn!("a message did not open: {error}"),
            }
        });
    }

    /// No opening in flight counts any more: the panel it came from closed.
    pub(super) fn cancel_opening(&mut self) {
        self.openings += 1;
    }

    /// The window opened around message `id` (opening number `opening`):
    /// shown, if that is still the newest opening and its server is still
    /// the one showing. Returns whether the view changed.
    fn message_opened(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        opening: u64,
        window: papo_sdk_ui::Timeline,
        offset_pt: f32,
        tx: &mut Tx,
    ) -> bool {
        if opening != self.openings {
            return false;
        }
        let at = Anchor::Message { id, offset_pt };
        if self.tabs.is_shown(key) {
            if let Some(opening) = self.tabs.adopt(key, window, at) {
                self.start_watch(opening);
            }
        } else {
            // Another channel of the server showing: it shows, at the hit.
            let same_server = self
                .nav
                .selected()
                .and_then(|server| server.account.as_ref())
                .is_some_and(|account| account.id == key.account);
            if !same_server || self.nav.direct_shown() {
                return false;
            }
            self.nav.select_channel(&key.channel.to_string());
            self.sync_timeline_at(Some(tx), Some((window, at)));
            if !self.tabs.is_shown(key) {
                return true;
            }
        }
        if let Some(open) = self.shown_timeline_mut(key) {
            open.flash = Some((id, tx.clock() + JUMP_HIGHLIGHT_S));
        }
        tx.wake_after(Duration::from_secs_f64(JUMP_HIGHLIGHT_S));
        true
    }
}
