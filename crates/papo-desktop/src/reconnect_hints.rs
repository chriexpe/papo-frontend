//! Hints that the SDK's sockets should reconnect now: the machine woke from
//! suspend (`sleep`), or the network came back (`network`).
//!
//! Owns the hint sources and turning a hint into `Session::reconnect` for
//! every signed-in account. The SDK reconnects on its own backoff; a
//! hint only shortens the wait, so a missing source (no logind, no network monitor)
//! costs time, not correctness. Runs on the SDK client's runtime, started when
//! the client opens and cancelled by the exit fence.

mod network;
mod sleep;
#[cfg(test)]
mod tests;

use papo_sdk::{ClientError, ClientHandle};
use tokio::sync::mpsc;
use zbus::Connection;

use self::network::NetworkUps;
use self::sleep::Resumes;

/// Why the sockets should reconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hint {
    /// The machine woke from suspend: its sockets are likely dead, and
    /// nothing told the server.
    Resumed,
    /// The internet came back (through the portal, also a network).
    NetworkUp,
}

/// Runs until the client closes: watches the hint sources and reconnects
/// every signed-in account on each hint.
pub(crate) async fn run(client: ClientHandle) {
    let (hints, mut heard) = mpsc::unbounded_channel();
    let sources = async move {
        let system = connect("system", zbus::connection::Builder::system()).await;
        let session = connect("session", zbus::connection::Builder::session()).await;
        tokio::join!(
            watch_resumes(system.as_ref(), hints.clone()),
            watch_network(session.as_ref(), system.as_ref(), hints),
        );
    };
    let reconnects = async {
        while let Some(hint) = heard.recv().await {
            reconnect_all(&client, hint);
        }
    };
    tokio::join!(sources, reconnects);
}

async fn connect(bus: &str, builder: zbus::connection::Builder<'_>) -> Option<Connection> {
    builder
        .build()
        .await
        .inspect_err(|error| log::info!("no {bus} bus for reconnect hints: {error}"))
        .ok()
}

async fn watch_resumes(system: Option<&Connection>, hints: mpsc::UnboundedSender<Hint>) {
    let Some(system) = system else {
        return;
    };
    let mut resumes = match Resumes::subscribe(system).await {
        Ok(resumes) => {
            log::debug!("resume hints from logind");
            resumes
        }
        Err(error) => {
            log::info!("no resume hints from logind: {error}");
            return;
        }
    };
    while resumes.next().await.is_some() {
        if hints.send(Hint::Resumed).is_err() {
            return;
        }
    }
}

async fn watch_network(
    session: Option<&Connection>,
    system: Option<&Connection>,
    hints: mpsc::UnboundedSender<Hint>,
) {
    let Some(mut network) = NetworkUps::subscribe(session, system).await else {
        log::info!("no network hints: neither the portal nor NetworkManager answered");
        return;
    };
    while network.next().await.is_some() {
        if hints.send(Hint::NetworkUp).is_err() {
            return;
        }
    }
}

/// Starts a new connection for every account that can connect, online or
/// not: after a resume or a network change an open socket may be dead
/// without knowing it, and would wait out the SDK's heartbeat (up to two
/// 25 s rounds). A signed-out account cannot connect, and is left alone.
fn reconnect_all(client: &ClientHandle, hint: Hint) {
    let registry = std::sync::Arc::clone(&client.subscribe_registry().borrow());
    let mut reconnected = 0_usize;
    for account in &registry.accounts {
        let Ok(session) = client.session(account.id) else {
            continue;
        };
        match session.reconnect() {
            Ok(()) => reconnected += 1,
            Err(ClientError::SignedOut) => {}
            Err(error) => log::debug!("an account did not reconnect: {error}"),
        }
    }
    log::info!("{hint:?}: reconnecting {reconnected} account(s)");
}
