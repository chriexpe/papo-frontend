//! Handing freed decoder memory back to the system when a playback, probe or waveform pipeline
//! is let go (Linux with glibc). Process startup (`allocator::configure`) locks glibc's
//! `mmap_threshold` and `trim_threshold` at 128 KiB and bounds arenas so large frame buffers
//! return via `munmap`; this module complements that by releasing smaller (`< 128 KiB`) interior
//! free pages that GStreamer/libav pipelines leave fragmented in the arenas after teardown.
//!
//! The one `unsafe` in the playback module. Safety: `malloc_trim` is glibc's, thread-safe (it
//! locks each arena while it trims), takes a plain integer and touches no memory of ours; it
//! only ever returns free pages. It runs on a thread of its own, at most every few seconds: it
//! locks each arena in turn, which a burst of players let go while scrolling shouldn't repeat.
//! A burst's requests are served by one trim after the wait, so the last pipeline let go is
//! always trimmed. On other platforms this does nothing.

#![cfg_attr(
    all(target_os = "linux", target_env = "gnu"),
    expect(
        unsafe_code,
        reason = "glibc's malloc_trim, a thread-safe call without arguments to check"
    )
)]

use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

/// The least time between two trims.
const TRIM_EVERY: Duration = Duration::from_secs(5);

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    /// glibc: returns free memory at the top of the heap and free pages in every arena to the
    /// system, keeping `pad` bytes at the top. Returns 1 when anything was released.
    fn malloc_trim(pad: usize) -> i32;
}

/// Returns the memory a let-go pipeline freed to the system, soon: the trimming thread does it,
/// at most every [`TRIM_EVERY`]. Without that thread (it couldn't start) nothing is trimmed.
pub(super) fn return_freed_memory() {
    static REQUESTS: OnceLock<Sender<()>> = OnceLock::new();
    let requests = REQUESTS.get_or_init(|| {
        let (requests, received) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("papo-trim".to_owned())
            .spawn(move || trim_on_request(&received));
        requests
    });
    let _ = requests.send(());
}

/// The trimming thread: one trim for every request made since the last, then a wait.
fn trim_on_request(requests: &Receiver<()>) {
    while requests.recv().is_ok() {
        while requests.try_recv().is_ok() {}
        trim();
        std::thread::sleep(TRIM_EVERY);
    }
}

fn trim() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim is thread-safe, reads no pointer of ours and only releases free
    // pages (see the module header).
    unsafe {
        malloc_trim(0);
    }
}
