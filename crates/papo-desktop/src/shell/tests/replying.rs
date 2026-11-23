//! Replying, editing and deleting through the keys (`docs/design.md` §11
//! and §12), headless: a focused message's R, E and Delete, ↑ and Ctrl+↑ in
//! an empty field, Escape, Save, and the delete confirmation.

use std::sync::Arc;

use papo_sdk::{Channel, Generation, Message, ResourceSnapshot, UserSummary, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{ReplyOriginal, TimelineItem};
use rune::{Key, KeyEvent, Mods, Runtime};

use super::composer::{field, writing};
use super::{FRAME_S, Shell, press};
use crate::composer::COMPOSER_KEY;
use crate::timeline::rows::message_key;
use crate::timeline::{OpenTimeline, TimelineKey};

/// Someone else in the conversation.
pub(super) fn lia() -> Uuid {
    Uuid::from_u128(0x2000)
}

/// Message `n`'s ID.
pub(super) fn message_id(n: u128) -> Uuid {
    Uuid::from_u128(0x3000 + n)
}

pub(super) fn item(
    n: u128,
    author: Uuid,
    text: &str,
    reply: Option<ReplyOriginal>,
) -> Arc<TimelineItem> {
    let id = message_id(n);
    Arc::new(TimelineItem::Message {
        id,
        message: Arc::new(Message {
            id: Some(id.to_string()),
            author_id: Some(author.to_string()),
            content: Some(text.to_owned()),
            created_at: Some(format!("2026-11-20T12:{n:02}:00Z")),
            reply_to: reply.as_ref().map(|_| message_id(1).to_string()),
            ..Message::default()
        }),
        content: Arc::new(MessageContent::parse(text)),
        reply,
        pending: None,
    })
}

/// [`writing`] `draft` in a conversation of five messages, yours at 2 and
/// 4 and Lia's at 1, 3 and 5, with a roster and the channel's permissions.
pub(super) fn conversation(draft: &str) -> Runtime<Shell> {
    let mut rt = writing(draft);
    rt.update(|shell, _tx| {
        let key = shell.composer.shown().unwrap().0;
        let me = key.account.user();
        let items = (1..=5)
            .map(|n| {
                let author = if n % 2 == 0 { me } else { lia() };
                item(n, author, &format!("message {n}"), None)
            })
            .collect();
        let mut open = OpenTimeline::loaded(key, items);
        open.resources = Some(Arc::new(resources(key)));
        shell.tabs.show(Some(key), false);
        *shell.tabs.shown_mut().unwrap() = open;
    });
    rt.frame(FRAME_S);
    rt
}

/// The account, Lia, the shown channel (no overwrites: everyone writes) and
/// a server owned by someone else.
fn resources(key: TimelineKey) -> ResourceSnapshot {
    let person = |id: Uuid, name: &str| UserSummary {
        id: Some(id.to_string()),
        username: Some(name.to_owned()),
        roles: Some(Vec::new()),
        ..UserSummary::default()
    };
    let server = serde_json::from_value(serde_json::json!({
        "id": Uuid::from_u128(0x4000).to_string(),
        "name": "Lumen",
        "owner_id": Uuid::from_u128(0x4001).to_string(),
    }))
    .unwrap();
    let channel = Channel {
        id: Some(key.channel.to_string()),
        name: Some("general".to_owned()),
        r#type: Some("text".to_owned()),
        position: Some(0),
        permissions: Some(Vec::new()),
        ..Channel::default()
    };
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        Some(server),
        vec![channel],
        Vec::new(),
        vec![person(key.account.user(), "me"), person(lia(), "lia")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        key.account.user(),
    )
    .unwrap()
}

fn ctrl_up(rt: &mut Runtime<Shell>) {
    rt.key(KeyEvent {
        key: Key::Up,
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
}

/// The message the composer points at: a reply's original or an edit's.
fn target(rt: &Runtime<Shell>) -> Option<Uuid> {
    rt.state.composer.shown().unwrap().1.mode.target()
}

fn editing(rt: &Runtime<Shell>) -> Option<Uuid> {
    rt.state.composer.shown().unwrap().1.mode.editing()
}

fn focus_composer(rt: &mut Runtime<Shell>) {
    rt.update(|_shell, tx| tx.focus(COMPOSER_KEY, false));
    rt.frame(FRAME_S);
}

#[test]
fn ctrl_up_reaches_the_newest_message_and_up_and_r_reply_to_the_one_above() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    ctrl_up(&mut rt);
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Text("r".to_owned()));
    assert_eq!(target(&rt), Some(message_id(4)), "R replied to message 4");
    assert!(
        rt.find(crate::composer::bars::BAR_KEY).is_some(),
        "the bar opened"
    );
    assert_eq!(editing(&rt), None);
}

#[test]
fn escape_ends_a_reply_and_keeps_what_was_typed() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    ctrl_up(&mut rt);
    press(&mut rt, Key::Text("r".to_owned()));
    focus_composer(&mut rt);
    press(&mut rt, Key::Text("o".to_owned()));
    press(&mut rt, Key::Text("i".to_owned()));
    press(&mut rt, Key::Escape);
    assert_eq!(target(&rt), None);
    assert_eq!(field(&rt), "oi");
    assert!(rt.find(crate::composer::bars::BAR_KEY).is_none());
}

#[test]
fn up_in_an_empty_field_edits_your_newest_message_and_escape_puts_the_field_back() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    press(&mut rt, Key::Up);
    assert_eq!(
        editing(&rt),
        Some(message_id(4)),
        "your newest is message 4"
    );
    assert_eq!(field(&rt), "message 4");
    assert!(
        rt.find(crate::composer::view::SAVE_KEY).is_some(),
        "Send became Save"
    );
    press(&mut rt, Key::Escape);
    assert_eq!(editing(&rt), None);
    assert_eq!(field(&rt), "");
}

#[test]
fn an_edit_sets_the_draft_aside_and_saving_brings_it_back() {
    let mut rt = conversation("half a thought");
    // Ctrl+↑ is for an empty field; the message takes focus another way.
    rt.update(|_shell, tx| tx.focus(message_key(message_id(4)), true));
    rt.frame(FRAME_S);
    press(&mut rt, Key::Text("e".to_owned()));
    assert_eq!(editing(&rt), Some(message_id(4)));
    assert_eq!(field(&rt), "message 4");
    press(&mut rt, Key::Text("!".to_owned()));
    press(&mut rt, Key::Enter);
    assert_eq!(editing(&rt), None, "Save ended the edit");
    assert_eq!(field(&rt), "half a thought", "the draft came back");
}

#[test]
fn e_on_someone_elses_message_edits_nothing() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    ctrl_up(&mut rt);
    press(&mut rt, Key::Text("e".to_owned()));
    assert_eq!(editing(&rt), None, "message 5 is Lia's");
}

#[test]
fn delete_asks_first_and_cancel_deletes_nothing() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    ctrl_up(&mut rt);
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Delete);
    assert!(
        rt.find(crate::shell::deleting::DELETE_KEY).is_some(),
        "it asked"
    );
    assert!(
        rt.find(&format!("standalone-{}", message_id(4))).is_some(),
        "showing the message"
    );
    assert_eq!(
        super::navigation::focused(&rt).as_deref(),
        Some("delete-cancel")
    );
    press(&mut rt, Key::Escape);
    assert!(rt.find(crate::shell::deleting::DELETE_KEY).is_none());
    assert!(rt.state.deleting.is_none(), "nothing waits to be deleted");
}

#[test]
fn an_emptied_edit_asks_to_delete_instead() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    press(&mut rt, Key::Up);
    for _ in 0.."message 4".len() {
        press(&mut rt, Key::Backspace);
    }
    press(&mut rt, Key::Enter);
    assert!(rt.find(crate::shell::deleting::DELETE_KEY).is_some());
    assert_eq!(
        editing(&rt),
        Some(message_id(4)),
        "the edit waits for the answer"
    );
}

#[test]
fn a_reply_draws_its_line_and_pressing_it_lights_the_original() {
    let mut rt = conversation("");
    rt.poll = Some(Shell::poll);
    rt.update(|shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        let original = match open.items[0].as_ref() {
            TimelineItem::Message {
                id,
                message,
                content,
                ..
            } => ReplyOriginal::Held {
                id: *id,
                message: Arc::clone(message),
                content: Arc::clone(content),
            },
            _ => unreachable!(),
        };
        let me = open.key.account.user();
        open.items.push_back(item(6, me, "a reply", Some(original)));
        open.keys_version += 1;
    });
    // The new row fades in; it takes presses once it has.
    for _ in 0..60 {
        rt.frame(FRAME_S);
    }
    let original = rt
        .elements()
        .into_iter()
        .find(|element| element.key.as_deref() == Some("reply-original"))
        .expect("the reply line shows");
    let [x, y, w, h] = original.rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.frame(FRAME_S);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(FRAME_S);
    let flash = rt.state.tabs.shown().unwrap().flash;
    assert_eq!(
        flash.map(|(id, _)| id),
        Some(message_id(1)),
        "a press jumped"
    );
    for _ in 0..120 {
        rt.frame(FRAME_S);
    }
    assert_eq!(
        rt.state.tabs.shown().unwrap().flash,
        None,
        "it fades after 1.5 s"
    );
}

#[test]
fn up_from_a_reply_goes_to_the_message_above_not_into_its_line() {
    let mut rt = conversation("");
    rt.update(|shell, _tx| {
        let open = shell.tabs.shown_mut().unwrap();
        let me = open.key.account.user();
        let original = match open.items[0].as_ref() {
            TimelineItem::Message {
                id,
                message,
                content,
                ..
            } => ReplyOriginal::Held {
                id: *id,
                message: Arc::clone(message),
                content: Arc::clone(content),
            },
            _ => unreachable!(),
        };
        // Its line is a link, focusable inside the row.
        open.items.push_back(item(6, me, "a reply", Some(original)));
        open.keys_version += 1;
    });
    for _ in 0..60 {
        rt.frame(FRAME_S);
    }
    assert!(rt.find("reply-original").is_some());
    focus_composer(&mut rt);
    ctrl_up(&mut rt);
    press(&mut rt, Key::Up);
    press(&mut rt, Key::Text("r".to_owned()));
    assert_eq!(target(&rt), Some(message_id(5)));
}

#[test]
fn a_click_on_a_message_leaves_the_keyboard_in_the_field() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    let avatar = rt
        .elements()
        .into_iter()
        .find(|element| {
            element.path.contains(&message_key(message_id(4))) && element.path.ends_with("/lead")
        })
        .expect("message 4's avatar");
    let [x, y, w, h] = avatar.rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.frame(FRAME_S);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    rt.frame(FRAME_S);
    press(&mut rt, Key::Text("e".to_owned()));
    assert_eq!(editing(&rt), None, "the click focused no message");
    assert_eq!(field(&rt), "e", "the key went to the field");
}

#[test]
fn alt_down_on_a_focused_message_goes_to_the_next_channel() {
    let mut rt = conversation("");
    focus_composer(&mut rt);
    ctrl_up(&mut rt);
    rt.key(KeyEvent {
        key: Key::Down,
        mods: Mods {
            alt: true,
            ..Mods::default()
        },
    });
    rt.frame(FRAME_S);
    assert_eq!(
        rt.state.nav.selected_channel(),
        Some(super::composer::channel_id(2).as_str())
    );
}
