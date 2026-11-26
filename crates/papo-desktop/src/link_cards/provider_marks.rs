//! A provider's marks on its card (`docs/design.md` §15.2, §15.3): the provider line with its
//! letter tile, standing for the favicon the server does not relay, and the chip on a player's
//! poster. Pure.

use papo_sdk_ui::LinkCard;
use rune::{Color, El, el, row, text};

use crate::design::{
    LETTER_TILE_PT, LETTER_TILE_RADIUS_PT, Look, PROVIDER_CHIP_GAP_PT, PROVIDER_CHIP_INSET_PT,
    PROVIDER_CHIP_PAD_PT, PROVIDER_CHIP_PT, PROVIDER_LINE_GAP_PT, TYPE_LETTER_TILE, TYPE_SMALL,
    TYPE_SMALL_MEDIUM,
};
use crate::message_media::glass;
use crate::strings;

/// The provider line: the letter tile, then the provider's name and the author's.
pub(super) fn provider_line(card: &LinkCard, tile: Color, look: &Look) -> Option<El> {
    let words = match (&card.provider, &card.author) {
        (Some(provider), Some(author)) => strings::embed_provider_author(provider, author),
        (Some(name), None) | (None, Some(name)) => name.clone(),
        (None, None) => return None,
    };
    Some(
        row()
            .w_full()
            .gap(PROVIDER_LINE_GAP_PT)
            .items_center()
            .child(letter_tile(&words, tile))
            .child(
                text(words)
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.text_muted)
                    .truncate()
                    .grow(1.0)
                    .min_w(0.0),
            ),
    )
}

/// A 12 pt tile in `fill` with the first letter of `name`, standing for the favicon the server
/// does not relay (§15.2).
pub(crate) fn letter_tile(name: &str, fill: Color) -> El {
    let letter: String = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    // The letter takes whichever of white and black reads on the tile.
    let ink = if crate::design::contrast(Color::WHITE, fill)
        >= crate::design::contrast(Color::BLACK, fill)
    {
        Color::WHITE
    } else {
        Color::BLACK
    };
    el().size(LETTER_TILE_PT, LETTER_TILE_PT)
        .radius(LETTER_TILE_RADIUS_PT)
        .fill(fill)
        .center()
        .a11y_hidden()
        .child(text(letter).font(TYPE_LETTER_TILE.0).color(ink))
}

/// The chip at a poster's top left naming whose player it is, on dark glass.
pub(super) fn provider_chip(provider: &str, tile: Color, look: &Look) -> El {
    el().pin(
        Some(PROVIDER_CHIP_INSET_PT),
        Some(PROVIDER_CHIP_INSET_PT),
        None,
        None,
    )
    .a11y_hidden()
    .child(glass(
        row()
            .h(PROVIDER_CHIP_PT)
            .pl(PROVIDER_CHIP_PAD_PT.0)
            .pr(PROVIDER_CHIP_PAD_PT.1)
            .gap(PROVIDER_CHIP_GAP_PT)
            .items_center()
            .radius(PROVIDER_CHIP_PT / 2.0)
            .child(letter_tile(provider, tile))
            .child(
                text(provider.to_owned())
                    .font(look.font(TYPE_SMALL_MEDIUM))
                    .color(Color::WHITE)
                    .truncate(),
            ),
    ))
}
