//! Unit tests exercising bounded latest-wins queuing, stalled FFI cancellation, and pixel checks.

use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

struct TestLease {
    current: AtomicBool,
}

impl TestLease {
    fn live() -> Self {
        Self {
            current: AtomicBool::new(true),
        }
    }
}

impl LeaseCheck for TestLease {
    fn is_current(&self) -> bool {
        self.current.load(Ordering::Acquire)
    }
}

fn test_frame(expected: ExpectedColor, bgra: [u8; 4], assign: u64) -> FrameWork<TestLease> {
    let mut bytes = Vec::with_capacity(4 * 4 * 4);
    for _ in 0..16 {
        bytes.extend_from_slice(&bgra);
    }
    FrameWork {
        lease: TestLease::live(),
        bgra: PackedBgra {
            size_px: [4, 4],
            bytes,
        },
        track_id: "papo-video-0".into(),
        assignment_id: assign,
        expected,
    }
}

struct FakeWindow {
    state: Arc<FakeWindowState>,
}

#[derive(Default)]
struct FakeWindowState {
    locked: AtomicUsize,
    wrote_pixels: AtomicUsize,
    zero_filled: AtomicUsize,
    dropped: AtomicBool,
}

impl Drop for FakeWindow {
    fn drop(&mut self) {
        self.state.dropped.store(true, Ordering::Release);
    }
}

impl PresentWindow for FakeWindow {
    fn present_rgba(
        &mut self,
        _bgra: &PackedBgra,
        still_current: &dyn Fn() -> bool,
        stall_hook: &dyn Fn(),
    ) -> PostOutcome {
        if !still_current() {
            return PostOutcome::DiscardedStale;
        }
        self.state.locked.fetch_add(1, Ordering::AcqRel);
        stall_hook();
        if still_current() {
            self.state.wrote_pixels.fetch_add(1, Ordering::AcqRel);
            PostOutcome::Posted
        } else {
            self.state.zero_filled.fetch_add(1, Ordering::AcqRel);
            PostOutcome::DiscardedStale
        }
    }
}

struct RecordingWindow {
    entered_tx: mpsc::Sender<()>,
    seen_tx: mpsc::Sender<u64>,
    gate_rx: mpsc::Receiver<()>,
}

impl PresentWindow for RecordingWindow {
    fn present_rgba(
        &mut self,
        _bgra: &PackedBgra,
        _still_current: &dyn Fn() -> bool,
        _stall_hook: &dyn Fn(),
    ) -> PostOutcome {
        let _ = self.entered_tx.send(());
        let _ = self.gate_rx.recv();
        let _ = self.seen_tx.send(1);
        PostOutcome::Posted
    }
}

#[test]
fn pending_frame_is_replaced_while_writer_is_busy() {
    let writer = Arc::new(TileWriter::new(ExpectedColor::Red));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (seen_tx, seen_rx) = mpsc::channel();
    let (gate_tx, gate_rx) = mpsc::channel();
    let w = Arc::clone(&writer);
    writer.offer(test_frame(ExpectedColor::Red, [0, 0, 250, 255], 1));
    let worker = thread::spawn(move || {
        w.run_with_window(RecordingWindow {
            entered_tx,
            seen_tx,
            gate_rx,
        })
    });

    // Wait until frame 1 is actively blocked inside present_rgba before offering frames 2 & 3.
    entered_rx.recv().unwrap();
    writer.offer(test_frame(ExpectedColor::Red, [0, 0, 250, 255], 2));
    writer.offer(test_frame(ExpectedColor::Red, [0, 0, 250, 255], 3));

    gate_tx.send(()).unwrap();
    assert_eq!(seen_rx.recv().unwrap(), 1);

    entered_rx.recv().unwrap();
    gate_tx.send(()).unwrap();
    assert_eq!(seen_rx.recv().unwrap(), 1);

    while writer.snapshot(0).posts_ok < 2 {
        thread::yield_now();
    }
    assert_eq!(writer.snapshot(0).last_assignment_id, 3);

    writer.cancel();
    let _ = gate_tx.send(());
    worker.join().unwrap();
    assert!(writer.is_drained());
}

#[test]
fn stalled_writer_discards_frame_and_drops_window_only_after_release() {
    let writer = Arc::new(TileWriter::new(ExpectedColor::Red));
    let fake_state = Arc::new(FakeWindowState::default());
    let w = Arc::clone(&writer);
    let fs = Arc::clone(&fake_state);
    assert!(writer.arm());
    writer.offer(test_frame(ExpectedColor::Red, [0, 0, 250, 255], 1));
    let worker = thread::spawn(move || w.run_with_window(FakeWindow { state: fs }));

    let mut guard = lock(&writer.inner);
    while !guard.stalled {
        guard = writer.wait(guard);
    }
    drop(guard);

    writer.offer(test_frame(ExpectedColor::Red, [0, 0, 250, 255], 2));
    writer.cancel();
    let snap = writer.snapshot(0);
    assert_eq!(snap.status, WAITING);
    assert!(snap.stalled);
    assert!(!writer.is_drained());
    assert!(!fake_state.dropped.load(Ordering::Acquire));

    writer.release();
    writer.wait_drained();
    worker.join().unwrap();

    let done = writer.snapshot(0);
    assert!(done.drained);
    assert!(!done.stalled);
    assert_eq!(done.status, WAITING);
    assert_eq!(done.posts_ok, 0);
    assert_eq!(done.late_posts_after_cancel, 0);
    assert_eq!(done.posts_stale_dropped, 1);
    assert_eq!(fake_state.locked.load(Ordering::Acquire), 1);
    assert_eq!(fake_state.wrote_pixels.load(Ordering::Acquire), 0);
    assert_eq!(fake_state.zero_filled.load(Ordering::Acquire), 1);
    assert!(fake_state.dropped.load(Ordering::Acquire));
}

#[test]
fn pixel_verification_counts_expected_and_wrong_colors() {
    let writer = Arc::new(TileWriter::new(ExpectedColor::Green));
    let fake_state = Arc::new(FakeWindowState::default());
    let w = Arc::clone(&writer);
    let fs = Arc::clone(&fake_state);
    let worker = thread::spawn(move || w.run_with_window(FakeWindow { state: fs }));

    writer.offer(test_frame(ExpectedColor::Green, [0, 128, 0, 255], 1));
    while writer.snapshot(0).posts_ok == 0 {
        thread::sleep(Duration::from_millis(2));
    }
    writer.offer(test_frame(ExpectedColor::Green, [0, 0, 250, 255], 2));
    while writer.snapshot(0).posts_wrong_color == 0 {
        thread::sleep(Duration::from_millis(2));
    }
    let snap = writer.snapshot(0);
    assert_eq!(snap.status, LIVE);
    assert_eq!(snap.posts_ok, 1);
    assert_eq!(snap.posts_wrong_color, 1);
    assert_eq!(snap.last_rgb, [250, 0, 0]);
    assert_eq!(snap.last_assignment_id, 2);

    writer.cancel();
    writer.wait_drained();
    worker.join().unwrap();
}
