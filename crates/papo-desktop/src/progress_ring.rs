//! The upload ring (`docs/design.md` §14.3): a track and an arc that fills
//! clockwise from 12 o'clock, drawn as line icons. The arcs are quantised
//! to [`RING_STEPS`] and built once, so a ring costs no allocation as it
//! fills and its look follows the colour it is given.
//!
//! Owns only the shapes; sizes and colours come from `design` through the
//! view that draws it.

use std::sync::LazyLock;

use rune::{Color, El, Icon, el, icon};

/// How finely the arc moves: one step is 2 %.
const RING_STEPS: usize = 50;
/// The ring's grid, in SVG units square.
const VIEW: f32 = 36.0;
/// The ring's radius on that grid, leaving room for a 3-unit stroke.
const RADIUS: f32 = 15.0;

/// The SVG body of each step's arc, 0 (empty) to [`RING_STEPS`] (whole).
static ARC_BODIES: LazyLock<Vec<String>> =
    LazyLock::new(|| (0..=RING_STEPS).map(arc_body).collect());

/// The arcs as icons, one a step.
static ARCS: LazyLock<Vec<Icon>> = LazyLock::new(|| {
    ARC_BODIES
        .iter()
        .map(|body| Icon {
            name: "papo-upload-arc",
            view: VIEW,
            body: body.as_str(),
        })
        .collect()
});

/// The whole ring, as the track.
static TRACK: LazyLock<Icon> = LazyLock::new(|| Icon {
    name: "papo-upload-track",
    view: VIEW,
    body: ARC_BODIES[RING_STEPS].as_str(),
});

/// The arc for `step` of [`RING_STEPS`].
fn arc_body(step: usize) -> String {
    let centre = VIEW / 2.0;
    let pen = "stroke=\"currentColor\" stroke-width=\"3\" stroke-linecap=\"round\" fill=\"none\"";
    if step == 0 {
        return String::new();
    }
    if step == RING_STEPS {
        return format!(r#"<circle cx="{centre}" cy="{centre}" r="{RADIUS}" {pen}/>"#);
    }
    #[expect(clippy::cast_precision_loss, reason = "at most fifty steps")]
    let turn = step as f32 / RING_STEPS as f32;
    let angle = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
    let (x, y) = (centre + RADIUS * angle.cos(), centre + RADIUS * angle.sin());
    let large = u8::from(step * 2 > RING_STEPS);
    let top = centre - RADIUS;
    format!(r#"<path d="M{centre} {top}A{RADIUS} {RADIUS} 0 {large} 1 {x:.3} {y:.3}" {pen}/>"#)
}

/// The step that shows `done` (0 to 1).
fn step_of(done: f32) -> usize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a clamped fraction of fifty steps"
    )]
    let step = (done.clamp(0.0, 1.0) * RING_STEPS as f32).round() as usize;
    step.min(RING_STEPS)
}

/// A ring of `side` points: the track in `track`, and the arc for `done`
/// (0 to 1; none draws the empty ring of an item still waiting) in `ink`.
/// Hidden from assistive technology: the element holding it says the
/// progress.
pub(crate) fn ring(done: Option<f32>, side: f32, track: Color, ink: Color) -> El {
    let mut ring = el()
        .size(side, side)
        .a11y_hidden()
        .child(icon(&TRACK, side).absolute(0.0, 0.0).color(track));
    if let Some(done) = done {
        ring = ring.child(
            icon(&ARCS[step_of(done)], side)
                .absolute(0.0, 0.0)
                .color(ink),
        );
    }
    ring
}

#[cfg(test)]
mod tests {
    //! The arcs' geometry and steps.
    use super::*;

    #[test]
    fn progress_rounds_to_its_step_and_stays_in_range() {
        assert_eq!(step_of(0.0), 0);
        assert_eq!(step_of(0.5), 25);
        assert_eq!(step_of(0.011), 1);
        assert_eq!(step_of(2.0), RING_STEPS);
        assert_eq!(step_of(-1.0), 0);
    }

    #[test]
    fn an_empty_arc_draws_nothing_and_a_whole_one_a_circle() {
        assert_eq!(arc_body(0), "");
        assert!(arc_body(RING_STEPS).starts_with("<circle"));
    }

    #[test]
    fn an_arc_past_half_takes_the_long_way_round() {
        assert!(arc_body(20).contains(" 0 0 1 "));
        assert!(arc_body(30).contains(" 0 1 1 "));
    }
}
