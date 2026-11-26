//! The open picker from the shell's side, whichever tab (`docs/design.md` §16.1): reading it,
//! closing it (the keyboard back to the field, or to the message a reaction was for), closing
//! it when what it stood over goes, and drawing the open tab. Each tab's keys and picks are
//! `emoji_picker`'s and `gif_picker`'s.

use rune::{Cx, El, Tx};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::design::Look;
use crate::emoji_picker::{EmojiPicker, PickerFor};
use crate::gif_picker::GifPicker;
use crate::picker::OpenPicker;

impl Shell {
    /// The Emoji tab, while it is the open one.
    pub(crate) fn emoji_picker(&self) -> Option<&EmojiPicker> {
        match &self.picker {
            Some(OpenPicker::Emoji(picker)) => Some(picker),
            _ => None,
        }
    }

    pub(super) fn emoji_picker_mut(&mut self) -> Option<&mut EmojiPicker> {
        match &mut self.picker {
            Some(OpenPicker::Emoji(picker)) => Some(picker),
            _ => None,
        }
    }

    /// The GIF tab, while it is the open one.
    pub(crate) fn gif_picker(&self) -> Option<&GifPicker> {
        match &self.picker {
            Some(OpenPicker::Gif(picker)) => Some(picker),
            _ => None,
        }
    }

    pub(super) fn gif_picker_mut(&mut self) -> Option<&mut GifPicker> {
        match &mut self.picker {
            Some(OpenPicker::Gif(picker)) => Some(picker),
            _ => None,
        }
    }

    /// The picker closes; the keyboard goes back to the field, or to the message a reaction was
    /// for.
    pub(crate) fn close_emoji_picker(&mut self, tx: &mut Tx) {
        let Some(picker) = self.picker.take() else {
            return;
        };
        self.gif_closed();
        match picker {
            OpenPicker::Emoji(EmojiPicker {
                target: PickerFor::Reaction { message, .. },
                ..
            }) => tx.focus(crate::timeline::rows::message_key(message), true),
            OpenPicker::Emoji(_) | OpenPicker::Gif(_) => tx.focus(COMPOSER_KEY, false),
        }
    }

    /// The picker closes without a word to the keyboard: the conversation it stood over left.
    pub(super) fn drop_emoji_picker(&mut self) {
        if self.picker.take().is_some() {
            self.gif_closed();
        } else {
            // A GIF tab on its way would open over what came since.
            self.gif.cancel_opening();
        }
    }

    /// The picker closed: a GIF tab opening on its way is dropped, and the store forgets its
    /// tiles, so nothing more reaches GIPHY.
    fn gif_closed(&mut self) {
        self.gif.cancel_opening();
        self.gif.tiles.closed();
        self.gif_tiles_changed();
    }

    /// Input that started outside the picker (a press elsewhere, a wheel over the conversation,
    /// typing in the field): it closes, as the pins panel does (§16.1: "a press outside closes
    /// it too"). Returns whether it closed.
    pub(super) fn picker_worked_outside(&mut self) -> bool {
        let open = self.picker.is_some();
        self.drop_emoji_picker();
        open
    }
}

/// The open tab drawn.
pub(super) fn view(shell: &Shell, picker: &OpenPicker, look: &Look, cx: &Cx<Shell>) -> El {
    match picker {
        OpenPicker::Emoji(picker) => super::emoji_picker::view(shell, picker, look, cx),
        OpenPicker::Gif(picker) => super::gif_picker::view(shell, picker, look, cx),
    }
}
