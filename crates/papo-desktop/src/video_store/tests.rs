//! The video rules of §14.5 and §5, without GStreamer.

use std::collections::HashMap;

use papo_sdk::{AccountId, ServerAddress, Uuid};

use super::*;

fn key(n: u128) -> MediaKey {
    MediaKey::attachment(
        AccountId::new(
            ServerAddress::parse("https://videos.example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(1),
        ),
        Uuid::from_u128(0x100 + n),
    )
}

fn want(n: u128) -> VideoWant {
    VideoWant {
        key: key(n),
        box_px: [896, 504],
        in_album: false,
        hidden: false,
    }
}

fn clip(duration_ms: u64) -> MediaInfo {
    MediaInfo {
        duration_ms: Some(duration_ms),
        video_px: Some([640, 360]),
        has_audio: true,
    }
}

/// A store wanting `wants`, each read as `info` in turn, autoplay on.
fn read(wants: &[VideoWant], info: MediaInfo) -> VideoStore {
    let mut store = VideoStore::default();
    store.set_autoplay(true);
    let mut jobs = store.want(wants.iter().copied());
    while let Some(job) = jobs.pop() {
        jobs.extend(store.probed(job.key, Ok(info)));
    }
    store
}

fn show(store: &mut VideoStore, shares: &[(u128, f32)]) {
    let shares: HashMap<MediaKey, f32> = shares.iter().map(|(n, s)| (key(*n), *s)).collect();
    store.shown(&shares);
}

#[test]
fn videos_are_probed_one_at_a_time_in_order() {
    let mut store = VideoStore::default();
    let jobs = store.want([want(1), want(2)]);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].key, key(1));
    assert_eq!(jobs[0].box_px, [896, 504]);
    let next = store.probed(key(1), Ok(clip(5_000)));
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].key, key(2));
}

#[test]
fn a_queued_video_no_longer_wanted_is_not_probed() {
    let mut store = VideoStore::default();
    store.want([want(1), want(2)]);
    store.want([want(1)]);
    assert_eq!(store.probed(key(1), Ok(clip(5_000))), Vec::new());
}

#[test]
fn a_short_clip_plays_muted_and_loops_while_half_shows() {
    let mut store = read(&[want(1)], clip(8_050));
    assert!(store.desired().is_empty());
    show(&mut store, &[(1, 0.6)]);
    let desired = store.desired()[&key(1)];
    assert!(desired.playing && desired.muted && desired.looping);
    show(&mut store, &[(1, 0.4)]);
    // Paused, kept live while any of it shows.
    assert!(!store.desired()[&key(1)].playing);
    show(&mut store, &[(1, 0.9)]);
    assert!(store.desired()[&key(1)].playing);
    show(&mut store, &[(1, 0.0)]);
    assert!(store.desired().is_empty());
}

#[test]
fn a_long_video_or_one_in_an_album_waits_to_be_played() {
    let mut long = read(&[want(1)], clip(61_000));
    show(&mut long, &[(1, 1.0)]);
    assert!(long.desired().is_empty());
    let mut album = read(
        &[VideoWant {
            in_album: true,
            ..want(1)
        }],
        clip(5_000),
    );
    show(&mut album, &[(1, 1.0)]);
    assert!(album.desired().is_empty());
}

#[test]
fn autoplay_off_or_hidden_never_plays_by_itself() {
    let mut store = read(&[want(1)], clip(5_000));
    store.set_autoplay(false);
    show(&mut store, &[(1, 1.0)]);
    assert!(store.desired().is_empty());
    let mut hidden = read(
        &[VideoWant {
            hidden: true,
            ..want(1)
        }],
        clip(5_000),
    );
    show(&mut hidden, &[(1, 1.0)]);
    assert!(hidden.desired().is_empty());
}

#[test]
fn played_with_sound_it_pauses_when_it_leaves_and_stays_paused() {
    let mut store = read(&[want(1)], clip(120_000));
    show(&mut store, &[(1, 1.0)]);
    store.play(key(1));
    let desired = store.desired()[&key(1)];
    assert!(desired.playing && !desired.muted && !desired.looping);
    show(&mut store, &[(1, 0.2)]);
    // Paused, kept live while it still shows.
    assert!(!store.desired()[&key(1)].playing);
    show(&mut store, &[(1, 1.0)]);
    assert!(!store.desired()[&key(1)].playing);
    show(&mut store, &[(1, 0.0)]);
    assert!(store.desired().is_empty());
}

#[test]
fn a_hidden_window_plays_nothing_and_sound_stays_paused() {
    let mut store = read(&[want(1), want(2)], clip(5_000));
    show(&mut store, &[(1, 1.0), (2, 1.0)]);
    store.set_sound(key(1), true);
    store.set_window_shown(false, false);
    assert!(store.desired().is_empty());
    store.set_window_shown(true, false);
    let desired = store.desired();
    assert!(!desired[&key(1)].playing, "played with sound: stays paused");
    assert!(desired[&key(2)].playing, "the autoplay plays again");
}

#[test]
fn kept_playing_when_hidden_only_sound_goes_on() {
    let mut store = read(&[want(1), want(2)], clip(5_000));
    show(&mut store, &[(1, 1.0), (2, 1.0)]);
    store.set_sound(key(1), true);
    store.set_window_shown(false, true);
    let desired = store.desired();
    assert_eq!(desired.len(), 1, "the muted autoplay stops");
    assert!(desired[&key(1)].playing && !desired[&key(1)].muted);
    store.set_window_shown(true, true);
    assert!(store.desired()[&key(2)].playing);
}

#[test]
fn one_sound_at_a_time() {
    let mut store = read(&[want(1), want(2)], clip(5_000));
    show(&mut store, &[(1, 1.0), (2, 1.0)]);
    store.set_sound(key(1), true);
    assert!(!store.desired()[&key(1)].muted);
    store.set_sound(key(2), true);
    let desired = store.desired();
    assert!(desired[&key(1)].muted, "the first was muted");
    assert!(desired[&key(1)].playing, "and keeps playing");
    assert!(!desired[&key(2)].muted);
    store.mute_all();
    assert!(store.desired()[&key(2)].muted);
}

#[test]
fn turning_sound_off_keeps_it_playing_muted() {
    let mut store = read(&[want(1)], clip(5_000));
    show(&mut store, &[(1, 1.0)]);
    store.set_sound(key(1), true);
    store.set_sound(key(1), false);
    let desired = store.desired()[&key(1)];
    assert!(desired.playing && desired.muted && !desired.looping);
}

#[test]
fn no_more_than_the_cap_are_live() {
    let wants: Vec<VideoWant> = (1..=6).map(want).collect();
    let mut store = read(&wants, clip(5_000));
    let shares: Vec<(u128, f32)> = (1..=6_u8)
        .map(|n| (u128::from(n), 0.5 + f32::from(n) / 20.0))
        .collect();
    show(&mut store, &shares);
    let desired = store.desired();
    assert_eq!(desired.len(), MAX_LIVE);
    // The most shown play.
    assert!(desired.contains_key(&key(6)) && desired.contains_key(&key(4)));
    assert!(!desired.contains_key(&key(1)));
}

#[test]
fn what_the_person_plays_comes_before_autoplay() {
    let wants: Vec<VideoWant> = (1..=4).map(want).collect();
    let mut store = read(&wants, clip(5_000));
    show(&mut store, &[(1, 0.6), (2, 1.0), (3, 1.0), (4, 1.0)]);
    store.play(key(1));
    assert!(store.desired().contains_key(&key(1)));
}

#[test]
fn an_ended_video_plays_again_from_the_start() {
    let mut store = read(&[want(1)], clip(90_000));
    show(&mut store, &[(1, 1.0)]);
    store.play(key(1));
    store.released(key(1), 40_000);
    store.ended(key(1));
    assert!(store.desired().is_empty());
    assert!(store.view().get(key(1)).unwrap().ended);
    store.play(key(1));
    assert_eq!(store.desired()[&key(1)].start_ms, 0);
}

#[test]
fn a_released_video_resumes_where_it_was() {
    let mut store = read(&[want(1)], clip(90_000));
    show(&mut store, &[(1, 1.0)]);
    store.play(key(1));
    store.released(key(1), 12_000);
    assert_eq!(store.desired()[&key(1)].start_ms, 12_000);
}

#[test]
fn the_viewer_pauses_it_inline_and_hands_back_its_place() {
    let mut store = read(&[want(1)], clip(5_000));
    show(&mut store, &[(1, 1.0)]);
    store.set_in_viewer(Some(key(1)), None);
    assert!(store.desired().is_empty());
    assert!(!store.view().get(key(1)).unwrap().playing);
    store.set_in_viewer(None, Some(3_000));
    // Nobody pressed it: autoplay takes it again, from where the viewer left it.
    let desired = store.desired()[&key(1)];
    assert!(desired.playing && desired.start_ms == 3_000);
}

#[test]
fn a_failure_shows_and_retry_reads_again() {
    let mut store = VideoStore::default();
    store.want([want(1)]);
    store.probed(key(1), Err(PlaybackFailure::Unsupported));
    assert_eq!(
        store.view().get(key(1)).unwrap().reading,
        Reading::Failed(PlaybackFailure::Unsupported)
    );
    let jobs = store.retry(key(1));
    assert_eq!(jobs.len(), 1);
}

#[test]
fn play_and_pause_keep_an_autoplays_silence() {
    let mut store = read(&[want(1)], clip(5_000));
    show(&mut store, &[(1, 1.0)]);
    store.pause(key(1));
    assert!(!store.desired()[&key(1)].playing);
    store.play(key(1));
    let desired = store.desired()[&key(1)];
    assert!(desired.playing && desired.muted);
}

#[test]
fn a_seek_keeps_an_autoplay_playing_as_the_persons() {
    let mut store = read(&[want(1)], clip(5_000));
    show(&mut store, &[(1, 1.0)]);
    store.seeked(key(1), 2_000);
    let desired = store.desired()[&key(1)];
    assert!(desired.playing && desired.muted && !desired.looping);
}
