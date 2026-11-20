//! Exercises native construction, the SDK stop contract and process resource counts without devices.

use gstreamer as gst;
use papo_call::{GstreamerMediaFactory, MediaDevices, ReceiveSlots};
use papo_sdk::{
    CallAttemptId, InitialMediaSettings, MediaCommand, MediaEngineFactory, MediaLeaseError,
    MediaReportKind, MediaReportSink, MediaStartError,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

struct Devices {
    microphones: AtomicUsize,
    valid_sink: bool,
}

impl MediaDevices for Devices {
    fn speaker(&self) -> Option<gst::Element> {
        let name = if self.valid_sink {
            "fakesink"
        } else {
            "fakesrc"
        };
        gst::ElementFactory::make(name)
            .property("num-buffers", 0i32)
            .build()
            .ok()
    }

    fn microphone(&self) -> Option<gst::Element> {
        self.microphones.fetch_add(1, Ordering::Relaxed);
        None
    }
}

struct Silent;
impl MediaDevices for Silent {
    fn speaker(&self) -> Option<gst::Element> {
        gst::ElementFactory::make("fakesink")
            .property("async", false)
            .property("sync", false)
            .build()
            .ok()
    }
}

#[tokio::test]
async fn link_failure_rolls_back_a_partial_graph() {
    let devices = Arc::new(Devices {
        microphones: AtomicUsize::new(0),
        valid_sink: false,
    });
    let factory = GstreamerMediaFactory::new(devices.clone(), ReceiveSlots::default());
    let (sink, mut reports) = MediaReportSink::channel(1);
    let result = factory
        .start(
            CallAttemptId::from_raw(1),
            vec![],
            InitialMediaSettings::default(),
            sink,
        )
        .await;
    assert!(matches!(result, Err(MediaStartError::DeviceUnavailable)));
    assert_eq!(devices.microphones.load(Ordering::Relaxed), 0);
    assert!(reports.recv().await.is_none());
}

#[tokio::test]
async fn stop_is_idempotent_with_saturated_commands_and_reports() {
    let factory = GstreamerMediaFactory::new(Arc::new(Silent), ReceiveSlots::default());
    let attempt = CallAttemptId::from_raw(2);
    let (sink, mut reports) = MediaReportSink::channel(1);
    sink.try_send(papo_sdk::MediaReport {
        attempt,
        kind: MediaReportKind::MicCapture(papo_sdk::CaptureState::Inactive),
    })
    .unwrap();
    let lease = factory
        .start(attempt, vec![], InitialMediaSettings::default(), sink)
        .await
        .unwrap();
    let mut full = false;
    for _ in 0..20_000 {
        if lease.send(MediaCommand::SetAudioRoutes {
            revision: 0,
            routes: vec![],
        }) == Err(MediaLeaseError::Full)
        {
            full = true;
            break;
        }
    }
    assert!(full);
    lease.stop();
    lease.stop();
    assert_eq!(
        lease.send(MediaCommand::SetMuted { muted: false }),
        Err(MediaLeaseError::Closed)
    );
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(report) = reports.recv().await {
            assert_eq!(report.attempt, attempt);
            if matches!(report.kind, MediaReportKind::Drained) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap();
    assert!(drained);
    assert!(reports.recv().await.is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "measure in a dedicated test process without concurrent media tests"]
async fn resources_plateau_over_one_hundred_lease_cycles() {
    fn resources() -> (usize, usize) {
        let count = |path| std::fs::read_dir(path).unwrap().count();
        (count("/proc/self/task"), count("/proc/self/fd"))
    }

    let factory = GstreamerMediaFactory::new(Arc::new(Silent), ReceiveSlots::default());
    let mut samples = Vec::new();
    for cycle in 1..=110 {
        let attempt = CallAttemptId::from_raw(cycle);
        let (sink, mut reports) = MediaReportSink::channel(1);
        let lease = factory
            .start(attempt, vec![], InitialMediaSettings::default(), sink)
            .await
            .unwrap();
        lease.stop();
        let report = tokio::time::timeout(Duration::from_secs(5), reports.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(report.kind, MediaReportKind::Drained));
        assert!(reports.recv().await.is_none());
        drop(lease);
        if cycle >= 11 {
            samples.push(resources());
        }
    }
    let (first_threads, first_fds) = samples[0];
    let (last_threads, last_fds) = samples[samples.len() - 1];
    println!(
        "100 warm cycles: threads {first_threads} → {last_threads}, FDs {first_fds} → {last_fds}"
    );
    assert!(last_threads <= first_threads + 2);
    assert_eq!(first_fds, last_fds);
}

#[tokio::test]
async fn cancelled_start_releases_a_device_acquired_after_cancellation() {
    use gst::prelude::*;
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    struct GatedSpeaker {
        entered: Mutex<Option<oneshot::Sender<gst::glib::WeakRef<gst::Element>>>>,
        resume: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl MediaDevices for GatedSpeaker {
        fn speaker(&self) -> Option<gst::Element> {
            let sink = Silent.speaker()?;
            let signal = self.entered.lock().unwrap().take();
            signal?.send(sink.downgrade()).ok()?;
            self.resume.lock().unwrap().recv().ok()?;
            Some(sink)
        }
    }

    let (entered_tx, entered_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::sync_channel(1);
    let factory = GstreamerMediaFactory::new(
        Arc::new(GatedSpeaker {
            entered: Mutex::new(Some(entered_tx)),
            resume: Mutex::new(resume_rx),
        }),
        ReceiveSlots::default(),
    );
    let attempt = CallAttemptId::from_raw(3);
    let (sink, mut reports) = MediaReportSink::channel(1);
    let task = tokio::spawn(factory.start(attempt, vec![], InitialMediaSettings::default(), sink));
    let device = entered_rx.await.unwrap();
    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    resume_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while reports.recv().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(device.upgrade().is_none());
}
