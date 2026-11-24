//! The panel's swatches (`docs/design.md` §8, Rows: "Swatches, slider"):
//! the accents, and a highlight's hues (Message highlights). Each a radio
//! group the arrow keys move through, the chosen one ringed.
//!
//! Owns the swatches' look and what a press asks for; the shell applies it.

use rune::{Color, El, el, row};

use super::rows::HIGHLIGHT_HUES;
use super::{Change, RowId};
use crate::design::settings::{SWATCH_PT, SWATCH_RADIUS, SWATCH_RING_PT};
use crate::design::{Accent, FOCUS_RING_PT, Look, SPACE_MD};
use crate::shell::Shell;
use crate::strings;

/// The accent swatches: one per accent, the chosen one ringed.
pub(super) fn accents(current: Accent, look: &Look) -> El {
    let dark = look.theme().is_dark();
    row()
        .gap(look.space(SPACE_MD))
        .role("radiogroup")
        .label(strings::settings_accent())
        .focus_group()
        .children(Accent::ALL.into_iter().map(|accent| {
            let lightness = if dark { 0.70 } else { 0.555 };
            let colour = rune::oklch(lightness, 0.18, accent.hue_degrees());
            let chosen = accent == current;
            el().key(format!("settings-accent-{accent:?}"))
                .size(SWATCH_PT, SWATCH_PT)
                .radius(look.radius(SWATCH_RADIUS))
                .fill(colour)
                .border(
                    SWATCH_RING_PT,
                    if chosen {
                        look.palette.text_primary
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .role("radio")
                .checked(chosen)
                .label(super::rows::accent_name(accent))
                .focusable(true)
                .focus_selected(chosen)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, tx| {
                    shell.settings_change(Change::Accent(accent), tx);
                })
        }))
}

/// A highlight's hue swatches (Message highlights): one per hue in
/// `HIGHLIGHT_HUES`, the chosen one ringed.
pub(super) fn hues(id: RowId, name: &str, current: f32, look: &Look) -> El {
    let dark = look.theme().is_dark();
    row()
        .gap(look.space(SPACE_MD))
        .role("radiogroup")
        .label(name.to_owned())
        .focus_group()
        .children(HIGHLIGHT_HUES.into_iter().map(|hue| {
            let lightness = if dark { 0.70 } else { 0.555 };
            let colour = rune::oklch(lightness, 0.15, hue);
            let chosen = (hue - current).abs() < 0.5;
            let degrees = strings::decimal(hue, 0);
            el().key(format!("{}-{degrees}", id.key()))
                .size(SWATCH_PT, SWATCH_PT)
                .radius(look.radius(SWATCH_RADIUS))
                .fill(colour)
                .border(
                    SWATCH_RING_PT,
                    if chosen {
                        look.palette.text_primary
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .role("radio")
                .checked(chosen)
                .label(strings::settings_hue_spoken(&degrees))
                .focusable(true)
                .focus_selected(chosen)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, tx| {
                    shell.settings_change(Change::Hue(id, hue), tx);
                })
        }))
}
