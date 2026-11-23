//! Direct messages from the shell's side (`docs/design.md` §10): taking the
//! SDK's list as `direct_list` posts it, showing Direct in the server's
//! place, picking a conversation, and the timeline a picked one shows.
//!
//! Owns these transitions; the list and its order are the SDK's, where the
//! person is lives in `navigation`, and the drawing in `direct_pane`.

use std::sync::Arc;

use papo_sdk_ui::{DirectMessageId, DirectMessageItem};
use rune::Tx;

use super::Shell;
use crate::direct_list::DirectRows;
use crate::notice::Refusal;
use crate::timeline::TimelineKey;

impl Shell {
    /// Takes the newest list of direct conversations, picking the one a
    /// card's Message waits for once it is there. Returns whether the view
    /// changed.
    pub(crate) fn direct_posted(&mut self, rows: DirectRows, tx: &mut Tx) -> bool {
        self.nav.direct_changed(rows.iter());
        self.direct = rows;
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

    /// The timeline Direct shows: the picked conversation's.
    pub(super) fn wanted_direct_timeline(&self) -> Option<TimelineKey> {
        let picked = self.nav.picked_dm()?;
        Some(TimelineKey {
            account: picked.id.account,
            channel: picked.id.channel,
        })
    }
}
