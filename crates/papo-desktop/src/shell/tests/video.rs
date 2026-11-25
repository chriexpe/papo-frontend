//! Video from the keyboard and in Settings (`docs/design.md` §12, §14.5, §14.8, §14.9),
//! headless and without GStreamer: a focused message's video keys (Space or K, M, F into the
//! viewer in full screen), the viewer on a video (no Copy image, Escape leaving full screen
//! first), and the Media group's Video size and Autoplay videos rows.

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::Message;
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;
use rune::{Key, Runtime};
use serde_json::json;

use super::navigation::click;
use super::replying::{conversation, message_id};
use super::settings::open;
use super::{FRAME_S, Shell, press};
use crate::media_store::MediaKey;
use crate::playback::MediaInfo;
use crate::settings::{PageId, RowId};
use crate::timeline::rows::message_key;
use crate::video_store::{VideoStore, VideoWant};

fn video_id() -> papo_sdk::Uuid {
    papo_sdk::Uuid::from_u128(0x9300)
}

/// The conversation with message 3 holding one video, read as a clip of `duration_ms`, shown
/// whole, autoplay off (so only the keys make it play).
fn with_video(duration_ms: u64) -> Runtime<Shell> {
    let mut rt = conversation("");
    rt.update(move |shell, _tx| {
        let id = message_id(3);
        let message: Message = serde_json::from_value(json!({
            "id": id.to_string(),
            "author_id": papo_sdk::Uuid::from_u128(0x4002).to_string(),
            "content": "o vídeo",
            "created_at": "2026-11-20T12:03:00Z",
            "attachments": [{
                "id": video_id().to_string(),
                "mime_type": "video/mp4",
                "original_file_name": "onda.mp4",
                "size_bytes": 540_929,
                "width": 640,
                "height": 360,
            }],
        }))
        .unwrap();
        let open = shell.tabs.shown_mut().unwrap();
        let account = open.key.account;
        open.items.set(
            2,
            Arc::new(TimelineItem::Message {
                id,
                message: Arc::new(message),
                content: Arc::new(MessageContent::parse("o vídeo")),
                reply: None,
                pending: None,
            }),
        );
        let key = MediaKey {
            account,
            attachment: video_id(),
        };
        let mut store = VideoStore::default();
        let jobs = store.want([VideoWant {
            key,
            box_px: [448, 252],
            in_album: false,
            hidden: false,
        }]);
        for job in jobs {
            store.probed(
                job.key,
                Ok(MediaInfo {
                    duration_ms: Some(duration_ms),
                    video_px: Some([640, 360]),
                    has_audio: true,
                }),
            );
        }
        store.shown(&HashMap::from([(key, 1.0)]));
        shell.videos.store = store;
    });
    settle(&mut rt);
    rt
}

fn video_key(rt: &Runtime<Shell>) -> MediaKey {
    MediaKey {
        account: rt.state.tabs.shown().unwrap().key.account,
        attachment: video_id(),
    }
}

fn settle(rt: &mut Runtime<Shell>) {
    for _ in 0..4 {
        rt.frame(FRAME_S);
    }
}

fn focus_message(rt: &mut Runtime<Shell>) {
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    settle(rt);
}

#[test]
fn k_and_m_on_a_focused_message_play_its_video_and_turn_the_sound_on_and_off() {
    let mut rt = with_video(90_000);
    let key = video_key(&rt);
    focus_message(&mut rt);
    press(&mut rt, Key::Text("k".to_owned()));
    let shown = *rt.state.videos.view().get(key).unwrap();
    assert!(
        shown.playing && !shown.muted,
        "a video that waited plays with sound"
    );
    press(&mut rt, Key::Text("m".to_owned()));
    assert!(rt.state.videos.view().get(key).unwrap().muted);
    press(&mut rt, Key::Text("m".to_owned()));
    assert!(!rt.state.videos.view().get(key).unwrap().muted);
    press(&mut rt, Key::Text(" ".to_owned()));
    assert!(!rt.state.videos.view().get(key).unwrap().playing);
}

#[test]
fn f_on_a_focused_message_opens_its_video_in_full_screen_and_escape_steps_out() {
    let mut rt = with_video(8_000);
    focus_message(&mut rt);
    press(&mut rt, Key::Text("f".to_owned()));
    settle(&mut rt);
    let viewer = rt.state.viewer.as_ref().expect("the viewer opened");
    assert!(viewer.full_screen);
    assert!(viewer.item().is_some_and(|item| item.video));
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains("onda.mp4, video"), "{tree}");
    assert!(
        rt.find("viewer-copy").is_none(),
        "no Copy image for a video"
    );
    press(&mut rt, Key::Escape);
    assert!(
        rt.state
            .viewer
            .as_ref()
            .is_some_and(|viewer| !viewer.full_screen)
    );
    press(&mut rt, Key::Escape);
    assert!(rt.state.viewer.is_none());
}

#[test]
fn the_viewer_pauses_the_inline_video_and_hands_its_place_back() {
    let mut rt = with_video(8_000);
    let key = video_key(&rt);
    rt.update(|shell, _| shell.videos.store.set_autoplay(true));
    assert!(rt.state.videos.store.desired().contains_key(&key));
    focus_message(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    settle(&mut rt);
    assert!(
        !rt.state.videos.store.desired().contains_key(&key),
        "the inline one waits while the viewer shows it"
    );
    press(&mut rt, Key::Escape);
    settle(&mut rt);
    assert!(rt.state.viewer.is_none());
    assert!(rt.state.videos.store.desired().contains_key(&key));
}

#[test]
fn video_size_and_autoplay_videos_sit_in_the_media_group() {
    let mut rt = super::app();
    open(&mut rt);
    click(
        &mut rt,
        &format!("settings-nav-{:?}", PageId::Conversations),
    );
    assert!(rt.find(&RowId::VideoSize.key()).is_some());
    assert!(
        rt.find(&RowId::VideoMaxWidth.key()).is_none(),
        "sliders only under Custom"
    );
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains("Muted, clips up to a minute"), "{tree}");
    click(&mut rt, &RowId::AutoplayVideos.key());
    let settings = *rt.state.conversations.settings();
    assert_eq!(settings.autoplay_videos, Some(false));
    rt.update(|shell, tx| {
        shell.settings_chosen(
            crate::shell::SettingChoice::MediaSize(
                crate::settings::SizedMedia::Videos,
                papo_sdk_ui::MediaSizeSetting::custom(504, 304),
            ),
            tx,
        );
    });
    rt.frame(FRAME_S);
    assert!(rt.find(&RowId::VideoMaxWidth.key()).is_some());
    assert!(rt.find(&RowId::VideoMaxHeight.key()).is_some());
    assert_eq!(
        rt.state.conversations.settings().video_size,
        papo_sdk_ui::MediaSizeSetting::custom(504, 304)
    );
    assert_eq!(
        rt.state.conversations.settings().image_size,
        papo_sdk_ui::MediaSizeSetting::Medium,
        "the pictures keep theirs"
    );
}
