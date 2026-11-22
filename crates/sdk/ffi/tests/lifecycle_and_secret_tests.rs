//! Integration tests covering Amendment 1 (single-instance `CallHandle` video binding),
//! Amendment 3 (backpressure `Reset`, panic isolation, honest `TimedOut` vs `Drained` shutdown),
//! and Amendment 4 (`settled` vs unfenced credential access through `FfiClient`).

mod ffi_fixture;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use ffi_fixture::{FfiTestHarness, ZeroUtcOffset};
use papo_ffi::{
    FfiAccountId, FfiCallbackDrainStatus, FfiConnectionListener, FfiConnectionStatus,
    FfiHistoryClosedReason, FfiTimelineConfig, FfiTimelineDiff, FfiTimelineListener,
    FfiTimelineSnapshot, FfiTimelineUpdate, FfiVideoKind, FfiVideoTarget,
};
use papo_sdk::{
    CallPhase, FakeMediaFactory, FakeVideoDestination, MediaReportKind, MediaTransportState,
    Message, VideoDestination, VideoDestinationId,
};
use papo_test_server::{Config, HistorySeed};

struct SlowListener {
    entered: mpsc::Sender<()>,
    gate: Mutex<Option<mpsc::Receiver<()>>>,
    diffs: Arc<Mutex<Vec<FfiTimelineDiff>>>,
}

impl FfiTimelineListener for SlowListener {
    fn on_snapshot(&self, _: FfiTimelineSnapshot) {}
    fn on_update(&self, update: FfiTimelineUpdate) {
        if let Some(rx) = self.gate.lock().unwrap().take() {
            let _ = self.entered.send(());
            let _ = rx.recv();
        }
        self.diffs.lock().unwrap().extend(update.diffs);
    }
    fn on_closed(&self, _reason: FfiHistoryClosedReason) {}
}

struct PanickingListener;

impl FfiTimelineListener for PanickingListener {
    fn on_snapshot(&self, _: FfiTimelineSnapshot) {}
    fn on_update(&self, _: FfiTimelineUpdate) {
        panic!("simulated foreign callback panic");
    }
    fn on_closed(&self, _reason: FfiHistoryClosedReason) {}
}

struct HealthyListener {
    count: Arc<AtomicUsize>,
    tx: mpsc::Sender<()>,
}

impl FfiTimelineListener for HealthyListener {
    fn on_snapshot(&self, _: FfiTimelineSnapshot) {}
    fn on_update(&self, _: FfiTimelineUpdate) {
        self.count.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.send(());
    }
    fn on_closed(&self, _reason: FfiHistoryClosedReason) {}
}

struct StalledConn {
    entered: mpsc::Sender<()>,
    release: Mutex<Option<mpsc::Receiver<()>>>,
}

impl FfiConnectionListener for StalledConn {
    fn on_snapshot(&self, _: FfiConnectionStatus) {
        let _ = self.entered.send(());
        if let Some(rx) = self.release.lock().unwrap().take() {
            let _ = rx.recv();
        }
    }
    fn on_update(&self, _: FfiConnectionStatus) {}
    fn on_closed(&self) {}
}

fn synth_msg(channel_id: &str, author_id: &str, seq: u128) -> Message {
    Message {
        id: Some(format!("30000000-0000-4000-8000-{seq:012x}")),
        channel_id: Some(channel_id.to_owned()),
        author_id: Some(author_id.to_owned()),
        content: Some(String::new()),
        created_at: Some(format!("2026-01-01T00:00:{:02}.000000Z", 10 + seq)),
        ..Message::default()
    }
}

fn publish_synced_burst(
    h: &FfiTestHarness,
    channel_id: &str,
    account: &FfiAccountId,
    healthy_rx: &mpsc::Receiver<()>,
) {
    for seq in 2..=7 {
        h.on_server(
            h.server()
                .publish_message(synth_msg(channel_id, &account.user_id, seq)),
        )
        .unwrap();
        healthy_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

#[test]
fn amendment1_call_handle_for_and_bind_video_operate_on_single_handle() {
    let factory = FakeMediaFactory::new();
    let h = FfiTestHarness::open(Config::default(), Some(factory.clone()));
    let voice = h
        .on_server(h.server().create_voice_channel("voice-a"))
        .unwrap();
    let voice_id = voice.id.unwrap();
    let remote_id = "70000000-0000-4000-8000-000000000777".to_owned();
    h.on_server(
        h.server()
            .seed_voice_occupant(&voice_id, &remote_id, false, true, false),
    );
    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "alice");

    let ffi_call = loop {
        match h.client().join_call(account.clone(), voice_id.clone()) {
            Ok(call) => break call,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    h.on_server(factory.wait_starts(1));

    let same_handle = h
        .client()
        .call_handle_for(account, ffi_call.attempt_id())
        .expect("call_handle_for must resolve the active handle");
    assert_eq!(same_handle.attempt().raw(), ffi_call.attempt_id());

    for _ in 0..100 {
        if matches!(
            same_handle.snapshot().phase,
            CallPhase::Joined | CallPhase::Connected
        ) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(same_handle.snapshot().phase, CallPhase::Joined);

    let lease = factory.latest_lease().expect("fake lease started");
    lease.emit(MediaReportKind::LocalOffer {
        sdp: "v=0\r\n".into(),
    });
    lease.emit(MediaReportKind::TransportState(
        MediaTransportState::Connected,
    ));
    for _ in 0..100 {
        if same_handle.snapshot().phase == CallPhase::Connected {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(same_handle.snapshot().phase, CallPhase::Connected);

    let dest_id = VideoDestinationId::from_raw(77);
    let dest = Arc::new(FakeVideoDestination::new(dest_id));
    ffi_call
        .bind_video_destination(
            remote_id.clone(),
            FfiVideoKind::Camera,
            Arc::clone(&dest) as Arc<dyn VideoDestination>,
        )
        .expect("bind video");
    ffi_call
        .set_video_interest(vec![FfiVideoTarget {
            participant_user_id: remote_id.clone(),
            kind: FfiVideoKind::Camera,
        }])
        .expect("set video interest");

    let mut bound = false;
    for _ in 0..100 {
        if same_handle
            .snapshot()
            .video_bindings
            .iter()
            .any(|b| b.destination_id == Some(dest_id))
        {
            bound = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        bound,
        "single CallHandle must reflect bound VideoDestination"
    );

    ffi_call
        .unbind_video(remote_id, FfiVideoKind::Camera)
        .expect("unbind video");
    assert!(dest.is_revoked());
    let report = h.shutdown(500);
    assert!(report.reads_drained);
}

#[test]
fn amendment3_slow_backpressure_panic_isolation_and_sibling_survival() {
    let seed = HistorySeed {
        channel_id: "20000000-0000-4000-8000-000000000001".into(),
        author_id: "00000000-0000-4000-8000-000000000099".into(),
        first_message_id: "20000000-0000-4000-8000-000000000100".into(),
        count: 3,
        first_created_at: "2026-01-01T00:00:01.000000Z".into(),
        timestamp_step_us: 1_000,
        equal_timestamp_count: 1,
    };
    let cfg = Config {
        history: vec![seed.clone()],
        ..Config::default()
    };
    let h = FfiTestHarness::open(cfg, None);
    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "bob");
    let tl_cfg = FfiTimelineConfig {
        page_size: 20,
        subscriber_capacity: 2,
    };
    let timeline = h
        .block_on(h.client().open_live_timeline(
            account.clone(),
            seed.channel_id.clone(),
            tl_cfg,
            Arc::new(ZeroUtcOffset),
        ))
        .unwrap();

    let (slow_gate_tx, slow_gate_rx) = mpsc::channel::<()>();
    let (slow_entered_tx, slow_entered_rx) = mpsc::channel::<()>();
    let slow_diffs = Arc::new(Mutex::new(Vec::<FfiTimelineDiff>::new()));
    let healthy_count = Arc::new(AtomicUsize::new(0));
    let (healthy_tx, healthy_rx) = mpsc::channel::<()>();

    let sub_slow = timeline
        .subscribe(Arc::new(SlowListener {
            entered: slow_entered_tx,
            gate: Mutex::new(Some(slow_gate_rx)),
            diffs: Arc::clone(&slow_diffs),
        }))
        .unwrap();
    let sub_panic = timeline.subscribe(Arc::new(PanickingListener)).unwrap();
    let sub_healthy = timeline
        .subscribe(Arc::new(HealthyListener {
            count: Arc::clone(&healthy_count),
            tx: healthy_tx,
        }))
        .unwrap();

    h.on_server(
        h.server()
            .publish_message(synth_msg(&seed.channel_id, &account.user_id, 1)),
    )
    .unwrap();
    slow_entered_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    healthy_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    publish_synced_burst(&h, &seed.channel_id, &account, &healthy_rx);
    let _ = slow_gate_tx.send(());

    for _ in 0..100 {
        if sub_panic.is_disposed()
            && slow_diffs
                .lock()
                .unwrap()
                .iter()
                .any(|d| matches!(d, FfiTimelineDiff::Reset { .. }))
            && healthy_count.load(Ordering::Acquire) >= 6
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(sub_panic.is_disposed());
    assert!(!sub_healthy.is_disposed());
    assert!(!h.client().is_closed());
    assert!(
        slow_diffs
            .lock()
            .unwrap()
            .iter()
            .any(|d| matches!(d, FfiTimelineDiff::Reset { .. })),
        "slow listener should observe coherent Reset after buffer overflow"
    );

    sub_slow.dispose();
    sub_healthy.dispose();
    let report = h.shutdown(1_000);
    assert_eq!(report.callback_drain, FfiCallbackDrainStatus::Drained);
}

#[test]
fn amendment3_stalled_callback_returns_honest_timed_out_status() {
    let h = FfiTestHarness::open(Config::default(), None);
    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "carol");

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let _sub = h
        .client()
        .subscribe_connection(
            account,
            Arc::new(StalledConn {
                entered: entered_tx,
                release: Mutex::new(Some(release_rx)),
            }),
        )
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let started = Instant::now();
    let report = h.shutdown(60);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        report.callback_drain,
        FfiCallbackDrainStatus::TimedOut {
            active_callbacks: 1
        }
    );
    let _ = release_tx.send(());
}
