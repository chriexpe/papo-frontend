//! The waveform reduction and its cache, from synthetic samples.

use super::*;
use papo_sdk::ServerAddress;

fn account(host: &str) -> AccountId {
    AccountId::new(
        ServerAddress::parse(&format!("https://{host}.example.invalid"))
            .unwrap()
            .id(),
        Uuid::from_u128(1),
    )
}

fn key(host: &str, attachment: u128) -> WaveformKey {
    WaveformKey {
        account: account(host),
        attachment: Uuid::from_u128(attachment),
    }
}

/// Mono audio: `frames` frames of each amplitude in turn, as a square wave (RMS = amplitude).
fn steps(amplitudes: &[f32], frames: usize) -> Vec<f32> {
    amplitudes
        .iter()
        .flat_map(|amplitude| {
            (0..frames).map(move |i| if i % 2 == 0 { *amplitude } else { -*amplitude })
        })
        .collect()
}

fn known(bars: &[Option<f32>]) -> Vec<f32> {
    bars.iter()
        .map(|bar| (bar.unwrap() * 1000.0).round() / 1000.0)
        .collect()
}

fn finished(samples: &[f32], channels: u16, expected: Option<u64>) -> Waveform {
    let mut builder = WaveformBuilder::new(expected);
    builder.push(samples, channels);
    builder.finish()
}

#[test]
fn bars_are_each_stretch_rms_over_the_loudest() {
    // Bars and windows share their edges.
    let samples = steps(&[0.5, 0.25, 1.0, 0.0], 1024);
    let waveform = finished(&samples, 1, Some(4096));
    assert_eq!(known(&waveform.bars(4)), vec![0.5, 0.25, 1.0, 0.0]);
}

#[test]
fn a_bar_counts_a_straddled_window_by_its_share() {
    // 16-frame windows: one loud, one silent. Frames 8..24 take half of each.
    let mut samples = vec![1.0; 16];
    samples.extend([0.0; 16]);
    let waveform = finished(&samples, 1, Some(16 * WINDOWS as u64));
    let level = waveform.level(8, 24).unwrap();
    assert!((level - 0.5_f64.sqrt()).abs() < 1e-9);
    assert_eq!(waveform.level(16, 32), Some(0.0));
}

#[test]
fn audio_longer_than_expected_merges_windows_exactly() {
    // Sized for 2048 frames, given 16 times that: the windows halve four times.
    let samples = steps(&[0.2, 0.8], 16_384);
    let merged = finished(&samples, 1, Some(2048));
    let sized = finished(&samples, 1, Some(32_768));
    assert!(merged.is_complete());
    assert_eq!(known(&merged.bars(2)), vec![0.25, 1.0]);
    assert_eq!(merged.bars(64), sized.bars(64));
}

#[test]
fn unknown_length_grows_its_windows() {
    let samples = steps(&[1.0, 0.5, 0.5, 1.0], 8192);
    let waveform = finished(&samples, 1, None);
    assert_eq!(known(&waveform.bars(4)), vec![1.0, 0.5, 0.5, 1.0]);
}

#[test]
fn channels_are_mixed_by_mean_square() {
    // Left at 1, right silent: each frame's mean square is 0.5.
    let stereo: Vec<f32> = (0..2000).flat_map(|_| [1.0, 0.0]).collect();
    let mut both = stereo.clone();
    both.extend((0..2000).flat_map(|_| [1.0_f32, 1.0]));
    let waveform = finished(&both, 2, Some(4000));
    let bars = known(&waveform.bars(2));
    assert_eq!(bars, vec![0.707, 1.0]);
}

#[test]
fn bad_samples_never_panic_or_poison_the_bars() {
    let samples = [f32::NAN, f32::INFINITY, 4.0, -4.0, 0.5];
    let waveform = finished(&samples, 1, None);
    let bars = waveform.bars(5);
    assert!(bars.iter().flatten().all(|bar| (0.0..=1.0).contains(bar)));
    assert_eq!(bars[0], Some(0.0));
    assert_eq!(bars[2], Some(1.0));
    // No channels, and a trailing half frame, add nothing.
    let mut builder = WaveformBuilder::new(None);
    builder.push(&[0.5; 3], 0);
    builder.push(&[0.5; 3], 2);
    assert_eq!(builder.finish().bars(1), vec![Some(1.0)]);
}

#[test]
fn a_partial_waveform_leaves_the_undecoded_bars_unknown() {
    let mut builder = WaveformBuilder::new(Some(4096));
    builder.push(&steps(&[0.5, 1.0], 1024), 1);
    let partial = builder.snapshot();
    assert!(!partial.is_complete());
    let bars = partial.bars(4);
    assert_eq!(known(&bars[..2]), vec![0.5, 1.0]);
    assert_eq!(&bars[2..], &[None, None]);
    // Nothing decoded: every bar unknown.
    assert_eq!(
        WaveformBuilder::new(Some(10)).snapshot().bars(3),
        vec![None; 3]
    );
}

#[test]
fn a_snapshot_is_never_complete_nor_cached() {
    // Unknown length, and more decoded than was expected: still decoding.
    for expected in [None, Some(10)] {
        let mut builder = WaveformBuilder::new(expected);
        builder.push(&[0.5; 400], 1);
        let snapshot = builder.snapshot();
        assert!(!snapshot.is_complete());
        let cache = WaveformCache::new(4);
        cache.insert(key("a", 1), snapshot);
        assert!(cache.is_empty());
        assert!(builder.finish().is_complete());
    }
}

#[test]
fn a_wrong_expected_length_keeps_the_bars_fine() {
    let samples = steps(&[1.0, 0.5, 0.5, 1.0], 8192);
    let unknown = finished(&samples, 1, None);
    for expected in [Some(100 * 32_768), Some(u64::MAX), Some(1)] {
        assert_eq!(finished(&samples, 1, expected).bars(64), unknown.bars(64));
    }
}

#[test]
fn silence_is_flat_and_known() {
    let waveform = finished(&[0.0; 400], 1, Some(400));
    assert_eq!(waveform.bars(4), vec![Some(0.0); 4]);
    assert_eq!(waveform.bars(0), Vec::<Option<f32>>::new());
}

#[test]
fn more_bars_than_frames_reads_each_frame() {
    let waveform = finished(&[1.0, 0.0, 0.5], 1, None);
    assert_eq!(known(&waveform.bars(6)), vec![1.0, 1.0, 0.0, 0.0, 0.5, 0.5]);
}

#[test]
fn the_cache_keeps_complete_waveforms_by_account_and_attachment() {
    let cache = WaveformCache::new(4);
    let waveform = finished(&steps(&[1.0], 100), 1, None);
    cache.insert(key("a", 1), waveform.clone());
    assert_eq!(cache.get(key("a", 1)), Some(waveform));
    assert_eq!(cache.get(key("b", 1)), None);

    let mut builder = WaveformBuilder::new(Some(1000));
    builder.push(&[0.5; 10], 1);
    cache.insert(key("a", 2), builder.snapshot());
    assert_eq!(cache.get(key("a", 2)), None);
}

#[test]
fn the_cache_drops_the_least_recently_read_past_its_capacity() {
    let cache = WaveformCache::new(2);
    let waveform = finished(&[1.0], 1, None);
    cache.insert(key("a", 1), waveform.clone());
    cache.insert(key("a", 2), waveform.clone());
    let _ = cache.get(key("a", 1));
    cache.insert(key("a", 3), waveform.clone());
    assert_eq!(cache.len(), 2);
    assert!(cache.get(key("a", 2)).is_none());
    assert!(cache.get(key("a", 1)).is_some());
    // Replacing a key doesn't count twice.
    cache.insert(key("a", 3), waveform);
    assert_eq!(cache.len(), 2);
}

#[test]
fn forgetting_an_account_leaves_the_others() {
    let cache = WaveformCache::default();
    let waveform = finished(&[1.0], 1, None);
    cache.insert(key("a", 1), waveform.clone());
    cache.insert(key("b", 1), waveform);
    cache.forget_account(account("a"));
    assert!(cache.get(key("a", 1)).is_none());
    assert!(cache.get(key("b", 1)).is_some());
    assert_eq!(cache.len(), 1);
}
