//! Alt+↑ and Alt+↓ between channels, headless: focus decides (a focused
//! rail tile moves itself; from anywhere else the keys change channel).

use rune::accesskit::Action;

use super::navigation::{click, focused, with_servers};
use super::*;

fn alt(rt: &mut Runtime<Shell>, key: Key, shift: bool) {
    rt.key(KeyEvent {
        key,
        mods: Mods {
            alt: true,
            shift,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
}

fn channel(rt: &Runtime<Shell>) -> Option<&str> {
    rt.state.nav.selected_channel()
}

#[test]
fn alt_arrows_step_through_the_channels_and_stop_at_the_ends() {
    let mut rt = with_servers();
    assert_eq!(channel(&rt), Some("Lumen-general"));
    alt(&mut rt, Key::Up, false);
    assert_eq!(channel(&rt), Some("Lumen-general"), "the first stays");
    alt(&mut rt, Key::Down, false);
    assert_eq!(channel(&rt), Some("Lumen-media"));
    alt(&mut rt, Key::Down, false);
    assert_eq!(
        channel(&rt),
        Some("Lumen-hangout"),
        "then the voice channels"
    );
    alt(&mut rt, Key::Down, false);
    assert_eq!(channel(&rt), Some("Lumen-hangout"), "the last stays");
    alt(&mut rt, Key::Up, false);
    assert_eq!(channel(&rt), Some("Lumen-media"));
}

#[test]
fn alt_arrows_change_channel_from_a_field_holding_several_lines() {
    use super::composer::{channel_id, writing};
    let mut rt = writing("one\ntwo\nthree");
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
    // The caret on a middle line, where a field's own arrows would move it.
    press(&mut rt, Key::Up);
    alt(&mut rt, Key::Down, false);
    assert_eq!(channel(&rt), Some(channel_id(2).as_str()));
    alt(&mut rt, Key::Up, false);
    assert_eq!(channel(&rt), Some(channel_id(1).as_str()));
}

#[test]
fn alt_arrows_on_a_channel_row_keep_the_keyboard_on_the_selected_row() {
    let mut rt = with_servers();
    let general = crate::channel_pane::channel_key("Lumen-general");
    rt.accessibility_action(&general, Action::Focus, None);
    rt.frame(FRAME_S);
    alt(&mut rt, Key::Down, false);
    assert_eq!(channel(&rt), Some("Lumen-media"));
    let media = crate::channel_pane::channel_key("Lumen-media");
    let row = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(media.as_str()))
        .unwrap();
    assert!(row.focused && row.focus_visible, "the ring shows on it");
}

#[test]
fn alt_shift_arrows_and_an_open_menu_leave_the_channel_alone() {
    let mut rt = with_servers();
    alt(&mut rt, Key::Down, true);
    assert_eq!(channel(&rt), Some("Lumen-general"), "Alt+⇧ is another key");
    click(&mut rt, "server-options");
    alt(&mut rt, Key::Down, false);
    assert_eq!(channel(&rt), Some("Lumen-general"), "behind the menu");
}

#[test]
fn alt_arrows_leave_the_channel_alone_behind_the_server_s_accounts() {
    let mut rt = with_servers();
    rt.update(|shell, _tx| shell.nav.open_accounts());
    rt.frame(FRAME_S);
    alt(&mut rt, Key::Down, false);
    assert_eq!(channel(&rt), Some("Lumen-general"));
}

#[test]
fn the_field_gets_the_keyboard_back_after_passing_a_voice_channel() {
    use super::composer::{channel_id, load, writing};
    let mut rt = writing("");
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
    alt(&mut rt, Key::Down, false);
    load(&mut rt, "");
    alt(&mut rt, Key::Down, false);
    assert_eq!(channel(&rt), Some("Lumen-hangout"), "no field there");
    alt(&mut rt, Key::Up, false);
    assert_eq!(channel(&rt), Some(channel_id(2).as_str()));
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
}
