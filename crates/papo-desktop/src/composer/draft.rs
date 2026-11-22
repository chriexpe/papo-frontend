//! One channel's edit buffer and where it stands against the SDK's stored
//! draft, with what follows from its text: the pairs open in it, its
//! Markdown spans and its length as sent. `Composer` moves it between the
//! states; nothing here talks to the SDK.

use papo_sdk_ui::message_content::{PreviewSpan, preview_spans_with_bindings};
use rune::TextEdit;

use super::mentions;
use super::pairs::Pairs;

/// One channel's buffer.
pub(crate) struct Draft {
    pub(crate) edit: TextEdit,
    pub(super) state: DraftState,
    /// The order of the last save asked for, so a failure is matched to it.
    pub(super) last_save: Option<u64>,
    pub(super) pairs: Pairs,
    /// The SDK's Markdown spans of the text, as of its last change.
    pub(super) preview: Vec<PreviewSpan>,
    /// The text's length as sent, in characters: a mention counts as its
    /// token, not its name.
    pub(super) wire_chars: usize,
}

/// Where a buffer stands against the SDK's stored draft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DraftState {
    /// The stored draft is being read; the field takes no typing until it
    /// arrives, so typing never races the restore.
    Loading,
    /// The stored draft holds what the field shows, or a save of it is on
    /// its way.
    Saved,
    /// The field changed; it is saved at `due_s` on the window's clock.
    Unsaved { due_s: f64 },
    /// The last save failed (the text is over the limit, or storage
    /// refused it): it is saved again on the next edit, switch or flush,
    /// never on a timer of its own, so a failure cannot loop.
    SaveFailed,
}

impl DraftState {
    /// Whether the field holds text the stored draft does not.
    pub(super) fn unsaved(self) -> bool {
        matches!(self, DraftState::Unsaved { .. } | DraftState::SaveFailed)
    }
}

impl Draft {
    /// A buffer whose stored draft is being read.
    pub(super) fn loading() -> Draft {
        Draft {
            edit: TextEdit::new(""),
            state: DraftState::Loading,
            last_save: None,
            pairs: Pairs::default(),
            preview: Vec::new(),
            wire_chars: 0,
        }
    }

    /// Recomputes what follows from the text; after every change to it.
    pub(super) fn refresh(&mut self) {
        let bindings = mentions::bindings(&self.edit);
        self.preview = preview_spans_with_bindings(&self.edit.text, &bindings);
        self.wire_chars = mentions::wire(&self.edit).chars().count();
    }

    /// The text was replaced, not edited: no pair is open in it.
    pub(super) fn replaced(&mut self) {
        self.pairs.reset(&self.edit.text);
        self.refresh();
    }
}
