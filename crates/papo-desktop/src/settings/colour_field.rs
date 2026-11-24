//! The Background colour page's field (`docs/design.md` §4, Background
//! colour): a disc on which the angle is the hue and the distance from the
//! centre the vividness, with up to three colours on it; under it the
//! tools and the presets (`colour_tools`).
//!
//! The pointer adds a colour where it presses (while there is room) or
//! drags the nearest one, drawn as it moves and saved when it lets go. A
//! colour takes the keyboard: ← → turn its hue, ↑ ↓ change its vividness,
//! each step saved. Owns the field's geometry and what its input asks for;
//! the shell applies it (`shell/settings.rs`).

use rune::{Color, El, Key, KeyEvent, Phase, Tx, col, el};

use super::{Change, ColourState};
use crate::appearance::Appearance;
use crate::appearance::presets::PresetId;
use crate::design::settings::{
    COLOUR_DOT_PT, COLOUR_DOT_RING_PT, FIELD_DOT_PT, FIELD_DOT_STEP_PT, FIELD_HEIGHT_PT, KNOB,
    KNOB_SHADOW, TILE_OUTER_RADIUS,
};
use crate::design::{
    BackgroundColour, FIELD_OUTLINE_PT, FOCUS_RING_PT, Look, MAX_BACKGROUND_COLOURS, SPACE_MD,
};
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
pub(super) const SHOWN_LIGHTNESS: f32 = 0.68;
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
        .child(super::colour_tools::tools(appearance, look))
        .child(super::colour_tools::presets(appearance, state, look))
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
pub(super) fn shown(colour: BackgroundColour) -> Color {
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
        .label(strings::settings_field_spoken());
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
            KNOB
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
                .shadow(1.0, 4.0, KNOB_SHADOW)
                .role("button")
                .label(strings::settings_colour_spoken(
                    index + 1,
                    colour.hue_degrees,
                    colour.vividness,
                ))
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                // A press on a colour lands on it, not the field: it drags
                // itself, from where it sits by how far the pointer moved.
                .on_drag(move |shell: &mut Shell, drag, tx| {
                    shell.settings_colour_drag(index, colour, drag, tx);
                })
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
    // The whole field takes the pointer, the disc centred in it: a press
    // on empty space grabs the nearest colour or adds one, and past the
    // disc's edge a colour is at full vividness on that side. A press on
    // a colour is that colour's own drag (and its arrow keys).
    let mut frame = el()
        .key("settings-colour-field")
        .w_full()
        .h(side)
        .center()
        .on_drag(|shell: &mut Shell, drag, tx| shell.settings_field_drag(drag, tx))
        // Concentric with the tile, alone in its group so round at every
        // corner, one `space.md` out.
        .radius(Look::nested_radius(
            look.radius(TILE_OUTER_RADIUS),
            look.space(SPACE_MD),
        ))
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

/// Colour `index`, which sat at `colour` when the press began, dragged by
/// `travel` points: drawn while the drag runs, saved when it lets go.
pub(crate) fn moved_by(
    index: usize,
    colour: BackgroundColour,
    travel: (f32, f32),
    phase: Phase,
) -> ColourChange {
    // Not moved (a press and a release on the dot): the colour as it was,
    // not as it comes back from its place on the field.
    let moved = if travel == (0.0, 0.0) {
        colour
    } else {
        let (x, y) = place(colour);
        colour_at(x + travel.0, y + travel.1)
    };
    ColourChange::Move {
        index,
        colour: moved,
        done: phase == Phase::Up,
    }
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

/// What a press at `(x, y)` on the field does, `centre` the disc's middle
/// in the field: grab a colour near it, add one there while there is room,
/// or move the selected one there.
pub(crate) fn pressed(
    appearance: &Appearance,
    selected: usize,
    (x, y): (f32, f32),
    centre: (f32, f32),
) -> (usize, ColourChange) {
    let colours = appearance.background.colours();
    let (dx, dy) = (x - centre.0, y - centre.1);
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
pub(crate) fn dragged(
    index: usize,
    (x, y): (f32, f32),
    centre: (f32, f32),
    done: bool,
) -> ColourChange {
    ColourChange::Move {
        index,
        colour: colour_at(x - centre.0, y - centre.1),
        done,
    }
}
