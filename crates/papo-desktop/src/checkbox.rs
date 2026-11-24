//! A checkbox (`docs/design.md` §9's link box, §8's Change password): a
//! square, filled in the accent with a check while on, outlined while off,
//! and its label beside it; the whole row toggles it.
//!
//! Owns the checkbox's look; what it holds is its caller's.

use rune::{El, el, icon, row, text};

use crate::design::{
    FOCUS_RING_PT, Look, RADIUS_CONTROL, SPACE_MD, TASK_BOX, TASK_BOX_OUTLINE_PT, TASK_BOX_RADIUS,
    TASK_CHECK, TYPE_SMALL,
};
use crate::shell::Shell;

/// A checkbox keyed `key`, named `label`, on when `checked`; a press runs
/// `toggle`.
pub(crate) fn checkbox(
    key: &'static str,
    label: String,
    checked: bool,
    toggle: fn(&mut Shell),
    look: &Look,
) -> El {
    let square = el()
        .size(look.text_multiple(TASK_BOX), look.text_multiple(TASK_BOX))
        .radius(look.radius(TASK_BOX_RADIUS))
        .center();
    let square = if checked {
        square.fill(look.palette.accent).child(
            icon(&crate::icons::CHECK, look.text_multiple(TASK_CHECK))
                .color(look.palette.accent_on),
        )
    } else {
        square.border(TASK_BOX_OUTLINE_PT, look.palette.line_strong)
    };
    // As wide as its row leaves it, so the label wraps beside what follows
    // instead of running under it.
    row()
        .key(key)
        .w_full()
        .min_w(0.0)
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.palette.surface_hover)
        .role("checkbox")
        .checked(checked)
        .label(label.clone())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(toggle)
        .child(square)
        .child(
            text(label)
                .key(format!("{key}-label"))
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_secondary)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
}
