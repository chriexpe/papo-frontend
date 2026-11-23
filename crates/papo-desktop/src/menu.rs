//! The app's menus, drawn in the window on every platform: one structure for
//! a menu grown from its pill (`docs/design.md` §1: the same width, from the
//! same edge), a context menu beside a tile, a row or the pointer, and the
//! message menu (§10): a header, the quick-reaction strip, items with an
//! icon, a sub-label and their shortcut, in groups split by dividers.
//!
//! Owns what an open menu holds ([`OpenMenu`]), how it is drawn and how the
//! keyboard and assistive technology reach it (↑ ↓, Home and End, a letter,
//! Enter or Space; Escape, Tab or a click outside closes it and the trigger
//! gets the keyboard back). One menu is open at a time, in the shell's slot;
//! picking an item hands its [`MenuAction`] to the shell, which carries it
//! out. What each menu lists is the view that opens it.

use papo_sdk::{ServerId, Uuid};
use rune::{Align, Cx, El, Icon, Side, col, el, icon, row, text};

use crate::design::{
    CONTEXT_MENU_WIDTH_PT, MENU_ROW_HEIGHT_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT,
    RADIUS_PILL_PT, ROW_ICON_PT, SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_CAPTION,
    TYPE_HINT,
};
use crate::shell::Shell;
use crate::timeline::TimelineKey;

pub(crate) mod strip;

pub(crate) use strip::Strip;

/// What picking an item does; the shell carries it out.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Something done to message `id` of timeline `key`.
    Message {
        key: TimelineKey,
        id: Uuid,
        action: MessageAction,
    },
    /// Open `url`, as a click on it would.
    OpenLink { key: TimelineKey, url: String },
    /// Copy `url`.
    CopyLink(String),
    /// Toggle your reaction `emoji` to message `id`.
    React {
        key: TimelineKey,
        id: Uuid,
        emoji: String,
    },
}

/// What the message menu does to its message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MessageAction {
    Reply,
    Edit,
    Pin,
    Unpin,
    CopyText,
    CopyMarkdown,
    CopyId,
    /// The menu turns into the reactions list.
    ViewReactions,
    /// Ask to delete it.
    Delete,
}

/// The reactions list a menu shows in place of its entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactionsView {
    pub(crate) key: TimelineKey,
    pub(crate) id: Uuid,
    /// The tab: one emoji's people, or everyone's.
    pub(crate) filter: Option<papo_sdk_ui::operations::ReactionEmoji>,
    /// Opened from a chip, not the menu: Escape closes it.
    pub(crate) from_chip: bool,
}

/// Where an open menu sits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Placement {
    /// Grown from the pill with this key: below it, as wide as it.
    GrowFrom(&'static str),
    /// A context menu beside the element with this key.
    Beside(String),
    /// Grown down from the small control with this key, from its left edge
    /// (the ⚠ of an unsent message).
    Below(String),
    /// Grown down from the control with this key, from its right edge (the
    /// hover bar's More).
    BelowEnd(String),
    /// Beside the pointer, at this point in window points.
    AtPointer([f32; 2]),
}

/// One item.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MenuItem {
    /// The item's element key, unique within the menu.
    pub(crate) key: String,
    pub(crate) label: String,
    /// A second line under the label (a link's host).
    pub(crate) sub: Option<String>,
    pub(crate) icon: Option<MenuIcon>,
    /// Its shortcut, drawn at the row's end.
    pub(crate) shortcut: Option<&'static str>,
    pub(crate) action: MenuAction,
    /// Destroys something: drawn in the danger colour.
    pub(crate) danger: bool,
}

impl MenuItem {
    /// An item that is only a label.
    pub(crate) fn plain(
        key: &str,
        label: impl Into<String>,
        action: MenuAction,
        danger: bool,
    ) -> MenuItem {
        MenuItem {
            key: key.to_owned(),
            label: label.into(),
            sub: None,
            icon: None,
            shortcut: None,
            action,
            danger,
        }
    }

    /// The item with `shape` before its label.
    pub(crate) fn with_icon(mut self, shape: &'static Icon) -> MenuItem {
        self.icon = Some(MenuIcon(shape));
        self
    }

    /// The item with its shortcut at the row's end.
    pub(crate) fn with_shortcut(mut self, shortcut: &'static str) -> MenuItem {
        self.shortcut = Some(shortcut);
        self
    }

    /// The item with a second line under its label.
    pub(crate) fn with_sub(mut self, sub: Option<String>) -> MenuItem {
        self.sub = sub;
        self
    }
}

/// An item's icon, compared by identity.
#[derive(Clone, Copy)]
pub(crate) struct MenuIcon(pub(crate) &'static Icon);

impl PartialEq for MenuIcon {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl std::fmt::Debug for MenuIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.name)
    }
}

/// One line of a menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Entry {
    /// A line of text that is not an item (the message menu's header).
    Header(String),
    /// The quick-reaction strip.
    Strip(Strip),
    Item(MenuItem),
    /// A hairline between groups.
    Divider,
}

/// The open menu.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenMenu {
    pub(crate) placement: Placement,
    /// The key of the control that opened it, which gets the keyboard back
    /// when an item is picked.
    pub(crate) trigger: String,
    pub(crate) entries: Vec<Entry>,
    /// Its width, in points.
    pub(crate) width_pt: f32,
    /// The reactions list it turned into, while it shows one.
    pub(crate) reactions: Option<ReactionsView>,
}

/// The element key of the open menu.
pub(crate) const MENU_KEY: &str = "menu";
/// The element key of the point a menu beside the pointer is anchored to.
const POINTER_KEY: &str = "menu-pointer";

impl OpenMenu {
    /// A menu with `items` at `placement`, opened by the control `trigger`.
    pub(crate) fn new(placement: Placement, trigger: String, items: Vec<MenuItem>) -> OpenMenu {
        OpenMenu::with_entries(
            placement,
            trigger,
            items.into_iter().map(Entry::Item).collect(),
            CONTEXT_MENU_WIDTH_PT,
        )
    }

    /// A menu of `entries`, `width_pt` wide.
    pub(crate) fn with_entries(
        placement: Placement,
        trigger: String,
        entries: Vec<Entry>,
        width_pt: f32,
    ) -> OpenMenu {
        OpenMenu {
            placement,
            trigger,
            entries,
            width_pt,
            reactions: None,
        }
    }

    /// The strip, when the menu has one.
    pub(crate) fn strip_mut(&mut self) -> Option<&mut Strip> {
        self.entries.iter_mut().find_map(|entry| match entry {
            Entry::Strip(strip) => Some(strip),
            _ => None,
        })
    }
}

/// The reactions list's width, in points (`docs/design.md` §10).
pub(crate) const REACTIONS_WIDTH_PT: f32 = 340.0;

/// Draws `menu` over everything, anchored by its placement: its entries,
/// or `body` in their place (the reactions list). Escape and a click
/// outside close it; Return or a click picks an item.
pub(crate) fn view(menu: &OpenMenu, body: Option<El>, palette: &Palette, cx: &Cx<Shell>) -> El {
    let (anchor, side, align, width) = match &menu.placement {
        Placement::GrowFrom(key) => {
            let width = cx.layout_size(key).map_or(menu.width_pt, |[w, _]| w);
            ((*key).to_owned(), Side::Bottom, Align::Start, width)
        }
        Placement::Beside(key) => (key.clone(), Side::Right, Align::Start, menu.width_pt),
        Placement::Below(key) => (key.clone(), Side::Bottom, Align::Start, menu.width_pt),
        Placement::BelowEnd(key) => (key.clone(), Side::Bottom, Align::End, menu.width_pt),
        Placement::AtPointer(_) => (
            POINTER_KEY.to_owned(),
            Side::Right,
            Align::Start,
            menu.width_pt,
        ),
    };
    let first = menu
        .entries
        .iter()
        .position(|entry| matches!(entry, Entry::Strip(_) | Entry::Item(_)));
    let lines = menu
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| line(entry, Some(index) == first, palette));
    let (width, role) = if body.is_some() {
        (REACTIONS_WIDTH_PT, "dialog")
    } else {
        (width, "menu")
    };
    let content: Vec<El> = match body {
        Some(body) => vec![body],
        None => lines.collect(),
    };
    col()
        .key(MENU_KEY)
        .w(width)
        .padding(SPACE_SM_PT)
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role(role)
        .focus_group()
        .modal(true, Shell::close_menu)
        .on_key(|shell: &mut Shell, event: &rune::KeyEvent, tx| shell.menu_escape(event, tx))
        .anchored_fit(anchor, side, align, SPACE_SM_PT)
        .children(content)
}

/// The zero-size point a menu beside the pointer hangs from, while one is
/// open there.
pub(crate) fn pointer_point(menu: &OpenMenu) -> Option<El> {
    let Placement::AtPointer([x, y]) = menu.placement else {
        return None;
    };
    Some(
        el().key(POINTER_KEY)
            .pin(Some(x), Some(y), None, None)
            .size(0.0, 0.0)
            .a11y_hidden(),
    )
}

fn line(entry: &Entry, first: bool, palette: &Palette) -> El {
    match entry {
        Entry::Header(words) => el().w_full().px(SPACE_LG_PT).py(SPACE_MD_PT).child(
            text(words.clone())
                .font(TYPE_CAPTION)
                .color(palette.text_muted),
        ),
        Entry::Strip(strip) => strip::view(strip, first, palette),
        Entry::Item(item) => item_row(item, first, palette),
        Entry::Divider => el()
            .w_full()
            .my(SPACE_SM_PT)
            .h(PILL_OUTLINE_PT)
            .fill(palette.line_subtle)
            .a11y_hidden(),
    }
}

fn item_row(item: &MenuItem, first: bool, palette: &Palette) -> El {
    let action = item.action.clone();
    let color = if item.danger {
        palette.danger
    } else {
        palette.text_primary
    };
    let mut words = col().grow(1.0).min_w(0.0).child(
        text(item.label.clone())
            .font(TYPE_BODY)
            .color(color)
            .max_lines(1),
    );
    if let Some(sub) = &item.sub {
        words = words.child(
            text(sub.clone())
                .font(TYPE_CAPTION)
                .color(palette.text_muted)
                .max_lines(1),
        );
    }
    let label = match &item.sub {
        Some(sub) => format!("{}, {sub}", item.label),
        None => item.label.clone(),
    };
    let mut row = row()
        .key(item.key.clone())
        .w_full()
        .min_h(MENU_ROW_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .role("menuitem")
        .label(label)
        .focusable(true)
        .focus_selected(first)
        // One highlight for the pointer and the keyboard, as a menu has.
        .focus_highlight(palette.surface_hover)
        .on_press_tx(move |shell: &mut Shell, tx| shell.menu_picked(action.clone(), tx));
    if let Some(MenuIcon(shape)) = item.icon {
        let tone = if item.danger {
            palette.danger
        } else {
            palette.text_secondary
        };
        row = row.child(icon(shape, ROW_ICON_PT).color(tone));
    }
    row = row.child(words);
    if let Some(shortcut) = item.shortcut {
        row = row.child(text(shortcut).font(TYPE_HINT).color(palette.text_muted));
    }
    row
}
