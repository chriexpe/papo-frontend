//! A reading boundary drawn over the rows (`docs/design.md` §6, Reading
//! controls): "New messages ↓", or a skipped block's "Unread ↓" and
//! "Unread ↑", as an accent line across the timeline ending in a chip at
//! the row's right inset, centred on the gap above a message.
//!
//! Out of flow, so a boundary coming or going moves no row (Android drew
//! its divider as a row first and stuttered). Which boundary goes where is
//! `reading`'s.

use rune::{El, el, row, text};

use super::reading::Boundary;
use crate::design::{
    BOUNDARY_CHIP_PAD_PT, BOUNDARY_GAP_PT, BOUNDARY_LINE_ALPHA, BOUNDARY_LINE_PT, CHIP_RADIUS_PT,
    Palette, TYPE_CAPTION_MEDIUM,
};
use crate::strings;

/// `boundary` over the `gap_pt` above a message row's top, pinned to the
/// row.
pub(super) fn boundary(boundary: Boundary, gap_pt: f32, palette: &Palette) -> El {
    let (shown, spoken) = match boundary {
        Boundary::NewMessages => (strings::new_messages(), strings::new_messages_spoken()),
        Boundary::UnreadBelow => (strings::unread_below(), strings::unread_below_spoken()),
        Boundary::UnreadAbove => (strings::unread_above(), strings::unread_above_spoken()),
    };
    let (pad_y, pad_x) = BOUNDARY_CHIP_PAD_PT;
    let chip = text(shown)
        .font(TYPE_CAPTION_MEDIUM)
        .color(palette.link)
        .fill(palette.accent_soft)
        .radius(CHIP_RADIUS_PT)
        .px(pad_x)
        .py(pad_y)
        .a11y_hidden();
    let line = el()
        .grow(1.0)
        .h(BOUNDARY_LINE_PT)
        .fill(palette.accent.alpha(BOUNDARY_LINE_ALPHA))
        .a11y_hidden();
    row()
        .key("boundary")
        .pin(Some(0.0), Some(0.0), Some(0.0), None)
        .h(gap_pt)
        .center_y()
        .gap(BOUNDARY_GAP_PT)
        // A labelled group: Rune leaves a separator that takes no press
        // out of the tree, so its name would never be heard.
        .label(spoken)
        .child(line)
        .child(chip)
}
