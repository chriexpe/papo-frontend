//! `:` in the field suggests emoji, as `@` suggests people (`docs/design.md` §16.2): the same
//! picker grows from the pill, the server's matches first; Enter or Tab makes the query the
//! emoji and a space; Escape closes it and leaves the text; code never asks.

use std::sync::Arc;

use papo_sdk::{CustomEmoji, EmojiCatalog, Uuid};
use rune::{Key, KeyEvent, Mods, Runtime};

use super::composer::field;
use super::replying::conversation;
use super::{FRAME_S, Shell, press};
use crate::composer::picker::PICKER_KEY;
use crate::live_watch::Posted;

/// Types `text` key by key, as a person would.
fn keys(rt: &mut Runtime<Shell>, text: &str) {
    for character in text.chars() {
        rt.key(KeyEvent {
            key: Key::Text(character.to_string()),
            mods: Mods::default(),
        });
        rt.frame(FRAME_S);
    }
}

/// The shown conversation, its account's catalog holding `:gato:`.
fn with_gato() -> Runtime<Shell> {
    let mut rt = conversation("");
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
    rt
}

fn rows(rt: &Runtime<Shell>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|element| element.path.contains(PICKER_KEY) && element.role == "option")
        .filter_map(|element| element.label)
        .collect()
}

#[test]
fn a_colon_suggests_the_servers_emoji_first_and_enter_makes_it_the_emoji() {
    let mut rt = with_gato();
    keys(&mut rt, "oi :ga");
    let found = rows(&rt);
    assert_eq!(found.first().map(String::as_str), Some(":gato:, Lumen"));
    press(&mut rt, Key::Enter);
    assert_eq!(field(&rt), "oi :gato: ", "Enter picked; it sent nothing");
    assert_eq!(rt.state.composer.emoji_ids(), [Uuid::from_u128(77)]);
    assert!(rt.find(PICKER_KEY).is_none());
}

#[test]
fn a_unicode_suggestion_goes_in_as_itself() {
    let mut rt = with_gato();
    keys(&mut rt, ":cheese_w");
    press(&mut rt, Key::Tab);
    assert_eq!(field(&rt), "🧀 ");
}

#[test]
fn escape_leaves_a_smiley_as_text_and_that_colon_stays_closed() {
    let mut rt = with_gato();
    keys(&mut rt, "ok :D");
    assert!(rt.find(PICKER_KEY).is_some(), "one letter is enough");
    press(&mut rt, Key::Escape);
    assert!(rt.find(PICKER_KEY).is_none());
    keys(&mut rt, "D");
    assert!(rt.find(PICKER_KEY).is_none(), "it stays closed for this :");
    assert_eq!(field(&rt), "ok :DD");
}

#[test]
fn an_escaped_mention_never_keeps_a_colon_at_its_place_closed() {
    let mut rt = with_gato();
    keys(&mut rt, "@li");
    assert!(rt.find(PICKER_KEY).is_some(), "Lia is suggested");
    press(&mut rt, Key::Escape);
    assert!(rt.find(PICKER_KEY).is_none());
    rt.update(|shell, _tx| {
        let field = shell.composer.field_mut();
        field.set_selection(0, 3);
        field.insert(":ga");
    });
    rt.update(Shell::composer_edited);
    rt.frame(FRAME_S);
    assert!(
        rt.find(PICKER_KEY).is_some(),
        "a `:` there is another query"
    );
}

#[test]
fn a_unicode_row_is_spoken_by_its_name() {
    let mut rt = with_gato();
    keys(&mut rt, ":cheese_w");
    assert_eq!(rows(&rt).first().map(String::as_str), Some("cheese wedge"));
}

#[test]
fn a_time_or_code_never_asks() {
    let mut rt = with_gato();
    keys(&mut rt, "às 10:30 `:ga");
    assert!(rt.find(PICKER_KEY).is_none());
}
