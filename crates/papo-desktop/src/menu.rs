//! The app's menus, drawn in the window on every platform: one structure for
//! a menu grown from its pill (`docs/design.md` §1: the same width, from the
//! same edge) and for a context menu beside a tile or row.
//!
//! Owns what an open menu holds ([`OpenMenu`]), how it is drawn and how the
//! keyboard and assistive technology reach it. One menu is open at a time,
//! in the shell's slot; picking an item hands its [`MenuAction`] to the
//! shell, which carries it out. What each menu lists is the view that opens
//! it.

use papo_sdk::{ServerId, Uuid};
use rune::{Align, Cx, El, Side, col, el, text};

use crate::design::{
    CONTEXT_MENU_WIDTH_PT, MENU_ROW_HEIGHT_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT,
    RADIUS_PILL_PT, SPACE_LG_PT, SPACE_SM_PT, TYPE_BODY,
};
use crate::shell::Shell;
use crate::timeline::TimelineKey;

/// What picking an item does; the shell carries it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    /// Ask to remove this server.
    RemoveServer(ServerId),
    /// Send your unsent message `local` again; `unknown`: it may have
    /// arrived, so ask first.
    RetrySend {
        key: TimelineKey,
        local: Uuid,
        unknown: bool,
    },
    /// Drop your unsent message `local`.
    DiscardSend { key: TimelineKey, local: Uuid },
    /// Send your failed edit `local` again.
    RetryChange { key: TimelineKey, local: Uuid },
    /// Drop your failed edit `local`: the confirmed text stays.
    DiscardChange { key: TimelineKey, local: Uuid },
}

/// Where an open menu sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Grown from the pill with this key: below it, as wide as it.
    GrowFrom(&'static str),
    /// A context menu beside the element with this key.
    Beside(String),
    /// Grown down from the small control with this key, from its left edge
    /// (the ⚠ of an unsent message).
    Below(String),
}

/// One item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MenuItem {
    /// The item's element key, unique within the menu.
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    pub(crate) action: MenuAction,
    /// Destroys something: drawn in the danger colour.
    pub(crate) danger: bool,
}

/// The open menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenMenu {
    pub(crate) placement: Placement,
    /// The key of the control that opened it, which gets the keyboard back
    /// when an item is picked.
    pub(crate) trigger: String,
    pub(crate) items: Vec<MenuItem>,
}

/// The element key of the open menu.
pub(crate) const MENU_KEY: &str = "menu";

impl OpenMenu {
    /// A menu with `items` at `placement`, opened by the control `trigger`.
    pub(crate) fn new(placement: Placement, trigger: String, items: Vec<MenuItem>) -> OpenMenu {
        OpenMenu {
            placement,
            trigger,
            items,
        }
    }
}

/// Draws `menu` over everything, anchored by its placement. Escape and a
/// click outside close it; Return or a click picks an item.
pub(crate) fn view(menu: &OpenMenu, palette: &Palette, cx: &Cx<Shell>) -> El {
    let (anchor, side, gap, width) = match &menu.placement {
        Placement::GrowFrom(key) => {
            let width = cx
                .layout_size(key)
                .map_or(CONTEXT_MENU_WIDTH_PT, |[w, _]| w);
            ((*key).to_owned(), Side::Bottom, SPACE_SM_PT, width)
        }
        Placement::Beside(key) => (key.clone(), Side::Right, SPACE_SM_PT, CONTEXT_MENU_WIDTH_PT),
        Placement::Below(key) => (
            key.clone(),
            Side::Bottom,
            SPACE_SM_PT,
            CONTEXT_MENU_WIDTH_PT,
        ),
    };
    let rows = menu
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| row_for(item, index == 0, palette));
    col()
        .key(MENU_KEY)
        .w(width)
        .padding(SPACE_SM_PT)
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role("menu")
        .focus_group()
        .modal(true, Shell::close_menu)
        .anchored(anchor, side, Align::Start, gap)
        .children(rows)
}

fn row_for(item: &MenuItem, first: bool, palette: &Palette) -> El {
    let action = item.action;
    let color = if item.danger {
        palette.danger
    } else {
        palette.text_primary
    };
    el().key(item.key)
        .w_full()
        .h(MENU_ROW_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .role("menuitem")
        .label(item.label)
        .focusable(true)
        .focus_selected(first)
        // One highlight for the pointer and the keyboard, as a menu has.
        .focus_highlight(palette.surface_hover)
        .on_press_tx(move |shell: &mut Shell, tx| shell.menu_picked(action, tx))
        .child(text(item.label).font(TYPE_BODY).color(color))
}
