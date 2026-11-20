//! One Papo per user. The first launch owns `io.github.chriexpe.Papo` on the
//! session bus and serves `org.freedesktop.Application` there; a later launch
//! calls its `Activate`, forwarding the launcher's activation token, and
//! exits. Flatpak lets an app own its own id on the session bus, so the same
//! code serves the sandbox and the host.
//!
//! Owns the name, the handoff and their deadlines. Raising the window is the
//! app's; one owner per profile is the SDK's lease, which stays the safety
//! net: a launch that cannot hand off opens anyway, and the SDK reports the
//! profile in use.
//!
//! **A launch while Papo quits.** The running instance refuses `Activate`
//! once it starts to quit, because its window is going. The later launch
//! stays queued for the name, which the bus hands it when the quitting
//! process lets go (after the SDK has released the profile), and opens then.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc;
use zbus::fdo::{DBusProxy, NameAcquiredStream, RequestNameFlags, RequestNameReply};
use zbus::{Connection, OwnedVariant, Variant};

use crate::window::APP_ID;

/// The service's object path: the app id as a path.
const OBJECT_PATH: &str = "/io/github/chriexpe/Papo";

/// The `platform-data` key of a Wayland activation token (Desktop Entry
/// specification, D-Bus activation).
const ACTIVATION_TOKEN_KEY: &str = "activation-token";
/// The `platform-data` key of an X11 startup-notification id.
const STARTUP_ID_KEY: &str = "desktop-startup-id";

/// What a launch brings from its launcher. Tokens are opaque and never logged.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Launch {
    /// `XDG_ACTIVATION_TOKEN`: lets a Wayland compositor focus the window.
    pub(crate) activation_token: Option<String>,
    /// `DESKTOP_STARTUP_ID`: the X11 startup-notification id.
    pub(crate) startup_id: Option<String>,
}

impl Launch {
    /// This process's launch, from its environment. An empty value counts as
    /// none.
    pub(crate) fn from_env() -> Launch {
        let read = |name| std::env::var(name).ok().filter(|value| !value.is_empty());
        Launch {
            activation_token: read("XDG_ACTIVATION_TOKEN"),
            startup_id: read("DESKTOP_STARTUP_ID"),
        }
    }

    /// The `platform-data` an `Activate` call carries for this launch.
    fn platform_data(&self) -> HashMap<&'static str, Variant<'_>> {
        let mut data = HashMap::new();
        if let Some(token) = &self.activation_token {
            data.insert(ACTIVATION_TOKEN_KEY, Variant::from(token.as_str()));
        }
        if let Some(id) = &self.startup_id {
            data.insert(STARTUP_ID_KEY, Variant::from(id.as_str()));
        }
        data
    }
}

impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Launch")
            .field("activation_token", &self.activation_token.is_some())
            .field("startup_id", &self.startup_id.is_some())
            .finish()
    }
}

/// A later launch asking for the window, with the token that lets the
/// compositor focus it.
#[derive(PartialEq, Eq)]
pub(crate) struct Activation {
    /// The Wayland activation token, else the X11 startup id; `None` when the
    /// launcher gave neither.
    pub(crate) token: Option<String>,
}

impl std::fmt::Debug for Activation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Activation")
            .field("token", &self.token.is_some())
            .finish()
    }
}

/// How long a launch waits on the bus and on the running instance.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadlines {
    /// For the session bus to connect and answer the name request. A hung
    /// bus would otherwise keep the window from ever opening.
    pub(crate) bus: Duration,
    /// For the running instance to answer `Activate`. It answers from its own
    /// thread, not the UI's, so only a hung process takes this long; then
    /// this launch opens and the SDK says the profile is in use.
    pub(crate) answer: Duration,
    /// For a quitting instance to let go of the name. Covers its exit fence
    /// (`sdk_bridge`'s ten seconds and the keyring's two) with room to spare.
    pub(crate) handover: Duration,
}

/// The deadlines the app uses.
pub(crate) const DEADLINES: Deadlines = Deadlines {
    bus: Duration::from_secs(3),
    answer: Duration::from_secs(3),
    handover: Duration::from_secs(15),
};

/// How a claim ended.
pub(crate) enum Claim {
    /// This process is Papo's instance: it owns the name, or waits in the
    /// bus's queue for it after the running one did not take the launch.
    First(Instance),
    /// The running instance took this launch; this process exits.
    HandedOff,
}

/// The claimed name and what later launches ask of it. Dropping it closes
/// the connection, which lets go of the name.
pub(crate) struct Instance {
    bus: Connection,
    activations: mpsc::UnboundedReceiver<Activation>,
}

impl Instance {
    /// The next later launch's activation, in arrival order.
    pub(crate) async fn next_activation(&mut self) -> Option<Activation> {
        self.activations.recv().await
    }

    /// The connection holding the name, for tests that act as it.
    #[cfg(test)]
    pub(super) fn bus(&self) -> &Connection {
        &self.bus
    }
}

/// Claims Papo's name on the bus `bus` builds, or hands `launch` to the
/// instance that owns it. Once `quitting` is set the instance refuses
/// `Activate`, so a later launch waits for the name instead of handing off to
/// a window that is going.
///
/// # Errors
///
/// The bus could not be reached, refused the request or did not answer
/// within `deadlines.bus`; the caller runs without a single instance.
pub(crate) async fn claim(
    bus: zbus::connection::Builder<'_>,
    launch: &Launch,
    deadlines: Deadlines,
    quitting: Arc<AtomicBool>,
) -> zbus::Result<Claim> {
    let request = tokio::time::timeout(deadlines.bus, request_name(bus, quitting))
        .await
        .map_err(|_| zbus::Error::Failure("the session bus did not answer".to_owned()))?;
    let (instance, reply, mut acquired) = request?;
    if matches!(
        reply,
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner
    ) {
        return Ok(Claim::First(instance));
    }
    match tokio::time::timeout(deadlines.answer, hand_off(&instance.bus, launch)).await {
        Ok(Ok(())) => return Ok(Claim::HandedOff),
        Ok(Err(error)) => {
            log::info!(
                "the running Papo did not take this launch ({error}); waiting for it to exit"
            );
            wait_for_name(&mut acquired, deadlines.handover).await;
        }
        Err(_) => log::warn!("the running Papo did not answer; opening anyway"),
    }
    Ok(Claim::First(instance))
}

/// Connects with the handoff service in place and asks for Papo's name.
/// Returns the instance, the bus's answer, and the grants to come.
async fn request_name(
    bus: zbus::connection::Builder<'_>,
    quitting: Arc<AtomicBool>,
) -> zbus::Result<(Instance, RequestNameReply, NameAcquiredStream)> {
    let (sender, activations) = mpsc::unbounded_channel();
    let service = Handoff {
        activations: sender,
        quitting,
    };
    // Served before the name is requested, so no call reaches the name
    // before its handler exists.
    let bus = bus.serve_at(OBJECT_PATH, service).build().await?;
    // Subscribed before the request, so the grant is not missed.
    let acquired = DBusProxy::new(&bus).await?.receive_name_acquired().await?;
    // No `DO_NOT_QUEUE`: a later launch waits in line, and gets the name
    // when the running instance exits.
    let reply = bus
        .request_name_with_flags(APP_ID, RequestNameFlags::empty())
        .await?;
    Ok((Instance { bus, activations }, reply, acquired))
}

/// Asks the name's current owner to activate. Addressed to the owner's
/// unique name, so a call made just as this process inherits the name does
/// not reach its own service and get taken for a handoff.
pub(super) async fn hand_off(bus: &Connection, launch: &Launch) -> zbus::Result<()> {
    let owner = DBusProxy::new(bus)
        .await?
        .get_name_owner(APP_ID.try_into()?)
        .await?;
    if bus.unique_name() == Some(&owner) {
        return Err(zbus::Error::Failure(
            "this launch owns the name now".to_owned(),
        ));
    }
    RunningProxy::builder(bus)
        .destination(owner.as_str())
        .build()
        .await?
        .activate(launch.platform_data())
        .await
}

/// Waits until the bus grants this connection Papo's name, or `deadline`
/// passes; either way the launch goes on.
async fn wait_for_name(acquired: &mut NameAcquiredStream, deadline: Duration) {
    let granted = async {
        while let Some(signal) = acquired.next().await {
            if signal.args().is_ok_and(|args| args.name.as_str() == APP_ID) {
                return true;
            }
        }
        false
    };
    match tokio::time::timeout(deadline, granted).await {
        Ok(true) => log::info!("the earlier Papo exited; this launch opens"),
        Ok(false) | Err(_) => log::warn!("the earlier Papo is still exiting; opening anyway"),
    }
}

/// The running instance's `org.freedesktop.Application`, as a later launch
/// calls it.
#[zbus::proxy(
    interface = "org.freedesktop.Application",
    default_path = "/io/github/chriexpe/Papo"
)]
trait Running {
    fn activate(&self, platform_data: HashMap<&str, Variant<'_>>) -> zbus::Result<()>;
}

/// The service a running instance serves. Only `Activate` is implemented:
/// Papo opens no files or URIs and exports no actions.
struct Handoff {
    activations: mpsc::UnboundedSender<Activation>,
    quitting: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl Handoff {
    fn activate(&self, platform_data: HashMap<String, OwnedVariant>) -> zbus::fdo::Result<()> {
        let refused = || zbus::fdo::Error::Failed("Papo is quitting".to_owned());
        if self.quitting.load(Ordering::Acquire) || rune::quitting() {
            return Err(refused());
        }
        let activation = Activation {
            token: token(platform_data),
        };
        self.activations.send(activation).map_err(|_| refused())
    }
}

/// The token in `platform_data`: the Wayland activation token when there is
/// one, else the X11 startup id. Rune hands either to the window, and uses
/// it only on the platform it belongs to.
fn token(mut platform_data: HashMap<String, OwnedVariant>) -> Option<String> {
    let mut text = |key: &str| {
        platform_data
            .remove(key)
            .and_then(|value| String::try_from(value).ok())
            .filter(|text| !text.is_empty())
    };
    text(ACTIVATION_TOKEN_KEY).or_else(|| text(STARTUP_ID_KEY))
}
