//! A slider row (`docs/design.md` §8, Rows): its title and value over a
//! track, and the thumb. The track shows what the slider sets: Colour
//! strength fades from grey to the colour, Grain shows the grain, the rest
//! are plain. A drag is drawn as it goes and saved when it lets go; the
//! arrows step it.
//!
//! Owns the slider's look and what its input asks for (a `Change`); the
//! shell carries it out.

use rune::{Color, El, Key, KeyEvent, Tx, col, el, row, text};

use super::Change;
use super::controls::Ctx;
use super::rows::{Row, Slider, Track, slider_value};
use crate::design::settings::{
    KNOB, KNOB_SHADOW, ROW_INSET_PT, ROW_PADDING, SLIDER_THUMB_PT, SLIDER_THUMB_RING_PT,
    SLIDER_TRACK_PT,
};
use crate::design::{
    FIELD_OUTLINE_PT, FOCUS_RING_PT, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_BODY_MEDIUM,
};
use crate::shell::Shell;

/// A slider row: its title and value on a line, the track under them. The
/// pointer drags the thumb (drawn, not saved, until it lets go); the arrow
/// keys move it a step, Page Up and Down ten, Home and End to the ends,
/// each one saved.
pub(super) fn view(setting: &Row, slider: Slider, cx: &Ctx<'_>) -> El {
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
        .border(SLIDER_THUMB_RING_PT, KNOB)
        .shadow(1.0, 4.0, KNOB_SHADOW);
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
