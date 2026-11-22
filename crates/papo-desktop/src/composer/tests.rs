//! The composer's state: drafts load once and save after a pause, on a
//! switch and on flush; a send takes the text and never sends a blank or
//! over-long one; typing is reported at most every few seconds; a failed
//! send gives the text back.

use papo_sdk::{AccountId, ServerAddress, Uuid};

use super::{Composer, Effect, MAX_MESSAGE_CHARS, SAVE_AFTER_S, Sent, TYPING_EVERY_S};
use crate::timeline::TimelineKey;

/// A stored draft with no mentions in it.
fn plain(text: impl Into<String>) -> papo_sdk_ui::message_content::DecodedComposerMentions {
    papo_sdk_ui::message_content::DecodedComposerMentions {
        text: text.into(),
        bindings: Vec::new(),
    }
}

fn key(channel: u128) -> TimelineKey {
    let server = ServerAddress::parse("https://papo.example.invalid")
        .unwrap()
        .id();
    TimelineKey {
        account: AccountId::new(server, Uuid::nil()),
        channel: Uuid::from_u128(channel),
    }
}

/// A composer showing channel 1, its stored draft `text` loaded.
fn showing(text: &str) -> Composer {
    let mut composer = Composer::default();
    assert_eq!(composer.show(Some(key(1))), [Effect::Load(key(1))]);
    assert!(composer.loaded(key(1), plain(text.to_owned())));
    composer
}

fn type_in(composer: &mut Composer, text: &str, clock_s: f64) -> Vec<Effect> {
    composer.field_mut().insert(text);
    composer.edited(clock_s)
}

fn text(composer: &Composer) -> &str {
    &composer.shown().unwrap().1.edit.text
}

#[test]
fn a_draft_loads_once_and_the_field_waits_for_it() {
    let mut composer = Composer::default();
    assert_eq!(composer.show(Some(key(1))), [Effect::Load(key(1))]);
    assert!(composer.is_loading());
    composer.field_mut().insert("lost");
    assert_eq!(text(&composer), "", "no typing while the draft loads");
    assert!(composer.loaded(key(1), plain("rascunho".to_owned())));
    assert_eq!(text(&composer), "rascunho");
    assert!(!composer.is_loading());
    composer.show(Some(key(2)));
    assert!(
        composer.show(Some(key(1))).is_empty(),
        "shown again, it is not read again"
    );
    assert!(
        !composer.loaded(key(1), plain("older".to_owned())),
        "a late read never replaces the field"
    );
    assert_eq!(text(&composer), "rascunho");
}

#[test]
fn typing_saves_once_it_pauses() {
    let mut composer = showing("");
    let effects = type_in(&mut composer, "o", 10.0);
    assert!(effects.contains(&Effect::WakeAfter(SAVE_AFTER_S)));
    let effects = type_in(&mut composer, "i", 10.5);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::WakeAfter(_))),
        "one wake at a time"
    );
    let early = composer.poll(10.0 + SAVE_AFTER_S);
    assert_eq!(
        early,
        [Effect::WakeAfter(0.5)],
        "moved since: wait out the rest"
    );
    let saved = composer.poll(10.5 + SAVE_AFTER_S);
    assert!(
        matches!(saved.as_slice(), [Effect::Save { key: saved, text, .. }] if *saved == key(1) && text == "oi"),
        "{saved:?}"
    );
    assert!(composer.poll(20.0).is_empty(), "nothing left to save");
}

#[test]
fn leaving_a_channel_saves_it_and_keeps_its_text() {
    let mut composer = showing("");
    type_in(&mut composer, "meio", 1.0);
    let effects = composer.show(Some(key(2)));
    assert!(
        matches!(&effects[0], Effect::Save { key: saved, text, .. } if *saved == key(1) && text == "meio"),
        "{effects:?}"
    );
    assert_eq!(effects[1], Effect::Load(key(2)));
    composer.loaded(key(2), plain(String::new()));
    composer.show(Some(key(1)));
    assert_eq!(text(&composer), "meio");
}

#[test]
fn the_keyboard_follows_the_field_across_channels() {
    let mut composer = Composer::default();
    composer.focus();
    composer.show(Some(key(1)));
    assert!(
        composer.shown().unwrap().1.edit.focused,
        "the app starts there"
    );
    composer.show(Some(key(2)));
    assert!(composer.shown().unwrap().1.edit.focused);
}

#[test]
fn flushing_saves_every_unsaved_draft_once() {
    let mut composer = showing("");
    type_in(&mut composer, "a", 1.0);
    let flushed = composer.flush();
    assert_eq!(flushed.len(), 1);
    assert_eq!(composer.flush(), [], "already saved");
}

#[test]
fn later_writes_carry_later_sequence_numbers() {
    let mut composer = showing("");
    type_in(&mut composer, "a", 1.0);
    let Some(Effect::Save { seq: first, .. }) = composer.flush().pop() else {
        panic!("a save");
    };
    type_in(&mut composer, "b", 2.0);
    let Sent::Queued(Effect::Send { seq: second, .. }) = composer.send(Uuid::from_u128(9)) else {
        panic!("a send");
    };
    assert!(second > first);
}

#[test]
fn typing_is_reported_at_most_every_few_seconds() {
    let mut composer = showing("");
    let reports = |effects: Vec<Effect>| {
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::ReportTyping(_)))
            .count()
    };
    assert_eq!(reports(type_in(&mut composer, "a", 1.0)), 1);
    assert_eq!(reports(type_in(&mut composer, "b", 2.0)), 0);
    assert_eq!(
        reports(type_in(&mut composer, "c", 1.0 + TYPING_EVERY_S)),
        1
    );
    composer.field_mut().replace_text("");
    assert_eq!(
        reports(composer.edited(20.0)),
        0,
        "emptying the field is not typing"
    );
}

#[test]
fn send_takes_the_text_and_never_a_blank_one() {
    let mut composer = showing(" \n ");
    assert_eq!(composer.send(Uuid::from_u128(9)), Sent::Nothing);
    composer.field_mut().replace_text("olá");
    let sent = composer.send(Uuid::from_u128(9));
    assert_eq!(
        sent,
        Sent::Queued(Effect::Send {
            key: key(1),
            text: "olá".to_owned(),
            local: Uuid::from_u128(9),
            seq: 1,
        })
    );
    assert_eq!(text(&composer), "");
    composer.field_mut().undo();
    assert_eq!(text(&composer), "", "undo never brings a sent message back");
    assert!(
        composer.flush().is_empty(),
        "the send clears the stored draft"
    );
}

#[test]
fn an_over_long_message_stays_in_the_field() {
    let mut composer = showing(&"a".repeat(MAX_MESSAGE_CHARS));
    assert_eq!(composer.chars_over(), 0);
    assert!(composer.can_send());
    // Counted in characters, not bytes.
    composer
        .field_mut()
        .replace_text("é".repeat(MAX_MESSAGE_CHARS + 2));
    composer.edited(1.0);
    assert_eq!(composer.chars_over(), 2);
    assert_eq!(composer.send(Uuid::from_u128(9)), Sent::TooLong);
    assert_eq!(text(&composer).chars().count(), MAX_MESSAGE_CHARS + 2);
}

#[test]
fn a_failed_send_gives_the_text_back_before_what_was_typed_since() {
    let mut composer = showing("");
    composer.field_mut().replace_text("primeira");
    composer.send(Uuid::from_u128(9));
    assert!(!composer.send_failed_here());
    type_in(&mut composer, "segunda", 1.0);
    let effects = composer.send_failed(key(1), plain("primeira".to_owned()));
    assert_eq!(text(&composer), "primeira\nsegunda");
    assert!(composer.send_failed_here());
    assert!(
        matches!(effects.as_slice(), [Effect::Save { text, .. }] if text == "primeira\nsegunda"),
        "{effects:?}"
    );
    type_in(&mut composer, "!", 2.0);
    assert!(!composer.send_failed_here(), "an edit clears the line");
}

#[test]
fn removing_an_account_forgets_its_buffers() {
    let mut composer = showing("x");
    composer.retain_accounts(|_| false);
    assert!(composer.shown().is_none());
}

#[test]
fn an_unreadable_draft_is_forgotten_and_read_again_next_time() {
    let mut composer = Composer::default();
    composer.focus();
    composer.show(Some(key(1)));
    assert!(composer.load_failed(key(1)));
    assert!(
        composer.shown().is_none(),
        "no field: typing would overwrite it"
    );
    assert!(composer.flush().is_empty(), "nothing is saved over it");
    composer.show(Some(key(2)));
    assert_eq!(composer.show(Some(key(1))), [Effect::Load(key(1))]);
    assert!(
        composer.shown().unwrap().1.edit.focused,
        "the keyboard waited for the field"
    );
}

#[test]
fn a_failed_save_is_saved_again_on_flush_but_not_on_a_timer() {
    let mut composer = showing("");
    type_in(&mut composer, "a", 1.0);
    let Some(Effect::Save { seq, .. }) = composer.flush().pop() else {
        panic!("a save");
    };
    assert!(composer.save_failed(key(1), seq));
    assert!(
        !composer
            .poll(100.0)
            .iter()
            .any(|effect| matches!(effect, Effect::Save { .. })),
        "a failure never loops"
    );
    assert_eq!(composer.flush().len(), 1, "the next flush tries again");
}

#[test]
fn a_late_failure_never_undoes_a_newer_save() {
    let mut composer = showing("");
    type_in(&mut composer, "a", 1.0);
    let Some(Effect::Save { seq: older, .. }) = composer.flush().pop() else {
        panic!("a save");
    };
    type_in(&mut composer, "b", 2.0);
    composer.flush();
    assert!(!composer.save_failed(key(1), older));
    assert_eq!(composer.flush(), []);
}

#[test]
fn a_wake_a_hair_early_still_saves() {
    let mut composer = showing("");
    type_in(&mut composer, "a", 10.0);
    let effects = composer.poll(10.0 + SAVE_AFTER_S - 0.0005);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Save { .. })),
        "{effects:?}"
    );
}
