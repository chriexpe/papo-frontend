//! Audio in the shell (`docs/design.md` §14.5), headless and without GStreamer: Space on a
//! focused message plays its audio and quiets videos, a played audio replays from the start
//! after it ended, leaving the conversation pauses it, and how a read that was stopped or ran
//! out of time lands. Your unsent audio is `unsent_audio`'s.

use std::sync::Arc;

use papo_sdk::Message;
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{TimelineItem, WaveformBuilder};
use rune::{Key, Runtime};
use serde_json::json;

use super::replying::{conversation, message_id};
use super::{FRAME_S, Shell, press};
use crate::audio_store::{AudioDesired, AudioKey, Reading};
use crate::media_store::MediaKey;
use crate::playback::{AudioRead, PlaybackFailure, Player, PlayerState, PlayerStatus, SeekMode};
use crate::timeline::rows::message_key;

pub(super) fn audio_id() -> papo_sdk::Uuid {
    papo_sdk::Uuid::from_u128(0x9400)
}

/// The conversation with message 3 holding one audio, wanted; `read` lands its read.
fn with_audio(read: Option<Result<AudioRead, PlaybackFailure>>) -> Runtime<Shell> {
    let mut rt = conversation("");
    rt.update(move |shell, _tx| {
        let id = message_id(3);
        let message: Message = serde_json::from_value(json!({
            "id": id.to_string(),
            "author_id": papo_sdk::Uuid::from_u128(0x4002).to_string(),
            "content": "",
            "created_at": "2026-11-20T12:03:00Z",
            "attachments": [{
                "id": audio_id().to_string(),
                "mime_type": "video/mp4",
                "original_file_name": "Voice note.m4a",
                "size_bytes": 96_000,
            }],
        }))
        .unwrap();
        let open = shell.tabs.shown_mut().unwrap();
        open.items.set(
            2,
            Arc::new(TimelineItem::Message {
                id,
                message: Arc::new(message),
                content: Arc::new(MessageContent::parse("")),
                reply: None,
                pending: None,
            }),
        );
        let key = audio_key_of(shell);
        shell.audio.store.want([key]);
        if let Some(read) = read {
            let job = shell.audio.store.next_read().unwrap();
            shell.audio.reading = Some(super::super::audio_reads::Running::new(job.key, 1));
            shell.audio_read(1, read);
        }
    });
    settle(&mut rt);
    rt
}

pub(super) fn audio_key_of(shell: &Shell) -> AudioKey {
    AudioKey::Received(MediaKey {
        account: shell.tabs.shown().unwrap().key.account,
        attachment: audio_id(),
    })
}

/// Audio `duration_ms` long, its waveform read.
pub(super) fn clip(duration_ms: u64) -> AudioRead {
    let mut builder = WaveformBuilder::new(None);
    builder.push(&[0.5; 1_024], 1);
    AudioRead {
        duration_ms: Some(duration_ms),
        waveform: Some(builder.finish()),
    }
}

fn settle(rt: &mut Runtime<Shell>) {
    for _ in 0..4 {
        rt.frame(FRAME_S);
    }
}

/// A player that says it is in `state` at `position_ms`.
pub(super) struct Fake {
    pub(super) state: PlayerState,
    pub(super) position_ms: u64,
}

impl Player for Fake {
    fn play(&self) {}
    fn pause(&self) {}
    fn seek(&self, _position_ms: u64, _mode: SeekMode) {}
    fn set_looping(&self, _looping: bool) {}
    fn set_muted(&self, _muted: bool) {}
    fn set_volume(&self, _volume: f32) {}
    fn set_frame_box(&self, _box_px: [u32; 2]) {}
    fn status(&self) -> PlayerStatus {
        PlayerStatus {
            state: self.state,
            position_ms: self.position_ms,
            duration_ms: Some(30_000),
        }
    }
}

#[test]
fn space_on_a_focused_message_plays_its_audio() {
    let mut rt = with_audio(Some(Ok(clip(30_000))));
    rt.update(|_shell, tx| tx.focus(message_key(message_id(3)), true));
    settle(&mut rt);
    press(&mut rt, Key::Text(" ".to_owned()));
    let key = audio_key_of(&rt.state);
    assert!(rt.state.audio.store.shown(key).unwrap().playing);
}

#[test]
fn an_audio_that_ended_plays_again_from_the_start() {
    let mut rt = with_audio(Some(Ok(clip(30_000))));
    rt.update(|shell, _tx| {
        let key = audio_key_of(shell);
        assert!(shell.audio.store.play(key));
        // The player lands at its end before its open answer does.
        shell.audio.opening = Some((key, 7));
        let want = AudioDesired {
            key,
            playing: true,
            start_ms: 0,
        };
        let ended = Fake {
            state: PlayerState::Ended,
            position_ms: 30_000,
        };
        shell.audio_opened(7, want, Some(Box::new(ended)));
        assert_eq!(shell.audio.store.desired(), None, "ended: its player went");
        assert!(shell.audio.store.play(key));
        assert_eq!(shell.audio.store.desired().unwrap().start_ms, 0);
    });
}

#[test]
fn a_read_that_runs_out_of_time_fails_and_waits_for_retry() {
    let rt = with_audio(Some(Err(PlaybackFailure::Broken)));
    let key = audio_key_of(&rt.state);
    assert_eq!(
        rt.state.audio.store.shown(key).unwrap().reading,
        Reading::Failed(PlaybackFailure::Broken)
    );
}

#[test]
fn a_read_let_go_because_it_left_the_view_waits_again() {
    let mut rt = with_audio(None);
    rt.update(|shell, _tx| {
        let key = audio_key_of(shell);
        let job = shell.audio.store.next_read().unwrap();
        let running = super::super::audio_reads::Running::new(job.key, 1);
        running.stop();
        shell.audio.reading = Some(running);
        shell.audio.store.want([]);
        shell.audio_read(
            1,
            Ok(AudioRead {
                duration_ms: None,
                waveform: None,
            }),
        );
        assert_eq!(
            shell.audio.store.shown(key).unwrap().reading,
            Reading::Queued
        );
        // Unless what it learned stands: a format nothing here plays.
        shell.audio.store.want([key]);
        let job = shell.audio.store.next_read().unwrap();
        let running = super::super::audio_reads::Running::new(job.key, 2);
        running.stop();
        shell.audio.reading = Some(running);
        shell.audio.store.want([]);
        shell.audio_read(2, Err(PlaybackFailure::Unsupported));
        assert_eq!(
            shell.audio.store.shown(key).unwrap().reading,
            Reading::Failed(PlaybackFailure::Unsupported)
        );
    });
}

#[test]
fn leaving_the_conversation_pauses_its_audio() {
    let mut rt = with_audio(Some(Ok(clip(30_000))));
    rt.update(|shell, tx| {
        let key = audio_key_of(shell);
        shell.audio.store.play(key);
        // It plays in another conversation than the one shown.
        let mut elsewhere = shell.tabs.shown().unwrap().key;
        elsewhere.channel = papo_sdk::Uuid::from_u128(0xFFFF);
        shell.audio.conversation = Some(elsewhere);
        assert!(shell.poll_audio(0.0, tx));
        assert_eq!(shell.audio.store.desired(), None);
    });
}
