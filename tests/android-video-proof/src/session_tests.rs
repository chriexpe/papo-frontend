//! Unit tests verifying bounded 1-live + 1-retiring capacity and real-drain reaping.

use super::*;
use crate::{
    surface::{PostOutcome, PresentWindow},
    verify::ExpectedColor,
    writer::FrameWork,
};
use papo_call::PackedBgra;
use std::thread;

struct DummyLease;

impl LeaseCheck for DummyLease {
    fn is_current(&self) -> bool {
        true
    }
}

struct PassWindow;

impl PresentWindow for PassWindow {
    fn present_rgba(
        &mut self,
        _bgra: &PackedBgra,
        still_current: &dyn Fn() -> bool,
        stall_hook: &dyn Fn(),
    ) -> PostOutcome {
        if !still_current() {
            return PostOutcome::DiscardedStale;
        }
        stall_hook();
        if still_current() {
            PostOutcome::Posted
        } else {
            PostOutcome::DiscardedStale
        }
    }
}

fn red_frame(assign: u64) -> FrameWork<DummyLease> {
    let mut bytes = Vec::with_capacity(4 * 4 * 4);
    for _ in 0..16 {
        bytes.extend_from_slice(&[0, 0, 240, 255]);
    }
    FrameWork {
        lease: DummyLease,
        bgra: PackedBgra {
            size_px: [4, 4],
            bytes,
        },
        track_id: "papo-video-0".into(),
        assignment_id: assign,
        expected: ExpectedColor::Red,
    }
}

type TestSession = (
    Arc<SessionHandle<DummyLease>>,
    oneshot::Receiver<()>,
    thread::JoinHandle<()>,
);

fn spawn_session(reg: &mut SessionRegistry<DummyLease>) -> Result<TestSession, AttachError> {
    let generation = reg.begin_attach()?;
    let writer = Arc::new(TileWriter::new(ExpectedColor::Red));
    let (tx, rx) = oneshot::channel();
    let w = Arc::clone(&writer);
    let handle = Arc::new(SessionHandle::new(generation, vec![writer], tx));
    let t = thread::spawn(move || w.run_with_window(PassWindow));
    reg.finish_attach(Arc::clone(&handle));
    Ok((handle, rx, t))
}

#[test]
fn recreate_succeeds_while_retiring_writer_is_stalled_and_third_attach_is_refused() {
    let mut reg = SessionRegistry::new();
    let (s1, _rx1, t1) = spawn_session(&mut reg).unwrap();
    assert!(reg.arm_stall_live());
    s1.writers[0].offer(red_frame(1));
    while !s1.writers[0].snapshot(0).stalled {
        thread::yield_now();
    }

    // Detach gen 1 while its writer stays stalled, then attach gen 2 immediately.
    reg.detach(Some(s1.generation()));
    assert_eq!(reg.overall_status(), WAITING);
    let (s2, _rx2, t2) = spawn_session(&mut reg).unwrap();
    s2.writers[0].offer(red_frame(2));
    while reg.overall_status() != LIVE {
        thread::yield_now();
    }

    // A third attach while live + stalled retiring are both occupied must refuse with CapacityFull.
    assert_eq!(reg.begin_attach(), Err(AttachError::CapacityFull));
    let json = reg.telemetry_json(10, 1);
    assert!(json.contains("\"capacity_refusals\":1"));
    assert!(json.contains("\"retiring_generation\":1"));
    assert!(json.contains("\"live_generation\":2"));

    // Release stall on gen 1 + mark media drained -> gen 1 reaps cleanly, opening a slot.
    reg.release_all_stalls();
    s1.mark_media_drained();
    t1.join().unwrap();
    reg.reap();
    assert_eq!(reg.total_drained_sessions, 1);
    assert!(reg.retiring.is_none());

    // Now gen 3 can recreate immediately!
    reg.detach(Some(s2.generation()));
    let (s3, _rx3, t3) = spawn_session(&mut reg).unwrap();
    s2.mark_media_drained();
    t2.join().unwrap();
    s3.cancel();
    s3.mark_media_drained();
    t3.join().unwrap();
    reg.reap();
    assert_eq!(reg.total_drained_sessions, 3);
}

#[test]
fn real_drain_requires_both_media_and_writers_before_reaping() {
    let mut reg = SessionRegistry::new();
    let (s1, _rx1, t1) = spawn_session(&mut reg).unwrap();
    assert!(reg.arm_stall_live());
    s1.writers[0].offer(red_frame(1));
    while !s1.writers[0].snapshot(0).stalled {
        thread::yield_now();
    }
    reg.detach(Some(s1.generation()));

    // Media finished first, but writer is still stalled -> must NOT reap yet.
    s1.mark_media_drained();
    reg.reap();
    assert!(reg.retiring.is_some());
    assert_eq!(reg.total_drained_sessions, 0);

    // Release writer stall -> now both media and writer drained -> reaps!
    reg.release_all_stalls();
    t1.join().unwrap();
    reg.reap();
    assert!(reg.retiring.is_none());
    assert_eq!(reg.total_drained_sessions, 1);
}

#[test]
fn in_flight_attach_refuses_concurrent_attach_and_recovers_on_abort() {
    let mut reg = SessionRegistry::new();
    assert_eq!(reg.begin_attach(), Ok(1));
    assert_eq!(reg.begin_attach(), Err(AttachError::CapacityFull));
    assert_eq!(reg.capacity_refusals, 1);

    reg.abort_attach();
    let (s2, _rx2, t2) = spawn_session(&mut reg).unwrap();
    assert_eq!(s2.generation(), 2);
    s2.cancel();
    s2.mark_media_drained();
    t2.join().unwrap();
    reg.reap();
    assert_eq!(reg.total_drained_sessions, 1);
}
