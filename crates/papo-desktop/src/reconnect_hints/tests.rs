//! The hint sources against fake logind, portal and NetworkManager services
//! on private buses: no real suspend, no real network change.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::network::{Network, NetworkUps, regained};
use super::sleep::{Resumes, is_resume};
use crate::private_bus::PrivateBus;

/// Guards against a hung bus, not timing assumptions: each wait ends as soon
/// as its hint arrives.
const HANG_GUARD: Duration = Duration::from_secs(20);

/// The portal's full connectivity, and less.
const FULL: u32 = 4;
const LIMITED: u32 = 2;
const LOCAL: u32 = 1;
/// NetworkManager's states: connected globally, and disconnected.
const GLOBAL: u32 = 70;
const DISCONNECTED: u32 = 20;

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(HANG_GUARD, future)
        .await
        .expect("an answer in time")
}

#[test]
fn only_waking_up_is_a_resume() {
    assert!(is_resume(false));
    assert!(!is_resume(true));
}

#[test]
fn a_network_or_the_internet_coming_back_regains_the_network() {
    let down = Network::from_manager_state(DISCONNECTED);
    let local = Network::from_manager_state(50);
    let global = Network::from_manager_state(GLOBAL);
    assert!(
        !regained(down, local),
        "NetworkManager: a network alone is not"
    );
    assert!(regained(local, global), "the internet came back");
    assert!(regained(down, global));
    assert!(!regained(global, global), "nothing came back");
    assert!(!regained(global, local));
    let portal_down = Network::from_portal(false, LOCAL);
    let portal_network = Network::from_portal(true, LIMITED);
    assert!(
        regained(portal_down, portal_network),
        "the portal: a network came back"
    );
    assert_eq!(
        Network::from_portal(true, FULL),
        Network {
            available: true,
            full: true
        }
    );
    assert!(!Network::from_portal(true, LIMITED).full);
}

/// Sends logind's sleep signal as `org.freedesktop.login1` would.
async fn prepare_for_sleep(logind: &zbus::Connection, start: bool) {
    logind
        .emit_signal(
            None::<&str>,
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "PrepareForSleep",
            &(start,),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn waking_from_suspend_is_heard() {
    let bus = PrivateBus::start();
    let logind = bus.connect().await;
    logind.request_name("org.freedesktop.login1").await.unwrap();
    let watcher = bus.connect().await;
    let mut resumes = Resumes::subscribe(&watcher).await.unwrap();
    prepare_for_sleep(&logind, true).await;
    prepare_for_sleep(&logind, false).await;
    assert_eq!(within(resumes.next()).await, Some(()));
}

/// The portal's network monitor, with a connectivity the test sets. The
/// network is available whenever the connectivity is more than none.
struct FakePortal {
    connectivity: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.freedesktop.portal.NetworkMonitor")]
impl FakePortal {
    fn get_available(&self) -> bool {
        self.connectivity.load(Ordering::Acquire) > 1
    }

    fn get_connectivity(&self) -> u32 {
        self.connectivity.load(Ordering::Acquire)
    }

    #[zbus(signal)]
    async fn changed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
}

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

async fn serve_portal(bus: &PrivateBus, connectivity: &Arc<AtomicU32>) -> zbus::Connection {
    let portal = FakePortal {
        connectivity: Arc::clone(connectivity),
    };
    bus.builder()
        .serve_at(PORTAL_PATH, portal)
        .name("org.freedesktop.portal.Desktop")
        .build()
        .await
        .unwrap()
}

async fn portal_reports(portal: &zbus::Connection, connectivity: &AtomicU32, value: u32) {
    connectivity.store(value, Ordering::Release);
    let iface = portal
        .object_server()
        .interface::<_, FakePortal>(PORTAL_PATH)
        .await
        .unwrap();
    FakePortal::changed(iface.signal_emitter()).await.unwrap();
}

#[tokio::test]
async fn without_network_manager_the_portal_says_when_the_internet_comes_back() {
    let session = PrivateBus::start();
    let system = PrivateBus::start();
    // The network is limited when the watch starts. (The watch reads the
    // state when it hears a change, so a drop and a return sent back to back
    // would both read as the return.)
    let connectivity = Arc::new(AtomicU32::new(LIMITED));
    let portal = serve_portal(&session, &connectivity).await;
    let (session_watcher, system_watcher) = (session.connect().await, system.connect().await);
    let mut network = NetworkUps::subscribe(Some(&session_watcher), Some(&system_watcher))
        .await
        .expect("the portal as the fallback");
    portal_reports(&portal, &connectivity, FULL).await;
    assert_eq!(within(network.next()).await, Some(()));
}

#[tokio::test]
async fn the_portal_saying_a_network_is_back_is_a_hint_before_the_internet_is() {
    let bus = PrivateBus::start();
    // Local only: the portal reports no network available.
    let connectivity = Arc::new(AtomicU32::new(LOCAL));
    let portal = serve_portal(&bus, &connectivity).await;
    let watcher = bus.connect().await;
    let mut network = NetworkUps::subscribe(Some(&watcher), None).await.unwrap();
    portal_reports(&portal, &connectivity, LIMITED).await;
    assert_eq!(within(network.next()).await, Some(()));
}

/// NetworkManager, with a state the test sets.
struct FakeManager {
    state: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl FakeManager {
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.state
    }
}

const MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";

async fn manager_reports(manager: &zbus::Connection, value: u32) {
    let iface = manager
        .object_server()
        .interface::<_, FakeManager>(MANAGER_PATH)
        .await
        .unwrap();
    iface.get_mut().await.state = value;
    iface
        .get()
        .await
        .state_changed(iface.signal_emitter())
        .await
        .unwrap();
}

#[tokio::test]
async fn network_manager_is_preferred_and_says_when_the_network_comes_back() {
    let session = PrivateBus::start();
    let system = PrivateBus::start();
    // A portal that never signals: the hint must come from NetworkManager.
    let _portal = serve_portal(&session, &Arc::new(AtomicU32::new(LIMITED))).await;
    let manager = system
        .builder()
        .serve_at(
            MANAGER_PATH,
            FakeManager {
                state: DISCONNECTED,
            },
        )
        .name("org.freedesktop.NetworkManager")
        .build()
        .await
        .unwrap();
    let (session_watcher, system_watcher) = (session.connect().await, system.connect().await);
    let mut network = NetworkUps::subscribe(Some(&session_watcher), Some(&system_watcher))
        .await
        .unwrap();
    manager_reports(&manager, GLOBAL).await;
    assert_eq!(within(network.next()).await, Some(()));
}

#[tokio::test]
async fn with_no_network_monitor_there_are_no_network_hints() {
    let bus = PrivateBus::start();
    let watcher = bus.connect().await;
    assert!(
        NetworkUps::subscribe(Some(&watcher), Some(&watcher))
            .await
            .is_none()
    );
}
