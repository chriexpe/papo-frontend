//! Carries out the flow's effects: the SDK calls behind each step, run on the
//! client's runtime through the bridge, and their answers stepped back into
//! the flow on the UI thread.
//!
//! Owns turning the SDK's results into the flow's events ([`Probe`],
//! [`Failure`], [`Created`]). What the answers mean for the screen is
//! `SignIn::step`'s; what the shell does around the flow is the shell's.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use papo_sdk::{
    AccountId, AuthFailure, AuthService, ClientError, ClientHandle, DiscoveryFailure, LoginRequest,
    LoginServerRequest, RegisterRequest, ServerAddress, ServerDiscovery, ServerId, UnixMicros,
};
use rune::machine::Effects;

use super::{
    AlreadyAdded, Created, Discovery, Effect, Event, Failure, Probe, ProbeFailure, Renewing,
    Screen, SignIn,
};
use crate::account_watch::Credentials;
use crate::sdk_bridge::Sdk;
use crate::shell::Shell;

/// Carries out `effects` in order.
pub(crate) fn run(shell: &mut Shell, effects: Effects<Effect>) {
    for effect in effects {
        match effect {
            Effect::LoadIcon(bytes) => shell.load_server_icon(bytes),
            Effect::Finish { server } => shell.finish_sign_in(server),
            Effect::OpenServer(_) => shell.end_sign_in(),
            effect => request(shell, effect),
        }
    }
}

/// Sends the SDK request behind `effect`. Without an open client the request
/// is dropped and the step stays busy; the screens open only once it is.
fn request(shell: &mut Shell, effect: Effect) {
    let Some(sdk) = shell.sdk().cloned() else {
        log::warn!("dropped a sign-in request: the SDK client is not open");
        return;
    };
    match effect {
        Effect::Probe { address } => {
            let ticket = shell.sign_in_requests_mut().issue();
            let writes = shell.registry_writes();
            let abandoned = shell
                .sign_in()
                .map(|flow| Arc::clone(&flow.abandoned))
                .unwrap_or_default();
            sdk.request(
                ticket,
                Shell::sign_in_requests,
                move |client| async move {
                    let _writes = writes.lock_owned().await;
                    probe(client, address, &abandoned).await
                },
                |shell, probe| shell.sign_in_event(Event::Probed(probe)),
            );
        }
        Effect::Discover { address } => {
            let ticket = shell.server_card_requests_mut().issue();
            sdk.request(
                ticket,
                Shell::server_card_requests,
                move |client| async move {
                    client
                        .discover(&address)
                        .await
                        .inspect_err(|error| {
                            log::info!("the server did not describe itself: {error}");
                        })
                        .ok()
                        .map(Discovery::from)
                },
                |shell, discovery| shell.sign_in_event(Event::Discovered(discovery)),
            );
        }
        Effect::Unlock { password } => {
            with_auth(shell, &sdk, Event::Unlocked, Err, move |auth| async move {
                auth.server_login(LoginServerRequest {
                    server_password: password,
                })
                .await
                .map_err(|error| failure("unlock", &error))
            });
        }
        Effect::SignIn { username, password } => {
            let expected = renewed_account(shell);
            with_auth(shell, &sdk, Event::SignedIn, Err, move |auth| async move {
                sign_in(&auth, username, password, expected).await
            });
        }
        Effect::CreateAccount { username, password } => {
            with_auth(
                shell,
                &sdk,
                Event::Created,
                Created::NotRegistered,
                move |auth| async move { create_account(&auth, username, password).await },
            );
        }
        Effect::RemoveServer(id) => remove_left_server(shell, id),
        Effect::LoadIcon(_) | Effect::Finish { .. } | Effect::OpenServer(_) => {}
    }
}

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

/// The account the flow signs in again, if it does.
fn renewed_account(shell: &Shell) -> Option<AccountId> {
    let Screen::Account { server, .. } = shell.sign_in()?.screen() else {
        return None;
    };
    server.renewing.map(|renewing| renewing.account)
}

/// Where the flow's sign-in handle comes from when it has none yet.
#[derive(Clone, Copy)]
enum AuthSource {
    /// A new account on a saved server, or a saved one whose credentials the
    /// keyring lost: its own handle would have nothing to send.
    Server(ServerId),
    /// A saved account signing in again with its credentials kept: its own
    /// handle, so the SDK checks the same person signs in.
    Account(AccountId),
}

impl AuthSource {
    fn resolve(self, client: &ClientHandle) -> Result<AuthService, ClientError> {
        match self {
            AuthSource::Server(server) => client.auth(server),
            AuthSource::Account(account) => Ok(client.session(account)?.auth()),
        }
    }
}

/// Runs `work` with the flow's sign-in handle (fetching it on first use),
/// keeps the handle for the next step, and steps the answer in as
/// `event(answer)`; `failed` makes the answer when there is no handle.
fn with_auth<T, Work>(
    shell: &mut Shell,
    sdk: &Sdk<Shell>,
    event: fn(T) -> Event,
    failed: fn(Failure) -> T,
    work: impl FnOnce(AuthService) -> Work + Send + 'static,
) where
    T: Send + 'static,
    Work: Future<Output = T> + Send + 'static,
{
    let Some((source, auth)) = shell.sign_in().and_then(SignIn::auth_source) else {
        return;
    };
    let ticket = shell.sign_in_requests_mut().issue();
    sdk.request(
        ticket,
        Shell::sign_in_requests,
        move |client| async move {
            let auth = match auth.map_or_else(|| source.resolve(&client), Ok) {
                Ok(auth) => auth,
                Err(error) => return (None, failed(failure("sign-in handle", &error))),
            };
            let answer = work(auth.clone()).await;
            (Some(auth), answer)
        },
        move |shell, (auth, answer)| {
            shell.keep_sign_in_auth(auth);
            shell.sign_in_event(event(answer))
        },
    );
}

impl SignIn {
    /// Where the flow's sign-in handle comes from, and the one it has.
    fn auth_source(&self) -> Option<(AuthSource, Option<AuthService>)> {
        let Screen::Account { server, .. } = self.screen() else {
            return None;
        };
        let source = match server.renewing {
            Some(Renewing {
                account,
                credentials: Credentials::Kept,
            }) => AuthSource::Account(account),
            Some(Renewing {
                credentials: Credentials::Missing,
                ..
            })
            | None => AuthSource::Server(server.id),
        };
        Some((source, self.auth.clone()))
    }
}

/// Checks the registry for the address before saving it: `add_server`
/// probes before its own duplicate check, so a saved server that is down
/// would read as unreachable.
/// If the person closes the screens meanwhile (`abandoned`), a server it
/// saves is removed again. Runs holding the registry writes.
async fn probe(client: ClientHandle, address: String, abandoned: &AtomicBool) -> Probe {
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
        let has_account = registry
            .accounts
            .iter()
            .any(|account| account.id.server() == id);
        let discovery = client.discover(&address).await.ok();
        let shown = super::address::shown(&address).to_owned();
        return Probe::AlreadyAdded(AlreadyAdded {
            id,
            name: discovery
                .as_ref()
                .and_then(|found| found.name.clone())
                .unwrap_or_else(|| shown.clone()),
            address: shown,
            has_account,
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

/// Signs in; when `expected` names the account signing in again, another
/// person's credentials are wrong credentials here.
async fn sign_in(
    auth: &AuthService,
    username: String,
    password: String,
    expected: Option<AccountId>,
) -> Result<(), Failure> {
    let session = auth
        .login(LoginRequest { username, password }, now())
        .await
        .map_err(|error| failure("sign-in", &error))?;
    if expected.is_some_and(|account| account != session.account()) {
        log::info!("sign-in again was for another account");
        return Err(Failure::WrongCredentials);
    }
    Ok(())
}

/// Registers, then signs in with the same credentials: one action for the
/// person.
async fn create_account(auth: &AuthService, username: String, password: String) -> Created {
    let registered = auth
        .register(RegisterRequest {
            username: username.clone(),
            password: password.clone(),
        })
        .await;
    if let Err(error) = registered {
        return Created::NotRegistered(failure("registration", &error));
    }
    match sign_in(auth, username, password, None).await {
        Ok(()) => Created::SignedIn,
        Err(failure) => Created::NotSignedIn(failure),
    }
}

/// Sorts an account step's error for the screen, and logs its category.
fn failure(step: &str, error: &ClientError) -> Failure {
    log::info!("{step} failed: {error}");
    match AuthFailure::classify(error) {
        Some(AuthFailure::ServerAccessRequired) => Failure::AccessRequired,
        Some(AuthFailure::InvalidCredentials) => Failure::WrongCredentials,
        Some(AuthFailure::Banned) => Failure::Banned,
        Some(AuthFailure::UsernameTaken) => Failure::NameTaken,
        Some(AuthFailure::RateLimited) => Failure::RateLimited,
        Some(AuthFailure::NoResponse) => Failure::NoResponse,
        _ => unclassified(error),
    }
}

/// An error that is not the server's answer to the credentials.
fn unclassified(error: &ClientError) -> Failure {
    match error {
        // Signing in again as someone other than the expired account.
        ClientError::Identity => Failure::WrongCredentials,
        ClientError::Secret(_) => Failure::Keyring,
        _ => Failure::Other,
    }
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

/// The wall clock, for the SDK's saved-at times.
fn now() -> UnixMicros {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    UnixMicros(i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use papo_sdk::secret::SecretError;

    use super::*;

    #[test]
    fn signing_in_again_as_someone_else_is_wrong_credentials() {
        assert_eq!(
            failure("test", &ClientError::Identity),
            Failure::WrongCredentials
        );
    }

    #[test]
    fn a_keyring_that_refuses_is_told_apart_from_other_failures() {
        let refused = ClientError::Secret(SecretError::UnknownAccount);
        assert_eq!(failure("test", &refused), Failure::Keyring);
        assert_eq!(failure("test", &ClientError::Closed), Failure::Other);
    }

    #[test]
    fn a_timeout_is_no_response() {
        assert_eq!(failure("test", &ClientError::Timeout), Failure::NoResponse);
    }
}
