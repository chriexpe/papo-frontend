//! The network coming back. NetworkManager on the system bus answers on a
//! host and reports every state change; the network-monitor portal answers
//! inside Flatpak, where NetworkManager is out of reach.
//!
//! Owns reading the network's state and deciding when it was regained. What
//! that does is `reconnect_hints`'.

use futures_util::StreamExt;
use zbus::Connection;
use zbus::proxy::{CacheProperties, PropertyStream};

/// What a monitor reports, reduced to what a reconnect cares about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Network {
    /// A network is connected, whether or not it reaches the internet.
    pub(super) available: bool,
    /// The internet is reachable.
    pub(super) full: bool,
}

impl Network {
    /// From NetworkManager's `State`: 70 is connected globally, which it says
    /// also when its connectivity check is off (its `Connectivity` property
    /// would stay unknown). It reports every state change, so only the
    /// internet coming back counts: a network without it (50 local, 60 site)
    /// is not one, and a drop gives one hint, not two.
    pub(super) fn from_manager_state(state: u32) -> Network {
        let full = state == 70;
        Network {
            available: full,
            full,
        }
    }

    /// From the portal's `GetAvailable` and `GetConnectivity` (4 is full).
    pub(super) fn from_portal(available: bool, connectivity: u32) -> Network {
        Network {
            available,
            full: connectivity == 4,
        }
    }
}

/// Whether going from `before` to `now` regained the network: a network
/// came back, or the internet did. The portal signals route changes, so the
/// internet coming back on a network it already reported may arrive without
/// a signal of its own; a network coming back counts for that reason.
/// Changes that leave both as they were are not: reconnecting then would only
/// cut working sockets.
pub(super) fn regained(before: Network, now: Network) -> bool {
    (!before.available && now.available) || (!before.full && now.full)
}

/// NetworkManager, for its state only.
#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait Manager {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

/// The network-monitor portal.
#[zbus::proxy(
    interface = "org.freedesktop.portal.NetworkMonitor",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait Portal {
    fn get_available(&self) -> zbus::Result<bool>;
    fn get_connectivity(&self) -> zbus::Result<u32>;
    /// The network configuration changed. Version 1 of the portal sent an
    /// argument here; it is not read.
    #[zbus(signal)]
    fn changed(&self) -> zbus::Result<()>;
}

/// Each time the network comes back.
pub(super) struct NetworkUps {
    source: Source,
    last: Network,
}

/// Where the state is read. The portal's proxy and stream are boxed, as they
/// are much larger than the property stream.
enum Source {
    Manager(PropertyStream<'static, u32>),
    Portal(Box<PortalWatch>),
}

struct PortalWatch {
    portal: PortalProxy<'static>,
    changes: ChangedStream,
}

impl NetworkUps {
    /// Subscribes to NetworkManager on `system`, else to the portal on
    /// `session`; `None` when neither answers.
    pub(super) async fn subscribe(
        session: Option<&Connection>,
        system: Option<&Connection>,
    ) -> Option<NetworkUps> {
        if let Some(bus) = system {
            match NetworkUps::manager(bus).await {
                Ok(watch) => {
                    log::debug!("network hints from NetworkManager");
                    return Some(watch);
                }
                Err(error) => log::debug!("no NetworkManager: {error}"),
            }
        }
        if let Some(bus) = session {
            match NetworkUps::portal(bus).await {
                Ok(watch) => {
                    log::debug!("network hints from the network-monitor portal");
                    return Some(watch);
                }
                Err(error) => log::debug!("no network-monitor portal: {error}"),
            }
        }
        None
    }

    async fn manager(bus: &Connection) -> zbus::Result<NetworkUps> {
        // Property caching stays on: the change stream is fed by the cache,
        // and ends at once without it.
        let manager = ManagerProxy::new(bus).await?;
        let changes = manager.receive_state_changed().await;
        let last = Network::from_manager_state(manager.state().await?);
        Ok(NetworkUps {
            source: Source::Manager(changes),
            last,
        })
    }

    async fn portal(bus: &Connection) -> zbus::Result<NetworkUps> {
        let portal = PortalProxy::builder(bus)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        // Subscribed before the first reading, so no change falls between.
        let changes = portal.receive_changed().await?;
        let last = read_portal(&portal).await?;
        Ok(NetworkUps {
            source: Source::Portal(Box::new(PortalWatch { portal, changes })),
            last,
        })
    }

    /// Waits until the network is regained; `None` once the monitor's bus
    /// connection ends.
    pub(super) async fn next(&mut self) -> Option<()> {
        loop {
            // Each pass waits for the monitor's next change, so a failed
            // reading does not spin.
            let reported = match &mut self.source {
                Source::Manager(changes) => changes
                    .next()
                    .await?
                    .get()
                    .await
                    .map(Network::from_manager_state),
                Source::Portal(watch) => {
                    watch.changes.next().await?;
                    read_portal(&watch.portal).await
                }
            };
            let now = match reported {
                Ok(now) => now,
                Err(error) => {
                    log::debug!("the network monitor did not say its state: {error}");
                    continue;
                }
            };
            let before = std::mem::replace(&mut self.last, now);
            if regained(before, now) {
                return Some(());
            }
        }
    }
}

async fn read_portal(portal: &PortalProxy<'static>) -> zbus::Result<Network> {
    let available = portal.get_available().await?;
    let connectivity = portal.get_connectivity().await?;
    Ok(Network::from_portal(available, connectivity))
}
