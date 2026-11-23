//! A message's menu from the shell's side (`docs/design.md` §10): what it
//! lists for this account (an item it may not use is left out, never
//! greyed), the three ways it opens, and carrying out its items: reply,
//! edit, pin or unpin, the copies, delete, opening a link it holds, and a
//! quick reaction. Reactions and pins go to the SDK (`ReactionService`,
//! pin and unpin); a refused one is said in the notice pill.

use std::collections::HashSet;

use papo_sdk::operations::reactions::{ReactionEmoji, ReactionIntent};
use papo_sdk::{MessageTarget, Uuid};
use papo_sdk_ui::{PinChange, TimelineItem, reaction_chips};
use rune::Tx;

use super::Shell;
use super::replies::item_actions;
use crate::composer::COMPOSER_KEY;
use crate::design::{Density, MESSAGE_MENU_WIDTH_PT};
use crate::icons;
use crate::links;
use crate::menu::{
    Entry, MenuAction, MenuIcon, MenuItem, MessageAction, OpenMenu, Placement, Strip,
};
use crate::notice::Refusal;
use crate::strings;
use crate::timeline::content::copy::{self, Source};
use crate::timeline::local_time::day_label;
use crate::timeline::rows::message_key;
use crate::timeline::{OpenTimeline, TimelineKey};

/// Pins and unpins on their way, by message: a second press waits for the
/// first (a control whose call is in flight waits).
#[derive(Default)]
pub(crate) struct Pinning(HashSet<(TimelineKey, Uuid)>);

impl Shell {
    /// Opens message `id`'s menu at `placement`; the message gets the
    /// keyboard back when it closes.
    pub(crate) fn open_message_menu(&mut self, key: TimelineKey, id: Uuid, placement: Placement) {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return;
        };
        let Some(entries) = self.message_entries(open, id) else {
            return;
        };
        let menu =
            OpenMenu::with_entries(placement, message_key(id), entries, MESSAGE_MENU_WIDTH_PT);
        self.nav.close_accounts();
        self.nav.menu = Some(menu);
    }

    /// A link's menu beside the pointer at `at`: Open link, with its host,
    /// and Copy link, nothing else (`docs/design.md` §10, Link menu).
    pub(super) fn open_link_menu(&mut self, key: TimelineKey, url: String, at: [f32; 2]) {
        let items = vec![
            MenuItem {
                key: "menu-link-open".to_owned(),
                label: strings::link_menu_open().to_owned(),
                sub: links::host(&url),
                icon: Some(MenuIcon(&icons::EXTERNAL)),
                shortcut: None,
                action: MenuAction::OpenLink {
                    key,
                    url: url.clone(),
                },
                danger: false,
            },
            MenuItem {
                key: "menu-link-copy".to_owned(),
                label: strings::link_menu_copy().to_owned(),
                sub: None,
                icon: Some(MenuIcon(&icons::COPY)),
                shortcut: None,
                action: MenuAction::CopyLink(url),
                danger: false,
            },
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
        let message = held_message(open, id)?;
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
        if !self.pinning.0.contains(&(key, id)) {
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
        let chips = self
            .outgoing(key.account)
            .map(|outgoing| reaction_chips(message, &outgoing))
            .unwrap_or_default();
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

    /// P on the focused message: pins it, or unpins it when pinned.
    /// Returns whether the key was taken.
    pub(super) fn toggle_pin(&mut self, key: TimelineKey, id: Uuid) -> bool {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return false;
        };
        match item_actions(open, id).and_then(|(actions, _)| actions.pin) {
            Some(PinChange::Pin) => self.set_pin(key, id, true),
            Some(PinChange::Unpin) => self.set_pin(key, id, false),
            _ => return false,
        }
        true
    }

    /// P, or Pin or Unpin in the menu: pins message `id` (`pin`) or
    /// unpins it, if the account may and no pin of it is on its way. The
    /// pins follow from the SDK's pin events; a refusal is said.
    pub(super) fn set_pin(&mut self, key: TimelineKey, id: Uuid, pin: bool) {
        let Some(open) = self.tabs.shown().filter(|open| open.key == key) else {
            return;
        };
        let wanted = if pin {
            PinChange::Pin
        } else {
            PinChange::Unpin
        };
        let allowed = item_actions(open, id).and_then(|(actions, _)| actions.pin);
        if allowed != Some(wanted) || !self.pinning.0.insert((key, id)) {
            return;
        }
        let Some(sdk) = &self.sdk else {
            self.pinning.0.remove(&(key, id));
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let done = match client.session(key.account) {
                Ok(session) => {
                    let operations = session.operations();
                    if pin {
                        operations.pin_message(key.channel, id).await.map(|_| ())
                    } else {
                        operations.unpin_message(key.channel, id).await.map(|_| ())
                    }
                }
                Err(error) => Err(error.into()),
            };
            let refused = done.err().map(|error| {
                log::warn!("a pin change did not happen: {error}");
                if pin { Refusal::Pin } else { Refusal::Unpin }
            });
            updates.send_tx(move |shell, tx| {
                shell.pinning.0.remove(&(key, id));
                if let Some(refusal) = refused {
                    shell.refuse(refusal, tx);
                }
                true
            });
        });
    }

    /// Whether your reaction `emoji` to message `id` is there, when you may
    /// change it now: none without the permission, or while a change of it
    /// is on its way (the chip waits).
    pub(super) fn reacts_with(&self, key: TimelineKey, id: Uuid, emoji: &str) -> Option<bool> {
        let open = self.tabs.shown().filter(|open| open.key == key)?;
        let message = held_message(open, id)?;
        let outgoing = self.outgoing(key.account)?;
        let unicode = ReactionEmoji::Unicode(emoji.to_owned());
        let chip = reaction_chips(message, &outgoing)
            .into_iter()
            .find(|chip| chip.emoji == unicode);
        match chip {
            Some(chip) => chip
                .can_toggle(super::replies::channel_capabilities(open))
                .then_some(chip.mine),
            None => item_actions(open, id)
                .is_some_and(|(actions, _)| actions.react)
                .then_some(false),
        }
    }

    /// A quick reaction or a chip: your reaction `emoji` to message `id`
    /// is added (`add`) or taken back. The chip waits for the SDK's answer;
    /// a refused one is said.
    pub(crate) fn react(&mut self, key: TimelineKey, id: Uuid, emoji: String, add: bool) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let local = (self.local_ids)();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let admitted = match client.session(key.account) {
                Ok(session) => session
                    .reactions()
                    .set(ReactionIntent {
                        channel: key.channel,
                        message: MessageTarget::Remote(id),
                        emoji: ReactionEmoji::Unicode(emoji),
                        add,
                        local,
                        at: super::composer::now(),
                    })
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = admitted {
                log::warn!("a reaction was not queued: {error}");
                updates.send_tx(|shell, tx| {
                    shell.refuse(Refusal::Reaction, tx);
                    true
                });
            }
        });
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

/// Message `id` as the timeline holds it.
fn held_message(open: &OpenTimeline, id: Uuid) -> Option<&papo_sdk::Message> {
    open.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id: current,
            message,
            ..
        } if *current == id => Some(message.as_ref()),
        _ => None,
    })
}
