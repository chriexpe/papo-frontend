//! Bounded latest-wins frame mailbox and dedicated writer loop with proof-only stall hook.

use crate::{
    session::{FAILED, LIVE, WAITING},
    surface::{PostOutcome, PresentWindow},
    verify::{ExpectedColor, verify_center_pixel},
};
use papo_call::{CpuVideoFrameLease, PackedBgra};
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, Instant},
};

const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) trait LeaseCheck: Send {
    fn is_current(&self) -> bool;
}

impl LeaseCheck for CpuVideoFrameLease {
    fn is_current(&self) -> bool {
        CpuVideoFrameLease::is_current(self)
    }
}

pub(crate) struct FrameWork<L> {
    pub lease: L,
    pub bgra: PackedBgra,
    pub track_id: String,
    pub assignment_id: u64,
    pub expected: ExpectedColor,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TileSnapshot {
    pub index: usize,
    pub expected: ExpectedColor,
    pub status: i32,
    pub stalled: bool,
    pub drained: bool,
    pub posts_ok: u64,
    pub posts_wrong_color: u64,
    pub posts_stale_dropped: u64,
    pub late_posts_after_cancel: u64,
    pub last_track_id: String,
    pub last_assignment_id: u64,
    pub last_rgb: [u8; 3],
}

pub(crate) struct TileWriter<L> {
    expected: ExpectedColor,
    inner: Mutex<WriterState<L>>,
    wake: Condvar,
}

struct WriterState<L> {
    pending: Option<FrameWork<L>>,
    canceled: bool,
    failed: bool,
    drained: bool,
    post: Option<Instant>,
    armed: bool,
    stalled: bool,
    posts_ok: u64,
    posts_wrong_color: u64,
    posts_stale_dropped: u64,
    late_posts_after_cancel: u64,
    last_track_id: String,
    last_assignment_id: u64,
    last_rgb: [u8; 3],
}

impl<L: LeaseCheck> TileWriter<L> {
    pub(crate) fn new(expected: ExpectedColor) -> Self {
        Self {
            expected,
            inner: Mutex::new(WriterState {
                pending: None,
                canceled: false,
                failed: false,
                drained: false,
                post: None,
                armed: false,
                stalled: false,
                posts_ok: 0,
                posts_wrong_color: 0,
                posts_stale_dropped: 0,
                late_posts_after_cancel: 0,
                last_track_id: String::new(),
                last_assignment_id: 0,
                last_rgb: [0, 0, 0],
            }),
            wake: Condvar::new(),
        }
    }

    pub(crate) fn offer(&self, frame: FrameWork<L>) {
        let mut state = lock(&self.inner);
        if state.canceled || state.failed {
            drop(state);
            drop(frame);
            return;
        }
        let old = state.pending.replace(frame);
        self.wake.notify_all();
        drop(state);
        drop(old);
    }

    pub(crate) fn mark_unrouted(&self) {
        let mut state = lock(&self.inner);
        state.post = None;
    }

    pub(crate) fn run_with_window<W: PresentWindow>(self: &Arc<Self>, window: W) {
        struct DrainGuard<'a, L>(&'a TileWriter<L>);

        impl<L> Drop for DrainGuard<'_, L> {
            fn drop(&mut self) {
                let mut state = lock(&self.0.inner);
                state.drained = true;
                self.0.wake.notify_all();
            }
        }

        // Declare drain guard before moving `window` so unwinding still releases window first.
        let _drain_guard = DrainGuard(self);
        let mut window = window;
        while let Some(frame) = self.next() {
            let check = verify_center_pixel(&frame.bgra, frame.expected);
            let still_current = || frame.lease.is_current() && self.is_active();
            let stall_hook = || self.stall_if_armed();
            let outcome = window.present_rgba(&frame.bgra, &still_current, &stall_hook);
            let mut state = lock(&self.inner);
            match outcome {
                PostOutcome::Posted => {
                    if state.canceled {
                        state.late_posts_after_cancel += 1;
                    } else if !state.failed {
                        state.post = Some(Instant::now());
                        state.last_track_id = frame.track_id.clone();
                        state.last_assignment_id = frame.assignment_id;
                        state.last_rgb = check.rgb;
                        if check.matched {
                            state.posts_ok += 1;
                        } else {
                            state.posts_wrong_color += 1;
                        }
                    }
                }
                PostOutcome::DiscardedStale => {
                    state.posts_stale_dropped += 1;
                }
                PostOutcome::Failed => {
                    if !state.canceled {
                        state.failed = true;
                    }
                }
            }
            drop(state);
            drop(frame);
        }
        // Release the native window before signaling drain so JVM TextureView destruction stays safe.
        drop(window);
    }

    fn next(&self) -> Option<FrameWork<L>> {
        let mut state = lock(&self.inner);
        loop {
            if state.canceled || state.failed {
                return None;
            }
            if let Some(frame) = state.pending.take() {
                return Some(frame);
            }
            state = self.wait(state);
        }
    }

    pub(crate) fn cancel(&self) {
        let mut state = lock(&self.inner);
        state.canceled = true;
        state.armed = false;
        let pending = state.pending.take();
        self.wake.notify_all();
        drop(state);
        drop(pending);
    }

    pub(crate) fn fail(&self) {
        let mut state = lock(&self.inner);
        state.failed = true;
        state.armed = false;
        let pending = state.pending.take();
        self.wake.notify_all();
        drop(state);
        drop(pending);
    }

    pub(crate) fn is_active(&self) -> bool {
        let state = lock(&self.inner);
        !state.canceled && !state.failed
    }

    pub(crate) fn is_drained(&self) -> bool {
        lock(&self.inner).drained
    }

    pub(crate) fn wait_drained(&self) {
        let mut state = lock(&self.inner);
        while !state.drained {
            state = self.wait(state);
        }
    }

    pub(crate) fn arm(&self) -> bool {
        let mut state = lock(&self.inner);
        if state.canceled || state.failed {
            return false;
        }
        if !state.stalled {
            state.armed = true;
        }
        true
    }

    pub(crate) fn release(&self) {
        let mut state = lock(&self.inner);
        state.armed = false;
        state.stalled = false;
        self.wake.notify_all();
    }

    /// Invoked after acquiring the native lock, before writing pixels.
    pub(crate) fn stall_if_armed(&self) {
        let mut state = lock(&self.inner);
        if !state.armed {
            return;
        }
        state.armed = false;
        state.stalled = true;
        self.wake.notify_all();
        while state.stalled {
            state = self.wait(state);
        }
    }

    pub(crate) fn snapshot(&self, index: usize) -> TileSnapshot {
        let state = lock(&self.inner);
        let status = if state.canceled {
            WAITING
        } else if state.failed {
            FAILED
        } else if state
            .post
            .is_some_and(|post| post.elapsed() <= FRAME_TIMEOUT)
        {
            LIVE
        } else {
            WAITING
        };
        TileSnapshot {
            index,
            expected: self.expected,
            status,
            stalled: state.stalled,
            drained: state.drained,
            posts_ok: state.posts_ok,
            posts_wrong_color: state.posts_wrong_color,
            posts_stale_dropped: state.posts_stale_dropped,
            late_posts_after_cancel: state.late_posts_after_cancel,
            last_track_id: state.last_track_id.clone(),
            last_assignment_id: state.last_assignment_id,
            last_rgb: state.last_rgb,
        }
    }

    fn wait<'a>(&self, state: MutexGuard<'a, WriterState<L>>) -> MutexGuard<'a, WriterState<L>> {
        self.wake
            .wait(state)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
