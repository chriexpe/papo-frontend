//! The emoji and GIF picker as a whole (`docs/design.md` §16.1): which tab is open, and the
//! frame both draw in. Grown upward from the text pill (or, for a reaction, from its message's
//! row), its right edge on the pill's, glass, at most 440 × 452, never wider than the pill or
//! taller than 60 % of the conversation. Each tab's state and drawing are `emoji_picker`'s and
//! `gif_picker`'s; opening, closing and the keys are the shell's.

use rune::{Align, El, Side, col};

use crate::design::{EMOJI_PICKER_GAP_PT, EMOJI_PICKER_MAX_PT, Look, RADIUS_PILL, Surfaces};
use crate::emoji_picker::EmojiPicker;
use crate::gif_picker::GifPicker;

/// The element key of the open picker, whichever tab.
pub(crate) const PICKER_KEY: &str = "emoji-picker";

/// The open picker: one tab at a time.
pub(crate) enum OpenPicker {
    Emoji(EmojiPicker),
    Gif(GifPicker),
}

/// Where the picker stands and how big it may be.
pub(crate) struct Frame<'a> {
    pub(crate) look: &'a Look,
    /// The element it grows from (the text pill, or a message's row).
    pub(crate) anchor: &'a str,
    /// The text pill's width: the picker is never wider.
    pub(crate) anchor_width_pt: Option<f32>,
    /// The conversation's height: the picker is at most 60 % of it.
    pub(crate) conversation_height_pt: Option<f32>,
}

impl Frame<'_> {
    /// The picker's width and height, in points.
    pub(crate) fn size(&self) -> (f32, f32) {
        let (max_width, max_height) = EMOJI_PICKER_MAX_PT;
        let width = self
            .anchor_width_pt
            .map_or(max_width, |anchor| anchor.min(max_width));
        let height = self
            .conversation_height_pt
            .map_or(max_height, |room| (room * 0.6).min(max_height));
        (width, height)
    }

    /// The panel, named `label`: sized, glass and grown from its anchor, for a tab to fill.
    pub(crate) fn panel(&self, label: &str) -> El {
        let look = self.look;
        let (width, height) = self.size();
        col()
            .key(PICKER_KEY)
            .w(width)
            .h(height)
            .radius(look.radius(RADIUS_PILL))
            .pill_surface(look)
            .clip_rounded(look.radius(RADIUS_PILL))
            .role("dialog")
            .label(label.to_owned())
            .anchored_fit(self.anchor, Side::Top, Align::End, EMOJI_PICKER_GAP_PT)
    }
}
