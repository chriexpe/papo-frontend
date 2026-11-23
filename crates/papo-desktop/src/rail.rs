//! Draws the server rail (`docs/design.md` §3): Direct at the top, a tile
//! per saved server in the order shown, the bar beside the selected one,
//! and + at the end, which opens the add-server popup. Pure: reads the
//! navigation state.
//!
//! Owns the rail's look and what the keyboard, the pointer and assistive
//! technology reach on it: arrows move between tiles, Alt+↑ and Alt+↓ move
//! the focused server, a drag moves it too, and the menu key or a secondary
//! click opens its menu. An unread server gets a short bar where the
//! selected bar goes, and mentions a red badge on the tile's corner.

use std::collections::HashMap;

use papo_sdk::ServerId;
use rune::{Drag, El, Image, Key, KeyEvent, ScrollSpec, col, el, icon, row, text};

use crate::badges::Badge;
use crate::badges::mention_pill;
use crate::design::{
    ADD_TILE_DASH_PT, ADD_TILE_OUTLINE_PT, FOCUS_RING_PT, Look, MENTION_PILL_OVERHANG_PT,
    MENTION_PILL_RING_PT, PILL_OUTLINE_PT, RADIUS_ICON, RADIUS_INDICATOR, RAIL_GAP_PT,
    RAIL_ICON_PT, RAIL_INDICATOR_PT, RAIL_TILE_PT, RAIL_UNREAD_BAR_PT, RAIL_WIDTH_PT, TYPE_TITLE,
};
use crate::icons::{DIRECT, PLUS};
use crate::navigation::Navigation;
use crate::server_list::ServerEntry;
use crate::shell::Shell;
use crate::strings;

/// The element key of a server's tile.
pub(crate) fn tile_key(server: ServerId) -> String {
    format!("server-{server}")
}

/// The element key of the rail's +.
pub(crate) const ADD_KEY: &str = "add-server";

/// The rail: the servers, then +.
pub(crate) fn view(nav: &Navigation, icons: &HashMap<ServerId, Image>, look: &Look) -> El {
    let selected = nav.selected().map(|server| server.id);
    let tiles = nav.shown().into_iter().map(|server| {
        let tile = Tile {
            server,
            selected: selected == Some(server.id),
            icon: icons.get(&server.id),
            offset_pt: nav.drag_offset(server.id),
        };
        tile.view(look)
    });
    let servers = col()
        .key("rail-servers")
        .w_full()
        .items_center()
        .gap(RAIL_GAP_PT)
        // Room for the last tile's badge, which overhangs its corner.
        .pb(MENTION_PILL_OVERHANG_PT + MENTION_PILL_RING_PT)
        .role("tablist")
        .label(strings::rail_label())
        .focus_group()
        .children(tiles);
    let servers = col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        // A scroll here is work outside a header panel; it scrolls as usual.
        .on_wheel(|shell: &mut Shell, _wheel, _tx| {
            shell.worked_outside();
            false
        })
        .child(servers);
    col()
        .key("rail")
        // A landmark, so F6 reaches it (`docs/design.md` §12).
        .role("navigation")
        .label(strings::rail_label())
        .w(RAIL_WIDTH_PT)
        .h_full()
        .items_center()
        .gap(RAIL_GAP_PT)
        .py(RAIL_GAP_PT)
        .fill(look.palette.surface_rail)
        .child(direct_tile(nav.direct_shown(), nav.servers().direct, look))
        .child(servers)
        .child(add_tile(look))
}

/// One server's place in the rail.
struct Tile<'a> {
    server: &'a ServerEntry,
    selected: bool,
    icon: Option<&'a Image>,
    /// How far a drag draws it from its place, in points.
    offset_pt: f32,
}

impl Tile<'_> {
    fn view(&self, look: &Look) -> El {
        let id = self.server.id;
        let face = match self.icon {
            Some(image) => el()
                .size(RAIL_TILE_PT, RAIL_TILE_PT)
                .radius(look.radius(RADIUS_ICON))
                .pixels(image.pixels()),
            None => el()
                .size(RAIL_TILE_PT, RAIL_TILE_PT)
                .center()
                .radius(look.radius(RADIUS_ICON))
                .fill(look.palette.surface_pill)
                .border(PILL_OUTLINE_PT, look.palette.line_pill)
                .child(
                    text(initial(&self.server.name))
                        .font(look.font(TYPE_TITLE))
                        .color(look.palette.text_secondary)
                        .a11y_hidden(),
                ),
        };
        let face = face
            .key(tile_key(id))
            .role("tab")
            .label(self.server.badge.label(&self.server.name))
            .description(strings::rail_move_hint())
            .focusable(true)
            .focus_selected(self.selected)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |shell: &mut Shell, tx| shell.select_server(id, tx))
            .on_drag(move |shell: &mut Shell, drag: &mut Drag, tx| shell.drag_server(id, drag, tx))
            .on_key(move |shell: &mut Shell, key: &KeyEvent, _tx| move_key(shell, id, key))
            .on_secondary_press(move |shell: &mut Shell, _press, _tx| {
                shell.open_server_menu_beside(id);
            });
        let face = with_mentions(face, self.server.badge, self.icon.is_none(), look);
        // Keyed by server, so a reorder moves the whole place and a held
        // press on its tile survives the preview.
        place(
            format!("rail-place-{id}"),
            self.selected,
            self.server.badge,
            look,
        )
        .offset(0.0, self.offset_pt)
        .child(face)
    }
}

/// `face` with `badge`'s mention count on its corner. Pinned inside the
/// tile's border, so an `outlined` tile's 1 pt outline is added back: every
/// badge overhangs the tile's corner alike.
fn with_mentions(face: El, badge: Badge, outlined: bool, look: &Look) -> El {
    let overhang_pt = if outlined {
        MENTION_PILL_OVERHANG_PT + PILL_OUTLINE_PT
    } else {
        MENTION_PILL_OVERHANG_PT
    };
    match badge.mentions() {
        Some(count) => face.child(
            el().pin(None, None, Some(-overhang_pt), Some(-overhang_pt))
                .child(mention_pill(count, Some(look.palette.surface_rail), look)),
        ),
        None => face,
    }
}

/// A tile's place in the rail, with the bar beside the tile: tall when
/// selected, short when `badge` is unread.
fn place(key: String, selected: bool, badge: Badge, look: &Look) -> El {
    let place = row().key(key).w(RAIL_WIDTH_PT).h(RAIL_TILE_PT).center();
    let bar = if selected {
        Some(RAIL_INDICATOR_PT)
    } else if badge.is_unread() {
        Some(RAIL_UNREAD_BAR_PT)
    } else {
        None
    };
    match bar {
        Some((width, height)) => place.child(
            el().absolute(0.0, (RAIL_TILE_PT - height) / 2.0)
                .size(width, height)
                .radius(look.radius(RADIUS_INDICATOR))
                .fill(look.palette.text_primary)
                .a11y_hidden(),
        ),
        None => place,
    }
}

/// The element key of the rail's Direct.
pub(crate) const DIRECT_KEY: &str = "rail-direct";

/// The rail's Direct: the direct conversations, in the channel pane.
fn direct_tile(shown: bool, badge: Badge, look: &Look) -> El {
    let face = el()
        .key(DIRECT_KEY)
        .size(RAIL_TILE_PT, RAIL_TILE_PT)
        .center()
        .radius(look.radius(RADIUS_ICON))
        .fill(look.palette.surface_pill)
        .border(PILL_OUTLINE_PT, look.palette.line_pill)
        .role("tab")
        .label(badge.label(strings::direct_title()))
        .focusable(true)
        .focus_selected(shown)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(|shell: &mut Shell, tx| shell.select_direct(tx))
        .child(icon(&DIRECT, RAIL_ICON_PT).color(look.palette.text_secondary));
    let face = with_mentions(face, badge, true, look);
    place("rail-place-direct".to_owned(), shown, badge, look).child(face)
}

/// Alt+↑ and Alt+↓ move the focused server; other keys pass on.
fn move_key(shell: &mut Shell, server: ServerId, key: &KeyEvent) -> bool {
    if !key.mods.alt {
        return false;
    }
    let by = match key.key {
        Key::Up => -1,
        Key::Down => 1,
        _ => return false,
    };
    shell.nudge_server(server, by);
    true
}

/// The rail's +: the add-server popup.
fn add_tile(look: &Look) -> El {
    el().key(ADD_KEY)
        .size(RAIL_TILE_PT, RAIL_TILE_PT)
        .center()
        .radius(look.radius(RADIUS_ICON))
        .border(ADD_TILE_OUTLINE_PT, look.palette.line_strong)
        .border_dash(ADD_TILE_DASH_PT.0, ADD_TILE_DASH_PT.1)
        .hover(look.palette.surface_hover)
        .role("button")
        .label(strings::rail_add_server())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::add_server)
        .child(icon(&PLUS, RAIL_ICON_PT).color(look.palette.text_muted))
}

/// A server without an icon: the first letter of its name.
pub(crate) fn initial(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect()
}
