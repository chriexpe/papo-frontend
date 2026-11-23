//! A message's menu and keys (`docs/design.md` §10 and §12), headless:
//! Enter opens the menu with what this account may do (an item it may not
//! use is left out), its copies reach the clipboard, Escape gives the
//! message the keyboard back, View reactions turns the menu into the list
//! and Escape goes back, and the quick reactions lead the bar and the
//! strip once known.

use std::sync::Arc;

use papo_sdk::operations::interactions::MessageReactionSummary;
use papo_sdk::{Message, Uuid};
use papo_sdk_ui::TimelineItem;
use rune::clipboard::memory;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::replying::{conversation, message_id};
use super::{FRAME_S, Shell, press};
use crate::live_watch::Posted;
use crate::menu::Entry;
use crate::timeline::rows::message_key;

/// The message whose row has the keyboard.
fn focused_message(rt: &Runtime<Shell>) -> Option<Uuid> {
    rt.elements()
        .into_iter()
        .find(|element| element.focused)
        .and_then(|element| crate::timeline::content::copy::message_of(&element.path))
}

/// Message `n` takes the keyboard.
fn focus_message(rt: &mut Runtime<Shell>, n: u128) {
    rt.update(move |_shell, tx| tx.focus(message_key(message_id(n)), true));
    rt.frame(FRAME_S);
}

/// The open menu's item labels, in order, and its header.
fn menu(rt: &Runtime<Shell>) -> (Option<String>, Vec<String>) {
    let Some(menu) = &rt.state.nav.menu else {
        return (None, Vec::new());
    };
    let header = menu.entries.iter().find_map(|entry| match entry {
        Entry::Header(words) => Some(words.clone()),
        _ => None,
    });
    let items = menu
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item.label.clone()),
            Entry::Strip(_) => Some("strip".to_owned()),
            _ => None,
        })
        .collect();
    (header, items)
}

#[test]
fn enter_opens_your_messages_menu_with_what_you_may_do() {
    let mut rt = conversation("");
    focus_message(&mut rt, 4);
    press(&mut rt, Key::Enter);
    let (header, items) = menu(&rt);
    assert!(header.is_some_and(|header| header.starts_with("me · ")));
    assert_eq!(
        items,
        [
            "Reply",
            "Edit",
            "Copy text",
            "Copy as Markdown",
            "Copy ID",
            "Delete message"
        ]
    );
}

#[test]
fn someone_elses_message_offers_no_edit_and_no_delete() {
    let mut rt = conversation("");
    focus_message(&mut rt, 3);
    press(&mut rt, Key::Enter);
    let (_, items) = menu(&rt);
    assert_eq!(items, ["Reply", "Copy text", "Copy as Markdown", "Copy ID"]);
}

#[test]
fn copy_id_puts_the_id_on_the_clipboard_and_the_message_gets_the_keyboard_back() {
    let mut rt = conversation("");
    let clipboard = memory();
    rt.use_clipboard(&clipboard);
    focus_message(&mut rt, 3);
    press(&mut rt, Key::Enter);
    let id = rt
        .state
        .nav
        .menu
        .as_ref()
        .and_then(|menu| {
            menu.entries.iter().find_map(|entry| match entry {
                Entry::Item(item) if item.label == "Copy ID" => Some(item.action.clone()),
                _ => None,
            })
        })
        .unwrap();
    rt.update(move |shell, tx| shell.menu_picked(id, tx));
    rt.frame(FRAME_S);
    assert_eq!(
        clipboard.text().map(|text| text.to_string()),
        Some(message_id(3).to_string())
    );
    assert!(rt.state.nav.menu.is_none());
    assert_eq!(focused_message(&rt), Some(message_id(3)));
}

#[test]
fn ctrl_c_on_a_focused_message_copies_its_text() {
    let mut rt = conversation("");
    let clipboard = memory();
    rt.use_clipboard(&clipboard);
    focus_message(&mut rt, 3);
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
    assert_eq!(
        clipboard.text().map(|text| text.to_string()),
        Some("message 3".to_owned())
    );
}

#[test]
fn escape_closes_the_menu_and_the_message_keeps_the_keyboard() {
    let mut rt = conversation("");
    focus_message(&mut rt, 3);
    press(&mut rt, Key::Enter);
    assert!(rt.state.nav.menu.is_some());
    press(&mut rt, Key::Escape);
    rt.frame(FRAME_S);
    assert!(rt.state.nav.menu.is_none());
    assert_eq!(focused_message(&rt), Some(message_id(3)));
}

/// Message 3 with two reactions.
fn with_reactions(rt: &mut Runtime<Shell>) {
    rt.update(|shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        let at = open
            .items
            .iter()
            .position(|item| matches!(item.as_ref(), TimelineItem::Message { id, .. } if *id == message_id(3)))
            .unwrap();
        let TimelineItem::Message {
            id,
            message,
            content,
            reply,
            pending,
        } = open.items[at].as_ref().clone()
        else {
            unreachable!()
        };
        let total = |unicode: &str, count| MessageReactionSummary {
            emoji_id: None,
            unicode: Some(unicode.to_owned()),
            count: Some(count),
        };
        let message = Message {
            reactions: Some(vec![total("🧀", 4), total("👍", 1)]),
            ..Message::clone(&message)
        };
        open.items.set(
            at,
            Arc::new(TimelineItem::Message {
                id,
                message: Arc::new(message),
                content,
                reply,
                pending,
            }),
        );
    });
    rt.frame(FRAME_S);
}

#[test]
fn view_reactions_turns_the_menu_into_the_list_and_escape_goes_back() {
    let mut rt = conversation("");
    with_reactions(&mut rt);
    focus_message(&mut rt, 3);
    press(&mut rt, Key::Enter);
    let (_, items) = menu(&rt);
    assert!(items.contains(&"View reactions".to_owned()));
    let view = rt
        .state
        .nav
        .menu
        .as_ref()
        .and_then(|menu| {
            menu.entries.iter().find_map(|entry| match entry {
                Entry::Item(item) if item.label == "View reactions" => Some(item.action.clone()),
                _ => None,
            })
        })
        .unwrap();
    rt.update(move |shell, tx| shell.menu_picked(view, tx));
    rt.frame(FRAME_S);
    let menu_open = rt.state.nav.menu.as_ref().unwrap();
    assert!(menu_open.reactions.is_some(), "the menu became the list");
    let tabs: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|element| element.role == "tab" && element.path.contains("menu"))
        .filter_map(|element| element.label)
        .collect();
    assert_eq!(tabs, ["All", "🧀, 4", "👍, 1"]);
    press(&mut rt, Key::Escape);
    rt.frame(FRAME_S);
    let menu_open = rt.state.nav.menu.as_ref().expect("still the menu");
    assert!(menu_open.reactions.is_none(), "back to the menu");
}

#[test]
fn the_quick_reactions_lead_the_bar_and_the_menus_strip() {
    let mut rt = conversation("");
    rt.update(|shell, tx| {
        let account = shell.tabs.shown().unwrap().key.account;
        let quick = ["🧀", "👍", "😂", "❤️", "🔥", "😭", "👀", "🙏"]
            .map(str::to_owned)
            .to_vec();
        shell.activity_posted(account, Posted::Quick(quick), tx);
    });
    rt.frame(FRAME_S);
    let bar: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|element| {
            element
                .path
                .contains(&format!("/{}/", message_key(message_id(3))))
                && element.path.contains("/bar/")
                && element.role == "button"
        })
        .filter_map(|element| element.label)
        .collect();
    assert_eq!(
        bar,
        [
            "React with 🧀",
            "React with 👍",
            "React with 😂",
            "Add reaction",
            "Reply",
            "More"
        ]
    );
    focus_message(&mut rt, 3);
    press(&mut rt, Key::Enter);
    let (_, items) = menu(&rt);
    assert_eq!(items.first().map(String::as_str), Some("strip"));
}

#[test]
fn a_stretched_bar_folds_when_the_pointer_moves_to_another_message() {
    let mut rt = conversation("");
    rt.update(|shell, tx| {
        let account = shell.tabs.shown().unwrap().key.account;
        let quick = ["🧀", "👍", "😂"].map(str::to_owned).to_vec();
        shell.activity_posted(account, Posted::Quick(quick), tx);
    });
    let key = rt.state.tabs.shown().unwrap().key;
    rt.update(move |shell, _tx| shell.toggle_strip(key, message_id(3)));
    rt.frame(FRAME_S);
    assert!(rt.state.reacting().stretched.is_some());
    rt.update(move |shell, _tx| {
        crate::timeline::TimelineHost::message_hovered(shell, key, message_id(3), true);
    });
    assert!(
        rt.state.reacting().stretched.is_some(),
        "its own message keeps it"
    );
    rt.update(move |shell, _tx| {
        crate::timeline::TimelineHost::message_hovered(shell, key, message_id(4), true);
    });
    assert!(
        rt.state.reacting().stretched.is_none(),
        "another message folds it"
    );
}
