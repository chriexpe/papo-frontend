//! A message's menu from the shell's side (`docs/design.md` §10): what it
//! lists for this account (an item it may not use is left out, never
//! greyed), the three ways it opens, and carrying out its items: reply,
//! edit, pin or unpin, the copies, delete, opening a link it holds. The
//! reactions and pins it asks for are `reacting`'s.

use papo_sdk::Uuid;
use papo_sdk::operations::reactions::ReactionEmoji;
use papo_sdk_ui::{PinChange, reaction_chips};
use rune::Tx;

use super::Shell;
use super::replies::item_actions;
use crate::composer::COMPOSER_KEY;
use crate::design::{Density, MESSAGE_MENU_WIDTH_PT};
use crate::icons;
use crate::links;
use crate::menu::{Entry, MenuAction, MenuItem, MessageAction, OpenMenu, Placement, Strip};
use crate::strings;
use crate::timeline::content::copy::{self, Source};
use crate::timeline::local_time::day_label;
use crate::timeline::{OpenTimeline, TimelineKey};

impl Shell {
    /// Opens message `id`'s menu at `placement`; `trigger` gets the
    /// keyboard back when it closes: the message when the keyboard opened
    /// it, else what had the keyboard (the pointer moves no focus).
    pub(crate) fn open_message_menu(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        placement: Placement,
        trigger: String,
    ) {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return;
        };
        let Some(entries) = self.message_entries(open, id) else {
            return;
        };
        let menu = OpenMenu::with_entries(placement, trigger, entries, MESSAGE_MENU_WIDTH_PT);
        self.nav.close_accounts();
        self.stretched = None;
        self.nav.menu = Some(menu);
    }

    /// A link's menu beside the pointer at `at`: Open link, with its host,
    /// and Copy link, nothing else (`docs/design.md` §10, Link menu).
    pub(super) fn open_link_menu(&mut self, key: TimelineKey, url: String, at: [f32; 2]) {
        let open_link = MenuAction::OpenLink {
            key,
            url: url.clone(),
        };
        let items = vec![
            MenuItem::plain(
                "menu-link-open",
                strings::link_menu_open(),
                open_link,
                false,
            )
            .with_icon(&icons::EXTERNAL)
            .with_sub(links::host(&url)),
            MenuItem::plain(
                "menu-link-copy",
                strings::link_menu_copy(),
                MenuAction::CopyLink(url),
                false,
            )
            .with_icon(&icons::COPY),
        ];
        let entries = items.into_iter().map(Entry::Item).collect();
        // Opened by the pointer, which moves no keyboard focus: writing gets
        // the keyboard back.
        let trigger = COMPOSER_KEY.to_owned();
        self.nav.close_accounts();
        self.nav.menu = Some(OpenMenu::with_entries(
            Placement::AtPointer(at),
            trigger,
            entries,
            MESSAGE_MENU_WIDTH_PT,
        ));
    }

    /// What message `id`'s menu lists, in §10's groups.
    fn message_entries(&self, open: &OpenTimeline, id: Uuid) -> Option<Vec<Entry>> {
        let key = open.key;
        let (actions, _) = item_actions(open, id)?;
        let message = open.message(id).map(|(message, _)| message.as_ref())?;
        let mut entries = vec![Entry::Header(self.menu_header(open, message))];
        if actions.react
            && let Some(strip) = self.menu_strip(key, id, message)
        {
            entries.push(Entry::Strip(strip));
        }
        let item = |name: &str, label: &str, shape, action| {
            MenuItem::plain(name, label, MenuAction::Message { key, id, action }, false)
                .with_icon(shape)
        };
        let mut group = Vec::new();
        if actions.reply {
            group.push(
                item(
                    "menu-reply",
                    strings::menu_reply(),
                    &icons::REPLY,
                    MessageAction::Reply,
                )
                .with_shortcut(strings::shortcut_reply()),
            );
        }
        if actions.edit {
            group.push(
                item(
                    "menu-edit",
                    strings::menu_edit(),
                    &icons::EDIT,
                    MessageAction::Edit,
                )
                .with_shortcut(strings::shortcut_edit()),
            );
        }
        if !self.pinning.waits(key, id) {
            let pin = match actions.pin {
                Some(PinChange::Pin) => Some(item(
                    "menu-pin",
                    strings::menu_pin(),
                    &icons::PIN,
                    MessageAction::Pin,
                )),
                Some(PinChange::Unpin) => Some(item(
                    "menu-unpin",
                    strings::menu_unpin(),
                    &icons::PIN_OFF,
                    MessageAction::Unpin,
                )),
                _ => None,
            };
            group.extend(pin.map(|pin| pin.with_shortcut(strings::shortcut_pin())));
        }
        if actions.view_reactions {
            group.push(item(
                "menu-view-reactions",
                strings::menu_view_reactions(),
                &icons::SMILE,
                MessageAction::ViewReactions,
            ));
        }
        push_group(&mut entries, group);
        let copies = copy::copies(&self.copy_source(open), id)?;
        push_group(&mut entries, link_items(key, &copies.links));
        push_group(&mut entries, copy_items(key, id, &copies));
        if actions.delete.is_some() {
            let mut delete = item(
                "menu-delete",
                strings::menu_delete(),
                &icons::TRASH,
                MessageAction::Delete,
            )
            .with_shortcut(strings::shortcut_delete());
            delete.danger = true;
            push_group(&mut entries, vec![delete]);
        }
        Some(entries)
    }

    /// The menu's quick-reaction strip for message `id`; none before the
    /// account's quick reactions are known.
    fn menu_strip(&self, key: TimelineKey, id: Uuid, message: &papo_sdk::Message) -> Option<Strip> {
        let chips = reaction_chips(message, &self.outgoing(key.account));
        let emoji: Vec<(String, bool)> = self
            .quick_reactions(key.account)
            .iter()
            .map(|emoji| {
                let unicode = ReactionEmoji::Unicode(emoji.clone());
                let mine = chips.iter().any(|chip| chip.mine && chip.emoji == unicode);
                (emoji.clone(), mine)
            })
            .collect();
        (!emoji.is_empty()).then_some(Strip {
            key,
            id,
            emoji,
            chosen: 0,
        })
    }

    /// "{author} · {day} at {time}": a continued row shows neither.
    fn menu_header(&self, open: &OpenTimeline, message: &papo_sdk::Message) -> String {
        let author = crate::timeline::author_name(message, open.resources.as_deref());
        let Some(at_us) = crate::timeline::created_us(message) else {
            return author;
        };
        let day = day_label(self.local_time.day_of(at_us), self.today);
        strings::menu_header(&author, &day, &self.local_time.time_label(at_us))
    }

    /// What a copy reads: the shown timeline as its rows are drawn.
    pub(super) fn copy_source(&self, open: &OpenTimeline) -> Source {
        Source {
            items: open.items.clone(),
            resources: open.resources.clone(),
            revealed: open.revealed.clone(),
            jumbo: self.message_layout.density != Density::ExtraCompact,
            time: self.local_time.clone(),
            today: self.today,
        }
    }

    /// Carries out `action` on message `id` of timeline `key`.
    pub(super) fn message_action(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        action: MessageAction,
        tx: &mut Tx,
    ) {
        match action {
            MessageAction::Reply => {
                self.reply_to(key, id, tx);
            }
            MessageAction::Edit => {
                self.edit(key, id, tx);
            }
            MessageAction::Pin => self.set_pin(key, id, true),
            MessageAction::Unpin => self.set_pin(key, id, false),
            MessageAction::CopyText | MessageAction::CopyMarkdown | MessageAction::CopyId => {
                if let Some(copied) = self.copy_of(key, id, action) {
                    tx.copy(copied);
                }
            }
            MessageAction::Delete => self.ask_delete(key, id),
            // The menu turned into the list instead (`menu_picked`).
            MessageAction::ViewReactions => {}
        }
    }

    /// What Copy text, Copy as Markdown or Copy ID put on the clipboard.
    pub(super) fn copy_of(
        &self,
        key: TimelineKey,
        id: Uuid,
        action: MessageAction,
    ) -> Option<String> {
        if action == MessageAction::CopyId {
            return Some(id.to_string());
        }
        let open = self.tabs.shown().filter(|open| open.key == key)?;
        let copies = copy::copies(&self.copy_source(open), id)?;
        match action {
            MessageAction::CopyText => Some(copies.text),
            MessageAction::CopyMarkdown => Some(copies.markdown),
            _ => None,
        }
    }
}

/// One Open link item per link the message holds, with its host.
fn link_items(key: TimelineKey, urls: &[String]) -> Vec<MenuItem> {
    urls.iter()
        .enumerate()
        .map(|(index, url)| {
            let action = MenuAction::OpenLink {
                key,
                url: url.clone(),
            };
            MenuItem::plain(
                &format!("menu-link-{index}"),
                strings::link_menu_open(),
                action,
                false,
            )
            .with_icon(&icons::EXTERNAL)
            .with_sub(links::host(url))
        })
        .collect()
}

/// Copy text and Copy as Markdown when the message has text, Copy ID always.
fn copy_items(key: TimelineKey, id: Uuid, copies: &copy::Copies) -> Vec<MenuItem> {
    let item = |name: &str, label: &str, shape, action| {
        MenuItem::plain(name, label, MenuAction::Message { key, id, action }, false)
            .with_icon(shape)
    };
    let mut items = Vec::new();
    if !copies.text.is_empty() {
        items.push(
            item(
                "menu-copy-text",
                strings::menu_copy_text(),
                &icons::COPY,
                MessageAction::CopyText,
            )
            .with_shortcut(strings::shortcut_copy()),
        );
    }
    if !copies.markdown.is_empty() {
        items.push(item(
            "menu-copy-markdown",
            strings::menu_copy_markdown(),
            &icons::MARKDOWN,
            MessageAction::CopyMarkdown,
        ));
    }
    items.push(item(
        "menu-copy-id",
        strings::menu_copy_id(),
        &icons::ID,
        MessageAction::CopyId,
    ));
    items
}

/// Adds `group` after a divider, when it holds anything.
fn push_group(entries: &mut Vec<Entry>, group: Vec<MenuItem>) {
    if group.is_empty() {
        return;
    }
    if entries.iter().any(|entry| matches!(entry, Entry::Item(_))) {
        entries.push(Entry::Divider);
    }
    entries.extend(group.into_iter().map(Entry::Item));
}
