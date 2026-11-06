//! One persistent server thread, bounded wake/delta queues and latest replacement.
//!
//! Only this thread opens, drains and stops its `ServerRuntime`. Replacements
//! occupy one mailbox slot, not a chain of successor OS threads. Cache completion
//! is an event; no page receipt blocks the actor. GPUI receives immutable deltas.
//! Runtime effects remain a screen-free slice limitation (plan §4.3).

use super::{SessionError, SessionIdentity, hydration::Hydration};
use crate::projection::{ProjectionBuilder, ProjectionUpdate};
use async_channel::{Receiver, Sender};
use papo_core::{
    api::net::{Command, Wake},
    runtime::{DEFAULT_DRAIN_LIMIT, ServerRuntime},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

/// One pending wake coalesces a burst without losing the next drain turn.
const WAKE_CAPACITY: usize = 1;
/// One pending delta bounds foreground backlog; deltas must never be dropped.
const PROJECTION_CAPACITY: usize = 1;
/// Only the newest unopened replacement is useful; stale factories are dropped.
const REPLACEMENT_CAPACITY: usize = 1;

/// An immutable projection delta and the session it was requested for.
pub(super) struct Publication {
    /// Identity is checked before applying any part of the delta.
    pub(super) identity: SessionIdentity,
    /// Only changed presentation inputs cross the thread boundary.
    pub(super) update: ProjectionUpdate,
    /// Network updates processed in this bounded actor turn.
    pub(super) processed: usize,
}

struct Replacement {
    identity: SessionIdentity,
    open: Box<dyn FnOnce(Wake) -> ServerRuntime + Send>,
}

/// Cancellation, replacement mailbox and join ownership for one server thread.
pub(super) struct SessionThread {
    stopped: Arc<AtomicBool>,
    wake: Sender<()>,
    replacements: Sender<Replacement>,
    publications: Receiver<Publication>,
    thread: Option<JoinHandle<()>>,
}

impl SessionThread {
    /// Starts the server's only owner thread. Runtime opening happens there.
    ///
    /// # Errors
    /// Returns `SessionError::Spawn` if the OS cannot start the thread.
    pub(super) fn start(
        identity: SessionIdentity,
        open: impl FnOnce(Wake) -> ServerRuntime + Send + 'static,
    ) -> Result<Self, SessionError> {
        let (wake, wakes) = async_channel::bounded(WAKE_CAPACITY);
        let (publish, publications) = async_channel::bounded(PROJECTION_CAPACITY);
        let (replacements, requests) = async_channel::bounded(REPLACEMENT_CAPACITY);
        let _ = replacements.try_send(Replacement {
            identity,
            open: Box::new(open),
        });
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let signal = wake.clone();
        let thread = std::thread::Builder::new()
            .name("papo-session".into())
            .spawn(move || run(&requests, &wakes, &signal, &stop, &publish))
            .map_err(SessionError::Spawn)?;
        Ok(Self {
            stopped,
            wake,
            replacements,
            publications,
            thread: Some(thread),
        })
    }

    /// Replaces the one pending factory without starting another OS thread.
    ///
    /// # Errors
    /// Returns `SessionError::OwnerStopped` if shutdown or an owner failure closed the mailbox.
    pub(super) fn replace(
        &self,
        identity: SessionIdentity,
        open: impl FnOnce(Wake) -> ServerRuntime + Send + 'static,
    ) -> Result<(), SessionError> {
        self.replacements
            .force_send(Replacement {
                identity,
                open: Box::new(open),
            })
            .map_err(|_| SessionError::OwnerStopped)?;
        let _ = self.wake.try_send(());
        Ok(())
    }

    /// A single GPUI foreground task consumes deltas in FIFO order.
    pub(super) fn publications(&self) -> Receiver<Publication> {
        self.publications.clone()
    }

    /// Interrupts wakes and a full publication queue, without waiting on GPUI.
    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.replacements.close();
        self.wake.close();
        self.publications.close();
    }

    /// Joins after `stop` interrupts queues. This runs off the foreground.
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
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("session: owner thread panicked during drop");
        }
    }
}

struct Running {
    identity: SessionIdentity,
    runtime: RuntimeOwner,
    projection: ProjectionBuilder,
}

fn run(
    requests: &Receiver<Replacement>,
    wakes: &Receiver<()>,
    signal: &Sender<()>,
    stopped: &AtomicBool,
    publish: &Sender<Publication>,
) {
    let mut current = None::<Running>;
    let mut hydration = Hydration::default();
    loop {
        if stopped.load(Ordering::Acquire) {
            break;
        }
        if let Ok(replacement) = requests.try_recv() {
            drop(current.take());
            if stopped.load(Ordering::Acquire) {
                break;
            }
            // A request that arrived during teardown supersedes the earlier one.
            let replacement = requests.try_recv().unwrap_or(replacement);
            let wake = signal.clone();
            let runtime = RuntimeOwner((replacement.open)(Wake::new(move || {
                let _ = wake.try_send(());
            })));
            current = Some(Running {
                identity: replacement.identity,
                runtime,
                projection: ProjectionBuilder::default(),
            });
        }
        if stopped.load(Ordering::Acquire) {
            break;
        }
        if !requests.is_empty() {
            continue;
        }
        let Some(running) = current.as_mut() else {
            break;
        };
        let start = std::time::Instant::now();
        let drain = running.runtime.0.drain_until_idle(DEFAULT_DRAIN_LIMIT);
        hydration.step(&mut running.runtime.0, &running.identity, signal);
        if let Some(channel) = running.runtime.0.store.channel_needing_messages() {
            let ticket = running.runtime.0.store.mark_loading(&channel);
            running.runtime.0.net.send(Command::LoadMessages { ticket });
        }
        let update = running.projection.build(&running.runtime.0.store);
        log::debug!(
            "session: drain processed={}, turn_us={}",
            drain.processed,
            start.elapsed().as_micros()
        );
        if let Some(update) = update
            && publish
                .send_blocking(Publication {
                    identity: running.identity.clone(),
                    update,
                    processed: drain.processed,
                })
                .is_err()
        {
            break;
        }
        if drain.limit_reached || !requests.is_empty() {
            continue;
        }
        if wakes.recv_blocking().is_err() {
            break;
        }
    }
    drop(current);
    log::debug!("session: runtime stopped, transport joined, cache flush returned");
}

/// Ensures even an unwinding session joins Net before releasing the cache.
struct RuntimeOwner(ServerRuntime);
impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        self.0.net.shutdown();
        let operations = self.0.store.take_cache_ops();
        if !operations.is_empty() {
            self.0.cache.submit(&self.0.server_key, operations);
        }
        self.0.cache.flush();
    }
}
