//! Carries out the flow's effects: the SDK calls behind each step, run on the
//! client's runtime through the bridge, and their answers stepped back into
//! the flow on the UI thread.
//!
//! Owns turning the SDK's results into the flow's events
//! ([`Probe`](super::Probe), [`Failure`], [`Created`]); the server step's
//! calls are in `server`. What the answers mean for the screen is
//! `SignIn::step`'s; what the shell does around the flow is the shell's.

mod server;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use papo_sdk::{
    AccountId, AuthFailure, AuthService, ClientError, ClientHandle, LoginRequest,
    LoginServerRequest, RegisterRequest, ServerId, UnixMicros,
};
use rune::machine::Effects;

pub(crate) use server::remove_left_server;

use super::{Created, Discovery, Effect, Event, Failure, Renewing, Screen, SignIn};
use crate::account_watch::Credentials;
use crate::sdk_bridge::Sdk;
use crate::shell::Shell;

/// Carries out `effects` in order.
pub(crate) fn run(shell: &mut Shell, effects: Effects<Effect>) {
    for effect in effects {
        match effect {
            Effect::LoadIcon(bytes) => shell.load_server_icon(bytes),
            Effect::Finish { id, name } => shell.finish_sign_in(id, name),
            Effect::OpenServer(id) => shell.open_server(id),
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
                    server::probe(client, address, &abandoned).await
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
            with_auth(
                shell,
                &sdk,
                Event::Unlocked,
                Err,
                move |auth, _| async move {
                    auth.server_login(LoginServerRequest {
                        server_password: password,
                    })
                    .await
                    .map_err(|error| failure("unlock", &error))
                },
            );
        }
        Effect::SignIn { username, password } => {
            let expected = renewed_account(shell);
            let adding = is_adding(shell);
            with_auth(
                shell,
                &sdk,
                Event::SignedIn,
                Err,
                move |auth, client| async move {
                    let account = sign_in(&auth, username, password, expected).await?;
                    if adding {
                        make_active(&client, account).await;
                    }
                    Ok(())
                },
            );
        }
        Effect::CreateAccount { username, password } => {
            with_auth(
                shell,
                &sdk,
                Event::Created,
                Created::NotRegistered,
                move |auth, _| async move { create_account(&auth, username, password).await },
            );
        }
        Effect::RemoveServer(id) => remove_left_server(shell, id),
        Effect::LoadIcon(_) | Effect::Finish { .. } | Effect::OpenServer(_) => {}
    }
}

/// Whether the flow adds an account to a saved server.
fn is_adding(shell: &Shell) -> bool {
    shell.sign_in().is_some_and(
        |flow| matches!(flow.screen(), Screen::Account { server, .. } if server.adding),
    )
}

/// Makes `account` its server's active one: Add account signing in an
/// account already saved there, which the SDK leaves as it was. A failure
/// keeps the account that was active, and the sign-in still stands.
async fn make_active(client: &ClientHandle, account: AccountId) {
    if let Err(error) = client
        .set_active_account(account.server(), Some(account))
        .await
    {
        log::warn!("the signed-in account was not made active: {error}");
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
    work: impl FnOnce(AuthService, ClientHandle) -> Work + Send + 'static,
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
            let answer = work(auth.clone(), client).await;
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

/// Signs in, returning the account; when `expected` names the account
/// signing in again, another person's credentials are wrong credentials here.
async fn sign_in(
    auth: &AuthService,
    username: String,
    password: String,
    expected: Option<AccountId>,
) -> Result<AccountId, Failure> {
    let session = auth
        .login(LoginRequest { username, password }, now())
        .await
        .map_err(|error| failure("sign-in", &error))?;
    if expected.is_some_and(|account| account != session.account()) {
        log::info!("sign-in again was for another account");
        return Err(Failure::WrongCredentials);
    }
    Ok(session.account())
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
        Ok(_) => Created::SignedIn,
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
