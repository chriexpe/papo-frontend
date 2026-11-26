//! The GIF tab, headless (`docs/design.md` §16.1, §16.3, §16.5): Ctrl+G opens it on the search;
//! the pill's tabs switch it in one press and close it when pressed again; a press outside
//! closes it; without a key the field's place says so and the chips are Favorites and Recent;
//! a tile picked keeps the field's text, and Shift keeps the tab open.

use papo_sdk::Gif;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::navigation::focused;
use super::replying::conversation;
use super::{FRAME_S, Shell, press};
use crate::composer::view::{EMOJI_BUTTON_KEY, GIF_BUTTON_KEY};
use crate::gif_picker::GifPicker;
use crate::gif_picker::view::GIF_SEARCH_KEY;
use crate::picker::{OpenPicker, PICKER_KEY};

fn key(rt: &mut Runtime<Shell>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(FRAME_S);
}

fn ctrl(letter: char) -> (Key, Mods) {
    (
        Key::Command(letter),
        Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    )
}

fn chord(rt: &mut Runtime<Shell>, letter: char) {
    let (pressed, mods) = ctrl(letter);
    key(rt, pressed, mods);
}

fn press_on(rt: &mut Runtime<Shell>, element: &str) {
    let (x, y) = rt.find(element).expect(element);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(FRAME_S);
}

/// The conversation, without a GIPHY key, so nothing asks the network.
fn unkeyed() -> Runtime<Shell> {
    let mut rt = conversation("oi");
    rt.update(|shell, _tx| shell.gif.keyed = false);
    rt
}

fn tab(rt: &Runtime<Shell>) -> Option<&'static str> {
    match &rt.state.picker {
        Some(OpenPicker::Emoji(_)) => Some("emoji"),
        Some(OpenPicker::Gif(_)) => Some("gif"),
        None => None,
    }
}

fn labels(rt: &Runtime<Shell>, role: &str) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|element| element.path.contains(PICKER_KEY) && element.role == role)
        .filter_map(|element| element.label)
        .collect()
}

#[test]
fn ctrl_g_opens_the_gif_tab_on_its_search_and_again_closes_it() {
    let mut rt = unkeyed();
    chord(&mut rt, 'g');
    assert_eq!(tab(&rt), Some("gif"));
    assert!(rt.find(PICKER_KEY).is_some());
    chord(&mut rt, 'g');
    assert_eq!(tab(&rt), None);
}

#[test]
fn the_pills_tabs_switch_in_one_press_and_close_when_pressed_again() {
    let mut rt = unkeyed();
    chord(&mut rt, 'e');
    assert_eq!(tab(&rt), Some("emoji"));
    press_on(&mut rt, GIF_BUTTON_KEY);
    assert_eq!(tab(&rt), Some("gif"), "switched, not closed");
    press_on(&mut rt, EMOJI_BUTTON_KEY);
    assert_eq!(tab(&rt), Some("emoji"));
    press_on(&mut rt, EMOJI_BUTTON_KEY);
    assert_eq!(tab(&rt), None);
}

#[test]
fn a_press_outside_closes_it() {
    let mut rt = unkeyed();
    chord(&mut rt, 'g');
    press_on(
        &mut rt,
        &crate::timeline::rows::message_key(super::replying::message_id(3)),
    );
    assert_eq!(tab(&rt), None, "a press on a message");
    chord(&mut rt, 'g');
    press_on(&mut rt, crate::shell::pins::PINS_BUTTON_KEY);
    assert_eq!(
        tab(&rt),
        None,
        "a press on the pins button, which opens its panel"
    );
}

#[test]
fn without_a_key_the_search_says_so_and_the_chips_are_favorites_and_recent() {
    let mut rt = unkeyed();
    chord(&mut rt, 'g');
    assert!(rt.find(GIF_SEARCH_KEY).is_none(), "no search field");
    assert_eq!(labels(&rt, "tab"), ["Favorites 0", "Recent"]);
    let spoken = rt.accessibility_json().to_string();
    assert!(spoken.contains("GIF search isn't set up"), "{spoken}");
}

#[test]
fn a_gif_that_cannot_be_sent_says_so_and_leaves_the_tab_and_the_field() {
    // No SDK here: nothing can be queued.
    let mut rt = unkeyed();
    rt.update(|shell, _tx| {
        let gif = Gif {
            id: "abc".to_owned(),
            url: "https://media.giphy.com/media/abc/giphy.gif".to_owned(),
            page_url: "https://giphy.com/gifs/abc".to_owned(),
            title: "Cat".to_owned(),
            ..Gif::default()
        };
        shell.picker = Some(OpenPicker::Gif(GifPicker::new(
            false,
            vec![gif],
            Vec::new(),
        )));
    });
    rt.frame(FRAME_S);
    assert_eq!(labels(&rt, "option"), ["GIF, Cat"]);
    rt.update(|shell, tx| shell.gif_pressed(0, false, tx));
    rt.frame(FRAME_S);
    assert_eq!(tab(&rt), Some("gif"), "the tab stays");
    assert!(rt.state.notice.is_some(), "it says so");
    assert_eq!(super::composer::field(&rt), "oi", "the field's text stays");
}

#[test]
fn without_a_search_the_keyboard_starts_on_the_chips_and_escape_closes() {
    let mut rt = unkeyed();
    chord(&mut rt, 'g');
    assert_eq!(focused(&rt).as_deref(), Some("gif-chip-favorites"));
    press(&mut rt, Key::Escape);
    assert_eq!(tab(&rt), None);
}

#[test]
fn escape_in_the_search_closes_the_tab() {
    let mut rt = unkeyed();
    rt.update(|shell, _tx| shell.gif.keyed = true);
    rt.update(|shell, _tx| {
        shell.picker = Some(OpenPicker::Gif(GifPicker::new(
            true,
            vec![Gif::default()],
            Vec::new(),
        )));
    });
    rt.update(|_shell, tx| tx.focus(GIF_SEARCH_KEY, false));
    rt.frame(FRAME_S);
    press(&mut rt, Key::Escape);
    assert_eq!(tab(&rt), None);
}
