//! A person's avatar (`docs/design.md` §10, Presence): the first letter of
//! their name on a rounded square, and their presence badge on its
//! bottom-right corner, the mark (a shape first, a colour second) on a disc
//! of the surface the avatar sits on.
//!
//! Owns only the drawing; the presence comes from the SDK
//! (`AccountActivity::presence_mark`), the sizes from `design`, and saying
//! the presence with the person's name is the caller's
//! (`strings::person_with_presence`). Pictures are a later slice.

use papo_sdk_ui::PresenceMark;
use rune::{Color, El, Icon, el, icon, text};

use crate::design::{AvatarSize, Palette};
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
    palette: &Palette,
) -> El {
    let tile = el()
        .size(size.side_pt, size.side_pt)
        .radius(size.radius_pt)
        .center()
        .fill(palette.surface_hover)
        .child(
            text(initial(name))
                .font(size.letter)
                .color(palette.text_secondary),
        );
    let avatar = el()
        .size(size.side_pt, size.side_pt)
        .a11y_hidden()
        .child(tile);
    match presence {
        Some(mark) => avatar.child(badge(mark, size, surface, palette)),
        None => avatar,
    }
}

/// The badge: `mark` on a disc of `surface`, reaching past the corner.
fn badge(mark: PresenceMark, size: AvatarSize, surface: Color, palette: &Palette) -> El {
    let offset_pt = size.side_pt - size.ring_pt + size.overhang_pt;
    let (shape, colour) = shape(mark, palette);
    el().key("presence")
        .absolute(offset_pt, offset_pt)
        .size(size.ring_pt, size.ring_pt)
        .radius(size.ring_pt / 2.0)
        .center()
        .fill(surface)
        .child(icon(shape, size.mark_pt).color(colour))
}

fn shape(mark: PresenceMark, palette: &Palette) -> (&'static Icon, Color) {
    match mark {
        PresenceMark::Available => (&PRESENCE_ONLINE, palette.presence_online),
        PresenceMark::Away => (&PRESENCE_AWAY, palette.presence_away),
        PresenceMark::Busy => (&PRESENCE_BUSY, palette.presence_busy),
        PresenceMark::Offline => (&PRESENCE_OFFLINE, palette.presence_offline),
    }
}
