//! A person's avatar (`docs/design.md` §10, Presence): the first letter of
//! their name on a rounded square, and their presence badge on its
//! bottom-right corner, the mark (a shape first, a colour second) on a disc
//! of the surface the avatar sits on.
//!
//! Owns only the drawing; the presence comes from the SDK
//! (`AccountActivity::presence_mark`), the sizes from `design`, and saying
//! the presence with the person's name is the caller's
//! (`strings::person_with_presence`). A photo, where the caller has one
//! (the profile cards), takes the letter's place; elsewhere photos are a
//! later slice.

use papo_sdk_ui::PresenceMark;
use rune::{Color, El, Icon, Image, SurfaceFit, el, icon, text};

use crate::design::{AvatarSize, Look};
use crate::icons::{PRESENCE_AWAY, PRESENCE_BUSY, PRESENCE_OFFLINE, PRESENCE_ONLINE};
use crate::rail::initial;

/// `name`'s avatar at `size`, with `presence`'s badge on a disc of
/// `surface` (none while presence is not known). Hidden from assistive
/// technology: the row or card says who it is.
pub(crate) fn person_avatar(
    name: &str,
    size: AvatarSize,
    presence: Option<PresenceMark>,
    surface: Color,
    look: &Look,
) -> El {
    photo_avatar(name, None, size, presence, surface, look)
}

/// [`person_avatar`] showing `photo`: the tile fills in as it decodes (a
/// decode redraws without rebuilding the view); the letter shows without
/// one, or when it fails.
pub(crate) fn photo_avatar(
    name: &str,
    photo: Option<&Image>,
    size: AvatarSize,
    presence: Option<PresenceMark>,
    surface: Color,
    look: &Look,
) -> El {
    let radius = look.radius(size.radius);
    let base = el()
        .size(size.side_pt, size.side_pt)
        .radius(radius)
        .fill(look.palette.surface_hover);
    let tile = match photo.filter(|photo| photo.error().is_none()) {
        Some(photo) => base
            .clip_rounded(radius)
            .pixels_fit(photo.pixels(), SurfaceFit::Fill),
        None => base.center().child(
            text(initial(name))
                .font(look.font(size.letter))
                .color(look.palette.text_secondary),
        ),
    };
    let avatar = el()
        .size(size.side_pt, size.side_pt)
        .a11y_hidden()
        .child(tile);
    match presence {
        Some(mark) => avatar.child(badge(mark, size, surface, look)),
        None => avatar,
    }
}

/// The badge: `mark` on a disc of `surface`, reaching past the corner.
fn badge(mark: PresenceMark, size: AvatarSize, surface: Color, look: &Look) -> El {
    let offset_pt = size.side_pt - size.ring_pt + size.overhang_pt;
    let (shape, colour) = shape(mark, look);
    el().key("presence")
        .absolute(offset_pt, offset_pt)
        .size(size.ring_pt, size.ring_pt)
        .radius(size.ring_pt / 2.0)
        .center()
        .fill(surface)
        .child(icon(shape, size.mark_pt).color(colour))
}

fn shape(mark: PresenceMark, look: &Look) -> (&'static Icon, Color) {
    match mark {
        PresenceMark::Away => (&PRESENCE_AWAY, look.palette.presence_away),
        PresenceMark::Busy => (&PRESENCE_BUSY, look.palette.presence_busy),
        PresenceMark::Offline => (&PRESENCE_OFFLINE, look.palette.presence_offline),
        // Available, and a mark this client does not know yet: connected.
        _ => (&PRESENCE_ONLINE, look.palette.presence_online),
    }
}
