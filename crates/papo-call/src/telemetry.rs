//! Proof-only bounded telemetry for 14c playback budget runs (`benchmark-telemetry` feature).
//! Disabled in default builds; never holds locks across I/O and never allocates on the hot path.

use gstreamer as gst;
use gstreamer::prelude::*;
use papo_sdk::{Uuid, VideoKind, VideoTarget};
use std::sync::{
    Mutex, MutexGuard, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Default preallocated capacity when [`reset`] is called.
pub const DEFAULT_RECORD_CAPACITY: usize = 131_072;

/// One access unit's marker packet passing the route fence downstream of the jitter buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JitterExitRecord {
    /// Target participant identity.
    pub participant: Uuid,
    /// Video stream kind.
    pub kind: VideoKind,
    /// Active SFU route assignment ID.
    pub assignment_id: u64,
    /// 90 kHz RTP timestamp from the marker packet.
    pub rtp_timestamp: u32,
    /// GStreamer buffer PTS in nanoseconds, if present.
    pub pts_ns: Option<u64>,
    /// `CLOCK_MONOTONIC` timestamp in nanoseconds when the marker packet exited the jitter gate.
    pub jitter_exit_ns: u64,
}

/// Snapshot of proof-only counters and preallocated jitter-exit records.
#[derive(Clone, Debug, Default)]
pub struct TelemetrySnapshot {
    /// RTP packets admitted by the route fence.
    pub fence_accepted_packets: u64,
    /// RTP packets dropped by the route fence.
    pub fence_rejected_packets: u64,
    /// Decoded frames delivered to the CPU appsink callback.
    pub decoded_frames: u64,
    /// First selected GStreamer video decoder factory name (`vp8dec` expected for software VP8).
    pub decoder_factory: Option<String>,
    /// Whether the bounded record buffer overflowed (invalidates the benchmark run).
    pub overflowed: bool,
    /// Whether `CLOCK_MONOTONIC` could not be read via GStreamer's monotonic clock.
    pub clock_unavailable: bool,
    /// Recorded access-unit exits in chronological arrival order.
    pub jitter_records: Vec<JitterExitRecord>,
}

struct BoundedJitterRing {
    records: Vec<JitterExitRecord>,
}

impl BoundedJitterRing {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            records: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, record: JitterExitRecord) {
        if self.records.len() >= self.records.capacity() {
            OVERFLOWED.store(true, Ordering::Relaxed);
            return;
        }
        self.records.push(record);
    }
}

static ACCEPTED_PACKETS: AtomicU64 = AtomicU64::new(0);
static REJECTED_PACKETS: AtomicU64 = AtomicU64::new(0);
static DECODED_FRAMES: AtomicU64 = AtomicU64::new(0);
static OVERFLOWED: AtomicBool = AtomicBool::new(false);
static CLOCK_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
static DECODER_FACTORY: Mutex<Option<String>> = Mutex::new(None);
static JITTER_RING: Mutex<Option<BoundedJitterRing>> = Mutex::new(None);
static MONOTONIC_CLOCK: OnceLock<Option<gst::Clock>> = OnceLock::new();

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Returns the current `CLOCK_MONOTONIC` timestamp in nanoseconds via a private monotonic clock.
#[must_use]
pub fn monotonic_ns() -> Option<u64> {
    let clock = MONOTONIC_CLOCK.get_or_init(|| {
        let clock: gst::SystemClock = gst::glib::Object::builder::<gst::SystemClock>()
            .property("clock-type", gst::ClockType::Monotonic)
            .build();
        (clock.property::<gst::ClockType>("clock-type") == gst::ClockType::Monotonic)
            .then(|| clock.upcast())
    });
    clock.as_ref().map(|clock| clock.time().nseconds())
}

/// Resets counters and preallocates the bounded jitter-exit record buffer for a run.
pub fn reset(capacity: usize) {
    ACCEPTED_PACKETS.store(0, Ordering::Relaxed);
    REJECTED_PACKETS.store(0, Ordering::Relaxed);
    DECODED_FRAMES.store(0, Ordering::Relaxed);
    OVERFLOWED.store(false, Ordering::Relaxed);
    CLOCK_UNAVAILABLE.store(false, Ordering::Relaxed);
    *lock(&DECODER_FACTORY) = None;
    *lock(&JITTER_RING) = Some(BoundedJitterRing::with_capacity(capacity));
}

/// Returns all recorded counters and drains the bounded jitter-exit buffer.
#[must_use]
pub fn take_snapshot() -> TelemetrySnapshot {
    let decoder_factory = lock(&DECODER_FACTORY).clone();
    let jitter_records = lock(&JITTER_RING)
        .take()
        .map(|ring| ring.records)
        .unwrap_or_default();
    TelemetrySnapshot {
        fence_accepted_packets: ACCEPTED_PACKETS.load(Ordering::Relaxed),
        fence_rejected_packets: REJECTED_PACKETS.load(Ordering::Relaxed),
        decoded_frames: DECODED_FRAMES.load(Ordering::Relaxed),
        decoder_factory,
        overflowed: OVERFLOWED.load(Ordering::Relaxed),
        clock_unavailable: CLOCK_UNAVAILABLE.load(Ordering::Relaxed),
        jitter_records,
    }
}

pub(crate) fn record_fence_rejected() {
    REJECTED_PACKETS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_fence_accepted(
    target: VideoTarget,
    assignment_id: u64,
    rtp_timestamp: u32,
    marker: bool,
    pts_ns: Option<u64>,
) {
    ACCEPTED_PACKETS.fetch_add(1, Ordering::Relaxed);
    if !marker {
        return;
    }
    let Some(jitter_exit_ns) = monotonic_ns() else {
        CLOCK_UNAVAILABLE.store(true, Ordering::Relaxed);
        return;
    };
    let mut ring = lock(&JITTER_RING);
    if let Some(ring) = ring.as_mut() {
        ring.push(JitterExitRecord {
            participant: target.participant,
            kind: target.kind,
            assignment_id,
            rtp_timestamp,
            pts_ns,
            jitter_exit_ns,
        });
    }
}

pub(crate) fn record_decoded_frame() {
    DECODED_FRAMES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_decoder(element: &gst::Element) {
    if let Some(factory) = element.factory() {
        let mut slot = lock(&DECODER_FACTORY);
        if slot.is_none() {
            *slot = Some(factory.name().to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_clock_advances_and_bounded_ring_flags_overflow() {
        gst::init().unwrap();
        let t0 = monotonic_ns().expect("monotonic clock available");
        let t1 = monotonic_ns().expect("monotonic clock available");
        assert!(t1 >= t0);

        reset(2);
        let target = VideoTarget {
            participant: Uuid::nil(),
            kind: VideoKind::Camera,
        };
        record_fence_accepted(target, 1, 100, false, Some(10));
        record_fence_accepted(target, 1, 100, true, Some(10));
        record_fence_accepted(target, 1, 200, true, Some(20));
        record_fence_accepted(target, 1, 300, true, Some(30));
        record_fence_rejected();
        record_decoded_frame();

        let snap = take_snapshot();
        assert_eq!(snap.fence_accepted_packets, 4);
        assert_eq!(snap.fence_rejected_packets, 1);
        assert_eq!(snap.decoded_frames, 1);
        assert!(snap.overflowed);
        assert!(!snap.clock_unavailable);
        assert_eq!(snap.jitter_records.len(), 2);
        assert_eq!(snap.jitter_records[0].rtp_timestamp, 100);
        assert_eq!(snap.jitter_records[1].rtp_timestamp, 200);

        // Late arrivals after take_snapshot() must not spuriously flag overflow on an uninitialized ring.
        reset(1);
        record_fence_accepted(target, 1, 400, true, Some(40));
        let drained = take_snapshot();
        assert!(!drained.overflowed);
        assert_eq!(drained.jitter_records.len(), 1);
        record_fence_accepted(target, 1, 500, true, Some(50));
        let post = take_snapshot();
        assert!(!post.overflowed);
        assert!(post.jitter_records.is_empty());
    }
}
