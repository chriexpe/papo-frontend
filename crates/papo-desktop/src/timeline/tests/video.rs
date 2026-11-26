//! A video in a message (`docs/design.md` §14.5): its marks at rest while it plays muted, the
//! play button of one that waits, the control bar with the pointer or keyboard on it, its keys,
//! its failures, and a video in an album. The store's state is set by hand: no GStreamer.

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{Message, Uuid};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;
use rune::accesskit::Action;
use rune::{Key, KeyEvent, Mods, Runtime};
use serde_json::{Value, json};

use super::{Host, key, runtime, until_idle};
use crate::design::MessageLayout;
use crate::media_store::MediaKey;
use crate::playback::{MediaInfo, PlaybackFailure};
use crate::video_store::{VideoStore, VideoWant};

const MESSAGE: u128 = 1;

fn video_id(n: u128) -> Uuid {
    Uuid::from_u128(8_000 + n)
}

fn media_key(n: u128) -> MediaKey {
    MediaKey::attachment(key().account, video_id(n))
}

fn video(n: u128) -> Value {
    json!({
        "id": video_id(n).to_string(),
        "mime_type": "video/mp4",
        "original_file_name": format!("clip-{n}.mp4"),
        "size_bytes": 540_929,
        "width": 640,
        "height": 360,
    })
}

fn message(attachments: Value) -> Arc<TimelineItem> {
    let id = Uuid::from_u128(MESSAGE);
    let mut wire = json!({
        "id": id.to_string(),
        "author_id": "ana",
        "content": "",
        "created_at": "2026-11-20T14:00:00Z",
    });
    wire["attachments"] = attachments;
    let message: Message = serde_json::from_value(wire).unwrap();
    Arc::new(TimelineItem::Message {
        reply: None,
        pending: None,
        id,
        message: Arc::new(message),
        content: Arc::new(MessageContent::parse("")),
    })
}

/// The timeline with `attachments`, its videos read as `outcome` and shown whole.
fn drawn(
    attachments: Value,
    videos: &[u128],
    outcome: Result<MediaInfo, PlaybackFailure>,
) -> Runtime<Host> {
    let mut rt = runtime(
        std::iter::once(message(attachments)).collect(),
        MessageLayout::default(),
    );
    let mut store = VideoStore::default();
    store.set_autoplay(true);
    let in_album = videos.len() > 1;
    let mut jobs = store.want(videos.iter().map(|n| VideoWant {
        key: media_key(*n),
        box_px: [448, 252],
        in_album,
        hidden: false,
    }));
    while let Some(job) = jobs.pop() {
        jobs.extend(store.probed(job.key, outcome));
    }
    let shares: HashMap<MediaKey, f32> = videos.iter().map(|n| (media_key(*n), 1.0)).collect();
    store.shown(&shares);
    rt.state.media.videos = store.view();
    rt.invalidate();
    until_idle(&mut rt);
    rt
}

fn clip(duration_ms: u64) -> MediaInfo {
    MediaInfo {
        duration_ms: Some(duration_ms),
        video_px: Some([640, 360]),
        has_audio: true,
    }
}

fn element(n: usize, part: &str) -> String {
    format!("media-{}-{n}{part}", Uuid::from_u128(MESSAGE))
}

fn tree(rt: &Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

fn press_key(rt: &mut Runtime<Host>, on: &str, typed: &str) {
    let on = on.to_owned();
    rt.update(move |_, tx| tx.focus(on, true));
    until_idle(rt);
    rt.key(KeyEvent {
        key: Key::Text(typed.to_owned()),
        mods: Mods::default(),
    });
    until_idle(rt);
}

#[test]
fn a_short_clip_plays_muted_with_its_marks_at_rest() {
    let rt = drawn(json!([video(1)]), &[1], Ok(clip(8_050)));
    let tree = tree(&rt);
    assert!(tree.contains("Video, 0:08, muted"), "{tree}");
    assert!(
        rt.presented(&element(0, "-sound")).is_some(),
        "the sound-off button"
    );
    // No play button: it plays.
    assert!(rt.presented(&element(0, "-play")).is_none());
}

#[test]
fn a_long_video_waits_with_its_play_button_and_length() {
    let mut rt = drawn(json!([video(1)]), &[1], Ok(clip(80_000)));
    let tree = tree(&rt);
    assert!(tree.contains("Video, 1:20"), "{tree}");
    assert!(!tree.contains("muted"), "{tree}");
    assert!(rt.presented(&element(0, "-sound")).is_none());
    rt.accessibility_action(&element(0, "-play"), Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(rt.state.controls, ["video-play"]);
}

#[test]
fn the_pointer_or_keyboard_on_it_shows_the_control_bar() {
    let mut rt = drawn(json!([video(1)]), &[1], Ok(clip(8_050)));
    rt.state.media.video_attention = Some(media_key(1));
    rt.invalidate();
    until_idle(&mut rt);
    for part in ["-playing", "-seek", "-sound", "-viewer"] {
        assert!(rt.presented(&element(0, part)).is_some(), "{part}");
    }
    rt.accessibility_action(&element(0, "-playing"), Action::Click, None);
    rt.accessibility_action(&element(0, "-sound"), Action::Click, None);
    rt.accessibility_action(&element(0, "-viewer"), Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(
        rt.state.controls,
        ["video-pause", "video-sound-on", "video-viewer"]
    );
}

#[test]
fn keys_on_a_focused_video_play_and_turn_the_sound_on() {
    let mut rt = drawn(json!([video(1)]), &[1], Ok(clip(8_050)));
    press_key(&mut rt, &element(0, ""), " ");
    press_key(&mut rt, &element(0, ""), "m");
    assert_eq!(rt.state.controls, ["video-pause", "video-sound-on"]);
}

#[test]
fn a_format_nothing_here_plays_offers_save() {
    let mut rt = drawn(json!([video(1)]), &[1], Err(PlaybackFailure::Unsupported));
    let tree = tree(&rt);
    assert!(
        tree.contains("This video's format isn't supported."),
        "{tree}"
    );
    rt.accessibility_action(&element(0, "-failure"), Action::Click, None);
    until_idle(&mut rt);
    // The host saves it, as a file card's Save does.
    assert_eq!(rt.state.controls, ["media"]);
}

#[test]
fn a_video_that_fails_offers_retry() {
    let mut rt = drawn(json!([video(1)]), &[1], Err(PlaybackFailure::Broken));
    assert!(tree(&rt).contains("Couldn't play this video"));
    rt.accessibility_action(&element(0, "-failure"), Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(rt.state.controls, ["video-retry"]);
}

#[test]
fn videos_in_an_album_show_their_length_and_never_play_there() {
    let rt = drawn(json!([video(1), video(2)]), &[1, 2], Ok(clip(8_050)));
    let tree = tree(&rt);
    assert!(tree.contains("Album, 2 items"), "{tree}");
    assert!(tree.contains("Video, 0:08"), "{tree}");
    assert!(
        !tree.contains("muted"),
        "never autoplays in an album: {tree}"
    );
    assert!(rt.presented(&element(0, "-sound")).is_none());
}
