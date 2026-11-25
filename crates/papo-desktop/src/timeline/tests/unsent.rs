//! Your unsent message's attachments (`docs/design.md` §14.3): pictures as
//! an album, files as cards, each waiting or uploading item with its ring
//! or bar, spoken; ✕ cancels the message; and no state change moves the
//! row.

use std::sync::Arc;

use eyeball_im::VectorDiff;
use papo_sdk::{
    Generation, Operation, OperationKind, OperationState, OutgoingAttachment, OutgoingMessage, Uuid,
};
use papo_sdk_ui::{TimelineItem, TimelineUpdate};
use rune::Runtime;
use rune::accesskit::Action;

use super::{FRAME_S, Host, runtime, until_idle};
use crate::composer::tray::TileKind;
use crate::design::MessageLayout;
use crate::timeline::rows::echo_key;
use crate::unsent_media::UnsentFile;

const LOCAL: u128 = 100;

/// Your message with two pictures and a file, in `state`.
fn echo(state: OperationState) -> Arc<TimelineItem> {
    echo_of(state, &["praia.jpg", "party.gif", "dump.zip"])
}

/// Your message with `names` attached, in `state`.
fn echo_of(state: OperationState, names: &[&str]) -> Arc<TimelineItem> {
    let id = Uuid::from_u128(LOCAL);
    let attachments = names
        .iter()
        .map(|name| OutgoingAttachment::from_bytes(*name, vec![0; 8]))
        .collect();
    let operation = Operation {
        id,
        channel_id: Uuid::from_u128(7),
        kind: Arc::new(OperationKind::Message(OutgoingMessage {
            content: "the photos".to_owned(),
            attachments,
            ..OutgoingMessage::default()
        })),
        state,
        attempts: 0,
        depends_on: None,
        failure: None,
        message_id: None,
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

fn file(kind: TileKind) -> UnsentFile {
    UnsentFile {
        kind,
        size_bytes: Some(2_516_582),
        media_size: papo_sdk_ui::MediaSize::exact(1600, 1200),
        image: None,
        sharp: None,
    }
}

fn layout() -> MessageLayout {
    MessageLayout::default()
}

fn with_echo(state: OperationState) -> Runtime<Host> {
    let mut rt = runtime(std::iter::once(echo(state)).collect(), layout());
    attachments_known(&mut rt);
    rt
}

/// The message's attachments become known (at Send, or read back).
fn attachments_known(rt: &mut Runtime<Host>) {
    rt.update(|host, _tx| {
        host.unsent.insert(
            Uuid::from_u128(LOCAL),
            vec![
                file(TileKind::Image),
                file(TileKind::Gif),
                file(TileKind::File),
            ],
        );
    });
    rt.frame(FRAME_S);
    until_idle(rt);
}

/// The queue moves the message to `state`.
fn moves_to(rt: &mut Runtime<Host>, state: OperationState) {
    let next = echo(state);
    rt.update(move |host, _tx| {
        host.open.update(
            TimelineUpdate {
                version: 2,
                generation: Generation::for_test(1),
                diffs: vec![VectorDiff::Set {
                    index: 0,
                    value: next,
                }],
            },
            true,
        );
    });
    rt.frame(FRAME_S);
    until_idle(rt);
}

fn tree(rt: &Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

fn height(rt: &Runtime<Host>) -> f32 {
    let key = echo_key(Uuid::from_u128(LOCAL));
    rt.presented(&key).map_or(f32::NAN, |(rect, _)| rect[3])
}

#[test]
fn a_waiting_message_shows_its_album_and_card_with_rings_spoken() {
    let rt = with_echo(OperationState::Queued);
    let tree = tree(&rt);
    assert!(tree.contains("Album, 2 items"), "{tree}");
    assert!(tree.contains("Uploading praia.jpg, 0 %"), "{tree}");
    assert!(tree.contains("Uploading dump.zip, 0 %"), "{tree}");
    assert!(tree.contains("Cancel upload"), "{tree}");
}

#[test]
fn cancel_on_any_item_cancels_the_message() {
    let mut rt = with_echo(OperationState::Queued);
    rt.accessibility_action(
        &format!("upload-cancel-{}-2", Uuid::from_u128(LOCAL)),
        Action::Click,
        None,
    );
    rt.frame(FRAME_S);
    assert_eq!(rt.state.controls, ["upload-cancel"]);
}

#[test]
fn a_message_keeps_its_attachments_and_its_height_through_every_state() {
    for state in [
        OperationState::Sending,
        OperationState::Failed,
        OperationState::Unknown,
        OperationState::Sent,
    ] {
        let mut rt = with_echo(OperationState::Queued);
        let waiting = height(&rt);
        moves_to(&mut rt, state);
        let tree = tree(&rt);
        assert!(tree.contains("Album, 2 items"), "{state:?}: {tree}");
        if state != OperationState::Sending {
            assert!(!tree.contains("Uploading"), "{state:?}: {tree}");
        }
        assert!(
            (height(&rt) - waiting).abs() < 0.5,
            "{state:?}: {waiting} → {}",
            height(&rt)
        );
    }
}

#[test]
fn a_message_read_back_after_a_relaunch_is_first_drawn_whole() {
    let mut rt = runtime(
        std::iter::once(echo(OperationState::Unknown)).collect(),
        layout(),
    );
    rt.frame(FRAME_S);
    until_idle(&mut rt);
    assert!(!tree(&rt).contains("the photos"), "held until read back");
    attachments_known(&mut rt);
    let tree = tree(&rt);
    assert!(
        tree.contains("the photos") && tree.contains("Album, 2 items"),
        "{tree}"
    );
}

#[test]
fn a_sharp_picture_landing_redraws_the_row() {
    let images = rune::ImageCache::new(Arc::new(|| {}));
    let mut media = crate::unsent_media::UnsentMedia::default();
    let local = Uuid::from_u128(LOCAL);
    let mut photo = file(TileKind::Image);
    photo.image = Some(images.rgba(1, 1, vec![0_u8; 4]));
    media.insert(local, vec![photo]);
    let files = |media: &crate::unsent_media::UnsentMedia| {
        media
            .get(local)
            .map(|entry| Arc::clone(&entry.files))
            .expect("kept")
    };
    let before = crate::timeline::row_media::pictures_drawn(&files(&media));
    media.sharpened(local, 0, images.rgba(2, 2, vec![0_u8; 16]));
    let after = crate::timeline::row_media::pictures_drawn(&files(&media));
    assert_ne!(before, after);
}

#[test]
fn an_album_of_more_than_four_reads_left_to_right_then_down() {
    let names = ["1.jpg", "2.jpg", "3.jpg", "4.jpg", "5.jpg"];
    let mut rt = runtime(
        std::iter::once(echo_of(OperationState::Queued, &names)).collect(),
        layout(),
    );
    rt.update(|host, _tx| {
        host.unsent
            .insert(Uuid::from_u128(LOCAL), vec![file(TileKind::Image); 5]);
    });
    rt.frame(FRAME_S);
    until_idle(&mut rt);
    let at = |index: usize| {
        let key = format!("upload-cancel-{}-{index}", Uuid::from_u128(LOCAL));
        rt.presented(&key).map(|(rect, _)| (rect[0], rect[1]))
    };
    let (Some(one), Some(two), Some(three), Some(four)) = (at(0), at(1), at(2), at(3)) else {
        panic!("four cells drawn: {}", tree(&rt));
    };
    assert!(at(4).is_none(), "the fifth is under +1");
    assert!(
        one.0 < two.0 && (one.1 - two.1).abs() < 0.5,
        "1 2 side by side"
    );
    assert!(
        three.0 < four.0 && (three.1 - four.1).abs() < 0.5,
        "3 4 side by side"
    );
    assert!(
        three.1 > one.1 && (three.0 - one.0).abs() < 0.5,
        "3 under 1"
    );
    assert!(tree(&rt).contains("Album, 5 items"), "{}", tree(&rt));
}
