//! What a channel's composer is for (`docs/design.md` §11): a new message,
//! perhaps a reply to one, or an edit of one of yours. One mode per
//! channel; the bars over the pill and Save in Send's place follow it.
//!
//! A reply is part of the draft: the SDK stores its original's ID with the
//! text (`DraftBody`'s `reply_to`), so it survives a switch and a relaunch.
//! An edit is not: while it lasts, the field holds the message's text and
//! nothing is saved, and the field as it was comes back when the edit ends,
//! so the stored draft never holds an edit.

use papo_sdk::Uuid;
use papo_sdk_ui::ReplyOriginal;
use rune::TextEdit;

/// A channel composer's mode.
pub(crate) enum Mode {
    /// A new message, replying to `reply` when there is one.
    Writing { reply: Option<Reply> },
    /// An edit of one of your messages; boxed, as it holds a whole field.
    Editing(Box<Editing>),
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Writing { reply: None }
    }
}

/// A reply being written.
pub(crate) struct Reply {
    /// The original's backend ID.
    pub(crate) id: Uuid,
    /// The original as the SDK last told it, for the bar while the
    /// conversation does not show it; none until a restored reply is
    /// looked up.
    pub(crate) original: Option<ReplyOriginal>,
    /// The @ switch, "Mention when replying". A mock until the backend
    /// stores `notify_reply` (Christian, 2026-11-22): it shows, toggles and
    /// is spoken, and nothing is sent for it.
    pub(crate) mention: bool,
}

/// An edit being written.
pub(crate) struct Editing {
    /// The message's backend ID.
    pub(crate) id: Uuid,
    /// The field as it was before the edit, back when it ends.
    pub(super) before: Before,
}

/// The field and reply an edit set aside.
pub(super) struct Before {
    pub(super) edit: TextEdit,
    pub(super) reply: Option<Reply>,
    /// A send the SDK refused came back into it during the edit: it is
    /// saved, and the note shows, once the edit ends.
    pub(super) send_failed: bool,
}

impl Mode {
    /// The reply being written, if any.
    pub(crate) fn reply(&self) -> Option<&Reply> {
        match self {
            Mode::Writing { reply } => reply.as_ref(),
            Mode::Editing(_) => None,
        }
    }

    /// The message an edit changes, if one is being written.
    pub(crate) fn editing(&self) -> Option<Uuid> {
        match self {
            Mode::Writing { .. } => None,
            Mode::Editing(editing) => Some(editing.id),
        }
    }

    /// The message the composer points at: the original of a reply, or the
    /// message being edited. It wears `highlight.selected`.
    pub(crate) fn target(&self) -> Option<Uuid> {
        match self {
            Mode::Writing { reply } => reply.as_ref().map(|reply| reply.id),
            Mode::Editing(editing) => Some(editing.id),
        }
    }
}
