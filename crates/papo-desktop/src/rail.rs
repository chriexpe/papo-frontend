//! Draws the server rail (`docs/design.md` §3): a tile per saved server in
//! the order shown, the bar beside the selected one, and + at the end, which
//! opens the add-server popup. Pure: reads the navigation state.
//!
//! Owns the rail's look and what the keyboard, the pointer and assistive
//! technology reach on it: arrows move between tiles, Alt+↑ and Alt+↓ move
//! the focused server, a drag moves it too, and the menu key or a secondary
//! click opens its menu. An unread server gets a short bar where the
//! selected bar goes, and mentions a red badge on the tile's corner.

use std::collections::HashMap;

use papo_sdk::ServerId;
use rune::{Drag, El, Image, Key, KeyEvent, ScrollSpec, col, el, icon, row, text};

use crate::badges::mention_pill;
use crate::design::{
    ADD_TILE_DASH_PT, ADD_TILE_OUTLINE_PT, FOCUS_RING_PT, MENTION_PILL_OVERHANG_PT,
    MENTION_PILL_RING_PT, PILL_OUTLINE_PT, Palette, RADIUS_ICON_PT, RADIUS_INDICATOR_PT,
    RAIL_ICON_PT, RAIL_INDICATOR_PT, RAIL_TILE_PT, RAIL_UNREAD_BAR_PT, RAIL_WIDTH_PT, SPACE_MD_PT,
    TYPE_TITLE,
};
use crate::icons::PLUS;
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
pub(crate) fn view(nav: &Navigation, icons: &HashMap<ServerId, Image>, palette: &Palette) -> El {
    let selected = nav.selected().map(|server| server.id);
    let tiles = nav.shown().into_iter().map(|server| {
        let tile = Tile {
            server,
            selected: selected == Some(server.id),
            icon: icons.get(&server.id),
            offset_pt: nav.drag_offset(server.id),
        };
        tile.view(palette)
    });
    let servers = col()
        .key("rail-servers")
        .w_full()
        .items_center()
        .gap(SPACE_MD_PT)
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
        .w(RAIL_WIDTH_PT)
        .h_full()
        .items_center()
        .gap(SPACE_MD_PT)
        .py(SPACE_MD_PT)
        .fill(palette.surface_rail)
        .child(servers)
        .child(add_tile(palette))
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
    fn view(&self, palette: &Palette) -> El {
        let id = self.server.id;
        let face = match self.icon {
            Some(image) => el()
                .size(RAIL_TILE_PT, RAIL_TILE_PT)
                .radius(RADIUS_ICON_PT)
                .pixels(image.pixels()),
            None => el()
                .size(RAIL_TILE_PT, RAIL_TILE_PT)
                .center()
                .radius(RADIUS_ICON_PT)
                .fill(palette.surface_pill)
                .border(PILL_OUTLINE_PT, palette.line_pill)
                .child(
                    text(initial(&self.server.name))
                        .font(TYPE_TITLE)
                        .color(palette.text_secondary)
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
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
            .on_press_tx(move |shell: &mut Shell, tx| shell.select_server(id, tx))
            .on_drag(move |shell: &mut Shell, drag: &mut Drag, tx| shell.drag_server(id, drag, tx))
            .on_key(move |shell: &mut Shell, key: &KeyEvent, _tx| move_key(shell, id, key))
            .on_secondary_press(move |shell: &mut Shell, _press, _tx| {
                shell.open_server_menu_beside(id);
            });
        // Pinned inside the tile's border, so a letter tile's 1 pt outline
        // is added back: every badge overhangs the tile's corner alike.
        let badge_overhang = if self.icon.is_some() {
            MENTION_PILL_OVERHANG_PT
        } else {
            MENTION_PILL_OVERHANG_PT + PILL_OUTLINE_PT
        };
        let face = match self.server.badge.mentions() {
            Some(count) => face.child(
                el().pin(None, None, Some(-badge_overhang), Some(-badge_overhang))
                    .child(mention_pill(count, Some(palette.surface_rail), palette)),
            ),
            None => face,
        };
        // Keyed by server, so a reorder moves the whole place and a held
        // press on its tile survives the preview.
        let mut place = row()
            .key(format!("rail-place-{id}"))
            .w(RAIL_WIDTH_PT)
            .h(RAIL_TILE_PT)
            .center()
            .offset(0.0, self.offset_pt);
        // The bar beside the tile: tall when selected, short when unread.
        let bar = if self.selected {
            Some(RAIL_INDICATOR_PT)
        } else if self.server.badge.is_unread() {
            Some(RAIL_UNREAD_BAR_PT)
        } else {
            None
        };
        if let Some((width, height)) = bar {
            place = place.child(
                el().absolute(0.0, (RAIL_TILE_PT - height) / 2.0)
                    .size(width, height)
                    .radius(RADIUS_INDICATOR_PT)
                    .fill(palette.text_primary)
                    .a11y_hidden(),
            );
        }
        place.child(face)
    }
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
fn add_tile(palette: &Palette) -> El {
    el().key(ADD_KEY)
        .size(RAIL_TILE_PT, RAIL_TILE_PT)
        .center()
        .radius(RADIUS_ICON_PT)
        .border(ADD_TILE_OUTLINE_PT, palette.line_strong)
        .border_dash(ADD_TILE_DASH_PT.0, ADD_TILE_DASH_PT.1)
        .hover(palette.surface_hover)
        .role("button")
        .label(strings::rail_add_server())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(Shell::add_server)
        .child(icon(&PLUS, RAIL_ICON_PT).color(palette.text_muted))
}

/// A server without an icon: the first letter of its name.
pub(crate) fn initial(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect()
}
