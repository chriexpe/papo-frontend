//! The attachment tray's row on the text pill (`docs/design.md` §14.2; the
//! mock's `.tray`): one tile per file, pictures cropped to fill, other files
//! as an extension block with their name and size, ✕ on hover or focus, a
//! ring while a file still copies, and the line where a dragged tile lands.
//!
//! Pure: it reads the tray and its interaction state and hands presses,
//! drags and keys to the shell (`shell/tray.rs`).

use rune::{
    Color, Curve, Drag, Easing, El, KeyEvent, LoopSpec, SurfaceFit, Transition, col, el, icon, row,
    text,
};

use super::tray::{Tile, TileId, TileKind, Tray};
use super::tray_ui::{Gesture, TrayUi};
use crate::attaching::extension_label;
use crate::design::{
    DRAGGED_TILE_OPACITY, DROP_SLOT_PT, EXTENSION_BLOCK_PT, EXTENSION_BLOCK_RADIUS,
    FILE_TILE_INSET_PT, FILE_TILE_WIDTH_PT, FOCUS_RING_PT, Look, MEDIA_BADGE_ALPHA,
    MEDIA_BADGE_PAD_PT, MEDIA_BADGE_RADIUS, PILL_OUTLINE_PT, PREPARING_RING_PT,
    PREPARING_SCRIM_ALPHA, RADIUS_CONTROL, SPACE_MD, SPACE_XS, SPINNER_PERIOD_S,
    TILE_BADGE_INSET_PT, TILE_EXIT_S, TILE_EXIT_SCALE, TILE_MARK_PT, TILE_PT, TILE_REMOVE_FADE_S,
    TILE_REMOVE_ICON_PT, TILE_REMOVE_INSET_PT, TILE_REMOVE_PT, TYPE_CAPTION, TYPE_EXTENSION,
    TYPE_MEDIA_BADGE, TYPE_SMALL_MEDIUM,
};
use crate::icons::{ANONYMOUS, RENAMED, SPINNER, X};
use crate::image_editor::cut::cut_picture;
use crate::shell::Shell;
use crate::strings;

/// The element key of the tray, its one tab stop.
pub(crate) const TRAY_KEY: &str = "composer-tray";

/// The element key of tile `tile`.
pub(crate) fn tile_key(tile: &Tile) -> String {
    tile_key_of(tile.id)
}

/// The element key of the tile `id`.
pub(crate) fn tile_key_of(id: TileId) -> String {
    format!("tray-tile-{}", id.0)
}

/// The tray's row, on top of the field and under a reply bar; none while
/// it is empty.
pub(crate) fn tray(tray: &Tray, ui: &TrayUi, look: &Look) -> Option<El> {
    if tray.is_empty() {
        return None;
    }
    let tiles = tray.tiles();
    let current = ui.current_in(&tray.ids()).filter(|_| ui.focused);
    let (dragged, slot) = match ui.gesture {
        Gesture::Dragging { id, slot } => (Some(id), Some(slot)),
        Gesture::Idle => (None, None),
    };
    let mut strip = row()
        .gap(look.space(SPACE_MD))
        .padding(look.space(SPACE_MD))
        .items_center();
    for (place, tile) in tiles.iter().enumerate() {
        if slot == Some(place) {
            strip = strip.child(slot_line(look));
        }
        let shown = Shown {
            current: current == Some(tile.id),
            dragged: dragged == Some(tile.id),
        };
        strip = strip.child(tile_view(tile, shown, look));
    }
    if slot == Some(tiles.len()) {
        strip = strip.child(slot_line(look));
    }
    let mut list = col()
        .key(TRAY_KEY)
        .w_full()
        .role("listbox")
        .label(strings::tray_label(tiles.len()));
    if let Some(tile) = current.and_then(|id| tray.get(id)) {
        // The tile the keyboard is on is spoken as the list's option.
        list = list.active_descendant(tile_key(tile));
    }
    Some(
        list.focusable(true)
            // The tile the keyboard is on shows the ring, not the row.
            .no_focus_ring()
            .on_focus_change(|shell: &mut Shell, focused, tx| shell.tray_focus_changed(focused, tx))
            .on_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.tray_key(key, tx))
            .on_secondary_press(|shell: &mut Shell, press, tx| {
                shell.tray_menu(None, Some(press), tx);
            })
            .border_sides([0.0, 0.0, PILL_OUTLINE_PT, 0.0], look.palette.line_pill)
            .scroll_x()
            .on_scroll_offset({
                let scrolled = ui.scrolled.clone();
                move |at| {
                    scrolled.set(at.offset[0]);
                }
            })
            // A mouse wheel turns the row sideways: more tiles than fit
            // are reached without a horizontal wheel or Shift.
            .on_wheel(|shell: &mut Shell, wheel, tx| shell.tray_wheel(wheel, tx))
            .child(strip),
    )
}

/// How a tile shows besides its file.
#[derive(Clone, Copy)]
struct Shown {
    /// The keyboard is on it: ring and ✕.
    current: bool,
    /// It is being dragged: dimmed.
    dragged: bool,
}

/// One tile: a picture or a file, ✕ at its corner.
fn tile_view(tile: &Tile, shown: Shown, look: &Look) -> El {
    let id = tile.id;
    let picture = tile.kind != TileKind::File
        && tile
            .thumbnail
            .as_ref()
            .is_none_or(|image| image.error().is_none());
    let face = if picture {
        media_tile(tile, look)
    } else {
        file_tile(tile, look)
    };
    let size = tile.size_bytes.map(strings::size).unwrap_or_default();
    let mut tile_el = face
        .key(tile_key(tile))
        .radius(look.radius(RADIUS_CONTROL))
        .clip_rounded(look.radius(RADIUS_CONTROL))
        .role("option")
        .label(spoken(tile))
        .help(strings::tray_tile_tip(&tile.name, &size))
        .hover_group()
        .hover_motion(
            Curve::tween(TILE_REMOVE_FADE_S, Easing::EASE_OUT),
            Curve::tween(TILE_REMOVE_FADE_S, Easing::EASE_OUT),
        )
        .transition(Transition::scale(TILE_EXIT_SCALE).combined(Transition::opacity()))
        .transition_motion(
            Curve::tween(TILE_EXIT_S, Easing::EASE_OUT),
            Curve::tween(TILE_EXIT_S, Easing::EASE_OUT),
        )
        .on_drawn_layout_tx(move |shell: &mut Shell, rect, tx| {
            shell.tray_ui.drawn(id, rect[0], rect[2]);
            tx.live_only();
        })
        .on_drag(move |shell: &mut Shell, drag: &mut Drag, tx| shell.tray_drag(id, drag, tx))
        .on_secondary_press(move |shell: &mut Shell, press, tx| {
            shell.tray_menu(Some(id), Some(press), tx);
        })
        .children(marks(tile, look))
        .child(remove_button(tile, shown.current, look));
    if shown.current {
        // Inside the tile, over the picture: the tile's rounded clip would
        // cut a ring drawn outside it.
        tile_el = tile_el.child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(look.radius(RADIUS_CONTROL))
                .border(FOCUS_RING_PT.0, look.palette.focus_ring)
                .a11y_hidden(),
        );
    }
    if shown.dragged {
        tile_el = tile_el.opacity(DRAGGED_TILE_OPACITY);
    }
    tile_el
}

/// A picture tile: its thumbnail cropped to fill, `surface.hover` while it
/// decodes, a "GIF" badge, and a ring over a scrim while it copies.
fn media_tile(tile: &Tile, look: &Look) -> El {
    let mut face = el().size(TILE_PT, TILE_PT).fill(look.palette.surface_hover);
    let reshaped = tile.edit.changes_pixels();
    match (&tile.thumbnail, tile.size_px()) {
        // Cropped or turned: the tile shows what will be sent.
        (Some(image), Some(size_px)) if reshaped => {
            face = face.child(cut_picture(image, size_px, &tile.edit, TILE_PT));
        }
        (Some(image), _) => face = face.pixels_fit(image.pixels(), SurfaceFit::Fill),
        (None, _) => {}
    }
    if tile.kind == TileKind::Gif {
        face = face.child(
            el().pin(
                Some(TILE_BADGE_INSET_PT),
                None,
                None,
                Some(TILE_BADGE_INSET_PT),
            )
            .child(media_badge(strings::media_gif(), look)),
        );
    }
    if tile.preparing() {
        face = face.child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .fill(Color::BLACK.alpha(PREPARING_SCRIM_ALPHA))
                .child(spinner(Color::WHITE)),
        );
    }
    face
}

/// A file tile: the extension block, the name in two lines and the size
/// (a ring before it while the file copies).
fn file_tile(tile: &Tile, look: &Look) -> El {
    let extension = el()
        .size(EXTENSION_BLOCK_PT, EXTENSION_BLOCK_PT)
        .center()
        .radius(look.radius(EXTENSION_BLOCK_RADIUS))
        .fill(look.palette.surface_code)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .child(
            text(extension_label(&tile.name))
                .font(look.font(TYPE_EXTENSION))
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        );
    let size_line = row().gap(look.space(SPACE_XS)).items_center();
    let size_line = if tile.preparing() {
        size_line.child(spinner(look.palette.text_muted))
    } else {
        size_line.child(
            text(tile.size_bytes.map(strings::size).unwrap_or_default())
                .font(look.font(TYPE_CAPTION).mono())
                .color(look.palette.text_muted),
        )
    };
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space(SPACE_XS))
        .child(
            text(&tile.name)
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(look.palette.text_primary)
                .wrap_anywhere()
                .max_lines(2),
        )
        .child(size_line)
        .a11y_hidden();
    row()
        .h(TILE_PT)
        .min_w(FILE_TILE_WIDTH_PT.0)
        .max_w(FILE_TILE_WIDTH_PT.1)
        .pl(FILE_TILE_INSET_PT.0)
        .pr(FILE_TILE_INSET_PT.1)
        .gap(look.space(SPACE_MD))
        .items_center()
        .fill(look.palette.surface_sidebar)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .hover(look.palette.surface_hover)
        .child(extension)
        .child(words)
}

/// ✕ at a tile's top-right, faded in while the tile is hovered or has
/// the keyboard. Not `danger`: nothing is lost (§14.2).
fn remove_button(tile: &Tile, current: bool, look: &Look) -> El {
    let id = tile.id;
    let button = row()
        .key(format!("tray-remove-{}", tile.id.0))
        .pin(
            None,
            Some(TILE_REMOVE_INSET_PT),
            Some(TILE_REMOVE_INSET_PT),
            None,
        )
        .size(TILE_REMOVE_PT, TILE_REMOVE_PT)
        .center()
        .radius(TILE_REMOVE_PT / 2.0)
        .fill(look.palette.surface_pill)
        .border(PILL_OUTLINE_PT, look.palette.line_pill)
        .hover(look.palette.surface_hover)
        .role("button")
        .label(strings::tray_remove(&tile.name))
        .on_press_tx(move |shell: &mut Shell, tx| {
            shell.remove_tile(id, tx);
        })
        .child(icon(&X, TILE_REMOVE_ICON_PT).color(look.palette.text_secondary));
    if current {
        button
    } else {
        button.group_opacity(0.0, 1.0)
    }
}

/// The marks of what a tile's file will go as, always shown at its top
/// left: anonymized, and a random name, side by side.
fn marks(tile: &Tile, look: &Look) -> Option<El> {
    let (anonymized, renamed) = (tile.anonymized(), tile.renamed());
    if !anonymized && !renamed {
        return None;
    }
    let (side, ink) = TILE_MARK_PT;
    let mark = |shape: &'static rune::Icon| {
        el().size(side, side)
            .center()
            .radius(side / 2.0)
            .fill(look.palette.surface_pill)
            .border(PILL_OUTLINE_PT, look.palette.line_pill)
            .child(icon(shape, ink).color(look.palette.text_secondary))
    };
    let mut line = row()
        .pin(
            Some(TILE_REMOVE_INSET_PT),
            Some(TILE_REMOVE_INSET_PT),
            None,
            None,
        )
        .gap(TILE_REMOVE_INSET_PT / 2.0)
        .a11y_hidden();
    if anonymized {
        line = line.child(mark(&ANONYMOUS));
    }
    if renamed {
        line = line.child(mark(&RENAMED));
    }
    Some(line)
}

/// A media badge ("GIF"): white mono on black at 60 %, as the timeline's.
fn media_badge(words: &str, look: &Look) -> El {
    text(words)
        .font(look.font(TYPE_MEDIA_BADGE))
        .color(Color::WHITE)
        .px(MEDIA_BADGE_PAD_PT.0)
        .py(MEDIA_BADGE_PAD_PT.1)
        .radius(look.radius(MEDIA_BADGE_RADIUS))
        .fill(Color::BLACK.alpha(MEDIA_BADGE_ALPHA))
        .a11y_hidden()
}

/// The small turning ring of a file still copying.
fn spinner(colour: Color) -> El {
    icon(&SPINNER, PREPARING_RING_PT)
        .color(colour)
        .looping(LoopSpec::rotation(SPINNER_PERIOD_S))
        .a11y_hidden()
}

/// The 2 pt accent line where a dragged tile would land.
fn slot_line(look: &Look) -> El {
    el().w(DROP_SLOT_PT)
        .h(TILE_PT)
        .radius(DROP_SLOT_PT / 2.0)
        .fill(look.palette.accent)
        .a11y_hidden()
}

/// What assistive technology hears for a tile: "Image, praia.jpg, 2.4 MB",
/// or "Preparing praia.jpg" while it copies.
fn spoken(tile: &Tile) -> String {
    if tile.preparing() {
        return strings::tray_preparing(&tile.name);
    }
    let size = tile.size_bytes.map(strings::size).unwrap_or_default();
    let mut spoken = match tile.kind {
        TileKind::Image => strings::tray_tile_image(&tile.name, &size),
        TileKind::Gif => strings::tray_tile_gif(&tile.name, &size),
        TileKind::File => strings::tray_tile_file(&tile.name, &size),
    };
    for (on, mark) in [
        (tile.anonymized(), strings::tray_anonymous()),
        (tile.renamed(), strings::tray_renamed()),
    ] {
        if on {
            spoken.push_str(", ");
            spoken.push_str(mark);
        }
    }
    spoken
}
