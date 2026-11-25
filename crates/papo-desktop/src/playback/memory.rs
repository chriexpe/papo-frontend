//! Handing freed decoder memory back to the system when a pipeline is let go (Linux with
//! glibc). Each player's streaming and decoder threads allocate frames from an allocator arena
//! of their own; freed, those frames stay in the arenas, so players opened and let go while a
//! conversation scrolls grew the process by about 200 MiB in 80 s (with two arenas it levelled
//! at about 410 MiB). `malloc_trim` gives every arena's free pages back.
//!
//! The one `unsafe` in the playback module. Safety: `malloc_trim` is glibc's, thread-safe (it
//! locks each arena while it trims), takes a plain integer and touches no memory of ours; it
//! only ever returns free pages. It runs at most every few seconds: it locks each arena in
//! turn, which a burst of players let go while scrolling shouldn't repeat. On other platforms
//! this does nothing.

#![cfg_attr(
    all(target_os = "linux", target_env = "gnu"),
    expect(
        unsafe_code,
        reason = "glibc's malloc_trim, a thread-safe call without arguments to check"
    )
)]

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The least time between two trims.
const TRIM_EVERY: Duration = Duration::from_secs(5);

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    /// glibc: returns free memory at the top of the heap and free pages in every arena to the
    /// system, keeping `pad` bytes at the top. Returns 1 when anything was released.
    fn malloc_trim(pad: usize) -> i32;
}

/// Returns the memory a let-go pipeline freed to the system.
pub(super) fn return_freed_memory() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        if last.is_some_and(|last| now.duration_since(last) < TRIM_EVERY) {
            return;
        }
        *last = Some(now);
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim is thread-safe, reads no pointer of ours and only releases free
    // pages (see the module header).
    unsafe {
        malloc_trim(0);
    }
}
