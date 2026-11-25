//! Deleting a message from the shell's side (`docs/design.md` §10 and
//! §11): Delete asks first, showing the message on its own, as "Delete
//! message?" or, for a moderator, "Delete {name}'s message?" with the audit
//! note; then the deletion is queued. A failed deletion's line offers Try
//! again and Dismiss (Christian, 2026-11-22: a refused deletion holds its
//! channel's queue until one of them), and a failed edit's ⚠ grows Retry
//! and Discard edit.

use papo_sdk::{OperationKind, RetryDecision, Uuid};
use papo_sdk_ui::DeleteAs;
use rune::{El, ScrollSpec, Tx, col};

use super::Shell;
use super::replies::item_actions;
use crate::composer::COMPOSER_KEY;
use crate::confirm::{self, Question, confirm};
use crate::design::CONFIRM_MESSAGE_MAX_PT;
use crate::menu::{MenuAction, MenuItem, OpenMenu, Placement};
use crate::strings;
use crate::timeline::change_marks::change_mark_key;
use crate::timeline::standalone::standalone;
use crate::timeline::{Style, TimelineKey};

/// The element key of the delete confirmation.
pub(crate) const DELETE_KEY: &str = "delete";

/// A deletion waiting for its confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeleteAsk {
    pub(super) key: TimelineKey,
    id: Uuid,
    as_who: DeleteAs,
}

impl Shell {
    /// The ⚠ of your failed edit `local` was pressed: Retry and Discard
    /// edit grow from it.
    pub(super) fn open_change_menu(&mut self, key: TimelineKey, local: Uuid) {
        let items = vec![
            MenuItem::plain(
                "menu-retry-edit",
                strings::edit_retry(),
                MenuAction::RetryChange { key, local },
                false,
            ),
            MenuItem::plain(
                "menu-discard-edit",
                strings::edit_discard(),
                MenuAction::DiscardChange { key, local },
                true,
            ),
        ];
        let mark = change_mark_key(local);
        self.nav.menu = Some(OpenMenu::new(Placement::Below(mark.clone()), mark, items));
    }

    /// Delete on message `id` of timeline `key` (or an edit emptied of
    /// text): the confirmation opens, if the account may delete it.
    pub(crate) fn ask_delete(&mut self, key: TimelineKey, id: Uuid) {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return;
        };
        let Some(as_who) = item_actions(open, id).and_then(|(actions, _)| actions.delete) else {
            return;
        };
        self.deleting = Some(DeleteAsk { key, id, as_who });
    }

    /// Cancel, Escape or Return on the confirmation: nothing is deleted.
    pub(crate) fn cancel_delete(&mut self) {
        self.deleting = None;
    }

    /// Delete on the confirmation: the deletion is queued, an edit of the
    /// message ends, and the keyboard goes back to writing.
    pub(crate) fn confirm_delete(&mut self, tx: &mut Tx) {
        let Some(DeleteAsk { key, id, .. }) = self.deleting.take() else {
            return;
        };
        if self
            .composer
            .shown()
            .is_some_and(|(shown, draft)| shown == key && draft.mode.editing() == Some(id))
        {
            let effects = self.composer.cancel_edit().unwrap_or_default();
            self.run_composer(effects, Some(tx));
        }
        tx.focus(COMPOSER_KEY, false);
        let Some(sdk) = &self.sdk else {
            return;
        };
        let local = (self.local_ids)();
        sdk.run(move |client| async move {
            let queued = match client.session(key.account) {
                Ok(session) => {
                    let kind = OperationKind::Delete {
                        target: papo_sdk::MessageTarget::Remote(id),
                    };
                    session
                        .outgoing()
                        .enqueue(key.channel, local, kind, super::draft_writes::now())
                        .await
                }
                Err(error) => Err(error.into()),
            };
            if let Err(error) = queued {
                log::warn!("a deletion was not queued: {error}");
            }
        });
    }

    /// Try again on a failed change (a deletion's line, a failed edit's
    /// menu): the operation `local` goes again. One that may have gone
    /// through is safe to repeat: an edit or a deletion lands the same.
    pub(crate) fn retry_change(&self, key: TimelineKey, local: Uuid) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        sdk.run(move |client| async move {
            let retried = match client.session(key.account) {
                Ok(session) => {
                    session
                        .outgoing()
                        .retry(local, RetryDecision::AcceptPossibleDuplicate)
                        .await
                }
                Err(error) => Err(error.into()),
            };
            if let Err(error) = retried {
                log::warn!("a change was not retried: {error}");
            }
        });
    }

    /// Dismiss on a failed deletion, Discard edit on a failed edit: the
    /// operation `local` leaves the queue and the message stays as the
    /// server has it.
    pub(crate) fn discard_change(&self, key: TimelineKey, local: Uuid) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        sdk.run(move |client| async move {
            let discarded = match client.session(key.account) {
                Ok(session) => session.outgoing().discard(local).await,
                Err(error) => Err(error.into()),
            };
            if let Err(error) = discarded {
                log::warn!("a change was not discarded: {error}");
            }
        });
    }
}

/// The delete confirmation over the dimmed app, while one is open.
pub(super) fn delete_confirm(shell: &Shell, style: &Style) -> Option<El> {
    let DeleteAsk { key, id, as_who } = shell.deleting?;
    let open = shell.tabs.shown().filter(|open| open.key == key)?;
    // A long message scrolls inside its box, so the buttons stay in view.
    let shown = standalone::<Shell>(open, id, style).map(|message| {
        col()
            .w_full()
            .max_h(CONFIRM_MESSAGE_MAX_PT)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(message)
    });
    let title = match as_who {
        DeleteAs::Moderator => strings::delete_title_other(&open.author_of(id)),
        _ => strings::delete_title_own().to_owned(),
    };
    let note = (as_who == DeleteAs::Moderator)
        .then(|| confirm::note(strings::delete_audit(), &style.look));
    let question = Question {
        key: DELETE_KEY,
        title: &title,
        body: None,
        shown,
        note,
        cancel_key: "delete-cancel",
        cancel: strings::button_cancel(),
        confirm_key: "delete-confirm",
        confirm: strings::button_delete(),
        danger: true,
    };
    Some(confirm(
        question,
        Shell::cancel_delete,
        Shell::confirm_delete,
        &style.look,
    ))
}
