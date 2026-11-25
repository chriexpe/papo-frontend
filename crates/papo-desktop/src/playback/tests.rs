//! Playback against real GStreamer: media made by GStreamer itself (VP8 and Vorbis in `WebM`, as
//! the base and good plugins every Linux build has), probed from memory. Players run on the
//! system clock, so they are checked in the app, not here.

use std::fmt::Write as _;
use std::sync::Arc;

use gstreamer as gst;
use gstreamer::prelude::*;

use super::*;

/// Whether this machine has the elements the tests encode with (GStreamer's base and good
/// plugins: every Linux build of Papo needs them; a test without them checks nothing here).
fn encoders_installed() -> bool {
    papo_call::init_gstreamer().is_ok()
        && [
            "vp8enc",
            "vorbisenc",
            "webmmux",
            "videotestsrc",
            "audiotestsrc",
        ]
        .iter()
        .all(|name| gst::ElementFactory::find(name).is_some())
}

/// A `WebM` of `seconds` (30 frames a second at `size_px`, with a Vorbis track when `sound`),
/// encoded to a file and read back.
fn webm(size_px: [i32; 2], seconds: i32, sound: bool) -> Arc<[u8]> {
    papo_call::init_gstreamer().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("clip.webm");
    let frames = seconds * 30;
    let mut description = format!(
        "videotestsrc num-buffers={frames} ! video/x-raw,width={},height={},framerate=30/1 \
         ! vp8enc deadline=1 ! webmmux name=mux ! filesink location={}",
        size_px[0],
        size_px[1],
        path.display()
    );
    if sound {
        // 1,024 samples a buffer at 44.1 kHz: enough buffers for the same length.
        let buffers = seconds * 44_100 / 1_024 + 1;
        let _ = write!(
            description,
            " audiotestsrc num-buffers={buffers} ! audioconvert ! vorbisenc ! mux."
        );
    }
    let pipeline = gst::parse::launch(&description).unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let bus = pipeline.bus().unwrap();
    let message = bus
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(30),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .unwrap();
    assert!(matches!(message.view(), gst::MessageView::Eos(_)));
    pipeline.set_state(gst::State::Null).unwrap();
    Arc::from(std::fs::read(path).unwrap())
}

#[test]
fn a_frame_fits_its_box_keeping_its_shape_and_even_sides() {
    assert_eq!(fitted_frame([1920, 1080], [896, 800]), [896, 504]);
    assert_eq!(fitted_frame([1080, 1920], [896, 800]), [450, 800]);
    // Never enlarged.
    assert_eq!(fitted_frame([640, 360], [1792, 1600]), [640, 360]);
    // Odd sides become even, never below two.
    assert_eq!(fitted_frame([641, 361], [2000, 2000]), [640, 360]);
    assert_eq!(fitted_frame([4000, 1], [100, 100]), [100, 2]);
}

#[test]
fn a_probe_reads_length_size_sound_and_the_first_frame_at_the_box() {
    if !encoders_installed() {
        return;
    }
    let bytes = webm([320, 240], 1, true);
    let probe = engine().probe(bytes, [160, 160]).unwrap();
    let duration_ms = probe.info.duration_ms.unwrap();
    assert!((950..=1100).contains(&duration_ms), "{duration_ms} ms");
    assert_eq!(probe.info.video_px, Some([320, 240]));
    assert!(probe.info.has_audio);
    let poster = probe.poster.unwrap();
    assert_eq!(poster.size_px, [160, 120]);
    assert_eq!(poster.planes[0].len(), 160 * 120);
    assert_eq!(poster.planes[1].len(), 160 * 120 / 2);
}

#[test]
fn a_silent_clip_has_no_sound() {
    if !encoders_installed() {
        return;
    }
    let probe = engine().probe(webm([64, 48], 1, false), [64, 64]).unwrap();
    assert!(!probe.info.has_audio);
    assert_eq!(probe.poster.unwrap().size_px, [64, 48]);
}

#[test]
fn bytes_that_are_no_media_are_unsupported() {
    let bytes: Arc<[u8]> = Arc::from(vec![0x42_u8; 64 * 1024]);
    let failure = engine().probe(bytes, [64, 64]).err();
    assert_eq!(failure, Some(PlaybackFailure::Unsupported));
}

#[test]
fn a_cut_file_is_broken_or_unsupported_never_a_hang() {
    if !encoders_installed() {
        return;
    }
    let whole = webm([64, 48], 1, false);
    let cut: Arc<[u8]> = Arc::from(&whole[..whole.len() / 8]);
    // Whatever the demuxer makes of it, the probe answers.
    let _ = engine().probe(cut, [64, 64]);
}

#[test]
#[ignore = "the packaging gate, run inside the Flatpak runtime on PAPO_GATE_MEDIA"]
#[expect(
    clippy::print_stderr,
    reason = "the gate reports what it saw to whoever runs it"
)]
fn the_packaging_gate_plays_or_refuses_its_media() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Count(AtomicUsize);
    impl FrameSink for Count {
        fn frame(&self, _frame: PackedNv12) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    // Colon-separated files; PAPO_GATE_EXPECT is "play" (the codec extension present) or
    // "unsupported" (absent: the MP4's H.264 has no software decoder).
    let media = std::env::var("PAPO_GATE_MEDIA").unwrap();
    let expect = std::env::var("PAPO_GATE_EXPECT").unwrap();
    for path in media.split(':') {
        let bytes: Arc<[u8]> = Arc::from(std::fs::read(path).unwrap());
        let name = path.rsplit('/').next().unwrap_or(path);
        let probe = engine().probe(Arc::clone(&bytes), [896, 504]);
        let video = std::path::Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"));
        if expect == "unsupported" && video {
            eprintln!("GATE {name}: {:?}", probe.as_ref().err());
            assert_eq!(probe.err(), Some(PlaybackFailure::Unsupported));
            continue;
        }
        let info = probe.unwrap().info;
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let player = engine().open(
            bytes,
            PlayerSetup {
                frame_px: Some([896, 504]),
                playing: true,
                start_ms: 0,
                muted: true,
                volume: 0.0,
                looping: false,
            },
            Arc::clone(&count) as Arc<dyn FrameSink>,
            Arc::new(|| {}),
        );
        std::thread::sleep(std::time::Duration::from_secs(3));
        let status = player.status();
        let frames = count.0.load(Ordering::Relaxed);
        eprintln!("GATE {name}: {info:?}, {status:?}, {frames} frames");
        assert!(status.position_ms > 1_000, "{name} plays");
        if info.video_px.is_some() {
            assert!(frames > 30, "{name} draws frames");
        }
    }
}
