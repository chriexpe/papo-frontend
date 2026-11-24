//! A group of rows and each row's control (`docs/design.md` §8, Rows): the
//! tile (rows in a group are one block, 2 pt apart, outer corners 16 and
//! inner 4), a switch, a choice with its menu, a sub-page's ›, the accent
//! swatches, a slider and the starting-point chips. Every control is
//! reached by the keyboard and named for assistive technology; a row whose
//! save failed shows one `danger` line under it with Try again.
//!
//! Owns how a row looks and what its input asks for (a `Change`, a page, a
//! menu); the shell carries them out.

use rune::{Color, El, Key, KeyEvent, Tx, col, el, icon, row, text};

use super::rows::{Group, Kind, Row, Slider, Track, direction_name, slider_value};
use super::{Change, RowId, colour_field};
use crate::appearance::Appearance;
use crate::appearance::presets::Direction;
use crate::design::settings::{
    CHIP_HEIGHT, GROUP_TITLE_GAP, ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING, SLIDER_THUMB_PT,
    SLIDER_THUMB_RING_PT, SLIDER_TRACK_PT, SWATCH_PT, SWATCH_RADIUS, SWATCH_RING_PT,
    SWITCH_KNOB_PT, SWITCH_PT, TILE_INNER_RADIUS, TILE_OUTER_RADIUS,
};
use crate::design::{
    Accent, FIELD_OUTLINE_PT, FOCUS_RING_PT, Look, ROW_ICON_PT, SPACE_LG, SPACE_MD, SPACE_SM,
    SPACE_XS, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION, TYPE_SMALL, TYPE_SMALL_MEDIUM,
};
use crate::icons::{CHEVRON_DOWN, CHEVRON_RIGHT};
use crate::shell::Shell;
use crate::strings;

/// What a group needs besides its rows.
pub(super) struct Ctx<'a> {
    pub(super) look: &'a Look,
    /// The settings drawn (a drag's draft while one runs).
    pub(super) appearance: &'a Appearance,
    pub(super) failed: Option<RowId>,
    pub(super) colour: super::ColourState,
    /// The grain's tile for this theme, for Grain's slider.
    pub(super) grain: Option<&'a rune::Pixels>,
}

/// A group: its optional title, then its rows as one block of tiles.
pub(super) fn group(group: Group, cx: &Ctx<'_>) -> El {
    let look = cx.look;
    let count = group.rows.len();
    let tiles = col().w_full().gap(look.space(SPACE_XS)).children(
        group
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| tile(row, index == 0, index + 1 == count, cx)),
    );
    let mut block = col().w_full().gap(look.space(GROUP_TITLE_GAP));
    if let Some(title) = group.title {
        block = block.child(
            el().px(look.space(SPACE_SM)).child(
                text(title)
                    .font(look.font(TYPE_SMALL_MEDIUM))
                    .color(look.palette.text_secondary)
                    .role("heading")
                    .level(3),
            ),
        );
    }
    block.child(tiles)
}

/// One row on its tile, and its danger line when its save failed.
pub(super) fn tile(setting: Row, first: bool, last: bool, cx: &Ctx<'_>) -> El {
    let look = cx.look;
    let (outer, inner) = (
        look.radius(TILE_OUTER_RADIUS),
        look.radius(TILE_INNER_RADIUS),
    );
    let top = if first { outer } else { inner };
    let bottom = if last { outer } else { inner };
    let failed = cx.failed == Some(setting.id);
    let body = match setting.kind {
        Kind::Slider(slider) => slider_row(&setting, slider, cx),
        Kind::ColourField => colour_field::view(cx.appearance, cx.colour, look),
        Kind::Directions(current) => directions(&setting, current, look),
        _ => plain_row(setting, look),
    };
    let tile = el()
        .w_full()
        .corners([top, top, bottom, bottom])
        .fill(look.palette.surface_tile)
        .child(body);
    if !failed {
        return tile;
    }
    col().w_full().gap(look.space(SPACE_XS)).child(tile).child(
        row()
            .px(ROW_INSET_PT)
            .gap(look.space(SPACE_MD))
            .role("alert")
            .child(
                text(strings::settings_save_failed())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.danger),
            )
            .child(
                text(strings::settings_save_retry())
                    .key("settings-retry")
                    .font(look.font(TYPE_SMALL_MEDIUM))
                    .color(look.palette.danger)
                    .role("button")
                    .focusable(true)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                    .on_press_tx(|shell: &mut Shell, tx| shell.settings_retry(tx)),
            ),
    )
}

/// A row of title, summary and its control at the end: a switch, a
/// choice, a sub-page or the accent swatches.
fn plain_row(setting: Row, look: &Look) -> El {
    let Row {
        id,
        title,
        sub,
        kind,
    } = setting;
    let mut words = col().grow(1.0).min_w(0.0).child(
        text(title.clone())
            .font(look.font(TYPE_BODY_MEDIUM))
            .color(look.palette.text_primary),
    );
    if let Some(sub) = &sub {
        words = words.child(
            text(sub.clone())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        );
    }
    let line = row()
        .key(id.key())
        .w_full()
        .min_h(look.space(ROW_MIN_HEIGHT))
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_LG))
        .center_y()
        .child(words);
    let spoken = match &sub {
        Some(sub) => format!("{title}, {sub}"),
        None => title.clone(),
    };
    match kind {
        Kind::Switch(on) => pressable(line, look)
            .role("switch")
            .checked(on)
            .label(spoken)
            .on_press_tx(move |shell: &mut Shell, tx| {
                shell.settings_change(Change::Switch(id, !on), tx);
            })
            .child(switch(on, look)),
        Kind::Choice(value) => pressable(line, look)
            .role("button")
            .label(format!("{spoken}, {value}"))
            .on_press_tx({
                let drawn = look.theme();
                move |shell: &mut Shell, tx| shell.settings_choice(id, drawn, tx)
            })
            .child(value_text(&value, look))
            .child(icon(&CHEVRON_DOWN, ROW_ICON_PT).color(look.palette.text_muted)),
        Kind::SubPage(page, value) => pressable(line, look)
            .role("link")
            .label(format!("{spoken}, {value}"))
            .on_press_tx(move |shell: &mut Shell, tx| shell.settings_page(page, tx))
            .child(value_text(&value, look))
            .child(icon(&CHEVRON_RIGHT, ROW_ICON_PT).color(look.palette.text_muted)),
        Kind::Accents(current) => line.label(spoken).child(accents(current, look)),
        Kind::Slider(_) | Kind::ColourField | Kind::Directions(_) => line,
    }
}

/// A row that is one control: it takes the keyboard and the pointer, and
/// lights under them.
fn pressable(line: El, look: &Look) -> El {
    line.focusable(true)
        .focus_highlight(look.palette.surface_hover)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
}

/// A choice's or sub-page's current value, before its chevron.
fn value_text(value: &str, look: &Look) -> El {
    text(value.to_owned())
        .font(look.font(TYPE_BODY))
        .color(look.palette.text_secondary)
        .max_lines(1)
}

/// A switch: the accent when on, a strong line when off, the knob at its
/// end.
fn switch(on: bool, look: &Look) -> El {
    let (width, height) = SWITCH_PT;
    let inset = (height - SWITCH_KNOB_PT) / 2.0;
    let track = if on {
        look.palette.accent
    } else {
        look.palette.line_strong
    };
    el().size(width, height)
        .radius(height / 2.0)
        .fill(track)
        .a11y_hidden()
        .child(
            el().absolute(
                if on {
                    width - inset - SWITCH_KNOB_PT
                } else {
                    inset
                },
                inset,
            )
            .size(SWITCH_KNOB_PT, SWITCH_KNOB_PT)
            .radius(SWITCH_KNOB_PT / 2.0)
            // White in both states, as the mock draws it: it carries no
            // text, so no contrast pairing applies.
            .fill(Color::WHITE),
        )
}

/// The accent swatches: one per accent, the chosen one ringed.
fn accents(current: Accent, look: &Look) -> El {
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

/// Shape and type's starting points as three chips, the one the values
/// match pressed.
fn directions(setting: &Row, current: Option<Direction>, look: &Look) -> El {
    el().w_full().padding(look.space(SPACE_MD)).child(
        row()
            .key(setting.id.key())
            .w_full()
            .gap(look.space(SPACE_SM))
            .role("radiogroup")
            .label(setting.title.clone())
            .focus_group()
            .children(Direction::ALL.into_iter().map(|direction| {
                let chosen = current == Some(direction);
                let (fill, ink) = if chosen {
                    (look.palette.accent_soft, look.palette.link)
                } else {
                    (look.palette.surface_tile, look.palette.text_secondary)
                };
                el().key(format!("settings-direction-{direction:?}"))
                    .grow(1.0)
                    .h(look.space(CHIP_HEIGHT))
                    .center()
                    .radius(look.radius(crate::design::RADIUS_CONTROL))
                    .fill(fill)
                    .border(FIELD_OUTLINE_PT, look.palette.line_subtle)
                    .role("radio")
                    .checked(chosen)
                    .label(direction_name(direction))
                    .focusable(true)
                    .focus_selected(chosen)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                    .on_press_tx(move |shell: &mut Shell, tx| {
                        shell.settings_change(Change::Direction(direction), tx);
                    })
                    .child(
                        text(direction_name(direction))
                            .font(look.font(TYPE_SMALL_MEDIUM))
                            .color(ink),
                    )
            })),
    )
}

/// A slider row: its title and value on a line, the track under them. The
/// pointer drags the thumb (drawn, not saved, until it lets go); the arrow
/// keys move it a step, Page Up and Down ten, Home and End to the ends,
/// each one saved.
fn slider_row(setting: &Row, slider: Slider, cx: &Ctx<'_>) -> El {
    let look = cx.look;
    let id = setting.id;
    let shown = slider_value(id, slider.value);
    let head = row()
        .w_full()
        .gap(look.space(SPACE_MD))
        .child(
            text(setting.title.clone())
                .font(look.font(TYPE_BODY_MEDIUM))
                .color(look.palette.text_primary)
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            text(shown.clone())
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_secondary),
        );
    let span = (slider.max - slider.min).max(f32::EPSILON);
    let (track, rail) = slider_track(slider, span, cx);
    let control = el()
        .key(id.key())
        .w_full()
        .h(SLIDER_THUMB_PT)
        .center_y()
        .role("slider")
        .label(format!("{}, {shown}", setting.title))
        .range_value(
            f64::from(slider.value),
            f64::from(slider.min),
            f64::from(slider.max),
        )
        .range_step(f64::from(slider.step))
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_drag(move |shell: &mut Shell, drag, tx| {
            if drag.phase == rune::Phase::Up && drag.cancelled() {
                shell.settings_drag_cancelled();
                return;
            }
            let travel = (drag.w - SLIDER_THUMB_PT).max(1.0);
            let at = ((drag.x - SLIDER_THUMB_PT / 2.0) / travel).clamp(0.0, 1.0);
            let value = slider.min + at * span;
            let done = drag.phase == rune::Phase::Up;
            shell.settings_change(
                Change::Slider {
                    row: id,
                    value,
                    done,
                },
                tx,
            );
        })
        .on_key(move |shell: &mut Shell, key: &KeyEvent, tx: &mut Tx| {
            let Some(value) = slider_key(key, slider) else {
                return false;
            };
            shell.settings_change(
                Change::Slider {
                    row: id,
                    value,
                    done: true,
                },
                tx,
            );
            true
        })
        .child(
            el().w_full()
                .child(track)
                .child(rail.absolute(0.0, -(SLIDER_THUMB_PT - SLIDER_TRACK_PT) / 2.0)),
        );
    col()
        .w_full()
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_SM))
        .child(head)
        .child(control)
}

/// A slider's track and the rail its thumb rides on, at `slider`'s value
/// over its `span`.
fn slider_track(slider: Slider, span: f32, cx: &Ctx<'_>) -> (El, El) {
    let look = cx.look;
    let share = ((slider.value - slider.min) / span).clamp(0.0, 1.0);
    let (track_fill, thumb_fill) = track_colours(slider.track, share, cx);
    let mut thumb = el()
        .size(SLIDER_THUMB_PT, SLIDER_THUMB_PT)
        .radius(SLIDER_THUMB_PT / 2.0)
        .clip_rounded(SLIDER_THUMB_PT / 2.0)
        .fill(thumb_fill)
        .border(SLIDER_THUMB_RING_PT, Color::WHITE)
        .shadow(1.0, 4.0, Color::BLACK.alpha(0.3));
    if let (Track::Grain, Some(grain)) = (slider.track, cx.grain) {
        thumb = thumb.child(
            el().w_full()
                .h_full()
                .pixels(grain)
                .opacity(grain_shown(slider.value, cx)),
        );
    }
    // The thumb rides between two spacers that share what is left of the
    // track, so it sits at `share` of the way without knowing the width.
    let rail = row()
        .w_full()
        .h(SLIDER_THUMB_PT)
        .center_y()
        .child(el().grow(share.max(1e-4)))
        .child(thumb)
        .child(el().grow((1.0 - share).max(1e-4)));
    let track = el()
        .w_full()
        .h(SLIDER_TRACK_PT)
        .radius(SLIDER_TRACK_PT / 2.0)
        .border(FIELD_OUTLINE_PT, look.palette.line_strong);
    let track = match track_fill {
        TrackFill::Solid(colour) => track.fill(colour),
        TrackFill::Gradient(from, to) => track
            .fill(from)
            .linear_gradient(90.0, [(0.0, from), (1.0, to)]),
    };
    // Grain's track runs from clean to coarse in steps, each the grain at
    // its share of the way (§4).
    let track = match (slider.track, cx.grain) {
        (Track::Grain, Some(grain)) => {
            track
                .clip_rounded(SLIDER_TRACK_PT / 2.0)
                .child(
                    row()
                        .w_full()
                        .h_full()
                        .children((0..GRAIN_STEPS).map(|step| {
                            #[expect(clippy::cast_precision_loss, reason = "a handful of steps")]
                            let at = step as f32 / (GRAIN_STEPS - 1) as f32;
                            el().grow(1.0)
                                .h_full()
                                .pixels(grain)
                                .opacity(grain_shown(at, cx))
                        })),
                )
        }
        _ => track,
    };
    (track, rail)
}

/// Where a key moves `slider`: a step, ten steps, or an end.
fn slider_key(key: &KeyEvent, slider: Slider) -> Option<f32> {
    let next = match key.key {
        Key::Left | Key::Down => slider.value - slider.step,
        Key::Right | Key::Up => slider.value + slider.step,
        Key::PageDown => slider.value - slider.step * 10.0,
        Key::PageUp => slider.value + slider.step * 10.0,
        Key::Home => slider.min,
        Key::End => slider.max,
        _ => return None,
    };
    Some(next.clamp(slider.min, slider.max))
}

/// What a track is filled with.
enum TrackFill {
    Solid(Color),
    Gradient(Color, Color),
}

/// A track's fill and its thumb's: Colour strength fades from grey to the
/// first colour, the thumb showing the result (§4); others are plain.
fn track_colours(track: Track, share: f32, cx: &Ctx<'_>) -> (TrackFill, Color) {
    let look = cx.look;
    match track {
        Track::Strength => {
            let first = cx.appearance.background.first();
            let lightness = if look.theme().is_dark() { 0.45 } else { 0.75 };
            let grey = rune::oklch(lightness, 0.0, 0.0);
            let full = first.map_or(grey, |colour| {
                rune::oklch(lightness, 0.15 * colour.vividness, colour.hue_degrees)
            });
            let thumb = first.map_or(grey, |colour| {
                rune::oklch(
                    lightness,
                    0.15 * colour.vividness * share,
                    colour.hue_degrees,
                )
            });
            (TrackFill::Gradient(grey, full), thumb)
        }
        Track::Plain => (
            TrackFill::Solid(look.palette.surface_hover),
            look.palette.surface_pill,
        ),
        Track::Grain => (
            TrackFill::Solid(look.palette.surface_hover),
            look.palette.surface_hover,
        ),
    }
}

/// The steps of Grain's track.
const GRAIN_STEPS: usize = 8;
/// How much stronger the grain shows on its slider than over the window,
/// so a small track and thumb show it (the mock's 3).
const GRAIN_SLIDER_EMPHASIS: f32 = 3.0;

/// The grain's opacity on its slider at `grain` (0–1).
fn grain_shown(grain: f32, cx: &Ctx<'_>) -> f32 {
    (crate::grain::GrainTiles::strength(grain, cx.look.theme().is_dark()) * GRAIN_SLIDER_EMPHASIS)
        .min(1.0)
}
