//! Your unsent message's audio in the shell (`docs/design.md` §14.3, §14.5), headless and
//! without GStreamer: sent, it keeps playing under the server's attachment whichever of the
//! queue and the timeline reports it first; a read still running from the local file is read
//! again from the server; discarded, it stops.

use std::sync::Arc;

use papo_sdk::{Message, OperationState, Uuid};
use rune::Runtime;
use serde_json::json;

use super::Shell;
use super::audio::{Fake, audio_id, audio_key_of, clip};
use super::replying::{conversation, message_id};
use crate::audio_store::{AudioKey, Reading};
use crate::playback::{AudioRead, PlaybackFailure, PlayerState};

const LOCAL: u128 = 0x9500;

fn unsent_key(shell: &Shell) -> AudioKey {
    AudioKey::Unsent {
        account: shell.tabs.shown().unwrap().key.account,
        local: Uuid::from_u128(LOCAL),
        index: 0,
    }
}

/// The queue's snapshot: your message with one audio, in `state`, sent as attachment
/// `audio_id` when sent.
fn queue(state: OperationState) -> papo_sdk::OutgoingSnapshot {
    let sent: Message = serde_json::from_value(json!({
        "id": message_id(9).to_string(),
        "content": "",
        "created_at": "2026-11-20T12:03:00Z",
        "attachments": [{
            "id": audio_id().to_string(),
            "mime_type": "audio/ogg",
            "original_file_name": "tone.ogg",
            "size_bytes": 96_000,
        }],
    }))
    .unwrap();
    papo_sdk::OutgoingSnapshot {
        account: None,
        version: 1,
        operations: vec![Arc::new(papo_sdk::Operation {
            id: Uuid::from_u128(LOCAL),
            channel_id: Uuid::from_u128(1),
            kind: Arc::new(papo_sdk::OperationKind::Message(
                papo_sdk::OutgoingMessage {
                    attachments: vec![papo_sdk::OutgoingAttachment::from_bytes(
                        "tone.ogg",
                        vec![0; 4],
                    )],
                    ..papo_sdk::OutgoingMessage::default()
                },
            )),
            state,
            attempts: 1,
            depends_on: None,
            failure: None,
            message_id: None,
            history_projected: false,
            message: Some(Arc::new(sent)),
            created_at_us: 0,
        })],
        upload_progress: None,
        fault: None,
    }
}

/// The conversation, with your unsent message's audio drawn as audio and wanted.
fn with_unsent() -> Runtime<Shell> {
    let mut rt = conversation("");
    rt.update(|shell, _tx| {
        shell.unsent.insert(
            Uuid::from_u128(LOCAL),
            vec![crate::unsent_media::UnsentFile {
                kind: crate::composer::tray::TileKind::Audio,
                size_bytes: Some(96_000),
                media_size: papo_sdk_ui::MediaSize::Unknown,
                image: None,
                sharp: None,
                video: None,
            }],
        );
        let from = unsent_key(shell);
        shell.audio.store.want([from]);
    });
    rt
}

/// Its audio read and playing at 2 s in its player.
fn playing(shell: &mut Shell) {
    let from = unsent_key(shell);
    let job = shell.audio.store.next_read().unwrap();
    shell.audio.store.read(job.key, Ok(clip(30_000)));
    assert!(shell.audio.store.play(from));
    shell.audio.opening = Some((from, 7));
    let want = shell.audio.store.desired().unwrap();
    let player = Fake {
        state: PlayerState::Playing,
        position_ms: 2_000,
    };
    shell.audio_opened(7, want, Some(Box::new(player)));
}

#[test]
fn a_sent_audio_keeps_playing_under_the_server_attachment() {
    let mut rt = with_unsent();
    rt.update(|shell, _tx| {
        playing(shell);
        let account = shell.tabs.shown().unwrap().key.account;
        shell.unsent_audio_follow(account, &queue(OperationState::Sent), &[]);
        let unsent = unsent_key(shell);
        let received = audio_key_of(shell);
        let audio = &shell.audio;
        assert_eq!(audio.live.as_ref().map(|live| live.key), Some(received));
        assert!(audio.store.shown(received).unwrap().playing);
        // A row or a want still naming the unsent file finds it there.
        assert_eq!(audio.store.resolve(unsent), received);
        assert!(audio.store.view().get(unsent).unwrap().playing);
        let AudioKey::Received(media) = received else {
            unreachable!()
        };
        let cached = papo_sdk_ui::WaveformKey {
            account: media.account,
            attachment: media.id,
        };
        assert!(
            audio.waveforms.get(cached).is_some(),
            "its waveform is cached"
        );
    });
}

#[test]
fn shown_sent_by_the_timeline_first_it_is_handed_over_once() {
    let mut rt = with_unsent();
    rt.update(|shell, _tx| {
        playing(shell);
        let (from, to) = (unsent_key(shell), audio_key_of(shell));
        // The timeline's echo turned Sent: `want_media` hands it over.
        shell.audio_sent(from, to);
        shell.audio.store.want([from, to]);
        // Then the queue says so too: nothing more moves, nothing is read again.
        let account = shell.tabs.shown().unwrap().key.account;
        shell.unsent_audio_follow(account, &queue(OperationState::Sent), &[]);
        let shown = shell.audio.store.shown(to).unwrap();
        assert!(shown.playing);
        assert_eq!(shown.duration_ms(), Some(30_000));
        assert_eq!(shell.audio.store.next_read(), None);
    });
}

#[test]
fn a_local_read_running_at_send_is_read_again_from_the_server() {
    let mut rt = with_unsent();
    rt.update(|shell, _tx| {
        let (from, to) = (unsent_key(shell), audio_key_of(shell));
        let job = shell.audio.store.next_read().unwrap();
        shell.audio.reading = Some(super::super::audio_reads::Running::new(job.key, 1));
        shell.audio_sent(from, to);
        // The local read answers late, failed (its file was let go): it is dropped.
        shell.audio_read(1, Err(PlaybackFailure::Broken));
        assert_eq!(
            shell.audio.store.shown(to).unwrap().reading,
            Reading::Queued
        );
        assert_eq!(shell.audio.store.next_read().unwrap().key, to);
    });
}

#[test]
fn a_server_read_running_at_send_never_stops_the_audio_playing() {
    let mut rt = with_unsent();
    rt.update(|shell, _tx| {
        playing(shell);
        let (from, to) = (unsent_key(shell), audio_key_of(shell));
        // The history row asked for the server's copy before the handover.
        shell.audio.store.want([from, to]);
        let job = shell.audio.store.next_read().unwrap();
        assert_eq!(job.key, to);
        shell.audio.reading = Some(super::super::audio_reads::Running::new(to, 3));
        shell.audio_sent(from, to);
        shell.audio_read(3, Err(PlaybackFailure::Broken));
        let shown = shell.audio.store.shown(to).unwrap();
        assert!(shown.playing);
        assert_eq!(shown.duration_ms(), Some(30_000));
    });
}

#[test]
fn a_read_let_go_answers_nothing_even_if_wanted_again() {
    let mut rt = with_unsent();
    rt.update(|shell, _tx| {
        let from = unsent_key(shell);
        let job = shell.audio.store.next_read().unwrap();
        let running = super::super::audio_reads::Running::new(job.key, 1);
        running.stop();
        shell.audio.reading = Some(running);
        // Scrolled away and back before it answered, empty for being cancelled.
        shell.audio_read(
            1,
            Ok(AudioRead {
                duration_ms: None,
                waveform: None,
            }),
        );
        assert_eq!(
            shell.audio.store.shown(from).unwrap().reading,
            Reading::Queued
        );
    });
}

#[test]
fn a_discarded_audio_stops() {
    let mut rt = with_unsent();
    rt.update(|shell, _tx| {
        playing(shell);
        let account = shell.tabs.shown().unwrap().key.account;
        shell.unsent_audio_follow(
            account,
            &queue(OperationState::Failed),
            &[Uuid::from_u128(LOCAL)],
        );
        let unsent = unsent_key(shell);
        assert!(shell.audio.live.is_none());
        assert_eq!(shell.audio.store.shown(unsent), None);
        assert_eq!(shell.audio.store.desired(), None);
    });
}
