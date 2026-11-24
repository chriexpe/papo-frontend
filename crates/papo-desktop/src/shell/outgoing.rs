//! Your unsent messages, from the shell's side (`docs/design.md` §3, Your
//! unsent messages): the menu their ⚠ grows, Retry and Discard through the
//! SDK's outgoing queue, and "Send again?" before retrying one that may
//! have arrived (`RetryDecision::AcceptPossibleDuplicate`).

use papo_sdk::{RetryDecision, Uuid};
use rune::{El, Tx};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::confirm::{Question, confirm};
use crate::design::Look;
use crate::menu::{MenuAction, MenuItem, OpenMenu, Placement};
use crate::strings;
use crate::timeline::TimelineKey;
use crate::timeline::echoes::SendState;
use crate::timeline::send_mark::mark_key;

/// The element key of "Send again?".
pub(crate) const RESEND_KEY: &str = "resend";

/// The unsent message "Send again?" asks about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Resend {
    key: TimelineKey,
    local: Uuid,
}

impl Shell {
    /// The ⚠ of the unsent message `local` was pressed: Retry and Discard
    /// grow from it.
    pub(super) fn open_send_menu(&mut self, key: TimelineKey, local: Uuid, state: SendState) {
        if state == SendState::Pending {
            return;
        }
        let items = vec![
            MenuItem::plain(
                "menu-retry-send",
                strings::outgoing_retry(),
                MenuAction::RetrySend {
                    key,
                    local,
                    unknown: state == SendState::Unknown,
                },
                false,
            ),
            MenuItem::plain(
                "menu-discard-send",
                strings::outgoing_discard(),
                MenuAction::DiscardSend { key, local },
                true,
            ),
        ];
        let mark = mark_key(local);
        self.nav.menu = Some(OpenMenu::new(Placement::Below(mark.clone()), mark, items));
    }

    /// Retry from the menu: at once for a refused message, after "Send
    /// again?" for one that may have arrived.
    pub(super) fn retry_send(&mut self, key: TimelineKey, local: Uuid, unknown: bool) {
        if unknown {
            self.resend = Some(Resend { key, local });
        } else {
            self.retry(key, local, RetryDecision::DefiniteFailure);
        }
    }

    /// Discard from the menu: the message leaves the queue and the
    /// timeline.
    pub(super) fn discard_send(&self, key: TimelineKey, local: Uuid) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        sdk.run(move |client| async move {
            let discarded = match client.session(key.account) {
                Ok(session) => session.outgoing().discard(local).await,
                Err(error) => Err(error.into()),
            };
            if let Err(error) = discarded {
                log::warn!("an unsent message was not discarded: {error}");
            }
        });
    }

    pub(crate) fn cancel_resend(&mut self) {
        self.resend = None;
    }

    /// "Send again" on the confirmation: the message goes again, knowing it
    /// may arrive twice. Its mark becomes the waiting clock, which takes no
    /// keyboard, so the keyboard goes back to writing.
    pub(crate) fn confirm_resend(&mut self, tx: &mut Tx) {
        let Some(Resend { key, local }) = self.resend.take() else {
            return;
        };
        self.retry(key, local, RetryDecision::AcceptPossibleDuplicate);
        tx.focus(COMPOSER_KEY, false);
    }

    fn retry(&self, key: TimelineKey, local: Uuid, decision: RetryDecision) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        sdk.run(move |client| async move {
            let retried = match client.session(key.account) {
                Ok(session) => session.outgoing().retry(local, decision).await,
                Err(error) => Err(error.into()),
            };
            if let Err(error) = retried {
                log::warn!("an unsent message was not retried: {error}");
            }
        });
    }
}

/// "Send again?" over the dimmed app, while it is open.
pub(super) fn resend_confirm(shell: &Shell, look: &Look) -> Option<El> {
    shell.resend?;
    let question = Question {
        key: RESEND_KEY,
        title: strings::resend_title(),
        body: Some(strings::resend_body()),
        shown: None,
        note: None,
        cancel_key: "resend-cancel",
        cancel: strings::resend_cancel(),
        confirm_key: "resend-confirm",
        confirm: strings::resend_confirm(),
        danger: false,
    };
    Some(confirm(
        question,
        Shell::cancel_resend,
        Shell::confirm_resend,
        look,
    ))
}
