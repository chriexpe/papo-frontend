//! One off-main shutdown and its reusable process-exit fence.
//!
//! Channel closure broadcasts completion to every asynchronous waiter. The
//! native quit callback also joins the reaper synchronously, before GPUI's
//! 200 ms quit grace starts. Only that termination boundary may block the
//! main thread; stopping runtimes and flushing caches remain on the reaper.

use std::thread::JoinHandle;

use async_channel::Receiver;

/// Retains completion even after the session owners have left the foreground.
#[derive(Default)]
pub(super) struct Shutdown {
    completion: Option<Receiver<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Shutdown {
    /// Starts the first shutdown only; subsequent calls share its completion.
    /// Work must not depend on GPUI or a foreground task making progress.
    pub(super) fn begin(&mut self, work: impl FnOnce() + Send + 'static) -> Receiver<()> {
        if let Some(completion) = &self.completion {
            return completion.clone();
        }
        let (finished, completion) = async_channel::bounded(1);
        self.thread = match std::thread::Builder::new()
            .name("papo-shutdown".into())
            .spawn(move || {
                work();
                drop(finished);
            }) {
            Ok(thread) => Some(thread),
            Err(error) => {
                // A failed spawn drops the closure here. SessionThread's Drop
                // joins its owner, so resource exhaustion cannot detach work.
                log::error!("session: could not start the shutdown thread: {error}");
                None
            }
        };
        self.completion = Some(completion.clone());
        completion
    }

    /// Fences native termination independently of GPUI's bounded quit future.
    /// Normal application Quit awaits completion instead of calling this.
    pub(super) fn wait(&mut self) {
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("session: shutdown thread panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    use super::Shutdown;

    #[test]
    fn repeated_shutdown_waiters_share_completion_until_work_finishes() {
        let mut shutdown = Shutdown::default();
        let (started, entered) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let first = shutdown.begin(move || {
            started.send(()).unwrap();
            released.recv().unwrap();
        });
        entered.recv().unwrap();
        let repeated_work = Arc::new(AtomicBool::new(false));
        let repeated = shutdown.begin({
            let repeated_work = Arc::clone(&repeated_work);
            move || repeated_work.store(true, Ordering::Release)
        });

        assert!(!first.is_closed());
        assert!(!repeated.is_closed());
        release.send(()).unwrap();
        shutdown.wait();
        assert!(first.recv_blocking().is_err());
        assert!(repeated.recv_blocking().is_err());
        assert!(!repeated_work.load(Ordering::Acquire));
    }

    #[test]
    fn dropping_async_waiters_does_not_cancel_shutdown() {
        let mut shutdown = Shutdown::default();
        let (started, entered) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let completion = shutdown.begin({
            let finished = Arc::clone(&finished);
            move || {
                started.send(()).unwrap();
                released.recv().unwrap();
                finished.store(true, Ordering::Release);
            }
        });
        entered.recv().unwrap();
        drop(completion);

        release.send(()).unwrap();
        shutdown.wait();
        assert!(finished.load(Ordering::Acquire));
    }

    #[test]
    fn native_exit_fence_waits_for_off_main_work_without_foreground_progress() {
        let mut shutdown = Shutdown::default();
        let caller = std::thread::current().id();
        let (worker_id, observed_id) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let finished = Arc::new(AtomicBool::new(false));
        let completion = shutdown.begin({
            let finished = Arc::clone(&finished);
            move || {
                worker_id.send(std::thread::current().id()).unwrap();
                released.recv().unwrap();
                finished.store(true, Ordering::Release);
            }
        });
        assert_ne!(observed_id.recv().unwrap(), caller);
        assert!(!completion.is_closed());
        let (waiting, entered_wait) = mpsc::channel();
        let fence = std::thread::spawn(move || {
            waiting.send(()).unwrap();
            shutdown.wait();
            assert!(finished.load(Ordering::Acquire));
            assert!(completion.is_closed());
        });

        entered_wait.recv().unwrap();
        release.send(()).unwrap();
        fence.join().unwrap();
    }
}
