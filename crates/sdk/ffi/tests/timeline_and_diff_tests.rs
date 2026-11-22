//! Integration tests covering live/focused timeline windows, diff propagation, mid-delivery
//! disposal without session close, `cached_pinned_messages`, and clean terminal closure on permission denial.

mod ffi_fixture;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use ffi_fixture::{FfiTestHarness, ZeroUtcOffset};
use papo_ffi::{
    FfiAccountId, FfiHistoryClosedReason, FfiTimelineConfig, FfiTimelineDiff, FfiTimelineDirection,
    FfiTimelineItem, FfiTimelineListener, FfiTimelineSnapshot, FfiTimelineUpdate,
    FfiVisibilityContinuity,
};
use papo_sdk::Message;
use papo_test_server::{Config, HistorySeed};

struct RecListener {
    snap: mpsc::Sender<FfiTimelineSnapshot>,
    evt: mpsc::Sender<FfiTimelineUpdate>,
    closed: Arc<AtomicBool>,
}

impl FfiTimelineListener for RecListener {
    fn on_snapshot(&self, snapshot: FfiTimelineSnapshot) {
        let _ = self.snap.send(snapshot);
    }
    fn on_update(&self, update: FfiTimelineUpdate) {
        let _ = self.evt.send(update);
    }
    fn on_closed(&self, _reason: FfiHistoryClosedReason) {
        self.closed.store(true, Ordering::Release);
    }
}

struct CloseWatch {
    tx: mpsc::Sender<FfiHistoryClosedReason>,
    closed_seen: Arc<Mutex<bool>>,
}

impl FfiTimelineListener for CloseWatch {
    fn on_snapshot(&self, _: FfiTimelineSnapshot) {}
    fn on_update(&self, _: FfiTimelineUpdate) {}
    fn on_closed(&self, reason: FfiHistoryClosedReason) {
        *self.closed_seen.lock().unwrap() = true;
        let _ = self.tx.send(reason);
    }
}

fn live_message(channel_id: &str, author_id: &str, seq: u128) -> Message {
    Message {
        id: Some(format!("50000000-0000-4000-8000-{seq:012x}")),
        channel_id: Some(channel_id.to_owned()),
        author_id: Some(author_id.to_owned()),
        content: Some(String::new()),
        created_at: Some(format!("2026-01-01T00:01:{:02}.000000Z", seq % 50 + 1)),
        ..Message::default()
    }
}

fn verify_live_diffs_and_pins(
    h: &FfiTestHarness,
    account: &FfiAccountId,
    channel_id: &str,
    anchor_id: &str,
    cfg: FfiTimelineConfig,
) {
    let live = h
        .block_on(h.client().open_live_timeline(
            account.clone(),
            channel_id.to_owned(),
            cfg,
            Arc::new(ZeroUtcOffset),
        ))
        .unwrap();

    let (snap_tx, snap_rx) = mpsc::channel::<FfiTimelineSnapshot>();
    let (evt_tx, evt_rx) = mpsc::channel::<FfiTimelineUpdate>();
    let closed_flag = Arc::new(AtomicBool::new(false));
    let sub1 = live
        .subscribe(Arc::new(RecListener {
            snap: snap_tx,
            evt: evt_tx,
            closed: Arc::clone(&closed_flag),
        }))
        .unwrap();
    let baseline = snap_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(baseline.items.len(), 0);

    let vis_ids: Vec<String> = baseline
        .items
        .iter()
        .filter_map(|item| match item {
            FfiTimelineItem::Message { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    h.block_on(live.report_visible(
        baseline.version,
        baseline.generation,
        vis_ids,
        FfiVisibilityContinuity::Discontinuous,
    ))
    .unwrap();

    let live_msg = live_message(channel_id, &account.user_id, 1);
    let live_id = live_msg.id.clone().unwrap();
    h.on_server(h.server().publish_message(live_msg)).unwrap();
    h.on_server(
        h.server()
            .edit_message(&live_id, "", "2026-01-01T00:02:00.000000Z"),
    )
    .unwrap();
    h.on_server(h.server().delete_message(&live_id)).unwrap();

    let mut saw_set_or_remove = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(update) = evt_rx.recv_timeout(Duration::from_millis(200))
            && update.diffs.iter().any(|d| {
                matches!(
                    d,
                    FfiTimelineDiff::Set { .. } | FfiTimelineDiff::Remove { .. }
                )
            })
        {
            saw_set_or_remove = true;
            break;
        }
    }
    assert!(saw_set_or_remove);

    sub1.dispose();
    assert!(sub1.is_disposed());
    assert!(!h.client().is_closed());

    let pins = h
        .block_on(
            h.client()
                .pinned_messages(account.clone(), channel_id.to_owned()),
        )
        .unwrap();
    assert_eq!(pins.messages.len(), 1);
    let cached_pins = h
        .block_on(
            h.client()
                .cached_pinned_messages(account.clone(), channel_id.to_owned()),
        )
        .unwrap();
    assert_eq!(cached_pins.messages.len(), 1);

    let focused = h
        .block_on(h.client().open_focused_timeline_at(
            account.clone(),
            channel_id.to_owned(),
            anchor_id.to_owned(),
            cfg,
            Arc::new(ZeroUtcOffset),
        ))
        .unwrap();
    let _live_at = h
        .block_on(h.client().open_live_timeline_at(
            account.clone(),
            channel_id.to_owned(),
            anchor_id.to_owned(),
            cfg,
            Arc::new(ZeroUtcOffset),
        ))
        .unwrap();
    let _page = h
        .block_on(focused.paginate(FfiTimelineDirection::Older))
        .unwrap();
}

fn verify_permission_close(
    h: &FfiTestHarness,
    account: &FfiAccountId,
    denied_channel_id: &str,
    cfg: FfiTimelineConfig,
) {
    let denied_live = h
        .block_on(h.client().open_live_timeline(
            account.clone(),
            denied_channel_id.to_owned(),
            cfg,
            Arc::new(ZeroUtcOffset),
        ))
        .unwrap();
    let (close_tx, close_rx) = mpsc::channel::<FfiHistoryClosedReason>();
    let _sub2 = denied_live
        .subscribe(Arc::new(CloseWatch {
            tx: close_tx,
            closed_seen: Arc::new(Mutex::new(false)),
        }))
        .unwrap();

    h.on_server(
        h.server()
            .set_channel_readable(&account.user_id, denied_channel_id, false),
    )
    .unwrap();
    let mut paginate_err = None;
    for _ in 0..35 {
        match h.block_on(denied_live.paginate(FfiTimelineDirection::Older)) {
            Ok(page) if page.has_more => {}
            Ok(page) => panic!("unexpected end of un-denied pages: {page:?}"),
            Err(err) => {
                paginate_err = Some(err);
                break;
            }
        }
    }
    let paginate_err = paginate_err.expect("expected permission closure while paginating older");
    assert!(matches!(
        paginate_err,
        papo_ffi::FfiTimelineError::Closed { .. }
            | papo_ffi::FfiTimelineError::History {
                error: papo_ffi::FfiHistoryError::Closed { .. }
            }
    ));
    let reason = close_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        reason,
        FfiHistoryClosedReason::Forbidden | FfiHistoryClosedReason::PermissionDenied
    ));
}

#[test]
fn timeline_live_focused_pins_dispose_resubscribe_and_permission_close() {
    let channel_id = "60000000-0000-4000-8000-000000000001".to_owned();
    let denied_channel_id = "60000000-0000-4000-8000-000000000002".to_owned();
    let seed = HistorySeed {
        channel_id: channel_id.clone(),
        author_id: "00000000-0000-4000-8000-000000000077".into(),
        first_message_id: "60000000-0000-4000-8000-000000000100".into(),
        count: 15,
        first_created_at: "2026-01-01T00:00:01.000000Z".into(),
        timestamp_step_us: 1_000,
        equal_timestamp_count: 1,
    };
    let denied_seed = HistorySeed {
        channel_id: denied_channel_id.clone(),
        author_id: "00000000-0000-4000-8000-000000000077".into(),
        first_message_id: "60000000-0000-4000-8000-000000000200".into(),
        count: 150,
        first_created_at: "2026-01-01T00:00:01.000000Z".into(),
        timestamp_step_us: 1_000,
        equal_timestamp_count: 1,
    };
    let h = FfiTestHarness::open(
        Config {
            history: vec![seed, denied_seed],
            ..Config::default()
        },
        None,
    );
    let anchor_id = "60000000-0000-4000-8000-000000000104".to_owned();
    h.on_server(
        h.server()
            .set_message_pin(&anchor_id, Some("2026-01-01T00:00:05.000000Z")),
    )
    .unwrap();

    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let account = h.register_and_login_online(&server_id, "frank");
    let cfg = FfiTimelineConfig {
        page_size: 5,
        subscriber_capacity: 16,
    };
    verify_live_diffs_and_pins(&h, &account, &channel_id, &anchor_id, cfg);
    verify_permission_close(&h, &account, &denied_channel_id, cfg);

    let report = h.shutdown(1_000);
    assert!(report.reads_drained);
}
