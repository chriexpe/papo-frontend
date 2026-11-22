//! One channel's edit buffer and where it stands against the SDK's stored
//! draft. `Composer` moves it between the states; nothing here talks to
//! the SDK.

use rune::TextEdit;

/// One channel's buffer.
pub(crate) struct Draft {
    pub(crate) edit: TextEdit,
    pub(super) state: DraftState,
    /// The order of the last save asked for, so a failure is matched to it.
    pub(super) last_save: Option<u64>,
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
        }
    }
}
