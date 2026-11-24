//! The tools and presets under the Background colour field (`docs/design.md`
//! §4, Background colour): add a colour, remove the selected one, arrange
//! them in harmony; and the presets, seven a page, with ‹ ›.
//!
//! Owns how they look and what they ask for (a `ColourChange`); the field
//! itself is `colour_field`'s, and the shell applies the change.

use rune::{Color, El, el, icon, row};

use super::colour_field::{ColourChange, SHOWN_LIGHTNESS, shown};
use super::{Change, ColourState};
use crate::appearance::Appearance;
use crate::appearance::presets::{PRESETS, PRESETS_PER_PAGE, Preset, PresetId};
use crate::design::settings::{PRESET_PT, SWATCH_RADIUS, SWATCH_RING_PT, TOOL_PT};
use crate::design::{
    BUTTON_DISABLED_OPACITY, BackgroundColour, FIELD_OUTLINE_PT, FOCUS_RING_PT, Look,
    MAX_BACKGROUND_COLOURS, PILL_ICON_PT, RADIUS_CONTROL, SPACE_SM,
};
use crate::icons::{CHEVRON_LEFT, CHEVRON_RIGHT, MINUS, PLUS, SHUFFLE};
use crate::shell::Shell;
use crate::strings;

/// +, − and harmony.
pub(super) fn tools(appearance: &Appearance, look: &Look) -> El {
    let count = appearance.background.colours().len();
    let tool = |key: &'static str,
                shape: &'static rune::Icon,
                label: &'static str,
                usable: bool,
                change: ColourChange| {
        let mut button = el()
            .key(key)
            .size(TOOL_PT, TOOL_PT)
            .center()
            .radius(look.radius(RADIUS_CONTROL))
            .fill(look.palette.surface_tile)
            .border(FIELD_OUTLINE_PT, look.palette.line_subtle)
            .role("button")
            .label(label)
            .child(icon(shape, PILL_ICON_PT).color(look.palette.text_secondary));
        if usable {
            button = button
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, tx| {
                    shell.settings_change(Change::Colours(change), tx);
                });
        } else {
            button = button.disabled(true).opacity(BUTTON_DISABLED_OPACITY);
        }
        button
    };
    let first = appearance.background.first();
    // + adds the selected colour's opposite, which harmony can rearrange.
    let added = first.map_or(
        BackgroundColour {
            hue_degrees: 255.0,
            vividness: 0.45,
        },
        |colour| BackgroundColour {
            hue_degrees: colour.hue_degrees + 180.0,
            vividness: colour.vividness,
        },
    );
    row()
        .gap(look.space(SPACE_SM))
        .child(tool(
            "settings-colour-add",
            &PLUS,
            strings::settings_colour_add(),
            count < MAX_BACKGROUND_COLOURS,
            ColourChange::Add(added),
        ))
        .child(tool(
            "settings-colour-remove",
            &MINUS,
            strings::settings_colour_remove(),
            count > 0,
            ColourChange::Remove,
        ))
        .child(tool(
            "settings-colour-harmony",
            &SHUFFLE,
            strings::settings_harmony(),
            count > 0,
            ColourChange::Harmony,
        ))
}

/// The presets, a page at a time, with ‹ and › between pages.
pub(super) fn presets(appearance: &Appearance, state: ColourState, look: &Look) -> El {
    let pages = PRESETS.len().div_ceil(PRESETS_PER_PAGE);
    let page = state.preset_page.min(pages - 1);
    let current = Preset::matching(&appearance.background).map(|preset| preset.id);
    let arrow =
        |key: &'static str, shape: &'static rune::Icon, label: &'static str, to: Option<usize>| {
            let mut button = el()
                .key(key)
                .size(TOOL_PT, PRESET_PT)
                .center()
                .role("button")
                .label(label)
                .child(icon(shape, PILL_ICON_PT).color(look.palette.text_secondary));
            if let Some(to) = to {
                button = button
                    .focusable(true)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                    .on_press_tx(move |shell: &mut Shell, _tx| shell.settings_preset_page(to));
            } else {
                button = button.disabled(true).opacity(BUTTON_DISABLED_OPACITY);
            }
            button
        };
    let swatches = row()
        .grow(1.0)
        .gap(look.space(SPACE_SM))
        .justify_center()
        .children(
            PRESETS
                .iter()
                .skip(page * PRESETS_PER_PAGE)
                .take(PRESETS_PER_PAGE)
                .map(|preset| preset_swatch(preset, current == Some(preset.id), look)),
        );
    row()
        .w_full()
        .gap(look.space(SPACE_SM))
        .center_y()
        .child(arrow(
            "settings-presets-prev",
            &CHEVRON_LEFT,
            strings::settings_presets_prev(),
            page.checked_sub(1),
        ))
        .child(swatches)
        .child(arrow(
            "settings-presets-next",
            &CHEVRON_RIGHT,
            strings::settings_presets_next(),
            (page + 1 < pages).then_some(page + 1),
        ))
}

/// A preset as a swatch: its colours across it, grey for Neutral.
#[expect(clippy::cast_precision_loss, reason = "at most three colours")]
fn preset_swatch(preset: &Preset, chosen: bool, look: &Look) -> El {
    let background = preset.background();
    let stops: Vec<(f32, Color)> = match background.colours() {
        [] => vec![(0.0, rune::oklch(SHOWN_LIGHTNESS, 0.0, 0.0))],
        colours => {
            let last = (colours.len().max(2) - 1) as f32;
            colours
                .iter()
                .enumerate()
                .map(|(index, colour)| (index as f32 / last, shown(*colour)))
                .collect()
        }
    };
    let id = preset.id;
    let mut swatch = el()
        .key(format!("settings-preset-{id:?}"))
        .size(PRESET_PT, PRESET_PT)
        .radius(look.radius(SWATCH_RADIUS))
        .fill(stops[0].1)
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
        .label(preset_name(id))
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |shell: &mut Shell, tx| {
            shell.settings_change(Change::Colours(ColourChange::Preset(id)), tx);
        });
    if stops.len() > 1 {
        swatch = swatch.linear_gradient(crate::design::CHROME_GRADIENT_DEGREES, stops);
    }
    swatch
}

/// A preset's name.
pub(crate) fn preset_name(id: PresetId) -> &'static str {
    match id {
        PresetId::Neutral => strings::preset_neutral(),
        PresetId::Ink => strings::preset_ink(),
        PresetId::Slate => strings::preset_slate(),
        PresetId::Moss => strings::preset_moss(),
        PresetId::Teal => strings::preset_teal(),
        PresetId::Plum => strings::preset_plum(),
        PresetId::Rose => strings::preset_rose(),
        PresetId::Ember => strings::preset_ember(),
        PresetId::Sand => strings::preset_sand(),
        PresetId::Dusk => strings::preset_dusk(),
        PresetId::Lagoon => strings::preset_lagoon(),
        PresetId::Aurora => strings::preset_aurora(),
        PresetId::Sunset => strings::preset_sunset(),
        PresetId::Forest => strings::preset_forest(),
        PresetId::MonoBlue => strings::preset_monoblue(),
        PresetId::MonoGreen => strings::preset_monogreen(),
    }
}
