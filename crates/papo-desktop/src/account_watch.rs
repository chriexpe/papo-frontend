//! Watches the SDK for the two facts the shell decides with: how many accounts
//! are saved, and which of them must sign in again.
//!
//! Owns only the watch loop and the summary it posts. The registry, sessions
//! and authentication states stay in the SDK; what the shell opens in answer
//! is the shell's.

use std::sync::Arc;

use papo_sdk::{AccountId, AuthState, ClientHandle, RegistrySnapshot};
use tokio::task::JoinSet;

use crate::sdk_bridge::Updates;

/// What the shell needs to know about the saved accounts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Accounts {
    /// How many accounts are saved, signed in or not.
    pub(crate) saved: usize,
    /// Saved accounts that must sign in again, in the registry's order.
    pub(crate) signed_out: Vec<SignedOut>,
}

impl Accounts {
    /// Whether every saved account must sign in again, so the app has nothing
    /// to show without one. True when none is saved.
    pub(crate) fn none_usable(&self) -> bool {
        self.signed_out.len() == self.saved
    }
}

/// A saved account whose credentials are gone, were rejected, or wait for the
/// server's password.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SignedOut {
    /// The account.
    pub(crate) id: AccountId,
    /// Whether the keyring still holds its credentials.
    pub(crate) credentials: Credentials,
    /// Its server's canonical address. Never logged.
    pub(crate) address: String,
    /// The username it last signed in with. Never logged.
    pub(crate) username: String,
}

/// What the keyring holds for a signed-out account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Credentials {
    /// Its credentials are kept but no longer work: the server rejected them,
    /// or wants its own password first.
    Kept,
    /// The keyring has none for it.
    Missing,
}

impl std::fmt::Debug for SignedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedOut")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Runs until the client closes: posts the accounts' summary now, then again
/// each time the registry, the set of sessions or any account's
/// authentication state changes it.
pub(crate) async fn run<S: 'static>(
    client: ClientHandle,
    updates: Updates<S>,
    apply: fn(&mut S, Accounts) -> bool,
) {
    let mut registry = client.subscribe_registry();
    let mut sessions = client.subscribe_session_changes();
    let mut posted = None;
    loop {
        let snapshot = Arc::clone(&registry.borrow_and_update());
        sessions.borrow_and_update();
        // Rebuilt on every pass; dropping the old set stops its waits.
        let mut auth_changes = JoinSet::new();
        let accounts = summarize(&client, &snapshot, &mut auth_changes);
        if posted.as_ref() != Some(&accounts) {
            posted = Some(accounts.clone());
            updates.send(move |state| apply(state, accounts));
        }
        tokio::select! {
            changed = registry.changed() => if changed.is_err() { return },
            changed = sessions.changed() => if changed.is_err() { return },
            Some(_) = auth_changes.join_next(), if !auth_changes.is_empty() => {}
        }
    }
}

/// Reads each saved account's authentication state, and queues a wait for
/// its next change in `auth_changes`.
fn summarize(
    client: &ClientHandle,
    registry: &RegistrySnapshot,
    auth_changes: &mut JoinSet<()>,
) -> Accounts {
    let mut signed_out = Vec::new();
    for account in &registry.accounts {
        // An account just saved may not have its session yet; the session
        // change that adds it wakes this loop again.
        let Ok(session) = client.session(account.id) else {
            continue;
        };
        let mut auth = session.auth().subscribe();
        let state = *auth.borrow_and_update();
        auth_changes.spawn(async move {
            if auth.changed().await.is_err() {
                // A closed session never changes again; the session change
                // that closed it wakes the loop instead.
                std::future::pending::<()>().await;
            }
        });
        let credentials = match state {
            AuthState::SignedOut => Credentials::Missing,
            _ if must_sign_in(state) => Credentials::Kept,
            _ => continue,
        };
        let server = registry
            .servers
            .iter()
            .find(|server| server.address.id() == account.id.server());
        if let Some(server) = server {
            signed_out.push(SignedOut {
                id: account.id,
                credentials,
                address: server.address.as_str().to_owned(),
                username: account.username.clone(),
            });
        }
    }
    Accounts {
        saved: registry.accounts.len(),
        signed_out,
    }
}

/// Whether an account in `state` needs the person to sign in before it works.
/// `NeedsServer` does not: it keeps its credentials and retries by itself.
/// `SigningIn` still does until the attempt succeeds, so an attempt in flight
/// does not read as the account coming back and lapsing again.
fn must_sign_in(state: AuthState) -> bool {
    matches!(
        state,
        AuthState::Expired
            | AuthState::SignedOut
            | AuthState::NeedsServerPassword
            | AuthState::SigningIn
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_signs_in_again_only_when_its_credentials_do_not_work() {
        for state in [
            AuthState::Expired,
            AuthState::SignedOut,
            AuthState::NeedsServerPassword,
            AuthState::SigningIn,
        ] {
            assert!(must_sign_in(state), "{state:?}");
        }
        for state in [AuthState::Ready, AuthState::NeedsServer] {
            assert!(!must_sign_in(state), "{state:?}");
        }
    }

    #[test]
    fn the_app_has_nothing_to_show_when_every_saved_account_is_signed_out() {
        assert!(Accounts::default().none_usable());
        let one_working = Accounts {
            saved: 1,
            signed_out: Vec::new(),
        };
        assert!(!one_working.none_usable());
    }
}
