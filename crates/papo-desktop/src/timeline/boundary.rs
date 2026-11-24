//! A reading boundary drawn over the rows (`docs/design.md` §6, Reading
//! controls): "New messages ↓", or a skipped block's "Unread ↓" and
//! "Unread ↑" (the arrows drawn as icons), as an accent line across the
//! timeline ending in a chip at the row's right inset, centred on the gap
//! above a message.
//!
//! Out of flow, so a boundary coming or going moves no row (Android drew
//! its divider as a row first and stuttered). Which boundary goes where is
//! `reading`'s.

use rune::{El, el, icon, row, text};

use super::reading::Boundary;
use crate::design::{
    BOUNDARY_CHIP_PAD_PT, BOUNDARY_GAP, BOUNDARY_LINE_ALPHA, BOUNDARY_LINE_PT, CHIP_ICON_PT,
    CHIP_RADIUS, Look, SPACE_XS, TYPE_CAPTION_MEDIUM,
};
use crate::icons::{ARROW_DOWN, ARROW_UP};
use crate::strings;

/// `boundary` over the `gap_pt` above a message row's top, pinned to the
/// row.
pub(super) fn boundary(boundary: Boundary, gap_pt: f32, look: &Look) -> El {
    let (shown, arrow, spoken) = match boundary {
        Boundary::NewMessages => (
            strings::new_messages(),
            &ARROW_DOWN,
            strings::new_messages_spoken(),
        ),
        Boundary::UnreadBelow => (
            strings::unread(),
            &ARROW_DOWN,
            strings::unread_below_spoken(),
        ),
        Boundary::UnreadAbove => (strings::unread(), &ARROW_UP, strings::unread_above_spoken()),
    };
    let (pad_y, pad_x) = BOUNDARY_CHIP_PAD_PT;
    let chip = row()
        .center_y()
        .gap(look.space(SPACE_XS))
        .child(
            text(shown)
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.link),
        )
        .child(icon(arrow, CHIP_ICON_PT).color(look.palette.link))
        .fill(look.palette.accent_soft)
        .radius(look.radius(CHIP_RADIUS))
        .px(pad_x)
        .py(pad_y)
        .a11y_hidden();
    let line = el()
        .grow(1.0)
        .h(BOUNDARY_LINE_PT)
        .fill(look.palette.accent.alpha(BOUNDARY_LINE_ALPHA))
        .a11y_hidden();
    row()
        .key("boundary")
        .pin(Some(0.0), Some(0.0), Some(0.0), None)
        .h(gap_pt)
        .center_y()
        .gap(look.space(BOUNDARY_GAP))
        // A labelled group: Rune leaves a separator that takes no press
        // out of the tree, so its name would never be heard.
        .label(spoken)
        .child(line)
        .child(chip)
}
