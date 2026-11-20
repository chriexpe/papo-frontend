//! Deterministic in-memory fake media engine and video destination for 14a tests without `GStreamer`.

use std::{
    any::Any,
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Notify, oneshot};

use super::{
    media_port::{
        IceServerConfig, InitialMediaSettings, MediaCommand, MediaEngineFactory, MediaLease,
        MediaLeaseError, MediaReport, MediaReportKind, MediaReportSink, MediaReportSinkError,
        MediaStartError, MediaStartFuture, VideoDestination, VideoDestinationId,
    },
    model::CallAttemptId,
};

/// Behavior of a scripted `FakeMediaFactory::start` call.
pub enum FakeStartBehavior {
    /// Succeed immediately with a controllable fake lease.
    Immediate,
    /// Fail startup immediately with the specified typed error.
    Fail(MediaStartError),
    /// Pause startup until the caller resolves the returned gate with `Ok(())` or `Err(error)`.
    Gated(oneshot::Receiver<Result<(), MediaStartError>>),
}

/// Inspectable record of one `FakeMediaFactory::start` invocation.
pub struct FakeMediaLeaseHandle {
    /// Attempt passed to `start`.
    pub attempt: CallAttemptId,
    /// ICE servers passed to `start`.
    pub ice_servers: Vec<IceServerConfig>,
    /// Initial settings passed to `start`.
    pub settings: InitialMediaSettings,
    reports: MediaReportSink,
    commands: Arc<Mutex<Vec<MediaCommand>>>,
    command_error: Arc<Mutex<Option<MediaLeaseError>>>,
    command_notify: Arc<Notify>,
    auto_drain: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl FakeMediaLeaseHandle {
    /// Emits a report tagged with this lease's `attempt`.
    ///
    /// # Panics
    /// Panics if the bounded report sink cannot accept the report in a test.
    pub fn emit(&self, kind: MediaReportKind) {
        self.reports
            .try_send(MediaReport {
                attempt: self.attempt,
                kind,
            })
            .expect("fake media report sink rejected report");
    }

    /// Emits a report tagged with an explicit (possibly stale) `attempt`.
    ///
    /// # Panics
    /// Panics if the bounded report sink cannot accept the report in a test.
    pub fn emit_for(&self, attempt: CallAttemptId, kind: MediaReportKind) {
        self.reports
            .try_send(MediaReport { attempt, kind })
            .expect("fake media report sink rejected report");
    }

    /// Snapshot of all `MediaCommand`s delivered to this lease so far.
    #[must_use]
    pub fn commands(&self) -> Vec<MediaCommand> {
        self.commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Sets the error returned by subsequent ordinary commands, if any.
    pub fn set_command_error(&self, error: Option<MediaLeaseError>) {
        *self
            .command_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = error;
    }

    /// True after the lease has received a stop request.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Controls whether stopping automatically emits `MediaReportKind::Drained`.
    pub fn set_auto_drain(&self, auto_drain: bool) {
        self.auto_drain.store(auto_drain, Ordering::Release);
    }

    /// Waits until at least `count` commands have been recorded on this lease.
    pub async fn wait_commands(&self, count: usize) {
        loop {
            let notified = self.command_notify.notified();
            if self.commands().len() >= count {
                return;
            }
            notified.await;
        }
    }

    /// Waits until the lease has received a stop request.
    pub async fn wait_stopped(&self) {
        loop {
            let notified = self.command_notify.notified();
            if self.is_stopped() {
                return;
            }
            notified.await;
        }
    }
}

struct FakeMediaLease {
    attempt: CallAttemptId,
    reports: MediaReportSink,
    commands: Arc<Mutex<Vec<MediaCommand>>>,
    command_error: Arc<Mutex<Option<MediaLeaseError>>>,
    command_notify: Arc<Notify>,
    auto_drain: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    runtime: tokio::runtime::Handle,
}

impl MediaLease for FakeMediaLease {
    fn send(&self, command: MediaCommand) -> Result<(), MediaLeaseError> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(MediaLeaseError::Closed);
        }
        if let Some(error) = *self
            .command_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return Err(error);
        }
        self.commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(command);
        self.command_notify.notify_waiters();
        Ok(())
    }

    fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.command_notify.notify_waiters();
        if self.auto_drain.load(Ordering::Acquire) {
            let report = MediaReport {
                attempt: self.attempt,
                kind: MediaReportKind::Drained,
            };
            if let Err(MediaReportSinkError::Full) = self.reports.try_send(report.clone()) {
                let reports = self.reports.clone();
                self.runtime.spawn(async move {
                    let _ = reports.send(report).await;
                });
            }
        }
    }
}

/// Controllable `MediaEngineFactory` fake for deterministic 14a unit and loopback tests.
#[derive(Clone)]
pub struct FakeMediaFactory {
    inner: Arc<FakeMediaFactoryInner>,
}

struct FakeMediaFactoryInner {
    auto_drain_default: AtomicBool,
    starts: Mutex<VecDeque<FakeStartBehavior>>,
    leases: Mutex<Vec<Arc<FakeMediaLeaseHandle>>>,
    start_notify: Notify,
    start_count: AtomicUsize,
}

impl Default for FakeMediaFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeMediaFactory {
    /// Creates a fake factory that starts immediately and auto-drains on stop.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(FakeMediaFactoryInner {
                auto_drain_default: AtomicBool::new(true),
                starts: Mutex::new(VecDeque::new()),
                leases: Mutex::new(Vec::new()),
                start_notify: Notify::new(),
                start_count: AtomicUsize::new(0),
            }),
        }
    }

    /// Sets whether newly created leases automatically emit `Drained` upon stop.
    pub fn set_auto_drain(&self, auto_drain: bool) {
        self.inner
            .auto_drain_default
            .store(auto_drain, Ordering::Release);
    }

    /// Queues a scripted startup behavior for the next `start()` call.
    pub fn push_start(&self, behavior: FakeStartBehavior) {
        self.inner
            .starts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(behavior);
    }

    /// Number of `start()` invocations observed so far.
    #[must_use]
    pub fn start_count(&self) -> usize {
        self.inner.start_count.load(Ordering::Acquire)
    }

    /// Returns all created lease handles in start order.
    #[must_use]
    pub fn leases(&self) -> Vec<Arc<FakeMediaLeaseHandle>> {
        self.inner
            .leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns the most recently started lease handle, if any.
    #[must_use]
    pub fn latest_lease(&self) -> Option<Arc<FakeMediaLeaseHandle>> {
        self.inner
            .leases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .last()
            .cloned()
    }

    /// Waits until `start()` has been invoked at least `count` times.
    pub async fn wait_starts(&self, count: usize) {
        loop {
            let notified = self.inner.start_notify.notified();
            if self.start_count() >= count {
                return;
            }
            notified.await;
        }
    }

    /// Waits until at least `count` leases have completed startup and returns the latest.
    pub async fn wait_lease(&self, count: usize) -> Arc<FakeMediaLeaseHandle> {
        loop {
            let notified = self.inner.start_notify.notified();
            {
                let leases = self
                    .inner
                    .leases
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if leases.len() >= count {
                    return Arc::clone(&leases[count - 1]);
                }
            }
            notified.await;
        }
    }
}

impl MediaEngineFactory for FakeMediaFactory {
    fn start(
        &self,
        attempt: CallAttemptId,
        ice_servers: Vec<IceServerConfig>,
        settings: InitialMediaSettings,
        reports: MediaReportSink,
    ) -> MediaStartFuture {
        let inner = Arc::clone(&self.inner);
        inner.start_count.fetch_add(1, Ordering::AcqRel);
        let behavior = inner
            .starts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
            .unwrap_or(FakeStartBehavior::Immediate);
        inner.start_notify.notify_waiters();

        Box::pin(async move {
            match behavior {
                FakeStartBehavior::Immediate => {}
                FakeStartBehavior::Fail(error) => return Err(error),
                FakeStartBehavior::Gated(gate) => match gate.await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => return Err(error),
                    Err(_) => return Err(MediaStartError::Cancelled),
                },
            }
            let commands = Arc::new(Mutex::new(Vec::new()));
            let command_error = Arc::new(Mutex::new(None));
            let command_notify = Arc::new(Notify::new());
            let auto_drain = Arc::new(AtomicBool::new(
                inner.auto_drain_default.load(Ordering::Acquire),
            ));
            let stopped = Arc::new(AtomicBool::new(false));
            let handle = Arc::new(FakeMediaLeaseHandle {
                attempt,
                ice_servers,
                settings,
                reports: reports.clone(),
                commands: Arc::clone(&commands),
                command_error: Arc::clone(&command_error),
                command_notify: Arc::clone(&command_notify),
                auto_drain: Arc::clone(&auto_drain),
                stopped: Arc::clone(&stopped),
            });
            inner
                .leases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(handle);
            inner.start_notify.notify_waiters();
            Ok(Box::new(FakeMediaLease {
                attempt,
                reports,
                commands,
                command_error,
                command_notify,
                auto_drain,
                stopped,
                runtime: tokio::runtime::Handle::current(),
            }) as Box<dyn MediaLease>)
        })
    }
}

/// Deterministic test implementation of `VideoDestination` tracking revocation.
pub struct FakeVideoDestination {
    id: VideoDestinationId,
    revoked: AtomicBool,
}

impl FakeVideoDestination {
    /// Creates a fake video destination with the given identifier.
    #[must_use]
    pub const fn new(id: VideoDestinationId) -> Self {
        Self {
            id,
            revoked: AtomicBool::new(false),
        }
    }

    /// True once `revoke()` has been invoked by the SDK.
    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }
}

impl VideoDestination for FakeVideoDestination {
    fn id(&self) -> VideoDestinationId {
        self.id
    }

    fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}
