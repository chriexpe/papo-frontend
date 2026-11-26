//! The GIF tab's body (`docs/design.md` §16.3): GIPHY's masonry and its tiles, placeholders
//! while a page comes, "Loading more", "Couldn't reach GIPHY." with Try again, Favorites' hint
//! and "No GIFs". The search and the chips are `view`'s.

use papo_sdk_ui::MediaFit;
use rune::{El, LoopSpec, ScrollSpec, col, el, icon, row, text};

use super::super::masonry::{self, GAP_PT, Placed};
use super::super::{GifPicker, Giphy, Shelf};
use super::{
    CHIP_PT, GIF_SCROLL_KEY, GifLook, PLACEHOLDER_PT, TILE_RADIUS_PT, tile_key, tile_media,
};
use crate::design::{
    FOCUS_RING_PT, Look, PILL_ICON_PT, RADIUS_CONTROL, RADIUS_PILL, SPACE_MD, SPACE_SM,
    SPINNER_PERIOD_S, TYPE_CAPTION, TYPE_SMALL, TYPE_SMALL_MEDIUM,
};
use crate::icons::{SPINNER, STAR};
use crate::message_media::{Layer, failed, painted};
use crate::shell::Shell;
use crate::strings;

/// What shows under the chips.
pub(super) fn body(picker: &GifPicker, given: &GifLook<'_>, column_pt: f32, padding: f32) -> El {
    let look = given.frame.look;
    let giphy_shelf = matches!(picker.shelf, Shelf::Trending | Shelf::Search(_));
    let gifs = picker.gifs();
    if picker.shelf == Shelf::Favorites && gifs.is_empty() {
        return favorites_hint(look, padding);
    }
    if giphy_shelf && gifs.is_empty() && picker.giphy == Giphy::Unreachable {
        return unreachable(look);
    }
    if picker.first_page_loading() {
        return scroll(placeholders(column_pt, 0), padding);
    }
    if let (Shelf::Search(query), true) = (&picker.shelf, gifs.is_empty()) {
        return centred(
            text(strings::picker_no_gifs(query))
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted)
                .role("status"),
        );
    }
    let columns = masonry::columns(gifs, column_pt);
    let mut grid = row()
        .key("gif-grid")
        .on_layout_tx(|shell: &mut Shell, rect, tx| shell.gif_grid_moved(rect, tx))
        .w_full()
        .gap(GAP_PT)
        .items_start()
        .role("listbox")
        .label(strings::picker_gifs())
        .children(columns.iter().map(|column| {
            col().w(column_pt).gap(GAP_PT).children(
                column
                    .iter()
                    .map(|placed| tile(picker, *placed, column_pt, given)),
            )
        }));
    if picker.more_loading() {
        grid = col()
            .w_full()
            .gap(GAP_PT)
            .child(grid)
            .child(placeholders(column_pt, 1))
            .child(loading_more(look));
    } else if picker.giphy == Giphy::Unreachable && giphy_shelf {
        grid = col()
            .w_full()
            .gap(GAP_PT)
            .child(grid)
            .child(unreachable(look));
    }
    scroll(grid, padding)
}

/// One tile (§16.3): radius 6, as tall as its GIF's proportions give, its picture once it
/// loads (§14.4's failed look if it can't), animating while it shows (`shell::gif_tiles`).
fn tile(picker: &GifPicker, placed: Placed, width_pt: f32, given: &GifLook<'_>) -> El {
    let look = given.frame.look;
    let Some(gif) = picker.gifs().get(placed.at) else {
        return el();
    };
    let key = tile_media(given.account, gif);
    let drawn = given.media.get(key);
    let image = drawn.and_then(|drawn| drawn.picture(given.media.animates(key)));
    let at = placed.at;
    let base = el()
        .key(tile_key(at, gif))
        .w(width_pt)
        .h(placed.height_pt)
        .radius(TILE_RADIUS_PT)
        .clip_rounded(TILE_RADIUS_PT);
    let mut tile = painted(base, image, MediaFit::Fill, Layer::default(), look)
        .role("option")
        .label(strings::picker_gif_tile(&gif.title))
        .on_press_tx(move |shell: &mut Shell, tx| {
            let keep = tx.mods().shift;
            shell.gif_pressed(at, keep, tx);
        })
        .on_hover_change(move |shell: &mut Shell, on, _tx| shell.gif_tile_hovered(key, on));
    if drawn.is_some_and(|drawn| drawn.failed) {
        let retry = text(strings::media_retry())
            .font(look.font(TYPE_CAPTION))
            .color(look.palette.link)
            .role("button")
            .on_press_tx(move |shell: &mut Shell, _tx| shell.gif_tile_retry(key));
        tile = tile.child(failed(retry, look));
    }
    if picker.cursor == Some(at) {
        tile = tile.outline(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring);
    }
    tile
}

/// Placeholder tiles while a page comes, shimmering (still under reduced motion: Rune holds a
/// loop's pose then). `phase` staggers a second set.
fn placeholders(column_pt: f32, phase: usize) -> El {
    let tiles = PLACEHOLDER_PT
        .iter()
        .enumerate()
        .map(|(index, height)| (index + phase, *height))
        .collect::<Vec<_>>();
    let mut columns: [Vec<El>; masonry::COLUMNS] = Default::default();
    for (index, height) in tiles {
        #[expect(clippy::cast_precision_loss, reason = "six placeholders")]
        let delay = (index % 3) as f64 * 0.15;
        columns[index % masonry::COLUMNS].push(
            el().w(column_pt)
                .h(height)
                .radius(TILE_RADIUS_PT)
                .fill(rune::Color::WHITE.alpha(0.06))
                .looping(LoopSpec::opacity(1.2, delay, 0.45, 1.0))
                .a11y_hidden(),
        );
    }
    row()
        .w_full()
        .gap(GAP_PT)
        .items_start()
        .busy(true)
        .children(
            columns
                .into_iter()
                .map(|column| col().w(column_pt).gap(GAP_PT).children(column)),
        )
}

/// "Loading more" with a spinner, under the placeholders.
fn loading_more(look: &Look) -> El {
    row()
        .w_full()
        .py(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM))
        .center()
        .role("status")
        .child(
            icon(&SPINNER, 14.0)
                .color(look.palette.text_muted)
                .looping(LoopSpec::rotation(SPINNER_PERIOD_S)),
        )
        .child(
            text(strings::picker_loading_more())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        )
}

/// "Couldn't reach GIPHY." with Try again (§16.3).
fn unreachable(look: &Look) -> El {
    centred(
        col()
            .gap(look.space(SPACE_SM))
            .center()
            .role("alert")
            .child(
                text(strings::picker_failed())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.text_secondary),
            )
            .child(
                row()
                    .key("gif-try-again")
                    .h(CHIP_PT)
                    .px(12.0)
                    .center()
                    .radius(look.radius(RADIUS_CONTROL))
                    .border(1.0, look.palette.line_subtle)
                    .role("button")
                    .focusable(true)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                    .hover(look.on_pill(look.palette.surface_hover))
                    .on_press_tx(|shell: &mut Shell, tx| shell.gif_try_again(tx))
                    .child(
                        text(strings::picker_try_again())
                            .font(look.font(TYPE_SMALL_MEDIUM))
                            .color(look.palette.link),
                    ),
            ),
    )
}

/// Favorites with none yet (§16.4): a dashed hint under the chips, with the `star` icon.
fn favorites_hint(look: &Look, padding: f32) -> El {
    col().w_full().px(padding).child(
        row()
            .w_full()
            .padding(look.space(SPACE_MD))
            .gap(look.space(SPACE_MD))
            .center_y()
            .radius(Look::nested_radius(look.radius(RADIUS_PILL), padding))
            .border(1.0, look.palette.line_subtle)
            .border_dash(4.0, 3.0)
            .role("note")
            .child(icon(&STAR, PILL_ICON_PT).color(look.palette.text_muted))
            .child(
                text(strings::picker_fav_empty())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.text_secondary)
                    .wrap(),
            ),
    )
}

/// `content` in the tiles' scroll viewport.
fn scroll(content: El, padding: f32) -> El {
    col()
        .key(GIF_SCROLL_KEY)
        .on_layout_tx(|shell: &mut Shell, rect, tx| shell.gif_list_moved(rect, tx))
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(padding)
        .pb(padding)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(content)
}

/// `content` in the middle of the body.
fn centred(content: El) -> El {
    col().w_full().grow(1.0).center().child(content)
}
