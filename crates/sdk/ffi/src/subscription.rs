//! Bounded FFI subscription lifecycle, panic-isolated callback execution, and honest shutdown drain.
//! Foreign callbacks run serially on dedicated per-subscription threads, never on Tokio workers
//! and never holding SDK locks. `FfiClient::shutdown` uses a bounded drain rather than claiming
//! completion while a foreign callback thread remains stalled.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use futures_util::{Stream, StreamExt, stream};
use tokio::sync::{mpsc, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::errors::FfiClientError;

/// Upper bound on FFI millisecond deadlines so `Instant::now() + duration` never overflows.
const MAX_FFI_DEADLINE_MS: u64 = 365 * 24 * 60 * 60 * 1_000;

/// Clamps millisecond deadlines to a safe horizon before computing future instants.
pub(crate) fn bounded_duration_ms(timeout_ms: u64) -> Duration {
    Duration::from_millis(timeout_ms.min(MAX_FFI_DEADLINE_MS))
}

/// Honest foreign-callback drain report returned by `FfiClient::shutdown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiCallbackDrainStatus {
    /// All active foreign subscription callbacks returned before the drain deadline.
    Drained,
    /// Shutdown proceeded after the deadline while some foreign callback remained running.
    TimedOut {
        /// Number of subscription callbacks still in flight when shutdown returned.
        active_callbacks: u32,
    },
}

/// Shared tracker counting in-flight foreign subscription callbacks across the client.
#[derive(Clone, Default)]
pub(crate) struct CallbackTracker {
    inner: Arc<CallbackTrackerInner>,
}

/// Inner synchronization state for active foreign callback counting.
#[derive(Default)]
struct CallbackTrackerInner {
    active: Mutex<usize>,
    idle: Condvar,
}

/// RAII guard keeping an in-flight foreign callback accounted for until exit or panic unwind.
struct CallbackGuard {
    inner: Arc<CallbackTrackerInner>,
}

impl CallbackTracker {
    /// Marks one foreign callback invocation as active until the returned guard drops.
    fn enter(&self) -> CallbackGuard {
        let mut active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active += 1;
        CallbackGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Waits up to `timeout` for all active foreign callbacks to finish without hanging forever.
    pub(crate) fn drain(&self, timeout: Duration) -> FfiCallbackDrainStatus {
        let clamped = timeout.min(Duration::from_millis(MAX_FFI_DEADLINE_MS));
        let deadline = Instant::now()
            .checked_add(clamped)
            .unwrap_or_else(Instant::now);
        let mut active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *active > 0 {
            let now = Instant::now();
            if now >= deadline {
                return FfiCallbackDrainStatus::TimedOut {
                    active_callbacks: u32::try_from(*active).unwrap_or(u32::MAX),
                };
            }
            let wait_for = deadline.saturating_duration_since(now);
            let (next_active, _) = self
                .inner
                .idle
                .wait_timeout(active, wait_for)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active = next_active;
        }
        FfiCallbackDrainStatus::Drained
    }
}

impl Drop for CallbackGuard {
    fn drop(&mut self) {
        let mut active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.saturating_sub(1);
        if *active == 0 {
            self.inner.idle.notify_all();
        }
    }
}

/// Lifecycle handle for one FFI subscription. Disposing stops callbacks without closing the session.
#[derive(uniffi::Object)]
pub struct FfiSubscriptionHandle {
    cancel: CancellationToken,
}

#[uniffi::export]
impl FfiSubscriptionHandle {
    /// Cancels this subscription without closing the underlying SDK session or affecting sibling views.
    pub fn dispose(&self) {
        self.cancel.cancel();
    }

    /// True after explicit `dispose`, panic isolation, stream termination, or client shutdown.
    #[must_use]
    pub fn is_disposed(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

impl Drop for FfiSubscriptionHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Runs `work` under active callback tracking and `catch_unwind`, isolating any panic to `cancel`.
fn run_guarded<R>(
    tracker: &CallbackTracker,
    cancel: &CancellationToken,
    work: impl FnOnce() -> R,
) -> Option<R> {
    let _guard = tracker.enter();
    if cancel.is_cancelled() {
        return None;
    }
    if let Ok(value) = catch_unwind(AssertUnwindSafe(work)) {
        Some(value)
    } else {
        cancel.cancel();
        None
    }
}

/// Bridges an atomic baseline plus bounded `Stream<Item>` onto a dedicated serial OS callback thread.
pub(crate) fn spawn_stream_subscription<B, Item>(
    runtime: &tokio::runtime::Handle,
    tasks: &TaskTracker,
    shutdown: &CancellationToken,
    tracker: CallbackTracker,
    baseline: B,
    stream: Pin<Box<dyn Stream<Item = Item> + Send>>,
    on_baseline: impl FnOnce(B) -> bool + Send + 'static,
    mut on_item: impl FnMut(Item) -> bool + Send + 'static,
    on_end: impl FnOnce() + Send + 'static,
) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError>
where
    B: Send + 'static,
    Item: Send + 'static,
{
    // Acquire TaskTracker token before checking shutdown/closed so tasks cannot escape `tasks.wait()`.
    let token = tasks.token();
    if shutdown.is_cancelled() || tasks.is_closed() {
        drop(token);
        return Err(FfiClientError::Closed);
    }
    let cancel = shutdown.child_token();
    let (tx, mut rx) = mpsc::channel::<Item>(1);

    // Spawn only the pull forwarder onto Tokio; foreign callbacks run on a dedicated OS thread
    // so stalled foreign code never blocks `Client::shutdown()` runtime teardown.
    let fwd_cancel = cancel.clone();
    runtime.spawn(async move {
        let _token = token;
        let mut stream = stream;
        loop {
            let item = tokio::select! {
                biased;
                () = fwd_cancel.cancelled() => break,
                next = stream.next() => match next {
                    Some(item) => item,
                    None => break,
                },
            };
            tokio::select! {
                biased;
                () = fwd_cancel.cancelled() => break,
                sent = tx.send(item) => {
                    if sent.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let worker_cancel = cancel.clone();
    let spawn_res = thread::Builder::new()
        .name("papo-ffi-sub".into())
        .spawn(move || {
            let keep_going = run_guarded(&tracker, &worker_cancel, || on_baseline(baseline));
            if !keep_going.unwrap_or(false) {
                worker_cancel.cancel();
                return;
            }
            while !worker_cancel.is_cancelled() {
                let Some(item) = rx.blocking_recv() else {
                    break;
                };
                let cont = run_guarded(&tracker, &worker_cancel, || on_item(item));
                if !cont.unwrap_or(false) {
                    worker_cancel.cancel();
                    return;
                }
            }
            if !worker_cancel.is_cancelled() {
                let _ = run_guarded(&tracker, &worker_cancel, on_end);
                worker_cancel.cancel();
            }
        });
    if spawn_res.is_err() {
        cancel.cancel();
        return Err(FfiClientError::Worker);
    }

    Ok(Arc::new(FfiSubscriptionHandle { cancel }))
}

/// Converts a Tokio `watch::Receiver<T>` into an atomic baseline and change-only stream.
pub(crate) fn watch_stream<T: Clone + Send + Sync + 'static>(
    mut receiver: watch::Receiver<T>,
) -> (T, Pin<Box<dyn Stream<Item = T> + Send>>) {
    let baseline = receiver.borrow_and_update().clone();
    let stream = Box::pin(stream::unfold(receiver, |mut rx| async move {
        rx.changed().await.ok()?;
        let next = rx.borrow_and_update().clone();
        Some((next, rx))
    }));
    (baseline, stream)
}
