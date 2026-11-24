//! "Quit while uploading?" (`docs/design.md` §14, Open: uploads don't
//! resume): closing the window or quitting while a message is still
//! preparing or uploading asks first, since what is cut off may need to be
//! sent again. Ctrl+Q, Ctrl+W and the window's close request (its close
//! button, the window manager, Alt+F4) all come here.
//!
//! Owns the question and what answering does; whether anything uploads is
//! read from the SDK's outgoing queues.

use papo_sdk::{OperationKind, OperationState, OutgoingSnapshot};
use rune::{El, Tx};

use crate::confirm::{Question, confirm};
use crate::design::Look;
use crate::shell::Shell;
use crate::strings;

/// The element key of the question.
pub(crate) const QUIT_ASK_KEY: &str = "quit-uploading";

/// What was asked for while an upload ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Leaving {
    /// Quit (Ctrl+Q).
    Quit,
    /// Close the window (Ctrl+W, its close button): without a tray this
    /// ends the app too.
    Close,
}

/// Whether `queue` holds a message whose files are still being prepared
/// or uploaded: one that quitting would cut off.
pub(crate) fn uploading(queue: &OutgoingSnapshot) -> bool {
    queue.operations.iter().any(|operation| {
        let has_files = matches!(
            operation.kind.as_ref(),
            OperationKind::Message(message) if !message.attachments.is_empty()
        );
        has_files
            && matches!(
                operation.state,
                OperationState::Preparing | OperationState::Sending
            )
    })
}

/// The question over the dimmed app; Cancel has the keyboard.
pub(crate) fn view(look: &Look) -> El {
    let question = Question {
        key: QUIT_ASK_KEY,
        title: strings::quit_uploading_title(),
        body: Some(strings::quit_uploading_body()),
        shown: None,
        note: None,
        cancel_key: "quit-uploading-cancel",
        cancel: strings::button_cancel(),
        confirm_key: "quit-uploading-confirm",
        confirm: strings::quit_uploading_confirm(),
        danger: true,
    };
    confirm(question, Shell::stay, Shell::leave_anyway, look)
}

impl Shell {
    /// Ctrl+Q or Ctrl+W: leaves at once, or asks first while an upload
    /// runs.
    pub(crate) fn leave_or_ask(&mut self, leaving: Leaving, tx: &mut Tx) {
        if self.uploading_anywhere() {
            self.leaving = Some(leaving);
            return;
        }
        leave(leaving, tx);
    }

    /// The window's close request (`App::on_close_request`): true keeps
    /// the window to ask first.
    pub(crate) fn close_requested(&mut self, _tx: &mut Tx) -> bool {
        if !self.uploading_anywhere() {
            return false;
        }
        self.leaving = Some(Leaving::Close);
        true
    }

    /// Cancel, Escape or Return: the app stays, the uploads go on.
    pub(crate) fn stay(&mut self) {
        self.leaving = None;
    }

    /// A queue changed: if you asked to leave and the uploads have since
    /// ended, Papo leaves as asked. Returns whether it did.
    pub(crate) fn uploads_ended(&mut self, tx: &mut Tx) -> bool {
        if self.leaving.is_none() || self.uploading_anywhere() {
            return false;
        }
        self.leave_anyway(tx);
        true
    }

    /// Quit (or close) after all.
    pub(crate) fn leave_anyway(&mut self, tx: &mut Tx) {
        if let Some(leaving) = self.leaving.take() {
            leave(leaving, tx);
        }
    }
}

/// Does what was asked for. A close that follows the question doesn't ask
/// again (`Tx::close_window` isn't a close request).
fn leave(leaving: Leaving, tx: &mut Tx) {
    match leaving {
        Leaving::Quit => tx.quit(),
        Leaving::Close => tx.close_window(),
    }
}
