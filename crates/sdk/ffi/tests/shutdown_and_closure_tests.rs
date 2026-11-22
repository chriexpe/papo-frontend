//! Regression tests for shutdown-drain sync admission guards, terminal `on_closed()` routing on
//! search and notification-status streams, bounded `u64` deadlines, and debug/error invariants.

mod ffi_fixture;

use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use ffi_fixture::FfiTestHarness;
use papo_ffi::{
    FfiAccountId, FfiCallError, FfiCallbackDrainStatus, FfiClientError, FfiConnectionListener,
    FfiConnectionStatus, FfiMessageTarget, FfiNotificationError, FfiNotificationStatus,
    FfiNotificationStatusListener, FfiNotificationTarget, FfiReactionEmoji, FfiReactionError,
    FfiReactionIntent, FfiSavedServer, FfiSearchListener, FfiSearchState, FfiTimelineConfig,
    FfiTimelineError,
};
use papo_sdk::FakeMediaFactory;
use papo_test_server::Config;

struct StalledConn {
    entered: mpsc::Sender<()>,
    release: Mutex<Option<mpsc::Receiver<()>>>,
}

impl FfiConnectionListener for StalledConn {
    fn on_snapshot(&self, _status: FfiConnectionStatus) {
        let _ = self.entered.send(());
        if let Some(rx) = self.release.lock().unwrap().take() {
            let _ = rx.recv_timeout(Duration::from_secs(10));
        }
    }
    fn on_update(&self, _status: FfiConnectionStatus) {}
    fn on_closed(&self) {}
}

struct SearchCloseWatch {
    snap: mpsc::Sender<()>,
    closed: mpsc::Sender<()>,
}

impl FfiSearchListener for SearchCloseWatch {
    fn on_snapshot(&self, _state: FfiSearchState) {
        let _ = self.snap.send(());
    }
    fn on_update(&self, _state: FfiSearchState) {}
    fn on_closed(&self) {
        let _ = self.closed.send(());
    }
}

struct NotifCloseWatch {
    snap: mpsc::Sender<()>,
    closed: mpsc::Sender<()>,
}

impl FfiNotificationStatusListener for NotifCloseWatch {
    fn on_snapshot(&self, _status: FfiNotificationStatus) {
        let _ = self.snap.send(());
    }
    fn on_update(&self, _status: FfiNotificationStatus) {}
    fn on_closed(&self) {
        let _ = self.closed.send(());
    }
}

#[test]
fn sync_calls_rejected_during_shutdown_callback_drain() {
    let factory = FakeMediaFactory::new();
    let mut h = FfiTestHarness::open(Config::default(), Some(factory.clone()));
    let ch_id = h
        .on_server(h.server().create_voice_channel("drain-guard"))
        .unwrap()
        .id
        .unwrap();
    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "ivan");

    let call = loop {
        match h.client().join_call(account.clone(), ch_id.clone()) {
            Ok(c) => break c,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    h.on_server(factory.wait_starts(1));

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let _sub = h
        .client()
        .subscribe_connection(
            account.clone(),
            Arc::new(StalledConn {
                entered: entered_tx,
                release: Mutex::new(Some(release_rx)),
            }),
        )
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let shutdown_client = Arc::clone(h.client());
    let shutdown_thread = std::thread::spawn(move || shutdown_client.shutdown(5_000));
    while !h.client().is_closed() {
        std::thread::yield_now();
    }
    std::thread::sleep(Duration::from_millis(30));

    assert!(
        h.client()
            .call_handle_for(account.clone(), call.attempt_id())
            .is_none()
    );
    assert!(h.client().active_call().is_none());
    assert_eq!(
        h.client().join_call(account.clone(), ch_id.clone()).err(),
        Some(FfiCallError::Closed)
    );
    assert_eq!(
        h.client().active_account_call(account.clone()).err(),
        Some(FfiCallError::Closed)
    );
    assert!(matches!(
        h.client().server_auth(server_id),
        Err(FfiClientError::Closed)
    ));
    assert!(matches!(
        h.client().create_search_surface(account.clone()),
        Err(FfiClientError::Closed)
    ));
    assert!(matches!(
        h.client()
            .validate_notification_target(FfiNotificationTarget {
                account,
                generation: 1,
                channel_id: ch_id,
                message_id: "60000000-0000-4000-8000-000000000002".into(),
            }),
        Err(FfiNotificationError::Session {
            error: FfiClientError::Closed
        })
    ));
    assert!(matches!(
        h.block_on(h.client().mark_notifications_read(Vec::new())),
        Err(FfiNotificationError::Session {
            error: FfiClientError::Closed
        })
    ));

    let _ = release_tx.send(());
    let report = shutdown_thread.join().unwrap().unwrap();
    assert_eq!(report.callback_drain, FfiCallbackDrainStatus::Drained);
    assert!(report.reads_drained);
    let _ = h.client.take();
    if let (Some(srv), Some(rt)) = (h.server.take(), h.server_rt.take()) {
        rt.block_on(srv.shutdown()).unwrap();
    }
}

fn assert_minor_redaction_and_bounds(h: &FfiTestHarness, server_id: String, server_url: &str) {
    let saved = FfiSavedServer {
        id: server_id,
        address: server_url.to_owned(),
        added_at_us: 100,
    };
    let dbg = format!("{saved:?}");
    assert!(dbg.contains("[redacted]"));
    assert!(!dbg.contains(server_url));

    let bad_account = FfiAccountId {
        server_id: "not-valid-server-id".into(),
        user_id: "bad-user".into(),
    };
    let bad_tl = h.block_on(h.client().open_live_timeline(
        bad_account.clone(),
        "60000000-0000-4000-8000-000000000001".into(),
        FfiTimelineConfig {
            page_size: 10,
            subscriber_capacity: 8,
        },
        Arc::new(ffi_fixture::ZeroUtcOffset),
    ));
    assert!(matches!(bad_tl, Err(FfiTimelineError::InvalidArgument)));
    let bad_react = h.block_on(h.client().set_reaction(
        bad_account,
        FfiReactionIntent {
            channel_id: "60000000-0000-4000-8000-000000000001".into(),
            message: FfiMessageTarget::Remote {
                message_id: "60000000-0000-4000-8000-000000000002".into(),
            },
            emoji: FfiReactionEmoji::Unicode {
                unicode: "👍".into(),
            },
            add: true,
            local_id: "60000000-0000-4000-8000-000000000003".into(),
            at_us: 100,
        },
    ));
    assert!(matches!(bad_react, Err(FfiReactionError::InvalidInput)));
    let _ = h.block_on(h.client().background_sync(u64::MAX)).unwrap();
}

#[test]
fn search_and_notification_status_invoke_on_closed_and_minor_edge_cases() {
    let h = FfiTestHarness::open(Config::default(), None);
    let server_url = h.server().base_url();
    let server_id = h
        .block_on(h.client().add_server(server_url.clone(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "judy");

    // Hold sibling Session handles alive across logout to prove streams still terminate cleanly.
    let auth = h.client().account_auth(account.clone()).unwrap();
    let search = h.client().create_search_surface(account.clone()).unwrap();

    let (s_snap_tx, s_snap_rx) = mpsc::channel();
    let (s_closed_tx, s_closed_rx) = mpsc::channel();
    let _s_sub = search
        .subscribe(Arc::new(SearchCloseWatch {
            snap: s_snap_tx,
            closed: s_closed_tx,
        }))
        .unwrap();
    let (n_snap_tx, n_snap_rx) = mpsc::channel();
    let (n_closed_tx, n_closed_rx) = mpsc::channel();
    let _n_sub = h
        .client()
        .subscribe_notification_status(
            account.clone(),
            Arc::new(NotifCloseWatch {
                snap: n_snap_tx,
                closed: n_closed_tx,
            }),
        )
        .unwrap();
    s_snap_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    n_snap_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    assert_minor_redaction_and_bounds(&h, server_id, &server_url);

    h.block_on(auth.logout()).unwrap();
    s_closed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    n_closed_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    assert!(matches!(
        search.subscribe(Arc::new(SearchCloseWatch {
            snap: mpsc::channel().0,
            closed: mpsc::channel().0,
        })),
        Err(FfiClientError::Closed)
    ));
    assert!(matches!(
        h.client().subscribe_notification_status(
            account,
            Arc::new(NotifCloseWatch {
                snap: mpsc::channel().0,
                closed: mpsc::channel().0,
            }),
        ),
        Err(FfiClientError::NotFound)
    ));

    let report = h.shutdown(u64::MAX);
    assert_eq!(report.callback_drain, FfiCallbackDrainStatus::Drained);
}
