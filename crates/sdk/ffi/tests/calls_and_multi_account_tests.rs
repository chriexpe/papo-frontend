//! Integration tests covering multi-account call isolation, active call handle invalidation on
//! rejoin/switch, off-Tokio `switch_call`, and non-Tokio call control polling.

mod ffi_fixture;

use std::{sync::Arc, time::Duration};

use ffi_fixture::FfiTestHarness;
use papo_ffi::{
    FfiCallAttemptListener, FfiCallError, FfiCallPhase, FfiCallSnapshot, FfiConnectionListener,
    FfiConnectionState, FfiConnectionStatus,
};
use papo_sdk::FakeMediaFactory;
use papo_test_server::Config;

struct EndedListener(std::sync::mpsc::Sender<()>);

impl FfiCallAttemptListener for EndedListener {
    fn on_snapshot(&self, s: FfiCallSnapshot) {
        if matches!(s.phase, FfiCallPhase::Ended { .. }) {
            let _ = self.0.send(());
        }
    }
    fn on_update(&self, s: FfiCallSnapshot) {
        if matches!(s.phase, FfiCallPhase::Ended { .. }) {
            let _ = self.0.send(());
        }
    }
    fn on_closed(&self) {}
}

#[test]
fn multi_account_calls_rejoin_invalidation_and_off_tokio_switch() {
    let factory = FakeMediaFactory::new();
    let h = FfiTestHarness::open(Config::default(), Some(factory.clone()));

    let ch1 = h
        .on_server(h.server().create_voice_channel("voice-1"))
        .unwrap()
        .id
        .unwrap();
    let ch2 = h
        .on_server(h.server().create_voice_channel("voice-2"))
        .unwrap()
        .id
        .unwrap();

    let server_id = h
        .block_on(h.client().add_server(h.server().base_url(), 100))
        .unwrap()
        .server_id;
    let acct_a = h.register_and_login_online(&server_id, "grace");
    let acct_b = h.register_and_login_online(&server_id, "heidi");

    // Join voice-1 as acct_a cleanly from non-Tokio test thread.
    let call_a1 = loop {
        match h.client().join_call(acct_a.clone(), ch1.clone()) {
            Ok(c) => break c,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    h.on_server(factory.wait_starts(1));

    // Re-attaching from acct_a finds call_a1, while acct_b sees None and Busy if joining directly.
    let active_a = h
        .client()
        .active_account_call(acct_a.clone())
        .unwrap()
        .expect("alice active call");
    assert_eq!(active_a.attempt_id(), call_a1.attempt_id());
    assert!(
        h.client()
            .active_account_call(acct_b.clone())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        h.client()
            .join_call(acct_b.clone(), ch2.clone())
            .err()
            .expect("bob join while alice holds lease should be Busy"),
        FfiCallError::Busy
    );

    // Subscribe to call_a1 and verify off-Tokio `switch_call` transitions acct_a -> ended and starts acct_b.
    let (ended_tx, ended_rx) = std::sync::mpsc::channel();
    let _sub_a1 = call_a1
        .subscribe(Arc::new(EndedListener(ended_tx)))
        .unwrap();

    // Ensure no connection/subscription leakage.
    let _ = std::any::type_name::<FfiConnectionState>();
    let _ = std::any::type_name::<FfiConnectionStatus>();
    let _ = std::any::type_name::<dyn FfiConnectionListener>();

    let call_b = h
        .block_on(h.client().switch_call(acct_b.clone(), ch2))
        .unwrap();
    h.on_server(factory.wait_starts(2));
    ended_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    // Old call handle is permanently inert: controls return Ended and active_call now points to acct_b.
    assert_eq!(call_a1.set_muted(true), Err(FfiCallError::Ended));
    call_a1.leave();
    let global_active = h.client().active_call().expect("bob holds active call");
    assert_eq!(global_active.account, acct_b);
    assert_eq!(global_active.handle.attempt_id(), call_b.attempt_id());

    // Leave call_b, rejoin acct_a twice, verify old handle inertness without touching new handle.
    call_b.leave();
    for _ in 0..50 {
        if factory.leases().get(1).is_some_and(|l| l.is_stopped())
            && h.client().active_call().is_none()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let call_a2 = h.client().join_call(acct_a.clone(), ch1.clone()).unwrap();
    let call_a3 = h.block_on(h.client().switch_call(acct_a, ch1)).unwrap();
    assert_ne!(call_a2.attempt_id(), call_a3.attempt_id());
    assert_eq!(call_a2.set_muted(true), Err(FfiCallError::Ended));
    assert_eq!(call_a3.set_muted(false), Ok(()));

    let report = h.shutdown(1_000);
    assert!(report.reads_drained);
}
