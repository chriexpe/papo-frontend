//! Audio's rules of §14.5, without GStreamer.

use papo_sdk::{AccountId, ServerAddress, Uuid};
use papo_sdk_ui::{MAX_AHEAD_DURATION_MS, WaveformBuilder};

use super::*;

fn key(n: u128) -> MediaKey {
    MediaKey {
        account: AccountId::new(
            ServerAddress::parse("https://audio.example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(1),
        ),
        attachment: Uuid::from_u128(0x200 + n),
    }
}

/// A finished waveform of a tone.
fn tone() -> Waveform {
    let mut builder = WaveformBuilder::new(None);
    builder.push(&[0.5; 4_096], 1);
    builder.finish()
}

/// A store wanting audio `ns`, each read as `duration_ms` long with a waveform.
fn read(ns: &[u128], duration_ms: u64) -> AudioStore {
    let mut store = AudioStore::default();
    store.want(ns.iter().map(|n| key(*n)));
    while let Some(job) = store.next_read() {
        store.read(
            job.key,
            Ok(AudioRead {
                duration_ms: Some(duration_ms),
                waveform: Some(tone()),
            }),
        );
    }
    store
}

#[test]
fn audio_is_read_one_at_a_time_in_order() {
    let mut store = AudioStore::default();
    store.want([key(1), key(2)]);
    let first = store.next_read().unwrap();
    assert_eq!(first.key, key(1));
    assert_eq!(first.ahead_max_ms, Some(MAX_AHEAD_DURATION_MS));
    // The first is running: the next asked for is the second, never the first again.
    assert_eq!(store.next_read().unwrap().key, key(2));
    assert_eq!(store.next_read(), None);
}

#[test]
fn a_dropped_read_waits_again() {
    let mut store = AudioStore::default();
    store.want([key(1)]);
    store.next_read();
    store.read_dropped(key(1));
    assert_eq!(store.next_read().unwrap().key, key(1));
}

#[test]
fn it_shows_its_length_at_rest_and_flat_bars_before_its_waveform() {
    let mut store = AudioStore::default();
    store.want([key(1)]);
    store.next_read();
    store.read(
        key(1),
        Ok(AudioRead {
            duration_ms: Some(700_000),
            waveform: None,
        }),
    );
    let view = store.view();
    let shown = view.get(key(1)).unwrap();
    assert_eq!(shown.bars, None, "longer than ten minutes: flat");
    assert_eq!(shown.shown_ms(), Some(700_000));
    assert!(store.needs_waveform(key(1)));
}

#[test]
fn it_plays_only_when_pressed_and_shows_the_time_played() {
    let mut store = read(&[1], 60_000);
    assert_eq!(store.desired(), None);
    assert!(store.play(key(1)));
    let desired = store.desired().unwrap();
    assert!(desired.playing && desired.start_ms == 0);
    store.moved(key(1), 12_400);
    let view = store.view();
    let shown = view.get(key(1)).unwrap();
    assert_eq!(shown.shown_ms(), Some(12_400));
    assert_eq!(shown.played_bars(64), 13);
}

#[test]
fn one_audio_at_a_time_the_other_keeps_its_place() {
    let mut store = read(&[1, 2], 60_000);
    store.play(key(1));
    store.released(key(1), 5_000);
    store.play(key(2));
    assert_eq!(store.desired().unwrap().key, key(2));
    store.play(key(1));
    assert_eq!(store.desired().unwrap().start_ms, 5_000);
}

#[test]
fn it_keeps_playing_scrolled_away_and_pauses_when_the_conversation_is_left() {
    let mut store = read(&[1], 60_000);
    store.play(key(1));
    // Scrolled away: nothing near the view wants it.
    store.want([]);
    assert!(store.desired().unwrap().playing);
    store.released(key(1), 7_000);
    store.leave();
    assert_eq!(store.desired(), None);
    assert_eq!(store.position_ms(key(1)), 7_000);
}

#[test]
fn a_video_sound_or_a_hidden_window_pauses_it() {
    let mut store = read(&[1], 60_000);
    store.play(key(1));
    store.pause_all();
    let desired = store.desired().unwrap();
    assert!(!desired.playing, "paused, its player kept");
    assert!(!store.is_playing());
}

#[test]
fn at_its_end_it_rests_at_the_start() {
    let mut store = read(&[1], 60_000);
    store.play(key(1));
    store.moved(key(1), 59_900);
    store.ended(key(1));
    assert_eq!(store.desired(), None);
    let view = store.view();
    let shown = view.get(key(1)).unwrap();
    assert_eq!(shown.shown_ms(), Some(60_000));
    assert_eq!(shown.played_bars(64), 0);
}

#[test]
fn a_seek_is_clamped_to_its_length() {
    let mut store = read(&[1], 60_000);
    assert_eq!(store.seek(key(1), 90_000), 60_000);
    assert_eq!(store.view().get(key(1)).unwrap().shown_ms(), Some(60_000));
}

#[test]
fn a_finished_waveform_is_never_replaced_by_a_partial_one() {
    let mut store = read(&[1], 60_000);
    let finished = store.view().get(key(1)).unwrap().bars.clone();
    let mut partial = WaveformBuilder::new(Some(100_000));
    partial.push(&[0.1; 100], 1);
    store.filled(key(1), partial.snapshot());
    assert_eq!(store.view().get(key(1)).unwrap().bars, finished);
    assert!(!store.needs_waveform(key(1)));
}

#[test]
fn a_failure_falls_back_and_retry_reads_again() {
    let mut store = AudioStore::default();
    store.want([key(1)]);
    store.next_read();
    store.read(key(1), Err(PlaybackFailure::Broken));
    assert!(!store.play(key(1)), "a failed audio doesn't play");
    assert_eq!(
        store.view().get(key(1)).unwrap().reading,
        Reading::Failed(PlaybackFailure::Broken)
    );
    store.retry(key(1));
    assert_eq!(store.next_read().unwrap().key, key(1));
}

#[test]
fn a_failing_player_lets_its_audio_go() {
    let mut store = read(&[1], 60_000);
    store.play(key(1));
    store.player_failed(key(1), PlaybackFailure::Unsupported);
    assert_eq!(store.desired(), None);
}
