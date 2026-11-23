//! The reactions list (`docs/design.md` §10, Reactions list): View
//! reactions turns the message menu into it in place, and a secondary
//! click on a chip opens it beside the chip on that emoji. "← Reactions"
//! on top, then the tabs (All, then each emoji with its count, in the
//! chips' order), then 36 px rows: the avatar, the name, "you" by yours,
//! and on All every emoji each person used at the row's end. You first,
//! then in the order people reacted. Pages are read as the end of the
//! list comes near, with skeleton rows meanwhile; counts are the chips'.
//! Escape goes back to the menu, or closes the list opened from a chip.
//!
//! A row is not pressable: the person's card comes with D5b.

use std::borrow::Cow;

use papo_sdk::Uuid;
use papo_sdk_ui::operations::ReactionEmoji;
use papo_sdk_ui::{Reactor, TimelineItem, display_name, reaction_chips};
use rune::{El, GrowthSpec, Key, KeyEvent, ScrollSpec, Tx, col, el, icon, row, text};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::design::{
    FOCUS_RING_PT, PILL_AVATAR_PT, PILL_BUTTON_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT,
    ROW_ICON_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_CAPTION, TYPE_SMALL, TYPE_SMALL_MEDIUM,
    TYPE_TITLE,
};
use crate::menu::{OpenMenu, Placement, ReactionsView};
use crate::strings;
use crate::timeline::reactions::chip_key;
use crate::timeline::{TimelineKey, avatar};

/// A row's height, in points.
const ROW_PT: f32 = 36.0;
/// The tallest the list grows, in points, before it scrolls.
const LIST_MAX_PT: f32 = 320.0;
/// Skeleton rows while a page is on its way.
const SKELETON_ROWS: usize = 2;
/// How near the list's end, in rows, the next page is read.
const NEAR_END_ROWS: usize = 4;
/// The element key of the back button.
const BACK_KEY: &str = "reactions-back";

impl Shell {
    /// View reactions in the message menu: the menu turns into the list,
    /// on All, and its first page is read.
    pub(super) fn show_reactions(&mut self, key: TimelineKey, id: Uuid, tx: &mut Tx) {
        let Some(menu) = &mut self.nav.menu else {
            return;
        };
        menu.reactions = Some(ReactionsView {
            key,
            id,
            filter: None,
            from_chip: false,
        });
        tx.focus(tab_key(None), true);
        self.load_reactors(key, id, None);
    }

    /// A secondary click on message `id`'s chip for `emoji`: the list
    /// opens beside the chip, on that emoji.
    pub(super) fn open_chip_reactions(&mut self, key: TimelineKey, id: Uuid, emoji: &str) {
        let filter = Some(ReactionEmoji::Unicode(emoji.to_owned()));
        let mut menu = OpenMenu::with_entries(
            Placement::Below(chip_key(id, emoji)),
            COMPOSER_KEY.to_owned(),
            Vec::new(),
            crate::menu::REACTIONS_WIDTH_PT,
        );
        menu.reactions = Some(ReactionsView {
            key,
            id,
            filter: filter.clone(),
            from_chip: true,
        });
        self.nav.close_accounts();
        self.nav.menu = Some(menu);
        self.load_reactors(key, id, filter.as_ref());
    }

    /// A tab: the list shows `filter`'s people, read further if needed.
    fn reactions_tab(&mut self, filter: Option<ReactionEmoji>) {
        let Some(view) = self
            .nav
            .menu
            .as_mut()
            .and_then(|menu| menu.reactions.as_mut())
        else {
            return;
        };
        view.filter = filter;
        let (key, id, filter) = (view.key, view.id, view.filter.clone());
        self.load_reactors(key, id, filter.as_ref());
    }

    /// ← on the list, or Escape: back to the menu, or closed when it was
    /// opened from a chip.
    fn reactions_back(&mut self, tx: &mut Tx) {
        let Some(menu) = &mut self.nav.menu else {
            return;
        };
        match &menu.reactions {
            Some(view) if !view.from_chip => {
                menu.reactions = None;
                tx.focus("menu-view-reactions", true);
            }
            _ => self.close_menu(),
        }
    }

    /// Escape on the open menu: the reactions list goes back first.
    /// Returns whether the key was taken.
    pub(crate) fn menu_escape(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        let showing = self
            .nav
            .menu
            .as_ref()
            .is_some_and(|menu| menu.reactions.is_some());
        if !showing || event.key != Key::Escape {
            return false;
        }
        self.reactions_back(tx);
        true
    }

    /// The list's rows `range` are visible: near the end, the next page is
    /// read. Returns whether the view changed.
    fn reactions_visible(&mut self, range: std::ops::Range<usize>, count: usize) -> bool {
        if range.end + NEAR_END_ROWS < count {
            return false;
        }
        let Some(view) = self
            .nav
            .menu
            .as_ref()
            .and_then(|menu| menu.reactions.clone())
        else {
            return false;
        };
        self.load_reactors(view.key, view.id, view.filter.as_ref());
        true
    }
}

/// The element key of the tab for `filter` (All when none).
fn tab_key(filter: Option<&ReactionEmoji>) -> String {
    match filter {
        Some(ReactionEmoji::Unicode(emoji)) => format!("reactions-tab-{emoji}"),
        _ => "reactions-tab-all".to_owned(),
    }
}

/// The list the open menu shows, when it shows one.
pub(super) fn body(shell: &Shell, palette: &Palette) -> Option<El> {
    let view = shell.nav.menu.as_ref()?.reactions.clone()?;
    let open = shell.tabs.shown().filter(|open| open.key == view.key)?;
    let message = open.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message { id, message, .. } if *id == view.id => Some(message),
        _ => None,
    })?;
    let chips = reaction_chips(message, &shell.outgoing(view.key.account));
    let list = open.reactors.get(view.id, &chips);
    let rows: Vec<Reactor> = list
        .map(|list| list.reactors.rows(view.filter.as_ref()))
        .unwrap_or_default();
    let waiting = list.is_none_or(|list| {
        list.loading.is_some()
            || (!list.failed && list.reactors.wants_more(view.filter.as_ref(), &chips))
    });
    let failed = list.is_some_and(|list| list.failed);
    let resources = open.resources.clone();
    let names: Vec<String> = rows
        .iter()
        .map(|reactor| {
            resources
                .as_deref()
                .and_then(|resources| resources.users.get(reactor.user))
                .and_then(|user| display_name(user.nickname.as_deref(), user.username.as_deref()))
                .map_or_else(|| strings::someone().to_owned(), str::to_owned)
        })
        .collect();
    let all = view.filter.is_none();
    let palette = *palette;
    let shown = rows.len();
    let count = shown + if waiting { SKELETON_ROWS } else { 0 };
    let list_rows = col()
        .key("reactions-rows")
        .w_full()
        .role("list")
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            ROW_PT,
            |_shell: &Shell, index| Cow::Owned(format!("reactor-{index}")),
            move |_shell: &Shell, index| match rows.get(index) {
                Some(reactor) => reactor_row(reactor, &names[index], all, &palette),
                None => skeleton_row(&palette),
            },
        )
        .on_visible_rows(move |shell: &mut Shell, range| shell.reactions_visible(range, shown));
    let mut body = col()
        .w_full()
        .gap(SPACE_SM_PT)
        .child(header(&palette))
        .child(tabs(&chips, view.filter.as_ref(), &palette))
        .child(
            el().w_full()
                .h(PILL_OUTLINE_PT)
                .fill(palette.line_subtle)
                .a11y_hidden(),
        )
        .child(
            col()
                .w_full()
                .max_h(LIST_MAX_PT)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list_rows),
        );
    if failed {
        body = body.child(
            text(strings::reactions_failed())
                .font(TYPE_SMALL)
                .color(palette.danger)
                .role("alert"),
        );
    }
    Some(body)
}

/// "← Reactions".
fn header(palette: &Palette) -> El {
    let back = row()
        .key(BACK_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::reactions_back())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press_tx(|shell: &mut Shell, tx| shell.reactions_back(tx))
        .child(icon(&crate::icons::ARROW_LEFT, ROW_ICON_PT).color(palette.text_secondary));
    row()
        .w_full()
        .gap(SPACE_SM_PT)
        .center_y()
        .child(back)
        .child(
            text(strings::reactions_title())
                .font(TYPE_TITLE)
                .color(palette.text_primary)
                .role("heading"),
        )
}

/// All, then each emoji with its count, in the chips' order; the chosen
/// one in the link colour on `accent.soft`.
fn tabs(
    chips: &[papo_sdk_ui::ReactionChip],
    chosen: Option<&ReactionEmoji>,
    palette: &Palette,
) -> El {
    let mut tabs = vec![tab(
        None,
        strings::reactions_all().to_owned(),
        None,
        chosen.is_none(),
        palette,
    )];
    for chip in chips {
        let ReactionEmoji::Unicode(emoji) = &chip.emoji else {
            continue;
        };
        let selected = chosen == Some(&chip.emoji);
        tabs.push(tab(
            Some(chip.emoji.clone()),
            format!("{emoji} {}", chip.count),
            Some(strings::reactions_tab(emoji, chip.count)),
            selected,
            palette,
        ));
    }
    row()
        .w_full()
        .wrap()
        .gap(SPACE_SM_PT)
        .role("tablist")
        .focus_group()
        .children(tabs)
}

fn tab(
    filter: Option<ReactionEmoji>,
    words: String,
    spoken: Option<String>,
    selected: bool,
    palette: &Palette,
) -> El {
    let (fill, ink) = if selected {
        (palette.accent_soft, palette.link)
    } else {
        (palette.surface_pill, palette.text_secondary)
    };
    let key = tab_key(filter.as_ref());
    row()
        .key(key)
        .h(PILL_BUTTON_PT)
        .px(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .fill(fill)
        .hover(palette.surface_hover)
        .role("tab")
        .label(spoken.unwrap_or_else(|| words.clone()))
        .focusable(true)
        .focus_selected(selected)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(move |shell: &mut Shell| shell.reactions_tab(filter.clone()))
        .child(text(words).font(TYPE_SMALL_MEDIUM).color(ink))
}

/// One person: the avatar, the name, "you" by yours, and on All every
/// emoji they used.
fn reactor_row(reactor: &Reactor, name: &str, all: bool, palette: &Palette) -> El {
    let mut line = row()
        .w_full()
        .h(ROW_PT)
        .px(SPACE_MD_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .role("listitem")
        .child(avatar(name, PILL_AVATAR_PT, palette))
        .child(
            text(name.to_owned())
                .font(TYPE_BODY)
                .color(palette.text_primary)
                .max_lines(1),
        );
    if reactor.you {
        line = line.child(
            text(strings::reactions_you())
                .font(TYPE_CAPTION)
                .color(palette.text_muted),
        );
    }
    if all {
        let emoji: String = reactor
            .emoji
            .iter()
            .filter_map(|emoji| match emoji {
                ReactionEmoji::Unicode(emoji) => Some(emoji.as_str()),
                ReactionEmoji::Custom(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        line = line
            .child(el().grow(1.0))
            .child(text(emoji).font(TYPE_BODY));
    }
    line
}

/// A row while its page is on its way.
fn skeleton_row(palette: &Palette) -> El {
    row()
        .w_full()
        .h(ROW_PT)
        .px(SPACE_MD_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .busy(true)
        .label(strings::reactions_loading())
        .child(
            el().size(PILL_AVATAR_PT, PILL_AVATAR_PT)
                .radius(RADIUS_CONTROL_PT)
                .fill(palette.surface_hover),
        )
        .child(
            el().w(120.0)
                .h(10.0)
                .radius(5.0)
                .fill(palette.surface_hover),
        )
}
