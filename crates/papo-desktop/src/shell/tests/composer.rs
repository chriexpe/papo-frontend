//! The composer in the shell, headless: it shows for a text channel, takes
//! typing once the draft is read, composes through the input method, sends
//! on Enter or Shift+Enter as the setting says, keeps each channel's text
//! across switches, and lets the window rest with the field focused.

use rune::{Ime, Key, KeyEvent, Mods, Runtime};

use super::navigation::{click, entry, focused};
use super::{FRAME_S, Shell, app, press, tree, type_text};
use crate::composer::COMPOSER_KEY;
use crate::server_list::{ChannelEntry, ServerList};
use crate::timeline::TimelineKey;

/// A stored draft with no mentions in it.
pub(super) fn plain(
    text: impl Into<String>,
) -> papo_sdk_ui::message_content::DecodedComposerMentions {
    papo_sdk_ui::message_content::DecodedComposerMentions {
        text: text.into(),
        bindings: Vec::new(),
    }
}

/// A channel's ID as the server list carries it: a UUID.
pub(super) fn channel_id(n: u128) -> String {
    papo_sdk::Uuid::from_u128(n).to_string()
}

/// The app showing Lumen's #general (#media beside it), its draft still
/// being read.
pub(super) fn in_general() -> Runtime<Shell> {
    let mut rt = app();
    let mut lumen = entry("Lumen", &[]);
    lumen.text_channels = ["general", "media"]
        .iter()
        .zip(1..)
        .map(|(name, n)| ChannelEntry {
            id: channel_id(n),
            name: (*name).to_owned(),
            badge: crate::badges::Badge::default(),
        })
        .collect();
    rt.update(|shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![lumen],
                direct: crate::badges::Badge::default(),
                account_badges: std::collections::BTreeMap::new(),
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    rt
}

/// [`in_general`], its stored draft `draft` read.
pub(super) fn writing(draft: &str) -> Runtime<Shell> {
    let mut rt = in_general();
    load(&mut rt, draft);
    rt
}

/// The SDK answers the shown channel's draft read with `draft`.
pub(super) fn load(rt: &mut Runtime<Shell>, draft: &str) {
    let draft = draft.to_owned();
    rt.update(move |shell, _tx| {
        let key = shown(shell);
        shell.composer.loaded(key, plain(draft), None, true);
    });
    rt.frame(FRAME_S);
}

pub(super) fn shown(shell: &Shell) -> TimelineKey {
    shell.composer.shown().unwrap().0
}

pub(super) fn field(rt: &Runtime<Shell>) -> String {
    rt.state.composer.shown().unwrap().1.edit.text.clone()
}

fn shift_enter(rt: &mut Runtime<Shell>) {
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
}

#[test]
fn no_channel_no_composer() {
    let rt = app();
    assert!(rt.find(COMPOSER_KEY).is_none());
}

#[test]
fn the_field_has_the_keyboard_from_the_start_and_takes_typing() {
    let mut rt = writing("");
    assert_eq!(focused(&rt).as_deref(), Some(COMPOSER_KEY));
    type_text(&mut rt, "olá");
    assert_eq!(field(&rt), "olá");
}

#[test]
fn the_stored_draft_comes_back_and_typing_waits_for_it() {
    let mut rt = in_general();
    assert!(rt.state.composer.is_loading());
    type_text(&mut rt, "x");
    load(&mut rt, "rascunho");
    assert_eq!(field(&rt), "rascunho");
    type_text(&mut rt, "!");
    assert_eq!(field(&rt), "rascunho!", "the caret is at its end");
}

#[test]
fn the_field_is_labelled_and_names_the_channel() {
    let rt = writing("");
    let tree = tree(&rt);
    assert!(tree.contains(&format!("\"{COMPOSER_KEY}\"")), "{tree}");
    assert!(tree.contains("\"Message\""), "the field is named: {tree}");
    assert!(tree.contains("Message #general"), "the hint: {tree}");
    assert!(tree.contains("\"Send\""), "{tree}");
}

#[test]
fn an_input_method_composition_commits_into_the_field() {
    let mut rt = writing("");
    rt.ime(Ime::Preedit("にほん".to_owned(), Some((9, 9)), Vec::new()));
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "", "a preedit is drawn, not inserted");
    rt.ime(Ime::Commit("日本".to_owned()));
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "日本");
}

#[test]
fn a_cancelled_composition_leaves_the_text_as_it_was() {
    let mut rt = writing("");
    type_text(&mut rt, "a");
    rt.ime(Ime::Preedit("か".to_owned(), Some((3, 3)), Vec::new()));
    rt.frame(FRAME_S);
    rt.ime(Ime::Preedit(String::new(), None, Vec::new()));
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "a");
}

#[test]
fn enter_during_a_composition_sends_nothing() {
    let mut rt = writing("");
    type_text(&mut rt, "a");
    rt.ime(Ime::Preedit("か".to_owned(), Some((3, 3)), Vec::new()));
    rt.frame(FRAME_S);
    press(&mut rt, Key::Enter);
    assert_eq!(field(&rt), "a", "the field kept its text: nothing was sent");
    rt.ime(Ime::Commit("か".to_owned()));
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "aか");
}

#[test]
fn enter_sends_and_shift_enter_adds_a_line() {
    let mut rt = writing("");
    type_text(&mut rt, "oi");
    shift_enter(&mut rt);
    type_text(&mut rt, "tudo bem");
    assert_eq!(field(&rt), "oi\ntudo bem");
    press(&mut rt, Key::Enter);
    assert_eq!(field(&rt), "", "sent");
    press(&mut rt, Key::Enter);
    assert_eq!(field(&rt), "", "an empty field sends nothing");
}

#[test]
fn with_enter_to_send_off_the_keys_swap() {
    let mut rt = writing("");
    rt.update(|shell, _tx| shell.writing.enter_sends = false);
    rt.frame(FRAME_S);
    type_text(&mut rt, "oi");
    press(&mut rt, Key::Enter);
    type_text(&mut rt, "tchau");
    assert_eq!(field(&rt), "oi\ntchau");
    shift_enter(&mut rt);
    assert_eq!(field(&rt), "", "sent");
}

#[test]
fn ctrl_enter_sends_whatever_enter_does() {
    for enter_sends in [true, false] {
        let mut rt = writing("");
        rt.update(move |shell, _tx| shell.writing.enter_sends = enter_sends);
        rt.frame(FRAME_S);
        type_text(&mut rt, "oi");
        rt.key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                cmd: true,
                ctrl: true,
                ..Mods::default()
            },
        });
        rt.frame(FRAME_S);
        assert_eq!(field(&rt), "", "sent with Enter to send {enter_sends}");
    }
}

#[test]
fn send_is_a_button_that_sends() {
    let mut rt = writing("");
    type_text(&mut rt, "oi");
    click(&mut rt, "composer-send");
    assert_eq!(field(&rt), "");
}

#[test]
fn an_over_long_message_says_so_and_does_not_send() {
    let mut rt = writing(&"a".repeat(crate::composer::MAX_MESSAGE_CHARS));
    type_text(&mut rt, "bc");
    assert!(
        tree(&rt).contains("2 characters over the limit"),
        "{}",
        tree(&rt)
    );
    press(&mut rt, Key::Enter);
    assert_eq!(
        field(&rt).chars().count(),
        crate::composer::MAX_MESSAGE_CHARS + 2
    );
}

#[test]
fn each_channel_keeps_its_text_across_a_switch() {
    let mut rt = writing("");
    type_text(&mut rt, "no geral");
    click(&mut rt, &format!("channel-{}", channel_id(2)));
    load(&mut rt, "");
    assert_eq!(field(&rt), "");
    type_text(&mut rt, "na mídia");
    click(&mut rt, &format!("channel-{}", channel_id(1)));
    assert_eq!(field(&rt), "no geral");
    click(&mut rt, &format!("channel-{}", channel_id(2)));
    assert_eq!(field(&rt), "na mídia");
}

#[test]
fn a_draft_that_cannot_be_read_is_never_written_over() {
    let mut rt = in_general();
    rt.update(|shell, _tx| {
        let key = shown(shell);
        shell.composer.load_failed(key);
    });
    rt.frame(FRAME_S);
    assert!(rt.find(COMPOSER_KEY).is_none(), "no field to type into");
    // Shown again, the draft is read again.
    click(&mut rt, &format!("channel-{}", channel_id(2)));
    click(&mut rt, &format!("channel-{}", channel_id(1)));
    assert!(rt.state.composer.is_loading());
}

#[test]
fn holding_enter_adds_a_line_per_repeat_when_enter_does_not_send() {
    let mut rt = writing("");
    rt.update(|shell, _tx| shell.writing.enter_sends = false);
    rt.frame(FRAME_S);
    type_text(&mut rt, "a");
    press(&mut rt, Key::Enter);
    rt.repeat_key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "a\n\n");
}

#[test]
fn a_failed_send_puts_the_text_back_and_says_so() {
    let mut rt = writing("");
    type_text(&mut rt, "oi");
    press(&mut rt, Key::Enter);
    rt.update(|shell, _tx| {
        let key = shown(shell);
        shell
            .composer
            .send_failed(key, plain("oi".to_owned()), None, true);
    });
    rt.frame(FRAME_S);
    assert_eq!(field(&rt), "oi");
    assert!(
        tree(&rt).contains("Couldn't send. Your message is back in the field."),
        "{}",
        tree(&rt)
    );
}

#[test]
fn an_idle_window_with_the_field_focused_asks_for_nothing() {
    let mut rt = writing("");
    rt.poll = Some(Shell::poll);
    type_text(&mut rt, "oi");
    assert!(rt.wake_deadline().is_some(), "typing arms the draft's save");
    // A second and a little more: the save fires.
    for _ in 0..90 {
        rt.frame(FRAME_S);
    }
    assert!(rt.state.composer.flush().is_empty(), "the draft was saved");
    assert!(!rt.frame(FRAME_S), "no frame asked for");
    let midnight_s = rt.state.day_ends_s.expect("the day's end is known");
    assert!(
        rt.wake_deadline()
            .is_none_or(|due_s| due_s >= midnight_s - 1.0),
        "only midnight is awaited: {:?}",
        rt.wake_deadline()
    );
}

#[test]
fn a_long_unbroken_word_wraps_inside_the_pill() {
    let mut rt = writing("");
    type_text(&mut rt, &"a".repeat(400));
    // The pill's height springs to the new lines.
    for _ in 0..60 {
        rt.frame(FRAME_S);
    }
    let rect = |key: &str| {
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(key))
            .unwrap()
            .rect
    };
    let pill = rect("composer-pill");
    let field = rect(COMPOSER_KEY);
    assert!(
        field[0] + field[2] <= pill[0] + pill[2],
        "the field {field:?} stays inside the pill {pill:?}"
    );
    assert!(field[3] > 30.0, "it wrapped onto more lines: {field:?}");
}

#[test]
fn a_field_past_its_lines_scrolls_under_the_wheel() {
    let mut rt = writing("");
    let lines: Vec<String> = (1..=30).map(|n| format!("linha {n}")).collect();
    type_text(&mut rt, &lines.join("\n"));
    for _ in 0..60 {
        rt.frame(FRAME_S);
    }
    let rect = |rt: &Runtime<Shell>, key: &str| {
        rt.elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some(key))
            .unwrap()
            .rect
    };
    let at_caret = rect(&rt, COMPOSER_KEY)[1];
    let pill = rect(&rt, "composer-pill");
    for _ in 0..10 {
        rt.wheel(pill[0] + 40.0, pill[1] + pill[3] / 2.0, 120.0);
        rt.frame(FRAME_S);
    }
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    let scrolled = rect(&rt, COMPOSER_KEY)[1];
    assert!(
        scrolled > at_caret + 1.0,
        "the wheel moved the text up the lines"
    );
    // What KWin's text input sends a focused field with no composition.
    rt.ime(Ime::Preedit(String::new(), None, Vec::new()));
    for _ in 0..30 {
        rt.frame(FRAME_S);
    }
    let after = rect(&rt, COMPOSER_KEY)[1];
    assert!(
        (after - scrolled).abs() < 1.0,
        "an empty preedit leaves the scroll alone"
    );
}

#[test]
fn an_account_switch_shows_the_other_accounts_draft_not_this_ones() {
    let mut rt = writing("half a thought");
    let before = shown(&rt.state);
    // Lumen now shows another account, as a switch does.
    rt.update(|shell, tx| {
        let mut lumen = shell.navigation().servers().servers[0].clone();
        let other = crate::server_list::AccountEntry {
            id: papo_sdk::AccountId::new(lumen.id, papo_sdk::Uuid::from_u128(0x77)),
            name: "Ana Two".to_owned(),
            username: "ana2".to_owned(),
        };
        lumen.account = Some(other.clone());
        lumen.accounts.push(other);
        shell.servers_changed(
            ServerList {
                servers: vec![lumen],
                direct: crate::badges::Badge::default(),
                account_badges: std::collections::BTreeMap::new(),
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    let after = shown(&rt.state);
    assert_eq!(after.channel, before.channel, "the same channel");
    assert_ne!(after.account, before.account, "as the other account");
    load(&mut rt, "");
    assert_eq!(field(&rt), "", "the other account's own draft");
}
