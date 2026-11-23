//! Someone else's profile card, headless: opened from an author's name and
//! from a reactor's row, its actions as the account may use them, and where
//! the keyboard goes.

use std::sync::Arc;

use super::navigation::click;
use super::replying::{conversation, lia, resources_owned_by};
use super::*;
use crate::timeline::rows::author_key;

/// The fixture's message `n`.
fn message_id(n: u128) -> Uuid {
    Uuid::from_u128(0x3000 + n)
}

/// Lia's message 1's author name, pressed.
fn open_lias_card(rt: &mut Runtime<Shell>) {
    click(rt, &author_key(message_id(1)));
    rt.frame(FRAME_S);
}

fn shown(rt: &Runtime<Shell>, key: &str) -> bool {
    rt.elements()
        .into_iter()
        .any(|element| element.key.as_deref() == Some(key) && !element.exiting)
}

fn focused(rt: &Runtime<Shell>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|element| element.focused)
        .and_then(|element| element.key)
}

#[test]
fn a_persons_name_opens_their_card_and_escape_gives_the_keyboard_back_to_what_had_it() {
    let mut rt = conversation("");
    open_lias_card(&mut rt);
    let tree = tree(&rt);
    assert!(tree.contains("lia's profile"), "the card names who it is");
    assert!(tree.contains("@lia"));
    assert!(shown(&rt, "card-message"));
    assert_eq!(focused(&rt).as_deref(), Some("card-message"));
    assert!(shown(&rt, "card-block"));
    assert!(!shown(&rt, "card-ban"), "not a moderator: no Ban");
    press(&mut rt, Key::Escape);
    assert!(rt.state.navigation().menu.is_none());
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
}

#[test]
fn your_own_card_offers_only_copy_id() {
    let mut rt = conversation("");
    // Message 2 is yours.
    click(&mut rt, &author_key(message_id(2)));
    rt.frame(FRAME_S);
    assert!(shown(&rt, "card-copy-id"));
    assert!(!shown(&rt, "card-message"));
    assert!(!shown(&rt, "card-block"));
    assert_eq!(
        focused(&rt).as_deref(),
        Some("card-copy-id"),
        "opens on what it has"
    );
}

#[test]
fn a_moderator_is_asked_inside_the_ban_row_and_cancel_puts_it_back() {
    let mut rt = conversation("");
    rt.update(|shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        let me = open.key.account.user();
        open.resources = Some(Arc::new(resources_owned_by(open.key, me)));
    });
    open_lias_card(&mut rt);
    assert!(shown(&rt, "card-ban"));
    click(&mut rt, "card-ban");
    rt.frame(FRAME_S);
    assert!(tree(&rt).contains("Ban lia from Lumen?"));
    assert!(
        !shown(&rt, "card-ban"),
        "the question takes the row's place"
    );
    assert_eq!(focused(&rt).as_deref(), Some("card-ban-cancel"));
    click(&mut rt, "card-ban-cancel");
    rt.frame(FRAME_S);
    assert!(shown(&rt, "card-ban"));
    assert_eq!(focused(&rt).as_deref(), Some("card-ban"));
    assert!(rt.state.navigation().menu.is_some(), "the card stays");
}

#[test]
fn copy_id_copies_the_persons_id_and_closes_the_card() {
    let mut rt = conversation("");
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    open_lias_card(&mut rt);
    click(&mut rt, "card-copy-id");
    rt.frame(FRAME_S);
    assert_eq!(
        clipboard.text().map(|text| text.to_string()),
        Some(lia().to_string())
    );
    assert!(rt.state.navigation().menu.is_none());
}

#[test]
fn a_reactors_row_turns_the_list_into_their_card_and_escape_turns_it_back() {
    let mut rt = conversation("");
    rt.update(|shell, tx| {
        let key = shell.tabs.shown().unwrap().key;
        let mut menu = crate::menu::OpenMenu::with_entries(
            crate::menu::Placement::Beside("anchor".to_owned()),
            COMPOSER_KEY.to_owned(),
            Vec::new(),
            0.0,
        );
        menu.reactions = Some(crate::menu::ReactionsView {
            key,
            id: message_id(1),
            filter: None,
            from_chip: true,
        });
        shell.nav.menu = Some(menu);
        shell.reactor_pressed(lia(), tx);
    });
    rt.frame(FRAME_S);
    let card = |rt: &Runtime<Shell>| {
        rt.state
            .navigation()
            .menu
            .as_ref()
            .and_then(|menu| menu.card.as_ref())
            .map(|card| card.user)
    };
    assert_eq!(card(&rt), Some(lia()));
    press(&mut rt, Key::Escape);
    assert_eq!(card(&rt), None);
    assert!(
        rt.state
            .navigation()
            .menu
            .as_ref()
            .is_some_and(|menu| menu.reactions.is_some()),
        "back to the list"
    );
}

#[test]
fn a_card_whose_conversation_went_closes_and_leaves_the_keys_free() {
    let mut rt = conversation("");
    open_lias_card(&mut rt);
    assert!(rt.state.navigation().menu.is_some());
    // The conversation goes (a server removed, a sign-out).
    rt.update(|shell, tx| {
        shell.tabs.show(None, false);
        // The window's poll, as Rune runs it each frame.
        shell.poll(0.0, tx);
    });
    assert!(rt.state.navigation().menu.is_none());
}

#[test]
fn a_persons_avatar_opens_their_card_too() {
    let mut rt = conversation("");
    // The avatar is hidden from assistive technology (the name opens the
    // card there): a pointer press.
    let key = crate::timeline::rows::avatar_key(message_id(1));
    let [x, y, w, h] = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some(key.as_str()))
        .expect("message 1's avatar")
        .rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.frame(FRAME_S);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(FRAME_S);
    let card = rt
        .state
        .navigation()
        .menu
        .as_ref()
        .and_then(|menu| menu.card.as_ref())
        .map(|card| card.user);
    assert_eq!(card, Some(lia()));
}

#[test]
fn view_profile_in_the_message_menu_turns_it_into_the_authors_card() {
    let mut rt = conversation("");
    let id = message_id(3);
    rt.update(move |_shell, tx| tx.focus(crate::timeline::rows::message_key(id), true));
    rt.frame(FRAME_S);
    press(&mut rt, Key::Enter);
    click(&mut rt, "menu-view-profile");
    rt.frame(FRAME_S);
    let card = rt
        .state
        .navigation()
        .menu
        .as_ref()
        .and_then(|menu| menu.card.as_ref())
        .map(|card| card.user);
    assert_eq!(card, Some(lia()));
    assert_eq!(focused(&rt).as_deref(), Some("card-message"));
}
