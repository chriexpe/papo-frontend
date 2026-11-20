//! The server step's SDK calls: the probe that checks the registry and then
//! saves the server, and removing a server the flow leaves without an
//! account. Each runs holding the registry writes, so a probe sees the
//! removals sent before it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use papo_sdk::{ClientHandle, RegistrySnapshot, ServerAddress, ServerDiscovery, ServerId};

use super::now;
use crate::account_watch::{self, SignedOut};
use crate::shell::Shell;
use crate::sign_in::{AlreadyAdded, Discovery, Probe, ProbeFailure, SavedAccounts};
use papo_sdk::DiscoveryFailure;

/// Removes the server the flow leaves behind, unless an account is saved for
/// it by then: a sign-in that saved the account but not its credentials
/// keeps both. The next probe waits for the removal, so it never finds the
/// server still saved.
pub(crate) fn remove_left_server(shell: &Shell, id: ServerId) {
    let Some(sdk) = shell.sdk() else {
        return;
    };
    let writes = shell.registry_writes();
    // Taken now, on the UI thread, so a probe sent after this waits for it.
    let held = Arc::clone(&writes).try_lock_owned();
    sdk.run(move |client| async move {
        let _writes = match held {
            Ok(held) => held,
            Err(_) => writes.lock_owned().await,
        };
        remove_if_no_account(&client, id).await;
    });
}

async fn remove_if_no_account(client: &ClientHandle, id: ServerId) {
    let registry = Arc::clone(&client.subscribe_registry().borrow());
    if registry
        .accounts
        .iter()
        .any(|account| account.id.server() == id)
    {
        return;
    }
    if let Err(error) = client.remove_server(id).await {
        log::warn!("a server left without an account was not removed: {error}");
    }
}

/// Checks the registry for the address before saving it: `add_server`
/// probes before its own duplicate check, so a saved server that is down
/// would read as unreachable.
/// If the person closes the screens meanwhile (`abandoned`), a server it
/// saves is removed again. Runs holding the registry writes.
pub(super) async fn probe(client: ClientHandle, address: String, abandoned: &AtomicBool) -> Probe {
    let Ok(parsed) = ServerAddress::parse(&address) else {
        // The screen sends only addresses it parsed.
        return Probe::Failed(ProbeFailure::NotPapo);
    };
    let id = parsed.id();
    let registry = Arc::clone(&client.subscribe_registry().borrow());
    if registry
        .servers
        .iter()
        .any(|saved| saved.address.id() == id)
    {
        let saved = saved_accounts(&client, &registry, id, &address);
        let discovery = client.discover(&address).await.ok();
        let shown = crate::sign_in::shown_address(&address).to_owned();
        return Probe::AlreadyAdded(AlreadyAdded {
            id,
            name: discovery
                .as_ref()
                .and_then(|found| found.name.clone())
                .unwrap_or_else(|| shown.clone()),
            address: shown,
            saved,
            public: discovery.as_ref().is_none_or(|found| found.public),
            icon: discovery
                .and_then(|found| found.icon)
                .map(|icon| icon.bytes),
        });
    }
    match client.add_server(&address, now()).await {
        Ok(added) if abandoned.load(Ordering::Relaxed) => {
            remove_if_no_account(&client, added.id).await;
            Probe::Failed(ProbeFailure::Unreachable)
        }
        Ok(added) => Probe::Added {
            id: added.id,
            discovery: Discovery::from(added.discovery),
        },
        Err(error) => {
            log::info!("the server probe failed: {error}");
            Probe::Failed(match DiscoveryFailure::classify(&error) {
                Some(DiscoveryFailure::NotSetUp) => ProbeFailure::NotSetUp,
                Some(DiscoveryFailure::NotPapo) => ProbeFailure::NotPapo,
                // Unreachable, or storage or a closed client: trying again is
                // all there is.
                _ => ProbeFailure::Unreachable,
            })
        }
    }
}

/// What the accounts saved for server `id` let Open do: a working one wins,
/// else the first signed-out one signs in again.
fn saved_accounts(
    client: &ClientHandle,
    registry: &RegistrySnapshot,
    id: ServerId,
    address: &str,
) -> SavedAccounts {
    let mut saved = SavedAccounts::None;
    for account in registry
        .accounts
        .iter()
        .filter(|account| account.id.server() == id)
    {
        // An account without a session yet is one just signed in.
        let lapse = client
            .session(account.id)
            .ok()
            .and_then(|session| account_watch::lapse(session.auth().state()));
        let Some(credentials) = lapse else {
            return SavedAccounts::Working;
        };
        if saved == SavedAccounts::None {
            saved = SavedAccounts::SignedOut(SignedOut {
                id: account.id,
                credentials,
                address: address.to_owned(),
                username: account.username.clone(),
            });
        }
    }
    saved
}

impl From<ServerDiscovery> for Discovery {
    fn from(discovery: ServerDiscovery) -> Self {
        Discovery {
            name: discovery.name,
            public: discovery.public,
            icon: discovery.icon.map(|icon| icon.bytes),
        }
    }
}
