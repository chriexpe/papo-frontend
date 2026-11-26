//! Replies and edits in the composer's state: a reply is saved with the
//! draft and goes with its message; an edit is never saved, never wakes the
//! poll, and puts the field back when it ends.

use papo_sdk::{AccountId, ServerAddress, Uuid};
use papo_sdk_ui::ReplyOriginal;
use papo_sdk_ui::message_content::CustomEmojis;
use papo_sdk_ui::message_content::DecodedComposerMentions;

use crate::composer::{Composer, Effect, Sent};
use crate::timeline::TimelineKey;

fn plain(text: &str) -> DecodedComposerMentions {
    DecodedComposerMentions {
        text: text.to_owned(),
        bindings: Vec::new(),
    }
}

fn key() -> TimelineKey {
    let server = ServerAddress::parse("https://papo.example.invalid")
        .unwrap()
        .id();
    TimelineKey {
        account: AccountId::new(server, Uuid::nil()),
        channel: Uuid::from_u128(1),
    }
}

fn original() -> Uuid {
    Uuid::from_u128(9)
}

/// A composer showing the channel, its stored draft `text` loaded.
fn showing(text: &str) -> Composer {
    let mut composer = Composer::default();
    composer.show(Some(key()));
    composer.loaded(key(), plain(text), None, true, Vec::new());
    composer
}

fn text(composer: &Composer) -> &str {
    &composer.shown().unwrap().1.edit.text
}

/// What the saves among `effects` stored as the reply, one entry a save.
fn saved_replies(effects: &[Effect]) -> Vec<Option<Uuid>> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Save { reply_to, .. } => Some(*reply_to),
            _ => None,
        })
        .collect()
}

#[test]
fn a_reply_is_saved_with_the_draft_and_ending_it_saves_again() {
    let mut composer = showing("oi");
    let started = composer.start_reply(key(), original(), None, true);
    assert_eq!(saved_replies(&started), [Some(original())]);
    let ended = composer.cancel_reply().expect("a reply ended");
    assert_eq!(saved_replies(&ended), [None]);
    assert_eq!(composer.cancel_reply(), None, "none is left");
    assert_eq!(text(&composer), "oi", "the text stays");
}

#[test]
fn a_reply_goes_with_its_message_and_comes_back_if_it_fails() {
    let mut composer = showing("");
    composer.start_reply(key(), original(), None, true);
    composer.field_mut().insert("agreed");
    composer.edited(1.0, &CustomEmojis::default());
    let Sent::Queued(Effect::Send { reply_to, .. }) = composer.send(Uuid::from_u128(5)) else {
        panic!("sent");
    };
    assert_eq!(reply_to, Some(original()));
    assert_eq!(composer.shown().unwrap().1.mode.reply().map(|r| r.id), None);
    composer.send_failed(key(), plain("agreed"), reply_to, true, Vec::new());
    assert_eq!(
        composer.shown().unwrap().1.mode.reply().map(|r| r.id),
        Some(original()),
        "the reply came back with its text"
    );
}

#[test]
fn an_edit_is_never_saved_and_never_wakes_the_poll() {
    let mut composer = showing("half a thought");
    composer.start_edit(key(), original(), plain("the message"));
    composer.field_mut().insert("!");
    assert_eq!(
        composer.edited(2.0, &CustomEmojis::default()),
        [],
        "no save, no typing report"
    );
    assert_eq!(composer.poll(10.0), []);
    assert_eq!(composer.flush(), []);
}

#[test]
fn saving_an_edit_queues_it_and_puts_the_draft_back() {
    let mut composer = showing("half a thought");
    composer.start_reply(key(), Uuid::from_u128(7), None, true);
    composer.start_edit(key(), original(), plain("the message"));
    assert_eq!(text(&composer), "the message");
    composer.field_mut().insert("!");
    composer.edited(2.0, &CustomEmojis::default());
    let Sent::Edited {
        edit: Effect::Edit {
            id, text: edited, ..
        },
        save: None,
    } = composer.send(Uuid::from_u128(5))
    else {
        panic!("an edit");
    };
    assert_eq!((id, edited.as_str()), (original(), "the message!"));
    assert_eq!(text(&composer), "half a thought", "the draft came back");
    assert_eq!(
        composer.shown().unwrap().1.mode.reply().map(|r| r.id),
        Some(Uuid::from_u128(7)),
        "and its reply"
    );
}

#[test]
fn an_emptied_edit_asks_to_delete_and_stays_open() {
    let mut composer = showing("");
    composer.start_edit(key(), original(), plain("x"));
    composer.field_mut().replace_text("");
    composer.edited(2.0, &CustomEmojis::default());
    assert_eq!(
        composer.send(Uuid::from_u128(5)),
        Sent::DeleteInstead {
            key: key(),
            id: original()
        }
    );
    assert_eq!(composer.shown().unwrap().1.mode.editing(), Some(original()));
}

#[test]
fn a_restored_reply_waits_for_its_original() {
    let mut composer = Composer::default();
    composer.show(Some(key()));
    composer.loaded(key(), plain("oi"), Some(original()), false, Vec::new());
    assert_eq!(composer.unresolved_replies(), [(key(), original())]);
    assert!(composer.reply_found(key(), original(), ReplyOriginal::Unknown));
    assert_eq!(composer.unresolved_replies(), []);
    let reply = composer
        .shown()
        .unwrap()
        .1
        .mode
        .reply()
        .map(|reply| reply.mention);
    assert_eq!(
        reply,
        Some(false),
        "the @ switch starts where the setting says"
    );
}

#[test]
fn a_send_refused_during_an_edit_waits_in_the_draft_it_set_aside() {
    let mut composer = showing("");
    composer.start_reply(key(), Uuid::from_u128(7), None, true);
    composer.field_mut().insert("sent");
    composer.edited(1.0, &CustomEmojis::default());
    let Sent::Queued(Effect::Send { reply_to, .. }) = composer.send(Uuid::from_u128(5)) else {
        panic!("sent");
    };
    composer.start_edit(key(), original(), plain("the message"));
    assert_eq!(
        composer.send_failed(key(), plain("sent"), reply_to, true, Vec::new()),
        []
    );
    assert_eq!(text(&composer), "the message", "the edit is untouched");
    assert!(
        !composer.send_failed_here(),
        "the note waits for the edit to end"
    );
    let saved = composer.cancel_edit().expect("an edit ended");
    assert_eq!(text(&composer), "sent", "the refused text came back");
    assert_eq!(
        saved_replies(&saved),
        [Some(Uuid::from_u128(7))],
        "and was saved"
    );
    assert!(composer.send_failed_here());
}

#[test]
fn a_tray_change_during_an_edit_saves_the_field_set_aside_not_the_edit() {
    use crate::composer::tray::{Tile, TileKind, TileSource};
    let mut composer = showing("my draft");
    let id = composer.next_tile_id();
    let tile = Tile {
        id,
        name: "a.pdf".to_owned(),
        size_bytes: Some(1),
        kind: TileKind::File,
        source: TileSource::Fixture,
        thumbnail: None,
        video: None,
        inspection: crate::composer::tray::Inspection::Pending,
        edit: papo_sdk::AttachmentEdit::default(),
        typed_name: None,
    };
    let _ = composer.attach(key(), vec![tile]);
    composer.start_edit(key(), original(), plain("the message"));
    // The file's copy failed during the edit: it leaves the tray.
    let (_, effects) = composer.detach(key(), id);
    let saved: Vec<(String, usize)> = effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Save { text, tiles, .. } => Some((text, tiles.len())),
            _ => None,
        })
        .collect();
    assert_eq!(saved, [("my draft".to_owned(), 0)]);
    assert_eq!(
        text(&composer),
        "the message",
        "the edit stays in the field"
    );
}
