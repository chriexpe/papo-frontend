//! The New message picker (`docs/design.md` §10, Direct messages),
//! headless: New message opens it with the keyboard in its field, its rows
//! name each person and their server, ↑ ↓ and Enter pick one and close it,
//! and Escape closes it, giving the keyboard back to New message.

use papo_sdk_ui::DirectCandidate;
use rune::{Key, Runtime};

use super::navigation::{ana, focused, with_servers};
use super::replying::lia;
use super::*;
use crate::shell::{FIND_FIELD_KEY, NEW_MESSAGE_BUTTON_KEY};

fn candidate(server: &str, user: Uuid, name: &str, username: &str) -> DirectCandidate {
    DirectCandidate {
        account: ana(server).id,
        user,
        name: name.to_owned(),
        username: Some(username.to_owned()),
    }
}

/// Direct, with the picker open and its first answer in: Lia on Lumen and
/// on Mirante, then Yuki.
fn picker() -> Runtime<Shell> {
    let mut rt = with_servers();
    rt.update(|shell, tx| {
        shell.select_direct(tx);
        shell.new_message_pressed(tx);
        shell.people_found(
            1,
            vec![
                candidate("Lumen", lia(), "Lia", "lia"),
                candidate("Mirante", lia(), "Lia", "lia"),
                candidate("Lumen", Uuid::from_u128(0x52), "Yuki", "yuki"),
            ],
        );
    });
    rt.frame(FRAME_S);
    rt
}

fn shows(rt: &Runtime<Shell>, words: &str) -> bool {
    rt.elements().iter().any(|element| {
        element.text.as_deref() == Some(words) || element.label.as_deref() == Some(words)
    })
}

#[test]
fn new_message_opens_the_picker_with_the_keyboard_in_its_field() {
    let rt = picker();
    assert!(rt.state.shown_dm_picker().is_some());
    assert_eq!(focused(&rt).as_deref(), Some(FIND_FIELD_KEY));
    assert!(shows(&rt, "Lia @lia, Lumen"));
    assert!(shows(&rt, "Lia @lia, Mirante"), "one row per server");
}

#[test]
fn the_arrows_and_enter_pick_a_person_and_close_the_picker() {
    let mut rt = picker();
    press(&mut rt, Key::Down);
    assert_eq!(rt.state.shown_dm_picker().unwrap().highlight, 1);
    press(&mut rt, Key::Enter);
    assert!(rt.state.shown_dm_picker().is_none());
    assert!(rt.state.navigation().menu.is_none());
}

#[test]
fn escape_closes_it_and_new_message_gets_the_keyboard_back() {
    let mut rt = picker();
    press(&mut rt, Key::Escape);
    assert!(rt.state.shown_dm_picker().is_none());
    assert_eq!(focused(&rt).as_deref(), Some(NEW_MESSAGE_BUTTON_KEY));
}

#[test]
fn a_stale_answer_is_dropped_and_no_one_found_says_so() {
    let mut rt = picker();
    rt.update(|shell, _tx| {
        assert!(!shell.people_found(0, Vec::new()), "an older ask");
        assert!(shell.people_found(1, Vec::new()));
    });
    rt.frame(FRAME_S);
    assert!(shows(&rt, "No one found"));
}
