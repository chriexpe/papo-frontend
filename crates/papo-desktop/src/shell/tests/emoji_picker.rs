//! The emoji picker, headless (`docs/design.md` §16.1, §16.2, §16.5): Ctrl+E opens the Emoji tab
//! on its search; a pick goes in at the field's caret and closes, Shift keeps it open; Escape
//! clears the search, then closes; the 😊 or a press outside closes it; More emoji on a
//! stretched strip opens it for a reaction.

use std::sync::Arc;

use papo_sdk::{CustomEmoji, EmojiCatalog, Uuid};
use rune::{Key, KeyEvent, Mods, Runtime};

use super::navigation::focused;
use super::replying::{conversation, message_id};
use super::{FRAME_S, Shell, press, type_text};
use crate::composer::COMPOSER_KEY;
use crate::composer::view::EMOJI_BUTTON_KEY;
use crate::emoji_picker::PickerFor;
use crate::emoji_picker::view::EMOJI_SEARCH_KEY;
use crate::live_watch::Posted;
use crate::picker::PICKER_KEY as EMOJI_PICKER_KEY;

fn key(rt: &mut Runtime<Shell>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(FRAME_S);
}

fn ctrl_e(rt: &mut Runtime<Shell>) {
    let ctrl = Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    };
    key(rt, Key::Command('e'), ctrl);
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

fn field(rt: &Runtime<Shell>) -> String {
    rt.state.composer.shown().unwrap().1.edit.text.clone()
}

fn open(rt: &Runtime<Shell>) -> bool {
    rt.find(EMOJI_PICKER_KEY).is_some()
}

/// The account's catalog holds `:gato:`.
fn with_gato(rt: &mut Runtime<Shell>) {
    rt.update(|shell, tx| {
        let account = shell.tabs.shown().unwrap().key.account;
        let catalog = EmojiCatalog {
            revision: 1,
            emojis: vec![CustomEmoji {
                id: Uuid::from_u128(77),
                name: "gato".to_owned(),
                format: "png".to_owned(),
                created_at: String::new(),
            }],
        };
        shell.activity_posted(account, Posted::Emojis(Arc::new(catalog)), tx);
    });
    rt.frame(FRAME_S);
}

fn press_on(rt: &mut Runtime<Shell>, [x, y]: [f32; 2]) {
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(FRAME_S);
}

fn centre_of(rt: &Runtime<Shell>, element: &str) -> [f32; 2] {
    let (x, y) = rt.find(element).expect(element);
    [x, y]
}

#[test]
fn ctrl_e_opens_the_search_and_a_pick_goes_in_at_the_caret_and_closes() {
    let mut rt = conversation("ab");
    rt.update(|shell, _tx| shell.composer.field_mut().set_caret(1));
    ctrl_e(&mut rt);
    assert!(open(&rt));
    assert_eq!(focused(&rt).as_deref(), Some(EMOJI_SEARCH_KEY));
    type_text(&mut rt, "cheese");
    press(&mut rt, Key::Enter);
    assert_eq!(field(&rt), "a🧀b");
    assert!(!open(&rt), "a pick closes it");
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
}

#[test]
fn shift_enter_keeps_it_open_and_a_custom_emoji_goes_in_as_its_pill() {
    let mut rt = conversation("");
    with_gato(&mut rt);
    ctrl_e(&mut rt);
    type_text(&mut rt, "gato");
    for _ in 0..3 {
        key(&mut rt, Key::Enter, shift());
    }
    assert!(open(&rt), "Shift keeps it open");
    assert_eq!(field(&rt), ":gato::gato::gato:");
    assert_eq!(rt.state.composer.emoji_ids(), [Uuid::from_u128(77); 3]);
}

#[test]
fn escape_clears_the_search_then_closes() {
    let mut rt = conversation("");
    ctrl_e(&mut rt);
    type_text(&mut rt, "x");
    press(&mut rt, Key::Escape);
    assert!(open(&rt));
    assert_eq!(rt.state.emoji_picker().unwrap().search.text, "");
    press(&mut rt, Key::Escape);
    assert!(!open(&rt));
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
}

#[test]
fn the_emoji_button_again_or_a_press_outside_closes_it() {
    let mut rt = conversation("");
    ctrl_e(&mut rt);
    let button = centre_of(&rt, EMOJI_BUTTON_KEY);
    press_on(&mut rt, button);
    assert!(!open(&rt), "closed, not closed and opened again");
    press_on(&mut rt, button);
    assert!(open(&rt), "the 😊 opens it");
    press_on(&mut rt, [10.0, 10.0]);
    assert!(!open(&rt));
}

#[test]
fn more_emoji_leads_a_stretched_strip_and_opens_the_tab_for_a_reaction() {
    let mut rt = conversation("");
    rt.update(|shell, tx| {
        let account = shell.tabs.shown().unwrap().key.account;
        let usage = vec![("🧀".to_owned(), 3)];
        shell.activity_posted(account, Posted::Usage(usage), tx);
    });
    let at = rt.state.tabs.shown().unwrap().key;
    let id = message_id(3);
    rt.update(move |shell, tx| {
        shell.toggle_strip(at, id);
        let left = KeyEvent {
            key: Key::Left,
            mods: Mods::default(),
        };
        assert!(shell.strip_key(at, id, &left, tx), "← from the first emoji");
        let enter = KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        };
        assert!(shell.strip_key(at, id, &enter, tx));
    });
    rt.frame(FRAME_S);
    let target = rt.state.emoji_picker().map(|picker| picker.target);
    assert_eq!(
        target,
        Some(PickerFor::Reaction {
            key: at,
            message: id
        })
    );
    rt.update(|shell, tx| shell.emoji_cell_pressed(0, false, tx));
    rt.frame(FRAME_S);
    assert!(rt.state.picker.is_none(), "a pick reacts and closes");
}

#[test]
fn the_apps_shortcuts_wait_while_the_picker_is_open() {
    let mut rt = conversation("");
    ctrl_e(&mut rt);
    let ctrl = Mods {
        cmd: true,
        ctrl: true,
        ..Mods::default()
    };
    key(&mut rt, Key::Command('f'), ctrl);
    assert!(
        rt.state.shown_search().is_none(),
        "no search behind the picker"
    );
    assert!(open(&rt));
    ctrl_e(&mut rt);
    assert!(!open(&rt), "Ctrl+E closes it");
}
