//! The single-instance handoff on a private bus: two launches of Papo, each
//! its own connection, as two processes would be.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::single_instance::{Activation, Claim, Deadlines, Instance, Launch, claim, hand_off};
use super::{AppControl, serve_app};
use crate::private_bus::PrivateBus;

/// Guards against a hung bus, not timing assumptions: each wait ends as soon
/// as its answer arrives.
const HANG_GUARD: Duration = Duration::from_secs(20);

const DEADLINES: Deadlines = Deadlines {
    bus: HANG_GUARD,
    answer: HANG_GUARD,
    handover: HANG_GUARD,
};

fn launch(activation_token: Option<&str>, startup_id: Option<&str>) -> Launch {
    Launch {
        activation_token: activation_token.map(str::to_owned),
        startup_id: startup_id.map(str::to_owned),
    }
}

async fn first(bus: &PrivateBus, quitting: &Arc<AtomicBool>) -> Instance {
    match claim(
        bus.builder(),
        &Launch::default(),
        DEADLINES,
        Arc::clone(quitting),
    )
    .await
    {
        Ok(Claim::First(instance)) => instance,
        Ok(Claim::HandedOff) => panic!("the first launch handed off"),
        Err(error) => panic!("the first launch could not claim: {error}"),
    }
}

/// Waits until `count` connections own or wait for Papo's name.
async fn wait_until_queued(bus: &PrivateBus, count: usize) {
    let observer = bus.connect().await;
    let dbus = zbus::fdo::DBusProxy::new(&observer).await.unwrap();
    let queued = async {
        loop {
            let owners = dbus
                .list_queued_owners(crate::window::APP_ID.try_into().unwrap())
                .await
                .unwrap_or_default();
            if owners.len() >= count {
                return;
            }
            // The bus has no signal for a queued owner; poll it gently.
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(HANG_GUARD, queued)
        .await
        .expect("the launch queued in time");
}

async fn next_activation(instance: &mut Instance) -> Activation {
    tokio::time::timeout(HANG_GUARD, instance.next_activation())
        .await
        .expect("an activation in time")
        .expect("an activation")
}

#[tokio::test]
async fn the_first_launch_owns_the_name() {
    let bus = PrivateBus::start();
    let _instance = first(&bus, &Arc::default()).await;
    let owner = zbus::fdo::DBusProxy::new(&bus.connect().await)
        .await
        .unwrap()
        .name_has_owner(crate::window::APP_ID.try_into().unwrap())
        .await
        .unwrap();
    assert!(owner);
}

#[tokio::test]
async fn a_second_launch_hands_its_activation_token_to_the_first() {
    let bus = PrivateBus::start();
    let mut running = first(&bus, &Arc::default()).await;
    let second = claim(
        bus.builder(),
        &launch(Some("wayland-token"), Some("x11-id")),
        DEADLINES,
        Arc::default(),
    )
    .await
    .unwrap();
    assert!(matches!(second, Claim::HandedOff));
    let activation = next_activation(&mut running).await;
    assert_eq!(activation.token.as_deref(), Some("wayland-token"));
}

#[tokio::test]
async fn without_a_wayland_token_the_startup_id_is_forwarded() {
    let bus = PrivateBus::start();
    let mut running = first(&bus, &Arc::default()).await;
    let second = claim(
        bus.builder(),
        &launch(None, Some("x11-id")),
        DEADLINES,
        Arc::default(),
    )
    .await
    .unwrap();
    assert!(matches!(second, Claim::HandedOff));
    assert_eq!(
        next_activation(&mut running).await.token.as_deref(),
        Some("x11-id")
    );
}

#[tokio::test]
async fn a_launch_without_a_token_still_raises_the_window() {
    let bus = PrivateBus::start();
    let mut running = first(&bus, &Arc::default()).await;
    let second = claim(bus.builder(), &Launch::default(), DEADLINES, Arc::default())
        .await
        .unwrap();
    assert!(matches!(second, Claim::HandedOff));
    assert_eq!(next_activation(&mut running).await.token, None);
}

#[tokio::test]
async fn a_launch_while_papo_quits_opens_once_the_name_is_let_go() {
    let bus = PrivateBus::start();
    let quitting = Arc::new(AtomicBool::new(false));
    let running = first(&bus, &quitting).await;
    quitting.store(true, Ordering::Release);
    let second = tokio::spawn({
        let builder = bus.builder();
        async move {
            claim(
                builder,
                &launch(Some("token"), None),
                DEADLINES,
                Arc::default(),
            )
            .await
        }
    });
    // The second launch is refused and waits in the bus's queue; letting go
    // of the name, as the end of the exit fence does, hands it over.
    wait_until_queued(&bus, 2).await;
    drop(running);
    let second = tokio::time::timeout(HANG_GUARD, second)
        .await
        .expect("the second launch in time")
        .unwrap()
        .unwrap();
    let Claim::First(_second) = second else {
        panic!("the second launch handed off to a quitting Papo");
    };
}

#[tokio::test]
async fn a_launch_that_owns_the_name_does_not_hand_off_to_itself() {
    let bus = PrivateBus::start();
    let owner = first(&bus, &Arc::default()).await;
    // As a launch that inherited the name between its request and its call.
    let handed = hand_off(owner.bus(), &launch(Some("token"), None)).await;
    assert!(handed.is_err());
}

/// The app as the lifecycle thread sees it, recording what it was asked.
#[derive(Default)]
struct FakeApp {
    asked: std::sync::Mutex<Vec<Option<String>>>,
}

impl AppControl for FakeApp {
    fn activate(&self, token: Option<String>) {
        self.asked.lock().unwrap().push(token);
    }

    fn quit(&self) {}
}

#[tokio::test]
async fn a_launch_before_the_app_is_attached_raises_it_once_attached_and_stop_lets_go_of_the_name()
{
    let bus = PrivateBus::start();
    let running = first(&bus, &Arc::default()).await;
    // A later launch hands off before the lifecycle thread serves the app.
    let second = claim(
        bus.builder(),
        &launch(Some("early"), None),
        DEADLINES,
        Arc::default(),
    )
    .await
    .unwrap();
    assert!(matches!(second, Claim::HandedOff));
    let app = FakeApp::default();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let quitting = AtomicBool::new(false);
    let serving = serve_app(&app, Some(running), None, &quitting, stopped);
    let asked = async {
        while app.asked.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
        stop.send(()).unwrap();
    };
    tokio::time::timeout(HANG_GUARD, async { tokio::join!(serving, asked) })
        .await
        .expect("served in time");
    assert_eq!(*app.asked.lock().unwrap(), [Some("early".to_owned())]);
    // The connection closes as the instance drops; the bus hears it soon
    // after, not at once.
    let observer = bus.connect().await;
    let dbus = zbus::fdo::DBusProxy::new(&observer).await.unwrap();
    let released = async {
        while dbus
            .name_has_owner(crate::window::APP_ID.try_into().unwrap())
            .await
            .unwrap()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(HANG_GUARD, released)
        .await
        .expect("stopping lets go of the name");
}
