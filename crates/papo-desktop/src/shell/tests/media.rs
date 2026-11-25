//! Received media from the keyboard alone (`docs/design.md` §12, §14.7, §14.8), headless: O
//! on a focused message opens the viewer on its first picture, ← → step without wrapping, and
//! Escape gives the keyboard back to the picture last shown; the menu key on a picture opens the media menu
//! (Open, Save image, Save as…, Copy image; no Copy link, B-024), and its Open opens the viewer;
//! while the viewer is open the app's keys do nothing underneath it, and Jump to message closes
//! it on the message.

use std::sync::Arc;

use papo_sdk::Message;
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;
use rune::{Key, KeyEvent, Mods, Runtime};
use serde_json::json;

use super::replying::{conversation, message_id};
use super::{FRAME_S, Shell, press};
use crate::menu::Entry;
use crate::timeline::rows::message_key;

/// The conversation with message 3 holding an album of two pictures.
fn with_album() -> Runtime<Shell> {
    let mut rt = conversation("");
    rt.update(|shell, _tx| {
        let id = message_id(3);
        let picture = |n: u128| {
            json!({
                "id": papo_sdk::Uuid::from_u128(0x9000 + n).to_string(),
                "mime_type": "image/jpeg",
                "original_file_name": format!("praia-{n}.jpg"),
                "size_bytes": 1000,
                "thumbnail_id": papo_sdk::Uuid::from_u128(0x9100 + n).to_string(),
                "thumbnail_width": 1024,
                "thumbnail_height": 768,
            })
        };
        let message: Message = serde_json::from_value(json!({
            "id": id.to_string(),
            "author_id": papo_sdk::Uuid::from_u128(0x4002).to_string(),
            "content": "fotos",
            "created_at": "2026-11-20T12:03:00Z",
            "attachments": [picture(1), picture(2)],
        }))
        .unwrap();
        let item = Arc::new(TimelineItem::Message {
            id,
            message: Arc::new(message),
            content: Arc::new(MessageContent::parse("fotos")),
            reply: None,
            pending: None,
        });
        let open = shell.tabs.shown_mut().unwrap();
        open.items.set(2, item);
    });
    rt.frame(FRAME_S);
    rt
}

/// The key of the element with the keyboard.
fn focused(rt: &Runtime<Shell>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|element| element.focused)
        .map(|element| {
            element
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
}

fn settle(rt: &mut Runtime<Shell>) {
    for _ in 0..4 {
        rt.frame(FRAME_S);
    }
}

#[test]
fn o_opens_the_viewer_and_escape_gives_the_keyboard_back() {
    let mut rt = with_album();
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    settle(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    settle(&mut rt);
    let viewer = rt.state.viewer.as_ref().expect("the viewer opened");
    assert_eq!(viewer.current, 0);
    assert_eq!(focused(&rt).as_deref(), Some("viewer"));
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains("praia-1.jpg, image 1 of 2"), "{tree}");

    press(&mut rt, Key::Right);
    press(&mut rt, Key::Right);
    assert_eq!(
        rt.state.viewer.as_ref().map(|v| v.current),
        Some(1),
        "no wrap"
    );
    press(&mut rt, Key::Left);
    assert_eq!(rt.state.viewer.as_ref().map(|v| v.current), Some(0));
    press(&mut rt, Key::Right);

    press(&mut rt, Key::Escape);
    settle(&mut rt);
    assert!(rt.state.viewer.is_none());
    assert_eq!(
        focused(&rt),
        Some(format!("media-{}-1", message_id(3))),
        "the keyboard goes back to the picture last shown"
    );
}

#[test]
fn the_menu_key_opens_the_media_menu_and_its_open_opens_the_viewer() {
    let mut rt = with_album();
    let second = format!("media-{}-1", message_id(3));
    let target = second.clone();
    rt.update(move |_shell, tx| tx.focus(target, true));
    settle(&mut rt);
    press(&mut rt, Key::ContextMenu);
    settle(&mut rt);
    let labels: Vec<String> = rt
        .state
        .nav
        .menu
        .as_ref()
        .expect("the media menu")
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item.label.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["Open", "Save image", "Save as…", "Copy image"]);
    press(&mut rt, Key::Enter);
    settle(&mut rt);
    assert_eq!(rt.state.viewer.as_ref().map(|v| v.current), Some(1));
    press(&mut rt, Key::Escape);
    settle(&mut rt);
    assert_eq!(focused(&rt), Some(second));
}

#[test]
fn the_apps_keys_wait_while_the_viewer_is_open() {
    let mut rt = with_album();
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    settle(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    settle(&mut rt);
    assert!(rt.state.viewer.is_some());
    let ctrl = Mods {
        cmd: true,
        ..Mods::default()
    };
    for key in [Key::Command('k'), Key::Command(',')] {
        rt.key(KeyEvent { key, mods: ctrl });
        settle(&mut rt);
    }
    press(&mut rt, Key::Down);
    settle(&mut rt);
    assert!(rt.state.jump.is_none(), "no Ctrl+K");
    assert!(rt.state.settings.is_none(), "no settings");
    assert_eq!(focused(&rt).as_deref(), Some("viewer"));
}

#[test]
fn jump_to_message_closes_the_viewer_on_the_message() {
    let mut rt = with_album();
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    settle(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    settle(&mut rt);
    rt.update(Shell::viewer_jump);
    settle(&mut rt);
    assert!(rt.state.viewer.is_none());
    assert_eq!(focused(&rt), Some(format!("message-{}", message_id(3))));
}
