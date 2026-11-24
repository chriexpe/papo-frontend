//! The panel's small buttons inside a row (`docs/design.md` §8): Cancel,
//! Save, Add, Remove, Unblock, Show, and a destructive row's red
//! confirmation. One shape for all, concentric with the row's tile like
//! Shape and type's chips.
//!
//! Owns that button's look; what a press does is its caller's.

use rune::{El, Tx, el, text};

use crate::design::settings::{CHIP_HEIGHT, TILE_OUTER_RADIUS};
use crate::design::{FOCUS_RING_PT, Look, SPACE_MD, TYPE_SMALL_MEDIUM};
use crate::shell::Shell;

/// A chip's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    /// Cancel, Remove, Show: the hover surface.
    Plain,
    /// Save and Add: the accent.
    Accent,
    /// A destructive row's confirmation: `danger`.
    Danger,
}

/// A chip named `word` (spoken as `spoken`) that runs `press`.
pub(super) fn chip(
    key: String,
    word: &str,
    spoken: &str,
    tone: Tone,
    press: impl Fn(&mut Shell, &mut Tx) + 'static,
    look: &Look,
) -> El {
    let (fill, ink) = match tone {
        Tone::Plain => (look.palette.surface_hover, look.palette.text_primary),
        Tone::Accent => (look.palette.accent, look.palette.accent_on),
        Tone::Danger => (look.palette.danger, look.palette.danger_on),
    };
    el().key(key)
        .h(look.space(CHIP_HEIGHT))
        .px(look.space(SPACE_MD))
        .center()
        // Concentric with a tile one `space.md` out, as Shape and type's
        // chips are.
        .radius(Look::nested_radius(
            look.radius(TILE_OUTER_RADIUS),
            look.space(SPACE_MD),
        ))
        .fill(fill)
        .role("button")
        .label(spoken.to_owned())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(press)
        .child(
            text(word.to_owned())
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(ink),
        )
}
