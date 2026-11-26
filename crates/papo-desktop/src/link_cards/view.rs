//! A message's link previews, drawn (`docs/design.md` §15.1, §15.3 in D8a): a card is the 2 pt
//! rule and its parts in order (the provider line with its letter tile, the title as the link,
//! the description, the fields, the picture or a provider video's poster, the footer), one
//! anatomy for every kind; a link to a bare GIF is media at Image size with its badge. Card text
//! is not selectable: the link is already in the message's text. Pure.

use papo_sdk::Message;
use papo_sdk_ui::{
    AttachmentKind, CardVideo, LinkCard, LinkPreview, MediaBounds, MediaFit, card_box, media_box,
};
use rune::color::srgb_decode;
use rune::{Color, El, Link, LinkPaint, Span, col, el, icon, rich, row, text};

use super::player::{self, PlayerBox};
use super::provider_marks::{provider_chip, provider_line};
use super::{
    CardPress, attachment_count, clip_of, is_provider_video, name_of, player_of, provider_of,
};
use crate::design::{
    CARD_DESCRIPTION_LINES, CARD_FIELD_COLUMN_GAP_PT, CARD_FIELD_ROW_GAP_PT,
    CARD_FIELD_VALUE_LINES, CARD_FIELDS_PER_ROW, CARD_RULE_PT, FOCUS_RING_PT, LINK_UNDERLINE_ALPHA,
    Look, MESSAGE_BLOCK_GAP, PROVIDER_PLAY_HOVER_ALPHA, RADIUS_CARD, TYPE_BODY_STRONG,
    TYPE_CAPTION, TYPE_CAPTION_MEDIUM, TYPE_SMALL, VIDEO_PLAY_ALPHA, VIDEO_PLAY_ICON_PT,
    VIDEO_PLAY_PT, card_rule,
};
use crate::icons::PLAY;
use crate::message_media::received::{MediaAt, MediaStyle, MediaSurface, Shown, picture_cell};
use crate::message_media::video::{self, VideoCell};
use crate::message_media::{inner_edge, single_box};
use crate::strings;
use crate::timeline::TimelineHost;
use crate::timeline::content::Press;

/// `message`'s previews at `at` (the message's place; each preview's follows its
/// attachments), one under the other at the parent's gap; none when it draws none.
/// `lead_gap_pt` is the density's lead gap, between a card's rule and its parts.
pub(crate) fn previews<S: TimelineHost>(
    at: MediaAt,
    message: &Message,
    style: &MediaStyle,
    lead_gap_pt: f32,
    look: &Look,
) -> Option<El> {
    let previews = super::previews(message, style.previews);
    if previews.is_empty() {
        return None;
    }
    let first = attachment_count(message);
    let mut block = col().w_full().gap(look.space(MESSAGE_BLOCK_GAP));
    for (place, preview) in previews.iter().enumerate() {
        let at = MediaAt {
            index: first + place,
            ..at
        };
        block = block.child(match preview {
            LinkPreview::Card(card) => card_view::<S>(at, preview, card, style, lead_gap_pt, look),
            LinkPreview::Media(media) => {
                let boxed = media_box(media.picture.size, style.bounds);
                let shown = shown_picture(at, preview);
                let cell = picture_cell::<S>(
                    single_box(boxed, look),
                    at,
                    &shown,
                    boxed.fit,
                    0,
                    style,
                    look,
                )
                .child(inner_edge(look));
                with_star::<S>(cell, at, message, style, look)
            }
        });
    }
    Some(block)
}

/// A GIF in the conversation that arrived as a GIPHY link wears its star (§16.4): shown while
/// it is pointed at or focus is in it, 6 pt in from its top-right corner; filled on a favorite.
/// Pins, search and a confirmation draw none.
fn with_star<S: TimelineHost>(
    cell: El,
    at: MediaAt,
    message: &Message,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let starred = style.interactive && style.surface == MediaSurface::Timeline;
    let Some(gif) = papo_sdk_ui::favorite_gif(message).filter(|_| starred) else {
        return cell;
    };
    let place = crate::gif_star::StarPlace {
        favorite: style.favorite_gifs.contains(&gif.id),
        inset_pt: 6.0,
        always: false,
    };
    let key = format!(
        "{}-star",
        crate::message_media::received::media_element_key(at)
    );
    cell.hover_group().child(crate::gif_star::star::<S>(
        key,
        place,
        move |host: &mut S, tx| host.gif_star_pressed(at, tx),
        look,
    ))
}

/// The element key of the card at `at`: menus open beside it.
pub(crate) fn card_key(at: MediaAt) -> String {
    format!(
        "{}-card",
        crate::message_media::received::media_element_key(at)
    )
}

/// One card.
fn card_view<S: TimelineHost>(
    at: MediaAt,
    preview: &LinkPreview,
    card: &LinkCard,
    style: &MediaStyle,
    lead_gap_pt: f32,
    look: &Look,
) -> El {
    let key = crate::timeline::TimelineKey {
        account: at.account,
        channel: at.channel,
    };
    let message = at.message;
    let rule = card.colour.map_or(look.palette.line_strong, |channels| {
        card_rule(srgb(channels), &look.palette)
    });
    let mut parts = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space(MESSAGE_BLOCK_GAP));
    if let Some(line) = provider_line(card, rule, look) {
        parts = parts.child(line);
    }
    // The card's picture box bounds: the card is as wide as Embed size.
    let bounds = super::picture_bounds(style.embed_bounds, lead_gap_pt);
    // A provider's track in the conversation: its cover row, then its strip (§15.4), in
    // place of the title, description, fields and picture.
    let track = player_of(at, preview)
        .filter(|(_, player)| card.track.as_ref() == Some(*player))
        .filter(|_| style.interactive && style.surface == MediaSurface::Timeline);
    if let Some((player_key, player)) = track {
        parts = parts.child(super::track::track::<S>(
            &super::track::TrackCard {
                at,
                preview,
                card,
                key: player_key,
                player,
                bounds,
                chip_tile: rule,
            },
            style,
            look,
        ));
    } else {
        if let Some(title) = &card.title {
            parts = parts.child(title_text::<S>(
                key,
                message,
                card,
                title,
                style.interactive,
                look,
            ));
        }
        if let Some(description) = &card.description {
            parts = parts.child(
                text(description.clone())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.text_secondary)
                    .max_lines(CARD_DESCRIPTION_LINES)
                    .w_full(),
            );
        }
        if !card.fields.is_empty() {
            parts = parts.child(fields(card, look));
        }
        if let Some(picture) = picture::<S>(at, preview, card, bounds, style, rule, look) {
            parts = parts.child(picture);
        }
    }
    if let Some(footer) = &card.footer {
        parts = parts.child(
            text(footer.clone())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .w_full(),
        );
    }
    let provider = provider_of(preview);
    let label = match &card.title {
        Some(title) => strings::embed_card_label(&provider, title),
        None => strings::embed_card_label_untitled(&provider),
    };
    let mut built = row()
        .key(card_key(at))
        .w_full()
        .max_w(style.embed_bounds.max_width_pt)
        .gap(lead_gap_pt)
        .items_stretch()
        .role("group")
        .label(label)
        .child(el().w(CARD_RULE_PT).fill(rule).a11y_hidden())
        .child(parts);
    if style.interactive {
        built = built.on_secondary_press(move |host: &mut S, press, tx| {
            host.card_pressed(at, CardPress::Menu([press.x, press.y]), tx);
        });
    }
    built
}

/// An embed's 8-bit sRGB colour as Rune's linear colour.
fn srgb(channels: [u8; 3]) -> Color {
    let [r, g, b] = channels.map(|channel| srgb_decode(f32::from(channel) / 255.0));
    Color { r, g, b, a: 1.0 }
}

/// The title, which is the link: underlined at 40 % under the pointer or the keyboard, opened
/// by §9's link rules. Without a safe page it is plain text.
pub(super) fn title_text<S: TimelineHost>(
    key: crate::timeline::TimelineKey,
    message: papo_sdk::Uuid,
    card: &LinkCard,
    title: &str,
    interactive: bool,
    look: &Look,
) -> El {
    let ink = look.palette.text_primary;
    // Inside a confirmation the message only shows: no link opens there.
    let Some(page) = card.page.as_ref().filter(|_| interactive) else {
        return text(title.to_owned())
            .font(look.font(TYPE_BODY_STRONG))
            .color(ink)
            .w_full();
    };
    let whole = LinkPaint::new().underline(ink.alpha(LINK_UNDERLINE_ALPHA));
    let address = page.address.clone();
    rich(title.to_owned(), vec![Span::new(0, title.len()).color(ink)])
        .font(look.font(TYPE_BODY_STRONG))
        .color(ink)
        .w_full()
        .link(
            Link::new(0..title.len(), Press::Url(address.clone()).id())
                .hover(whole)
                .focused(whole)
                .help(address),
        )
        .on_link(move |host: &mut S, id: &str, tx| {
            if let Some(press) = Press::parse(id) {
                host.content_pressed(key, message, press, tx);
            }
        })
}

/// The fields, three to a row, each its label over its value.
fn fields(card: &LinkCard, look: &Look) -> El {
    let mut rows = col().w_full().gap(CARD_FIELD_ROW_GAP_PT);
    for chunk in card.fields.chunks(CARD_FIELDS_PER_ROW) {
        let mut line = row().w_full().gap(CARD_FIELD_COLUMN_GAP_PT).items_start();
        for field in chunk {
            line = line.child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(field.label.clone())
                            .font(look.font(TYPE_CAPTION_MEDIUM))
                            .color(look.palette.text_muted)
                            .truncate(),
                    )
                    .child(
                        text(field.value.clone())
                            .font(look.font(TYPE_SMALL))
                            .color(look.palette.text_primary)
                            .max_lines(CARD_FIELD_VALUE_LINES)
                            .w_full(),
                    ),
            );
        }
        // A short last row keeps its columns where the full rows have them.
        for _ in chunk.len()..CARD_FIELDS_PER_ROW {
            line = line.child(el().grow(1.0).min_w(0.0));
        }
        rows = rows.child(line);
    }
    rows
}

/// A preview's picture as a picture cell shows it.
pub(super) fn shown_picture(at: MediaAt, preview: &LinkPreview) -> Shown {
    let gif = super::picture_of(preview).is_some_and(|picture| picture.kind == AttachmentKind::Gif);
    let label = if gif {
        strings::media_gif().to_owned()
    } else {
        strings::media_image(&name_of(preview))
    };
    Shown {
        key: super::picture_key(at.account, preview).map(|(key, _)| key),
        gif,
        hidden: false,
        label,
    }
}

/// The card's picture, a relayed clip, or a provider video's poster, in its reserved box.
fn picture<S: TimelineHost>(
    at: MediaAt,
    preview: &LinkPreview,
    card: &LinkCard,
    bounds: MediaBounds,
    style: &MediaStyle,
    chip_tile: Color,
    look: &Look,
) -> Option<El> {
    let boxed = card_box(card, bounds)?;
    let base = single_box(boxed, look);
    if let Some((key, CardVideo::Clip { .. })) = clip_of(at.account, preview) {
        let cell = VideoCell {
            at,
            key,
            fit: boxed.fit,
            hidden: false,
        };
        return Some(video::single::<S>(base, &cell, style, look).child(inner_edge(look)));
    }
    let shown = shown_picture(at, preview);
    let mut cell = if shown.key.is_some() {
        picture_cell::<S>(base, at, &shown, MediaFit::Fill, 0, style, look)
    } else {
        // A provider video without a relayed poster: its box, the chip and the disc only.
        base.fill(look.palette.surface_hover).role("img")
    };
    if is_provider_video(preview) {
        let provider = provider_of(preview);
        let label = match &card.title {
            Some(title) => strings::embed_poster_label(&provider, title),
            None => strings::embed_poster_label_untitled(&provider),
        };
        cell = cell.label(label);
        let chip = provider_chip(&provider, chip_tile, look);
        let plays = style.interactive && style.surface == MediaSurface::Timeline;
        cell = match player_of(at, preview).filter(|_| plays) {
            // The player's layers sit beside the poster, not in it: the poster's keys (Enter
            // and Space open the viewer) must not take the ask's.
            // The box at its exact size: beside it sits the column for the live player's
            // buttons, outside the card's width, so the box keeps Embed size.
            Some((key, descriptor)) => player::player::<S>(
                el().size(boxed.width_pt, boxed.height_pt)
                    .radius(look.radius(RADIUS_CARD))
                    .clip_rounded(look.radius(RADIUS_CARD)),
                cell,
                chip,
                &PlayerBox {
                    at,
                    key,
                    provider: descriptor.provider.name(),
                    title: card.title.as_deref(),
                    players: &style.players,
                },
                look,
            ),
            // Pins, search and confirmations never play (§15.1), and a provider off the
            // allowlist has no player: the page opens in the browser.
            None => cell.child(chip).child(open_disc::<S>(at, style, look)),
        };
    }
    Some(cell.child(inner_edge(look)))
}

/// The play disc of a poster that never plays here: it opens the page in the browser, by §9's
/// link rules, and says so.
fn open_disc<S: TimelineHost>(at: MediaAt, style: &MediaStyle, look: &Look) -> El {
    let mut disc = el()
        .size(VIDEO_PLAY_PT, VIDEO_PLAY_PT)
        .radius(VIDEO_PLAY_PT / 2.0)
        .center()
        .fill(Color::BLACK.alpha(VIDEO_PLAY_ALPHA))
        .child(icon(&PLAY, VIDEO_PLAY_ICON_PT).color(Color::WHITE));
    if style.interactive {
        disc = disc
            .key(format!("{}-open", card_key(at)))
            .role("button")
            .label(strings::embed_open_browser())
            .hover(Color::BLACK.alpha(PROVIDER_PLAY_HOVER_ALPHA))
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| {
                host.card_pressed(at, CardPress::OpenInBrowser, tx);
            });
    } else {
        disc = disc.a11y_hidden();
    }
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .child(disc)
}
