//! An audio in a message (`docs/design.md` §14.5): the player with its name, flat bars until
//! its peaks are known, its length at rest and the time played once played, its play disc and
//! its waveform as a slider, and a file card for audio that can't be read. The store's state is
//! set by hand: no GStreamer.

use std::sync::Arc;

use papo_sdk::{Message, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{TimelineItem, WaveformBuilder};
use rune::accesskit::Action;
use rune::{Key, KeyEvent, Mods, Runtime};
use serde_json::{Value, json};

use super::{Host, key, runtime, until_idle};
use crate::audio_store::{AudioKey, AudioStore};
use crate::design::MessageLayout;
use crate::media_store::MediaKey;
use crate::playback::{AudioRead, PlaybackFailure};

const MESSAGE: u128 = 1;
const AUDIO: u128 = 9_001;

fn audio_key() -> AudioKey {
    AudioKey::Received(MediaKey::attachment(key().account, Uuid::from_u128(AUDIO)))
}

fn audio() -> Value {
    json!({
        "id": Uuid::from_u128(AUDIO).to_string(),
        "mime_type": "video/mp4",
        "original_file_name": "Voice note.m4a",
        "size_bytes": 96_000,
    })
}

fn message() -> Arc<TimelineItem> {
    let id = Uuid::from_u128(MESSAGE);
    let message: Message = serde_json::from_value(json!({
        "id": id.to_string(),
        "author_id": "ana",
        "content": "",
        "created_at": "2026-11-20T14:00:00Z",
        "attachments": [audio()],
    }))
    .unwrap();
    Arc::new(TimelineItem::Message {
        reply: None,
        pending: None,
        id,
        message: Arc::new(message),
        content: Arc::new(MessageContent::parse("")),
    })
}

/// The timeline with the audio read as `outcome`, then `then` done to the store.
fn drawn(
    outcome: Result<AudioRead, PlaybackFailure>,
    then: impl FnOnce(&mut AudioStore),
) -> Runtime<Host> {
    let mut rt = runtime(
        std::iter::once(message()).collect(),
        MessageLayout::default(),
    );
    let mut store = AudioStore::default();
    store.want([audio_key()]);
    store.next_read();
    store.read(audio_key(), outcome);
    then(&mut store);
    rt.state.media.audio = store.view();
    rt.invalidate();
    until_idle(&mut rt);
    rt
}

/// Audio `duration_ms` long, its waveform read.
fn read(duration_ms: u64) -> AudioRead {
    let mut builder = WaveformBuilder::new(None);
    builder.push(&[0.5; 4_096], 1);
    AudioRead {
        duration_ms: Some(duration_ms),
        waveform: Some(builder.finish()),
    }
}

fn element(part: &str) -> String {
    format!("media-{}-0{part}", Uuid::from_u128(MESSAGE))
}

fn tree(rt: &Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

fn press_key(rt: &mut Runtime<Host>, on: &str, key: Key) {
    let on = on.to_owned();
    rt.update(move |_, tx| tx.focus(on, true));
    until_idle(rt);
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    until_idle(rt);
}

#[test]
fn an_audio_file_is_a_player_with_its_name_and_length() {
    let rt = drawn(Ok(read(80_000)), |_| {});
    let tree = tree(&rt);
    assert!(tree.contains("Audio, Voice note.m4a"), "{tree}");
    assert!(tree.contains("1:20"), "its length at rest: {tree}");
    assert!(tree.contains("Position, 0:00 of 1:20"), "{tree}");
    assert!(rt.presented(&element("-play")).is_some());
    // A player, never a file card: no Save on it (Save is in the media menu).
    assert!(!tree.contains("File, Voice note.m4a"), "{tree}");
}

#[test]
fn once_played_it_shows_the_time_played() {
    let rt = drawn(Ok(read(80_000)), |store| {
        store.play(audio_key());
        store.moved(audio_key(), 12_400);
    });
    let tree = tree(&rt);
    assert!(tree.contains("0:12"), "{tree}");
    assert!(tree.contains("Pause"), "{tree}");
}

#[test]
fn the_play_disc_and_the_waveform_take_presses_and_keys() {
    let mut rt = drawn(Ok(read(80_000)), |_| {});
    rt.accessibility_action(&element("-play"), Action::Click, None);
    until_idle(&mut rt);
    press_key(&mut rt, &element("-wave"), Key::Right);
    press_key(&mut rt, &element("-wave"), Key::Text(" ".to_owned()));
    assert_eq!(
        rt.state.controls,
        ["audio-play", "audio-seek", "audio-play"]
    );
}

#[test]
fn audio_that_cant_be_read_is_a_file_card() {
    let rt = drawn(Err(PlaybackFailure::Unsupported), |_| {});
    let tree = tree(&rt);
    assert!(!tree.contains("Audio, Voice note.m4a"), "{tree}");
    assert!(tree.contains("Voice note.m4a"), "{tree}");
    assert!(rt.presented(&element("-play")).is_none());
}

#[test]
fn before_it_is_read_it_waits_flat_without_a_time() {
    let mut rt = runtime(
        std::iter::once(message()).collect(),
        MessageLayout::default(),
    );
    rt.invalidate();
    until_idle(&mut rt);
    let tree = tree(&rt);
    assert!(tree.contains("Audio, Voice note.m4a"), "{tree}");
    // Nothing to play yet: the disc takes no press.
    assert!(rt.presented(&element("-play")).is_none());
}
