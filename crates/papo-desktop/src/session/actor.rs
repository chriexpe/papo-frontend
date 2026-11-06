//! A server's session thread and its bounded wake/projection queues.
//!
//! Only this thread opens, drains and stops its `ServerRuntime`. GPUI owns
//! the receiver, never the runtime or Store. Shutdown closes both queues
//! before joining, so a hidden window cannot strand a producer in a send.
//! Runtime effects are not presented by this screen-free slice.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

use async_channel::{Receiver, Sender};
use papo_core::{
    api::net::Wake,
    runtime::{DEFAULT_DRAIN_LIMIT, ServerRuntime},
};

use super::{SessionError, SessionIdentity};
use crate::projection::{ProjectionBuilder, ProjectionUpdate};

/// One pending wake coalesces a burst without losing the next drain turn.
const WAKE_CAPACITY: usize = 1;
/// One pending delta bounds foreground backlog; deltas must never be dropped.
const PROJECTION_CAPACITY: usize = 1;

/// An immutable projection delta and the session it was requested for.
pub(super) struct Publication {
    /// Identity is checked before applying any part of the delta.
    pub(super) identity: SessionIdentity,
    /// Only changed presentation inputs cross the thread boundary.
    pub(super) update: ProjectionUpdate,
    /// Network updates processed in this bounded actor turn.
    pub(super) processed: usize,
}

/// Cancellation and join ownership for one session, independent of its view.
pub(super) struct SessionThread {
    stopped: Arc<AtomicBool>,
    wake: Sender<()>,
    publications: Receiver<Publication>,
    thread: Option<JoinHandle<()>>,
}

impl SessionThread {
    /// Starts one owner thread. Runtime creation and initial cache reads happen there.
    ///
    /// # Errors
    /// Returns `SessionError::Spawn` if the OS cannot start the thread.
    pub(super) fn start(
        identity: SessionIdentity,
        open: impl FnOnce(Wake) -> ServerRuntime + Send + 'static,
    ) -> Result<Self, SessionError> {
        let (wake, wakes) = async_channel::bounded(WAKE_CAPACITY);
        let (publish, publications) = async_channel::bounded(PROJECTION_CAPACITY);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let signal = wake.clone();
        let thread = std::thread::Builder::new()
            .name("papo-session".into())
            .spawn(move || {
                let wake = Wake::new(move || {
                    let _ = signal.try_send(());
                });
                let mut runtime = RuntimeOwner(open(wake));
                let mut projection = ProjectionBuilder::default();
                loop {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let start = std::time::Instant::now();
                    let drain = runtime.0.drain_until_idle(DEFAULT_DRAIN_LIMIT);
                    super::hydration::hydrate_selected(&mut runtime.0, &stop);
                    if let Some(channel) = runtime.0.store.channel_needing_messages() {
                        let ticket = runtime.0.store.mark_loading(&channel);
                        runtime
                            .0
                            .net
                            .send(papo_core::api::net::Command::LoadMessages { ticket });
                    }
                    let update = projection.build(&runtime.0.store);
                    log::debug!(
                        "session: drain processed={}, turn_us={}",
                        drain.processed,
                        start.elapsed().as_micros()
                    );
                    if let Some(update) = update
                        && publish
                            .send_blocking(Publication {
                                identity: identity.clone(),
                                update,
                                processed: drain.processed,
                            })
                            .is_err()
                    {
                        break;
                    }
                    if drain.limit_reached {
                        continue;
                    }
                    if wakes.recv_blocking().is_err() {
                        break;
                    }
                }
                drop(runtime);
                log::debug!("session: runtime stopped, transport joined, cache flush returned");
            })
            .map_err(SessionError::Spawn)?;
        Ok(Self {
            stopped,
            wake,
            publications,
            thread: Some(thread),
        })
    }

    /// A single GPUI foreground task consumes these deltas in FIFO order.
    pub(super) fn publications(&self) -> Receiver<Publication> {
        self.publications.clone()
    }

    /// Interrupts even a full publication queue. This does not wait on GPUI.
    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.wake.close();
        self.publications.close();
    }

    /// Joins on a background executor, after `stop` has interrupted the queues.
    ///
    /// # Errors
    /// Returns `SessionError::Panicked` if the session owner panicked.
    pub(super) fn join(mut self) -> Result<(), SessionError> {
        self.stop();
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| SessionError::Panicked)?;
        }
        Ok(())
    }
}

impl Drop for SessionThread {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            // Normal application shutdown moves this owner to GPUI's background
            // executor. This fallback prevents accidental detached runtimes.
            if thread.join().is_err() {
                log::error!("session: owner thread panicked during drop");
            }
        }
    }
}

/// Ensures even an unwinding session joins Net before releasing the cache.
struct RuntimeOwner(ServerRuntime);

impl RuntimeOwner {
    fn stop(&mut self) {
        self.0.net.shutdown();
        let operations = self.0.store.take_cache_ops();
        if !operations.is_empty() {
            self.0.cache.submit(&self.0.server_key, operations);
        }
        self.0.cache.flush();
    }
}

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        self.stop();
    }
}
