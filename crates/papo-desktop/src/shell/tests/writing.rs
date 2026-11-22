//! Markdown in place, pairs and mentions in the real field, headless: keys,
//! paste and the input method go through Rune as a window's would, so the
//! field's own sorting of typing from everything else is what is tested.

use std::sync::Arc;

use papo_sdk::{Generation, ResourceSnapshot, UserSummary, Uuid};
use rune::{Ime, Key, KeyEvent, Mods, Runtime};

use super::composer::{field, writing};
use super::{FRAME_S, Shell, press};
use crate::composer::COMPOSER_KEY;
use crate::composer::picker::PICKER_KEY;
use crate::timeline::OpenTimeline;

/// Types `text` one key at a time, as a keyboard does.
fn keys(rt: &mut Runtime<Shell>, text: &str) {
    for ch in text.chars() {
        press(rt, Key::Text(ch.to_string()));
    }
}

fn chord(rt: &mut Runtime<Shell>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(FRAME_S);
}

fn ctrl() -> Mods {
    Mods {
        cmd: true,
        ..Mods::default()
    }
}

/// The field's text with `|` at the caret.
fn caret(rt: &Runtime<Shell>) -> String {
    let edit = &rt.state.composer.shown().unwrap().1.edit;
    let (_, end) = edit.range();
    format!("{}|{}", &edit.text[..end], &edit.text[end..])
}

#[test]
fn a_typed_opener_closes_and_its_closer_steps_over() {
    let mut rt = writing("");
    keys(&mut rt, "**");
    assert_eq!(caret(&rt), "**|**");
    keys(&mut rt, "oi**");
    assert_eq!(caret(&rt), "**oi** |");
}

#[test]
fn tab_steps_past_the_closer_in_the_field() {
    let mut rt = writing("");
    keys(&mut rt, "(oi");
    press(&mut rt, Key::Tab);
    assert_eq!(caret(&rt), "(oi)|");
    assert!(
        rt.state.composer.shown().unwrap().1.edit.focused,
        "the field kept the keyboard"
    );
}

#[test]
fn a_paste_never_closes_a_pair() {
    let mut rt = writing("");
    // One character, as a typed opener would be.
    rt.paste("(".to_owned());
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "(");
    rt.paste("*".to_owned());
    rt.frame(FRAME_S);
    rt.paste("*".to_owned());
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "(**");
}

#[test]
fn an_input_method_commit_never_closes_a_pair() {
    let mut rt = writing("");
    // A typed key that changes nothing (Backspace in an empty field), then
    // a commit in the same frame: the key's mark must not carry over.
    rt.key(KeyEvent {
        key: Key::Backspace,
        mods: Mods::default(),
    });
    rt.ime(Ime::Commit("(".to_owned()));
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "(");
}

#[test]
fn undo_takes_back_the_closer_then_the_typing() {
    let mut rt = writing("");
    keys(&mut rt, "a **");
    assert_eq!(field(&rt), "a ****");
    chord(&mut rt, Key::Command('z'), ctrl());
    assert_eq!(field(&rt), "a **", "the closer goes first");
    for _ in 0..4 {
        chord(&mut rt, Key::Command('z'), ctrl());
    }
    assert_eq!(field(&rt), "", "undo still reaches the start");
}

#[test]
fn a_restored_draft_opens_no_pair() {
    let mut rt = writing("**oi");
    keys(&mut rt, "*");
    assert_eq!(caret(&rt), "**oi*|");
}

#[test]
fn a_delimiter_over_a_selection_wraps_it() {
    let mut rt = writing("oi");
    chord(&mut rt, Key::Command('a'), ctrl());
    keys(&mut rt, "~");
    assert_eq!(field(&rt), "~~oi~~");
    chord(&mut rt, Key::Command('b'), ctrl());
    assert_eq!(field(&rt), "~~**oi**~~");
}

#[test]
fn a_heading_or_bold_text_grows_no_line() {
    let height = |source: &str| {
        let mut rt = writing(source);
        rt.frame(FRAME_S);
        rt.find(COMPOSER_KEY).unwrap();
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(COMPOSER_KEY))
            .unwrap()
            .rect[3]
    };
    let plain = height("Título e corpo");
    assert_eq!(height("# Título e **corpo**"), plain);
    assert_eq!(height("`Título` e ||corpo||"), plain);
}

fn person(n: u128, name: &str) -> UserSummary {
    UserSummary {
        id: Some(Uuid::from_u128(n).to_string()),
        username: Some(name.to_owned()),
        roles: Some(Vec::new()),
        ..UserSummary::default()
    }
}

/// [`writing`], the shown channel's timeline open with a roster of Ana
/// (the account itself), Marina and Mariana.
fn with_roster(draft: &str) -> Runtime<Shell> {
    let mut rt = writing(draft);
    rt.update(|shell, _tx| {
        let key = shell.composer.shown().unwrap().0;
        let roster = ResourceSnapshot::fixture(
            Generation::for_test(1),
            None,
            Vec::new(),
            Vec::new(),
            vec![person(1, "ana"), person(2, "marina"), person(3, "mariana")],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            key.account.user(),
        );
        shell.tabs.show(Some(key), false);
        let mut open = OpenTimeline::loaded(key, imbl::Vector::new());
        open.resources = roster.ok().map(Arc::new);
        *shell.tabs.shown_mut().unwrap() = open;
    });
    rt.frame(FRAME_S);
    rt
}

#[test]
fn an_at_opens_the_picker_and_enter_picks() {
    let mut rt = with_roster("");
    keys(&mut rt, "oi @mari");
    assert!(rt.find(PICKER_KEY).is_some(), "the picker grew");
    press(&mut rt, Key::Down);
    press(&mut rt, Key::Enter);
    assert_eq!(field(&rt), "oi @mariana ", "Enter picked; it sent nothing");
    assert!(rt.find(PICKER_KEY).is_none());
    let pills: Vec<String> = rt
        .state
        .composer
        .shown()
        .unwrap()
        .1
        .edit
        .pills()
        .map(|(_, id)| id.to_owned())
        .collect();
    assert_eq!(pills, [Uuid::from_u128(3).to_string()]);
}

#[test]
fn escape_closes_the_picker_and_keeps_the_text() {
    let mut rt = with_roster("");
    keys(&mut rt, "@ma");
    press(&mut rt, Key::Escape);
    assert!(rt.find(PICKER_KEY).is_none());
    assert_eq!(field(&rt), "@ma");
    keys(&mut rt, "ri");
    assert!(rt.find(PICKER_KEY).is_none(), "it stays closed for this @");
    keys(&mut rt, " @m");
    assert!(rt.find(PICKER_KEY).is_some(), "another @ gets it back");
}

#[test]
fn a_mention_is_written_as_its_token() {
    let mut rt = with_roster("");
    keys(&mut rt, "@marin");
    press(&mut rt, Key::Tab);
    keys(&mut rt, "olá");
    let sent = rt.state.composer.wire_of_shown();
    assert_eq!(sent, format!("@mention(<@{}>) olá", Uuid::from_u128(2)));
}

#[test]
fn the_picker_leaves_with_the_caret_and_enter_goes_back_to_the_field() {
    let mut rt = with_roster("");
    // Enter adds a line, so what it did stays in the field.
    rt.update(|shell, _tx| shell.writing.enter_sends = false);
    keys(&mut rt, "oi @mar");
    assert!(rt.find(PICKER_KEY).is_some());
    press(&mut rt, Key::Left);
    assert!(rt.find(PICKER_KEY).is_none(), "the caret left the query");
    press(&mut rt, Key::Enter);
    assert_eq!(field(&rt), "oi @ma\nr", "Enter was the field's, not a pick");
    assert_eq!(rt.state.composer.shown().unwrap().1.edit.pills().count(), 0);
}

#[test]
fn a_channel_switch_closes_the_picker() {
    let mut rt = with_roster("");
    keys(&mut rt, "@mar");
    assert!(rt.find(PICKER_KEY).is_some());
    rt.update(|shell, _tx| {
        let key = shell.composer.shown().unwrap().0;
        let other = crate::timeline::TimelineKey {
            account: key.account,
            channel: Uuid::from_u128(99),
        };
        shell.composer.show(Some(other));
        shell.composer.show(Some(key));
    });
    rt.frame(FRAME_S);
    assert!(rt.find(PICKER_KEY).is_none());
}

#[test]
fn wrapping_a_selection_keeps_its_mention() {
    let mut rt = with_roster("");
    keys(&mut rt, "oi @marin");
    press(&mut rt, Key::Enter);
    // The space after the pick.
    press(&mut rt, Key::Backspace);
    chord(&mut rt, Key::Command('a'), ctrl());
    chord(&mut rt, Key::Command('i'), ctrl());
    let mention = format!("@mention(<@{}>)", Uuid::from_u128(2));
    assert_eq!(rt.state.composer.wire_of_shown(), format!("*oi {mention}*"));
    chord(&mut rt, Key::Command('i'), ctrl());
    assert_eq!(rt.state.composer.wire_of_shown(), format!("oi {mention}"));
}

#[test]
fn everyone_is_not_offered_without_the_permission() {
    let mut rt = with_roster("");
    keys(&mut rt, "@ev");
    assert!(
        rt.find(PICKER_KEY).is_none(),
        "no permission data: no @everyone"
    );
}
