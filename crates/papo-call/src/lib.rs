//! GStreamer implementation of the SDK media port. The SDK owns signalling and participant state.

mod audio;
mod camera;
mod diagnostics;
mod engine;
mod ice;
mod mic;
mod negotiation;
mod pipeline;
pub mod platform;
mod receive_audio;
mod receive_video;
mod slots;
mod video_route;

pub use platform::MediaDevices;
pub use receive_video::{CpuVideoDestination, CpuVideoFrame};
pub use slots::ReceiveSlots;

use engine::{Engine, QUEUE_CAPACITY};
use papo_sdk::{
    CallAttemptId, IceServerConfig, InitialMediaSettings, MediaCommand, MediaEngineFactory,
    MediaLease, MediaLeaseError, MediaReport, MediaReportKind, MediaReportSink, MediaStartError,
    MediaStartFuture,
};
use platform::DefaultDevices;
use std::sync::Arc;
use tokio::{
    runtime::Handle,
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

/// Injectable GStreamer factory. Use with the SDK's asynchronous client runtime.
pub struct GstreamerMediaFactory {
    devices: Arc<dyn MediaDevices>,
    slots: ReceiveSlots,
}

impl Default for GstreamerMediaFactory {
    fn default() -> Self {
        Self::new(Arc::new(DefaultDevices), ReceiveSlots::default())
    }
}

impl GstreamerMediaFactory {
    /// Selects host devices and first-offer receive capacity for each lease.
    pub fn new(devices: Arc<dyn MediaDevices>, slots: ReceiveSlots) -> Self {
        Self { devices, slots }
    }
}

struct Lease {
    tx: mpsc::Sender<MediaCommand>,
    stop: CancellationToken,
}

impl MediaLease for Lease {
    fn send(&self, command: MediaCommand) -> Result<(), MediaLeaseError> {
        if self.stop.is_cancelled() {
            return Err(MediaLeaseError::Closed);
        }
        self.tx.try_send(command).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => MediaLeaseError::Full,
            mpsc::error::TrySendError::Closed(_) => MediaLeaseError::Closed,
        })
    }

    fn stop(&self) {
        self.stop.cancel();
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.stop();
    }
}

impl MediaEngineFactory for GstreamerMediaFactory {
    fn start(
        &self,
        attempt: CallAttemptId,
        ice: Vec<IceServerConfig>,
        settings: InitialMediaSettings,
        reports: MediaReportSink,
    ) -> MediaStartFuture {
        let devices = Arc::clone(&self.devices);
        let slots = self.slots;
        Box::pin(async move {
            let runtime = Handle::try_current().map_err(|_| MediaStartError::Internal)?;
            let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
            let (ready, started) = oneshot::channel();
            let stop = CancellationToken::new();
            let lease = Lease {
                tx,
                stop: stop.clone(),
            };
            let worker_reports = reports.clone();
            let worker = runtime.clone().spawn_blocking(move || {
                if ready.is_closed() {
                    return false;
                }
                let engine = Engine::new(devices, slots, ice, settings, &stop);
                match engine {
                    Ok(mut engine) => {
                        if ready.send(Ok(())).is_ok() {
                            let _entered = runtime.enter();
                            let result =
                                runtime.block_on(engine.run(rx, &stop, attempt, &worker_reports));
                            stop.cancel();
                            let drained = engine.drain(Some(&runtime));
                            drop(engine);
                            return finish_worker(
                                result,
                                &runtime,
                                attempt,
                                &worker_reports,
                                drained,
                            );
                        }
                        engine.drain(None)
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        false
                    }
                }
            });
            tokio::spawn(async move {
                if let Ok(true) = worker.await {
                    let _ = reports
                        .send(MediaReport {
                            attempt,
                            kind: MediaReportKind::Drained,
                        })
                        .await;
                }
            });
            started.await.map_err(|_| MediaStartError::Internal)??;
            Ok(Box::new(lease) as Box<dyn MediaLease>)
        })
    }
}

fn finish_worker(
    outcome: pipeline::MediaResult<()>,
    runtime: &Handle,
    attempt: CallAttemptId,
    reports: &MediaReportSink,
    drained: bool,
) -> bool {
    if let Err(error) = outcome {
        diagnostics::fatal(error);
        let _ = runtime.block_on(reports.send(MediaReport {
            attempt,
            kind: MediaReportKind::Fatal(error),
        }));
    }
    drained
}
