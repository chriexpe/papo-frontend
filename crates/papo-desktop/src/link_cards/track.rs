//! A provider's track in its card (`docs/design.md` §15.4): at rest the 96 pt cover with its
//! 36 pt play disc, the title (the card's link) and the description beside it; from the ask on,
//! the strip, 152 pt tall and as wide as Embed size allows, in the same place, with the
//! player's states (`player`). A press anywhere on the cover plays, as the disc does. Pure.

use papo_sdk_ui::{LinkCard, LinkPreview, MediaBounds, MediaFit, ProviderPlayer};
use rune::{Color, El, Tx, col, el, row, text};

use super::player::{self, PlayerBox, play_disc_of, poster_press};
use super::provider_marks::provider_chip;
use super::view::{shown_picture, title_text};
use crate::design::{
    CARD_DESCRIPTION_LINES, Look, MESSAGE_BLOCK_GAP, PLAYER_COVER_DISC_PT, PLAYER_COVER_GAP_PT,
    PLAYER_COVER_ICON_PT, PLAYER_COVER_PT, PLAYER_STRIP_PT, RADIUS_CARD, RADIUS_CONTROL,
    TYPE_SMALL,
};
use crate::message_media::received::{MediaAt, MediaStyle, picture_cell};
use crate::provider_players::{Phase, PlayerKey};
use crate::strings;
use crate::timeline::TimelineHost;

/// What a track is drawn from.
pub(super) struct TrackCard<'a> {
    pub(super) at: MediaAt,
    pub(super) preview: &'a LinkPreview,
    pub(super) card: &'a LinkCard,
    pub(super) key: PlayerKey,
    pub(super) player: &'a ProviderPlayer,
    /// The card's picture box bounds (Embed size, in the card's column).
    pub(super) bounds: MediaBounds,
    /// The colour of the provider chip's letter tile.
    pub(super) chip_tile: Color,
}

/// The track: its cover row at rest, else the strip with the player's state.
pub(super) fn track<S: TimelineHost>(track: &TrackCard<'_>, style: &MediaStyle, look: &Look) -> El {
    let state = style.players.shown.get(&track.key);
    let at_rest = state.is_none_or(|state| state.phase == Phase::Stopped);
    if at_rest {
        cover_row::<S>(track, style, look)
    } else {
        strip::<S>(track, style, look)
    }
}

/// The cover with its disc, and the title and description beside it.
fn cover_row<S: TimelineHost>(track: &TrackCard<'_>, style: &MediaStyle, look: &Look) -> El {
    let TrackCard { at, key, card, .. } = *track;
    let base = el()
        .size(PLAYER_COVER_PT, PLAYER_COVER_PT)
        .radius(look.radius(RADIUS_CONTROL))
        .clip_rounded(look.radius(RADIUS_CONTROL));
    let label = strings::embed_track_label(track.player.provider.name(), card.title.as_deref());
    let loadable = style.players.loadable;
    let cover = picture::<S>(base, track, style, look)
        .label(label)
        .child(poster_press::<S>(at, key, loadable))
        .child(play_disc_of::<S>(
            at,
            key,
            loadable,
            [PLAYER_COVER_DISC_PT, PLAYER_COVER_ICON_PT],
            look,
        ));
    let mut words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space(MESSAGE_BLOCK_GAP));
    let timeline = crate::timeline::TimelineKey {
        account: at.account,
        channel: at.channel,
    };
    if let Some(title) = &card.title {
        words = words.child(title_text::<S>(
            timeline,
            at.message,
            card,
            title,
            style.interactive,
            look,
        ));
    }
    if let Some(description) = &card.description {
        words = words.child(
            text(description.clone())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_secondary)
                .max_lines(CARD_DESCRIPTION_LINES)
                .w_full(),
        );
    }
    // The strip takes the cover row's place: its box is known before play, so the page opens
    // at the strip's size rather than a video's and Spotify picks its compact layout at once.
    let strip_width = strip_width(track);
    row()
        .w_full()
        .items_start()
        .gap(PLAYER_COVER_GAP_PT)
        .on_layout_tx(move |host: &mut S, rect: [f32; 4], tx: &mut Tx| {
            host.player_moved(key, [rect[0], rect[1], strip_width, PLAYER_STRIP_PT]);
            tx.live_only();
        })
        .child(cover)
        .child(words)
}

/// The strip's width: as wide as Embed size allows, never narrower than the cover.
fn strip_width(track: &TrackCard<'_>) -> f32 {
    track.bounds.max_width_pt.max(PLAYER_COVER_PT)
}

/// The strip: the cover filling it under the player's dim, ask, ring, page or failure.
fn strip<S: TimelineHost>(track: &TrackCard<'_>, style: &MediaStyle, look: &Look) -> El {
    let width = strip_width(track);
    let holder = el()
        .size(width, PLAYER_STRIP_PT)
        .radius(look.radius(RADIUS_CARD))
        .clip_rounded(look.radius(RADIUS_CARD));
    let poster = picture::<S>(el().size(width, PLAYER_STRIP_PT), track, style, look);
    let provider = track.player.provider.name();
    player::player::<S>(
        holder,
        poster,
        provider_chip(provider, track.chip_tile, look),
        &PlayerBox {
            at: track.at,
            key: track.key,
            provider,
            title: track.card.title.as_deref(),
            players: &style.players,
        },
        look,
    )
}

/// The cover picture in `base`, cropped to fill it; a plain box when the server relayed none.
fn picture<S: TimelineHost>(
    base: El,
    track: &TrackCard<'_>,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let shown = shown_picture(track.at, track.preview);
    if shown.key.is_some() {
        picture_cell::<S>(base, track.at, &shown, MediaFit::Fill, 0, style, look)
    } else {
        base.fill(look.palette.surface_hover).role("img")
    }
}
