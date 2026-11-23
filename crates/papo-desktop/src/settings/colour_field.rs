//! The Background colour page's field (`docs/design.md` §4, Background
//! colour): a disc on which the angle is the hue and the distance from the
//! centre the vividness, with up to three colours on it; the tools that add
//! a colour, remove the selected one and arrange them in harmony; and the
//! presets, seven a page.
//!
//! The pointer adds a colour where it presses (while there is room) or
//! drags the nearest one, drawn as it moves and saved when it lets go. A
//! colour takes the keyboard: ← → turn its hue, ↑ ↓ change its vividness,
//! each step saved. Owns the field's geometry and what its input asks for;
//! the shell applies it (`shell/settings.rs`).

use rune::{Color, El, Key, KeyEvent, Tx, col, el, icon, row};

use super::{Change, ColourState};
use crate::appearance::Appearance;
use crate::appearance::presets::{PRESETS, PRESETS_PER_PAGE, Preset, PresetId};
use crate::design::settings::{
    COLOUR_DOT_PT, COLOUR_DOT_RING_PT, FIELD_DOT_PT, FIELD_DOT_STEP_PT, FIELD_HEIGHT_PT,
    FIELD_RADIUS, PRESET_PT, SWATCH_RADIUS, SWATCH_RING_PT, TOOL_PT,
};
use crate::design::{
    BUTTON_DISABLED_OPACITY, BackgroundColour, FIELD_OUTLINE_PT, FOCUS_RING_PT, Look,
    MAX_BACKGROUND_COLOURS, PILL_ICON_PT, RADIUS_CONTROL, SPACE_MD, SPACE_SM,
};
use crate::icons::{CHEVRON_LEFT, CHEVRON_RIGHT, MINUS, PLUS, SHUFFLE};
use crate::shell::Shell;
use crate::strings;

/// The disc's radius, in points: the field's height, less a dot's room.
const DISC_RADIUS_PT: f32 = (FIELD_HEIGHT_PT - COLOUR_DOT_PT) / 2.0;
/// How near a press must land to a colour to drag it, in points.
const GRAB_PT: f32 = COLOUR_DOT_PT;
/// What one arrow key turns a colour's hue by, in degrees, and changes its
/// vividness by.
const HUE_STEP_DEGREES: f32 = 5.0;
const VIVIDNESS_STEP: f32 = 0.05;
/// The lightness and chroma a colour is shown at on the field.
const SHOWN_LIGHTNESS: f32 = 0.68;
const SHOWN_CHROMA: f32 = 0.16;

/// What the field's input asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ColourChange {
    /// Colour `index` to `colour`; `done` saves it.
    Move {
        index: usize,
        colour: BackgroundColour,
        done: bool,
    },
    /// A colour more, at `colour`, selected.
    Add(BackgroundColour),
    /// The selected colour goes.
    Remove,
    /// The colours rearranged by the next harmony.
    Harmony,
    /// A preset's colours and strength.
    Preset(PresetId),
}

/// The field, its tools and the presets, as one block.
pub(crate) fn view(appearance: &Appearance, state: ColourState, look: &Look) -> El {
    col()
        .w_full()
        .padding(look.space(SPACE_MD))
        .gap(look.space(SPACE_MD))
        .child(field(appearance, state, look))
        .child(tools(appearance, look))
        .child(presets(appearance, state, look))
}

/// Where `colour` sits on the disc, from its centre, in points.
fn place(colour: BackgroundColour) -> (f32, f32) {
    let (sin, cos) = colour.hue_degrees.to_radians().sin_cos();
    let reach = colour.vividness * DISC_RADIUS_PT;
    (reach * cos, -reach * sin)
}

/// The colour at a point `(dx, dy)` from the disc's centre.
fn colour_at(dx: f32, dy: f32) -> BackgroundColour {
    let hue_degrees = (-dy).atan2(dx).to_degrees().rem_euclid(360.0);
    let vividness = (dx.hypot(dy) / DISC_RADIUS_PT).clamp(0.0, 1.0);
    BackgroundColour {
        hue_degrees,
        vividness,
    }
}

/// How a background colour is shown: its hue at its vividness.
fn shown(colour: BackgroundColour) -> Color {
    rune::oklch(
        SHOWN_LIGHTNESS,
        SHOWN_CHROMA * colour.vividness,
        colour.hue_degrees,
    )
}

fn field(appearance: &Appearance, state: ColourState, look: &Look) -> El {
    let colours = appearance.background.colours();
    let side = FIELD_HEIGHT_PT;
    let centre = side / 2.0;
    let mut disc = el()
        .key("settings-colour-disc")
        .size(side, side)
        .role("group")
        .label(strings::settings_field_spoken())
        .on_drag(move |shell: &mut Shell, drag, tx| shell.settings_field_drag(drag, centre, tx));
    // The hue dots: a grid, each dot its hue at its distance's vividness.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a field's height over the dot step: a few dozen"
    )]
    let steps = (side / FIELD_DOT_STEP_PT).floor() as u16;
    for gx in 0..=steps {
        for gy in 0..=steps {
            let (x, y) = (
                f32::from(gx) * FIELD_DOT_STEP_PT,
                f32::from(gy) * FIELD_DOT_STEP_PT,
            );
            let (dx, dy) = (x - centre, y - centre);
            if dx.hypot(dy) > DISC_RADIUS_PT {
                continue;
            }
            disc = disc.child(
                el().absolute(x - FIELD_DOT_PT / 2.0, y - FIELD_DOT_PT / 2.0)
                    .size(FIELD_DOT_PT, FIELD_DOT_PT)
                    .radius(FIELD_DOT_PT / 2.0)
                    .fill(shown(colour_at(dx, dy)))
                    .a11y_hidden(),
            );
        }
    }
    for (index, colour) in colours.iter().copied().enumerate() {
        let (dx, dy) = place(colour);
        let selected = index == state.selected.min(colours.len().saturating_sub(1));
        let ring = if selected {
            look.palette.text_primary
        } else {
            Color::WHITE
        };
        disc = disc.child(
            el().key(format!("settings-colour-{index}"))
                .absolute(
                    centre + dx - COLOUR_DOT_PT / 2.0,
                    centre + dy - COLOUR_DOT_PT / 2.0,
                )
                .size(COLOUR_DOT_PT, COLOUR_DOT_PT)
                .radius(COLOUR_DOT_PT / 2.0)
                .fill(shown(colour))
                .border(COLOUR_DOT_RING_PT, ring)
                .shadow(1.0, 4.0, Color::BLACK.alpha(0.35))
                .role("button")
                .label(strings::settings_colour_spoken(
                    index + 1,
                    colour.hue_degrees,
                    colour.vividness,
                ))
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, _tx| shell.settings_select_colour(index))
                .on_key(move |shell: &mut Shell, key: &KeyEvent, tx: &mut Tx| {
                    let Some(moved) = colour_key(key, colour) else {
                        return false;
                    };
                    let change = ColourChange::Move {
                        index,
                        colour: moved,
                        done: true,
                    };
                    shell.settings_change(Change::Colours(change), tx);
                    true
                }),
        );
    }
    let mut frame = el()
        .w_full()
        .h(side)
        .center()
        .radius(look.radius(FIELD_RADIUS))
        .fill(look.palette.surface_code)
        .border(FIELD_OUTLINE_PT, look.palette.line_subtle)
        .child(disc);
    if colours.is_empty() {
        frame = frame.child(
            rune::text(strings::settings_field_empty())
                .font(look.font(crate::design::TYPE_SMALL_MEDIUM))
                .color(look.palette.text_secondary)
                .absolute(0.0, 0.0),
        );
    }
    frame
}

/// Where an arrow key moves `colour`: ← → its hue, ↑ ↓ its vividness.
fn colour_key(key: &KeyEvent, colour: BackgroundColour) -> Option<BackgroundColour> {
    let BackgroundColour {
        hue_degrees,
        vividness,
    } = colour;
    let moved = match key.key {
        Key::Left => (hue_degrees + HUE_STEP_DEGREES, vividness),
        Key::Right => (hue_degrees - HUE_STEP_DEGREES, vividness),
        Key::Up => (hue_degrees, vividness + VIVIDNESS_STEP),
        Key::Down => (hue_degrees, vividness - VIVIDNESS_STEP),
        _ => return None,
    };
    Some(BackgroundColour {
        hue_degrees: moved.0.rem_euclid(360.0),
        vividness: moved.1.clamp(0.0, 1.0),
    })
}

/// What a press at `(x, y)` on the disc does, `centre` its middle: grab a
/// colour near it, add one there while there is room, or move the selected
/// one there.
pub(crate) fn pressed(
    appearance: &Appearance,
    selected: usize,
    (x, y): (f32, f32),
    centre: f32,
) -> (usize, ColourChange) {
    let colours = appearance.background.colours();
    let (dx, dy) = (x - centre, y - centre);
    let at = colour_at(dx, dy);
    let near = colours
        .iter()
        .enumerate()
        .map(|(index, colour)| {
            let (cx, cy) = place(*colour);
            (index, (cx - dx).hypot(cy - dy))
        })
        .filter(|(_, distance)| *distance <= GRAB_PT)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((index, _)) = near {
        return (
            index,
            ColourChange::Move {
                index,
                colour: at,
                done: false,
            },
        );
    }
    if colours.len() < MAX_BACKGROUND_COLOURS {
        return (colours.len(), ColourChange::Add(at));
    }
    let index = selected.min(colours.len() - 1);
    (
        index,
        ColourChange::Move {
            index,
            colour: at,
            done: false,
        },
    )
}

/// The colour a drag at `(x, y)` puts at `index`.
pub(crate) fn dragged(index: usize, (x, y): (f32, f32), centre: f32, done: bool) -> ColourChange {
    ColourChange::Move {
        index,
        colour: colour_at(x - centre, y - centre),
        done,
    }
}

/// +, − and harmony.
fn tools(appearance: &Appearance, look: &Look) -> El {
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
fn presets(appearance: &Appearance, state: ColourState, look: &Look) -> El {
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
