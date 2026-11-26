//! A GIF's star (`docs/design.md` §16.4): a 24 pt disc of the picker's glass with a 14 pt
//! `star`, empty in `text.secondary`, filled in the accent on a favorite (§13: the thing is
//! on). Its hover is a glass tint and its icon turns `text.primary`; it is never `danger`:
//! nothing asks and nothing is lost. Also which of a message's GIFs take one, and the star on a
//! conversation GIF. What a press does is the shell's.

use papo_sdk::{Gif, Message, Uuid};
use rune::{El, Tx, el, icon, row};

use crate::design::{Look, Surfaces};
use crate::icons::{STAR, STAR_FILLED};
use crate::message_media::received::{MediaAt, MediaStyle, MediaSurface, media_element_key};
use crate::strings;
use crate::timeline::TimelineHost;

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
        // Not a Tab stop (S and Ctrl+D are the keyboard's way), and a press leaves the keyboard
        // where it was: the picker's search keeps its typing.
        .preserve_focus_on_press(true)
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

/// The GIF of `message`'s preview drawn from embed `embed`, when it takes a star: each picture
/// its own GIF, so a message's second GIF (or a GIF beside another provider's) is told apart.
pub(crate) fn preview_gif(message: &Message, embed: Uuid) -> Option<Gif> {
    message
        .embeds
        .as_deref()?
        .iter()
        .find(|wire| wire.id.as_deref().and_then(|id| Uuid::parse_str(id).ok()) == Some(embed))
        .and_then(papo_sdk_ui::favorite_gif_of)
}

/// Which of `message`'s GIFs are among `favorites`, in order: what its stars' fill draws from.
pub(crate) fn starred(
    message: &Message,
    favorites: &std::collections::HashSet<String>,
) -> Vec<bool> {
    message
        .embeds
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(papo_sdk_ui::favorite_gif_of)
        .map(|gif| favorites.contains(&gif.id))
        .collect()
}

/// `cell`, the conversation's picture of embed `embed` of `message` at `at`, with its GIF's star
/// when it takes one: shown while the GIF is pointed at or focus is in it, 6 pt in from its
/// top-right corner. Pins, search and a confirmation draw none.
pub(crate) fn with_star<S: TimelineHost>(
    cell: El,
    at: MediaAt,
    message: &Message,
    embed: Uuid,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let starred = style.interactive && style.surface == MediaSurface::Timeline;
    let Some(gif) = preview_gif(message, embed).filter(|_| starred) else {
        return cell;
    };
    let place = StarPlace {
        favorite: style.favorite_gifs.contains(&gif.id),
        inset_pt: 6.0,
        always: false,
    };
    // S on the message reaches it from the keyboard: the star needn't be a Tab stop.
    let disc = star::<S>(
        format!("{}-star", media_element_key(at)),
        place,
        move |host: &mut S, tx| host.gif_star_pressed(at, tx),
        look,
    );
    cell.hover_group().child(disc)
}
