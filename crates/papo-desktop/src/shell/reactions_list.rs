//! The reactions list (`docs/design.md` §10, Reactions list): View
//! reactions turns the message menu into it in place, and a secondary
//! click on a chip opens it beside the chip on that emoji. "← Reactions"
//! on top when it came from the menu (from a chip nothing is behind it),
//! then the tabs (All, then each emoji with its count, in the
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
use rune::{El, GrowthSpec, Key, KeyEvent, ScrollSpec, Tx, col, el, text};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::design::{Look, PILL_OUTLINE_PT, SPACE_SM, TYPE_SMALL};
use crate::menu::{OpenMenu, Placement, ReactionsView};
use crate::strings;
use crate::timeline::TimelineKey;
use crate::timeline::reactions::chip_key;
use crate::timeline::reactors::Want;

mod rows;

/// The tallest the list grows, in points, before it scrolls.
const LIST_MAX_PT: f32 = 320.0;
/// Skeleton rows while a page is on its way.
const SKELETON_ROWS: usize = 2;
/// How near the list's end, in rows, the next page is read.
const NEAR_END_ROWS: usize = 4;

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
        tx.focus(rows::tab_key(None), true);
        self.load_reactors(key, id, Want::People(None));
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
        self.load_reactors(key, id, Want::People(filter.as_ref()));
    }

    /// A tab: the list shows `filter`'s people, read further if needed.
    pub(super) fn reactions_tab(&mut self, filter: Option<ReactionEmoji>) {
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
        self.load_reactors(key, id, Want::People(filter.as_ref()));
    }

    /// ← on the list, or Escape: back to the menu, or closed when it was
    /// opened from a chip.
    pub(super) fn reactions_back(&mut self, tx: &mut Tx) {
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

    /// Closes a reactions list whose message the shown conversation no
    /// longer holds. Returns whether the view changed.
    pub(super) fn poll_reactions(&mut self) -> bool {
        let Some(view) = self
            .nav
            .menu
            .as_ref()
            .and_then(|menu| menu.reactions.clone())
        else {
            return false;
        };
        let held = self
            .tabs
            .shown()
            .filter(|open| open.key == view.key)
            .is_some_and(|open| open.holds(view.id));
        if held {
            return false;
        }
        self.close_menu();
        true
    }

    /// Escape on the open menu: the reactions list goes back first.
    /// Returns whether the key was taken.
    pub(crate) fn menu_escape(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        if event.key == Key::Escape && self.card_back(tx) {
            return true;
        }
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
        self.load_reactors(view.key, view.id, Want::People(view.filter.as_ref()))
    }
}

/// The list the open menu shows, when it shows one.
pub(super) fn body(shell: &Shell, look: &Look) -> Option<El> {
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
    let keys: Vec<Uuid> = rows.iter().map(|reactor| reactor.user).collect();
    let all = view.filter.is_none();
    let look = *look;
    let shown = rows.len();
    let count = shown + if waiting { SKELETON_ROWS } else { 0 };
    let list_rows = col()
        .key("reactions-rows")
        .w_full()
        .role("list")
        .growth(GrowthSpec::default())
        .virtual_rows(
            count,
            rows::ROW_PT,
            move |_shell: &Shell, index| {
                Cow::Owned(match keys.get(index) {
                    Some(user) => format!("reactor-{user}"),
                    None => format!("reactor-skeleton-{index}"),
                })
            },
            move |_shell: &Shell, index| match rows.get(index) {
                Some(reactor) => rows::reactor_row(reactor, &names[index], all, &look),
                None => rows::skeleton_row(&look),
            },
        )
        .on_visible_rows(move |shell: &mut Shell, range| shell.reactions_visible(range, shown));
    // Opened from a chip, nothing is behind the list to go back to: no
    // "← Reactions" (Christian, 2026-11-23).
    let header = (!view.from_chip).then(|| rows::header(&look));
    let mut body = col()
        .w_full()
        .gap(look.space(SPACE_SM))
        .children(header)
        .child(rows::tabs(&chips, view.filter.as_ref(), &look))
        .child(
            el().w_full()
                .h(PILL_OUTLINE_PT)
                .fill(look.palette.line_subtle)
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
                .font(look.font(TYPE_SMALL))
                .color(look.palette.danger)
                .role("alert"),
        );
    }
    Some(body)
}
