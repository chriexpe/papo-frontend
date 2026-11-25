//! Your unsent message's audio (`docs/design.md` §14.3, §14.5): the player a sent one has,
//! playable once read, with its upload's size after its name, ✕ in its corner and the bar
//! spoken while it uploads; the capsule keeps its size when sent.

use std::sync::Arc;

use eyeball_im::VectorDiff;
use papo_sdk::{
    AccountId, Generation, Operation, OperationKind, OperationState, OutgoingAttachment,
    OutgoingMessage, Uuid,
};
use papo_sdk_ui::{TimelineItem, TimelineUpdate, WaveformBuilder};
use rune::Runtime;
use rune::accesskit::Action;

use super::{FRAME_S, Host, key, runtime, until_idle};
use crate::audio_store::{AudioKey, AudioStore};
use crate::composer::tray::TileKind;
use crate::design::MessageLayout;
use crate::playback::AudioRead;
use crate::unsent_media::UnsentFile;

const LOCAL: u128 = 120;

/// Your message with one audio, in `state`.
fn echo(state: OperationState) -> Arc<TimelineItem> {
    let id = Uuid::from_u128(LOCAL);
    let operation = Operation {
        id,
        channel_id: Uuid::from_u128(7),
        kind: Arc::new(OperationKind::Message(OutgoingMessage {
            attachments: vec![OutgoingAttachment::from_bytes("tone.ogg", vec![0; 8])],
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

fn audio_key(account: AccountId) -> AudioKey {
    AudioKey::Unsent {
        account,
        local: Uuid::from_u128(LOCAL),
        index: 0,
    }
}

/// The message in `state`, its audio read as 6 s long when `read`.
fn with_audio(state: OperationState, read: bool) -> Runtime<Host> {
    let mut rt = runtime(
        std::iter::once(echo(state)).collect(),
        MessageLayout::default(),
    );
    rt.update(move |host, _tx| {
        host.unsent.insert(
            Uuid::from_u128(LOCAL),
            vec![UnsentFile {
                kind: TileKind::Audio,
                size_bytes: Some(96_000),
                media_size: papo_sdk_ui::MediaSize::Unknown,
                image: None,
                sharp: None,
                video: None,
            }],
        );
        if read {
            let audio = audio_key(key().account);
            let mut store = AudioStore::default();
            store.want([audio]);
            store.next_read();
            let mut builder = WaveformBuilder::new(None);
            builder.push(&[0.5; 1_024], 1);
            store.read(
                audio,
                Ok(AudioRead {
                    duration_ms: Some(6_000),
                    waveform: Some(builder.finish()),
                }),
            );
            host.media.audio = store.view();
        }
    });
    rt.invalidate();
    rt.frame(FRAME_S);
    until_idle(&mut rt);
    rt
}

fn element(part: &str) -> String {
    format!("media-{}-0{part}", Uuid::from_u128(LOCAL))
}

fn tree(rt: &Runtime<Host>) -> String {
    rt.accessibility_json().to_string()
}

fn width(rt: &Runtime<Host>) -> f32 {
    rt.presented(&element(""))
        .map_or(f32::NAN, |(rect, _)| rect[2])
}

#[test]
fn an_unsent_audio_plays_like_a_sent_one_with_its_upload_on_it() {
    let mut rt = with_audio(OperationState::Queued, true);
    let tree = tree(&rt);
    assert!(tree.contains("Audio, tone.ogg"), "{tree}");
    assert!(tree.contains("0:06"), "its length: {tree}");
    assert!(tree.contains(" · 0 of "), "its upload's size: {tree}");
    assert!(tree.contains("Uploading tone.ogg, 0 %"), "{tree}");
    assert!(tree.contains("Cancel upload"), "{tree}");
    rt.accessibility_action(&element("-play"), Action::Click, None);
    until_idle(&mut rt);
    rt.accessibility_action(
        &format!("upload-cancel-{}-0", Uuid::from_u128(LOCAL)),
        Action::Click,
        None,
    );
    rt.frame(FRAME_S);
    assert_eq!(rt.state.controls, ["audio-play", "upload-cancel"]);
}

#[test]
fn before_it_is_read_it_shows_flat_and_takes_no_press() {
    let rt = with_audio(OperationState::Queued, false);
    assert!(tree(&rt).contains("Audio, tone.ogg"));
    assert!(rt.presented(&element("-play")).is_none());
}

#[test]
fn sent_it_drops_the_upload_and_keeps_its_size() {
    let mut rt = with_audio(OperationState::Queued, true);
    let uploading = width(&rt);
    let next = echo(OperationState::Failed);
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
    until_idle(&mut rt);
    let tree = tree(&rt);
    assert!(!tree.contains("Cancel upload"), "{tree}");
    assert!(!tree.contains("Uploading"), "{tree}");
    // Still its player: a failed send plays from its local file until discarded.
    assert!(rt.presented(&element("-play")).is_some());
    assert!(
        (width(&rt) - uploading).abs() < 0.5,
        "{uploading} → {}",
        width(&rt)
    );
}
