//! A received message's media (`docs/design.md` §14.4, §14.6): the box reserved before
//! anything loads, albums, file cards and their presses, a sensitive picture hidden until
//! revealed, a sent message keeping its unsent row's height, and the reader's place through
//! every Image size.

use std::collections::HashMap;
use std::sync::Arc;

use eyeball_im::VectorDiff;
use imbl::Vector;
use papo_sdk::{
    Generation, Message, Operation, OperationKind, OperationState, OutgoingAttachment,
    OutgoingMessage, Uuid,
};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{MediaBounds, MediaSize, MediaSizeSetting, TimelineItem, TimelineUpdate};
use rune::Runtime;
use rune::accesskit::Action;
use serde_json::{Value, json};

use super::{FRAME_S, HOLD_PT, Host, rect, runtime, until_idle};
use crate::composer::tray::TileKind;
use crate::design::MessageLayout;
use crate::timeline::anchor::Anchor;
use crate::timeline::rows::{anchor_key, echo_key, message_key};
use crate::timeline::scroll_to;
use crate::unsent_media::UnsentFile;

const LOCAL: u128 = 100;
const SENT: u128 = 200;

fn attachment_id(n: u128) -> Uuid {
    Uuid::from_u128(9_000 + n)
}

/// A received message `id` by Ana with `attachments` (wire JSON).
fn received(id: u128, seconds: u32, body: &str, attachments: Value) -> Arc<TimelineItem> {
    let id = Uuid::from_u128(id);
    let mut wire = json!({
        "id": id.to_string(),
        "author_id": "ana",
        "content": body,
        "created_at": format!("2026-11-20T14:{:02}:{:02}Z", seconds / 60 % 60, seconds % 60),
    });
    wire["attachments"] = attachments;
    let message: Message = serde_json::from_value(wire).unwrap();
    Arc::new(TimelineItem::Message {
        reply: None,
        pending: None,
        id,
        message: Arc::new(message),
        content: Arc::new(MessageContent::parse(body)),
    })
}

fn picture(n: u128, mime: &str, thumbnail: Option<(u32, u32)>) -> Value {
    let mut wire = json!({
        "id": attachment_id(n).to_string(),
        "mime_type": mime,
        "original_file_name": format!("{n}.jpg"),
        "size_bytes": 240_000,
    });
    if let Some((width, height)) = thumbnail {
        wire["thumbnail_id"] = json!(Uuid::from_u128(n + 50).to_string());
        wire["thumbnail_width"] = json!(width);
        wire["thumbnail_height"] = json!(height);
    }
    wire
}

fn file(n: u128) -> Value {
    json!({
        "id": attachment_id(n).to_string(),
        "mime_type": "application/zip",
        "original_file_name": "dump.zip",
        "size_bytes": 2_516_582,
    })
}

fn tree(rt: &Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

#[test]
fn the_box_is_reserved_from_the_thumbnail_before_anything_loads() {
    let item = received(
        1,
        0,
        "pôr do sol",
        json!([picture(1, "image/jpeg", Some((1024, 664)))]),
    );
    let mut rt = runtime(std::iter::once(item).collect(), MessageLayout::default());
    until_idle(&mut rt);
    let boxed = rect(&rt, &format!("media-{}-0", Uuid::from_u128(1)));
    // Medium: 448 wide, the thumbnail's shape.
    assert!((boxed[2] - 448.0).abs() < 0.5, "{boxed:?}");
    assert!((boxed[3] - 291.0).abs() < 0.5, "{boxed:?}");
    assert!(tree(&rt).contains("Image, 1.jpg"), "spoken by its name");
}

#[test]
fn an_album_and_a_card_draw_in_order_and_take_their_presses() {
    let pictures: Vec<Value> = (1..=5)
        .map(|n| picture(n, "image/jpeg", Some((1024, 576))))
        .chain(std::iter::once(file(6)))
        .collect();
    let item = received(1, 0, "", json!(pictures));
    let mut rt = runtime(std::iter::once(item).collect(), MessageLayout::default());
    until_idle(&mut rt);
    let tree = tree(&rt);
    assert!(tree.contains("Album, 5 items"), "{tree}");
    assert!(tree.contains("File, dump.zip, 2.4 MB"), "{tree}");
    let id = Uuid::from_u128(1);
    // The fifth is under "+1": four cells are drawn.
    assert!(rt.presented(&format!("media-{id}-4")).is_none());
    rt.accessibility_action(&format!("media-{id}-0"), Action::Click, None);
    rt.accessibility_action(&format!("media-{id}-5"), Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(rt.state.controls, ["media", "media"]);
}

/// Your message with a picture, a GIF and a file, sent: the queue holds its receipt.
fn sent_echo() -> Arc<TimelineItem> {
    let id = Uuid::from_u128(LOCAL);
    let names = ["praia.jpg", "party.gif", "dump.zip"];
    let operation = Operation {
        id,
        channel_id: Uuid::from_u128(7),
        kind: Arc::new(OperationKind::Message(OutgoingMessage {
            content: "the photos".to_owned(),
            attachments: names
                .iter()
                .map(|name| OutgoingAttachment::from_bytes(*name, vec![0; 8]))
                .collect(),
            ..OutgoingMessage::default()
        })),
        state: OperationState::Sent,
        attempts: 1,
        depends_on: None,
        failure: None,
        message_id: Some(Uuid::from_u128(SENT)),
        history_projected: false,
        message: None,
        created_at_us: 1_795_183_200_000_000,
    };
    Arc::new(TimelineItem::LocalEcho {
        id,
        operation: Arc::new(operation),
        reply: None,
    })
}

fn unsent_file(kind: TileKind) -> UnsentFile {
    UnsentFile {
        kind,
        size_bytes: Some(2_516_582),
        media_size: MediaSize::exact(1600, 1200),
        image: None,
        sharp: None,
        video: None,
    }
}

#[test]
fn a_sent_message_keeps_its_unsent_rows_height() {
    let mut rt = runtime(
        std::iter::once(sent_echo()).collect(),
        MessageLayout::default(),
    );
    rt.update(|host, _tx| {
        host.unsent.insert(
            Uuid::from_u128(LOCAL),
            vec![
                unsent_file(TileKind::Image),
                unsent_file(TileKind::Gif),
                unsent_file(TileKind::File),
            ],
        );
    });
    until_idle(&mut rt);
    let unsent = rt
        .presented(&echo_key(Uuid::from_u128(LOCAL)))
        .map(|(rect, _)| rect[3])
        .expect("the echo");
    // History takes the echo's place; the sizes this device read stay with the attachments.
    // The server's thumbnails would say another shape: the device's own size wins.
    let item = received(
        SENT,
        0,
        "the photos",
        json!([
            picture(1, "image/jpeg", Some((1024, 683))),
            picture(2, "image/gif", Some((512, 341))),
            file(3),
        ]),
    );
    rt.update(move |host, _tx| {
        let own: HashMap<Uuid, MediaSize> = [1, 2]
            .into_iter()
            .map(|n| (attachment_id(n), MediaSize::exact(1600, 1200)))
            .collect();
        host.media.own_sizes = Arc::new(own);
        host.open.update(
            TimelineUpdate {
                version: 2,
                generation: Generation::for_test(1),
                diffs: vec![VectorDiff::Set {
                    index: 0,
                    value: item,
                }],
            },
            true,
        );
    });
    until_idle(&mut rt);
    let sent = rt
        .presented(&message_key(Uuid::from_u128(SENT)))
        .map(|(rect, _)| rect[3])
        .expect("the received row");
    assert!((sent - unsent).abs() < 0.5, "unsent {unsent}, sent {sent}");
}

/// `count` messages, every third with a picture.
fn history_with_pictures(count: u128) -> Vector<Arc<TimelineItem>> {
    (0..count)
        .map(|n| {
            let seconds = u32::try_from(n * 60 % 3_600).unwrap();
            let attachments = if n % 3 == 0 {
                json!([picture(n, "image/jpeg", Some((1024, 768)))])
            } else {
                json!([])
            };
            received(n + 1_000, seconds, "a line of words", attachments)
        })
        .collect()
}

#[test]
fn the_message_being_read_keeps_its_place_when_image_size_changes() {
    let mut rt = runtime(history_with_pictures(120), MessageLayout::default());
    for _ in 0..4 {
        rt.wheel(400.0, 300.0, 400.0);
        rt.frame(FRAME_S);
    }
    until_idle(&mut rt);
    let viewport = format!("timeline-{}-{}", Uuid::nil(), Uuid::from_u128(7));
    for size in [
        MediaSizeSetting::Large,
        MediaSizeSetting::Small,
        MediaSizeSetting::custom(720, 640),
        MediaSizeSetting::Medium,
    ] {
        let Anchor::Message { id, offset_pt } = rt.state.open.current_anchor() else {
            panic!("reading, not following the newest");
        };
        // What the Conversations page does: change it, keep the place.
        rt.update(move |host, tx| {
            let anchor = host.open.current_anchor();
            host.media.bounds = MediaBounds::new(size, f32::INFINITY, 400.0);
            scroll_to(anchor, tx);
        });
        until_idle(&mut rt);
        let now_pt = rect(&rt, &anchor_key(id))[1] - rect(&rt, &viewport)[1];
        let moved = now_pt - offset_pt;
        assert!(moved.abs() < HOLD_PT, "{size:?} moved it {moved}");
    }
}

#[test]
fn a_sensitive_picture_is_a_spoiler_until_revealed_in_the_same_box() {
    let mut wire = picture(1, "image/jpeg", Some((1024, 664)));
    wire["moderation_status"] = json!("sensitive");
    let item = received(1, 0, "", json!([wire]));
    let mut rt = runtime(std::iter::once(item).collect(), MessageLayout::default());
    until_idle(&mut rt);
    let key = format!("media-{}-0", Uuid::from_u128(1));
    let hidden_box = rect(&rt, &key);
    assert!(
        tree(&rt).contains("Hidden spoiler. Press to reveal."),
        "spoken as a spoiler"
    );
    rt.accessibility_action(&key, Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(
        rt.state.controls,
        ["reveal"],
        "a press reveals, it doesn't open"
    );

    rt.update(|host, _tx| {
        let revealed = crate::media_store::MediaKey {
            account: host.open.key.account,
            attachment: attachment_id(1),
        };
        host.media.revealed = Arc::new(std::iter::once(revealed).collect());
    });
    until_idle(&mut rt);
    assert!(tree(&rt).contains("Image, 1.jpg"), "shown once revealed");
    assert_eq!(rect(&rt, &key), hidden_box, "the same box");
    rt.accessibility_action(&key, Action::Click, None);
    until_idle(&mut rt);
    assert_eq!(
        rt.state.controls,
        ["reveal", "media"],
        "now a press opens it"
    );
}
