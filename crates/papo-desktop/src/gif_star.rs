//! A GIF's star (`docs/design.md` §16.4): a 24 pt disc of the picker's glass with a 14 pt
//! `star`, empty in `text.secondary`, filled in the accent on a favorite (§13: the thing is
//! on). Its hover is a glass tint and its icon turns `text.primary`; it is never `danger`:
//! nothing asks and nothing is lost. Where it sits and when it shows are its holder's; what a
//! press does is the shell's.

use rune::{El, Tx, el, icon, row};

use crate::design::{FOCUS_RING_PT, Look, Surfaces};
use crate::icons::{STAR, STAR_FILLED};
use crate::strings;

/// The disc's side, in points.
const DISC_PT: f32 = 24.0;
/// The star's size, in points.
const STAR_PT: f32 = 14.0;

/// How a star stands on its GIF.
#[derive(Clone, Copy)]
pub(crate) struct StarPlace {
    /// Its GIF is a favorite: the star is filled.
    pub(crate) favorite: bool,
    /// How far in from the GIF's top-right corner, in points.
    pub(crate) inset_pt: f32,
    /// Shown at rest too; else only while its GIF's hover group is lit (pointed at, or focus
    /// inside it).
    pub(crate) always: bool,
}

/// The star over a GIF, pressed with `press` on the host `S` that draws it.
pub(crate) fn star<S: 'static>(
    key: String,
    place: StarPlace,
    press: impl Fn(&mut S, &mut Tx) + 'static,
    look: &Look,
) -> El {
    let (shape, ink) = if place.favorite {
        (&STAR_FILLED, look.palette.accent)
    } else {
        (&STAR, look.palette.text_secondary)
    };
    let hover_ink = if place.favorite {
        look.palette.accent
    } else {
        look.palette.text_primary
    };
    let label = if place.favorite {
        strings::fav_remove()
    } else {
        strings::fav_add()
    };
    let disc = row()
        .key(key)
        .size(DISC_PT, DISC_PT)
        .center()
        .radius(DISC_PT / 2.0)
        .pill_surface(look)
        .role("button")
        .label(label)
        .help(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .hover(look.on_pill(look.palette.surface_hover))
        .hover_group()
        .on_press_tx(press)
        .child(icon(shape, STAR_PT).color(ink).group_color(hover_ink));
    // Its own wrapper takes the GIF's group: the disc is a group of its own, for its icon.
    let mut holder = el()
        .pin(None, Some(place.inset_pt), Some(place.inset_pt), None)
        .child(disc);
    if !place.always {
        // At rest nothing shows; it comes and goes at once, as the hover bar does.
        holder = holder.group_opacity(0.0, 1.0);
    }
    holder
}
