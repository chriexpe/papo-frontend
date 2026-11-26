//! The GIF tab drawn (`docs/design.md` §16.3): the search well with GIPHY's mark at its right
//! end (or, without a key, the words that say so), the chips, and the body: GIPHY's masonry,
//! Favorites' hint, "No GIFs", "Couldn't reach GIPHY." with Try again, or placeholder tiles while
//! a page comes. No headings: the chip or the search already says what shows. Pure: it reads
//! the tab and hands presses to the shell.

use papo_sdk::{AccountId, Gif};
use rune::{El, KeyEvent, el, icon, row, text, text_input};

use super::masonry;
use super::{GifPicker, Giphy, Shelf};
use crate::design::{
    EMOJI_SEARCH_PT, FOCUS_RING_PT, Look, RADIUS_CONTROL, RADIUS_PILL, SPACE_MD, SPACE_SM,
    TYPE_BODY, TYPE_CAPTION, TYPE_SMALL, TYPE_SMALL_MEDIUM,
};
use crate::icons::{SEARCH, STAR, TRENDING, X};
use crate::media_store::{MediaKey, MediaView};
use crate::picker::Frame;
use crate::shell::Shell;
use crate::strings;

/// The element key of the GIF tab's search field.
pub(crate) const GIF_SEARCH_KEY: &str = "gif-search";
/// The element key of the tiles' scroll viewport.
pub(crate) const GIF_SCROLL_KEY: &str = "gif-scroll";
mod body;

/// A chip's height, in points (§16.3); its corners are `radius-control`, as a reaction chip's
/// (Christian, 2026-11-26: squircle-ish, not round).
const CHIP_PT: f32 = 28.0;
/// A tile's corner radius, in points (§16.3).
const TILE_RADIUS_PT: f32 = 6.0;
/// The placeholders' heights while a page comes, as the canvas draws them.
const PLACEHOLDER_PT: [f32; 6] = [96.0, 132.0, 110.0, 150.0, 88.0, 120.0];

/// What drawing the tab reads besides the tab.
pub(crate) struct GifLook<'a> {
    pub(crate) frame: Frame<'a>,
    /// The tiles' pictures, as the media store holds them.
    pub(crate) media: &'a MediaView,
    /// The account the tab sends as: tiles are fetched as its.
    pub(crate) account: AccountId,
}

/// The element key of GIF `gif`'s tile at place `at`: GIPHY may repeat a GIF across pages.
pub(crate) fn tile_key(at: usize, gif: &Gif) -> String {
    format!("gif-tile-{at}-{}", gif.id)
}

/// The media key of `gif`'s picture in the grid: its preview, else its original.
pub(crate) fn tile_media(account: AccountId, gif: &Gif) -> MediaKey {
    MediaKey::external(account, tile_url(gif))
}

/// The URL a tile draws: GIPHY's grid rendition, else the original.
pub(crate) fn tile_url(gif: &Gif) -> &str {
    if gif.preview_url.is_empty() {
        &gif.url
    } else {
        &gif.preview_url
    }
}

/// The open GIF tab.
pub(crate) fn view(picker: &GifPicker, given: &GifLook<'_>) -> El {
    let look = given.frame.look;
    let padding = look.space(SPACE_SM);
    let (width, _) = given.frame.size();
    let column_pt = masonry::column_width(width - 2.0 * padding);
    given
        .frame
        .panel(strings::picker_gifs())
        // Escape and the arrows work from the chips too, and without a search field at all.
        .on_key(|shell: &mut Shell, event: &KeyEvent, tx| shell.gif_picker_key(event, tx))
        .child(search(picker, look, padding))
        .child(chips(picker, look, padding))
        .child(body::body(picker, given, column_pt, padding))
}

/// The search well (§16.3): 36 pt on `surface.hover`, the field, ✕, then a hairline and
/// "Powered by GIPHY", which stays while typing. Without a key, the words that say so in its
/// place.
fn search(picker: &GifPicker, look: &Look, padding: f32) -> El {
    let radius = Look::nested_radius(look.radius(RADIUS_PILL), padding);
    if picker.giphy == Giphy::NoKey {
        return row().w_full().padding(padding).pb(padding / 2.0).child(
            text(strings::picker_no_key())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .wrap()
                .role("note"),
        );
    }
    let mut field = text_input(&picker.search, look.font(TYPE_BODY))
        .key(GIF_SEARCH_KEY)
        .label(strings::picker_search_gifs())
        .grow(1.0)
        .min_w(0.0)
        .color(look.palette.text_primary)
        .placeholder(strings::picker_search_gifs(), look.palette.text_muted)
        .caret(look.palette.accent, look.palette.text_selection)
        .on_edit(Shell::gif_search_mut)
        .on_input_change(Shell::gif_searched)
        .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.gif_picker_key(key, tx));
    if let Some(at) = picker.cursor
        && let Some(gif) = picker.chosen()
    {
        field = field.active_descendant(tile_key(at, gif)).expanded(true);
    }
    let clear = (!picker.search.text.is_empty()).then(|| {
        row()
            .key("gif-search-clear")
            .size(22.0, 22.0)
            .center()
            .radius(11.0)
            .role("button")
            .label(strings::picker_clear())
            .on_press_tx(|shell: &mut Shell, tx| shell.gif_search_cleared(tx))
            .hover(look.on_pill(look.palette.surface_hover))
            .child(icon(&X, 14.0).color(look.palette.text_secondary))
    });
    let mark = row()
        .gap(look.space(SPACE_SM))
        .center_y()
        .child(
            el().w(1.0)
                .h(16.0)
                .fill(look.palette.line_subtle)
                .a11y_hidden(),
        )
        .child(
            text(strings::picker_powered())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .max_lines(1),
        );
    row().w_full().padding(padding).pb(padding / 2.0).child(
        row()
            .w_full()
            .h(EMOJI_SEARCH_PT)
            .px(look.space(SPACE_MD))
            .gap(look.space(SPACE_MD))
            .center_y()
            .radius(radius)
            .fill(look.palette.surface_hover)
            .child(icon(&SEARCH, 16.0).color(look.palette.text_muted))
            .child(field)
            .children(clear)
            .child(mark),
    )
}

/// The chips (§16.3): ★ Favorites with its count, Recent, Trending, a hairline, then GIPHY's
/// trending searches. One row that scrolls sideways and never wraps; without a key, Favorites
/// and Recent only.
fn chips(picker: &GifPicker, look: &Look, padding: f32) -> El {
    let keyed = picker.giphy != Giphy::NoKey;
    let mut chips = vec![
        chip(
            Shelf::Favorites,
            Some(&STAR),
            format!(
                "{} {}",
                strings::picker_favorites(),
                picker.favorite_count()
            ),
            picker,
            look,
        ),
        chip(
            Shelf::Recent,
            None,
            strings::picker_recent().to_owned(),
            picker,
            look,
        ),
    ];
    if keyed {
        chips.push(chip(
            Shelf::Trending,
            Some(&TRENDING),
            strings::picker_trending().to_owned(),
            picker,
            look,
        ));
        if !picker.trending_searches.is_empty() {
            chips.push(
                el().w(1.0)
                    .h(16.0)
                    .mx(2.0)
                    .fill(look.palette.line_subtle)
                    .a11y_hidden(),
            );
            chips.extend(picker.trending_searches.iter().map(|query| {
                chip(
                    Shelf::Search(query.clone()),
                    None,
                    query.clone(),
                    picker,
                    look,
                )
            }));
        }
    }
    row().key("gif-chips").w_full().scroll_x().child(
        row()
            .px(padding)
            .pb(padding)
            .gap(6.0)
            .center_y()
            .role("tablist")
            .focus_group()
            .children(chips),
    )
}

fn chip(
    shelf: Shelf,
    glyph: Option<&'static rune::Icon>,
    words: String,
    picker: &GifPicker,
    look: &Look,
) -> El {
    let chosen = picker.shelf == shelf;
    let (fill, ink) = if chosen {
        (look.palette.accent_soft, look.palette.link)
    } else {
        (rune::Color::TRANSPARENT, look.palette.text_secondary)
    };
    let key = match &shelf {
        Shelf::Favorites => "gif-chip-favorites".to_owned(),
        Shelf::Recent => "gif-chip-recent".to_owned(),
        Shelf::Trending => "gif-chip-trending".to_owned(),
        Shelf::Search(query) => format!("gif-chip-search-{query}"),
    };
    row()
        .key(key)
        .h(CHIP_PT)
        .px(10.0)
        .gap(4.0)
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .fill(fill)
        .border(1.0, look.palette.line_subtle)
        .role("tab")
        .label(words.clone())
        .focusable(true)
        .focus_selected(chosen)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .hover(look.on_pill(look.palette.surface_hover))
        .on_press_tx(move |shell: &mut Shell, tx| shell.gif_chip_pressed(shelf.clone(), tx))
        .children(glyph.map(|glyph| icon(glyph, 14.0).color(ink)))
        .child(
            text(words)
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(ink)
                .max_lines(1)
                .a11y_hidden(),
        )
}
