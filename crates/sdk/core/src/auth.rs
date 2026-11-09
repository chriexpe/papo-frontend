//! Account authentication policy; typed wire calls belong to papo-api and secrets to `SecretStore`.

mod account;
mod restore;
mod sign_in;
pub(crate) use restore::restore;

use crate::{ClientError, ClientHandle, Generated, ServerAddress, Session, generation::Epoch};
use std::{future::Future, sync::Arc};
use tokio::sync::{Mutex, watch};

/// Authentication state, independent of socket connectivity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthState {
    /// A private server requires its shared password before user authentication.
    NeedsServerPassword,
    /// No usable signed-in credentials are available.
    SignedOut,
    /// An explicit authentication operation is in flight.
    SigningIn,
    /// Credentials exist but the server is unavailable for identity verification.
    NeedsServer,
    /// The account identity has been verified against the server.
    Ready,
    /// The server rejected existing credentials; explicit login is required.
    Expired,
}

/// Typed authentication operations. Server bootstrap login returns an account-scoped Session.
/// Use `Session::auth` for authenticated operations; bootstrap handles never represent an account.
/// Clones serialize explicit auth mutations, not unrelated account operations.
#[derive(Clone)]
pub struct AuthService {
    pub(crate) client: ClientHandle,
    pub(crate) address: ServerAddress,
    pub(crate) session: Option<Session>,
    pub(crate) state: Arc<crate::observable::StateCell<AuthState>>,
    temporary: Arc<Mutex<Option<papo_api::RestClient>>>,
    serial: Arc<Mutex<()>>,
}
impl AuthService {
    pub(crate) fn for_server(client: ClientHandle, address: ServerAddress) -> Self {
        let state = crate::observable::StateCell::new(AuthState::SignedOut);
        client.inner.pool.register_auth(address.id(), &state);
        Self {
            client,
            address,
            session: None,
            state,
            temporary: Arc::new(Mutex::new(None)),
            serial: Arc::new(Mutex::new(())),
        }
    }
    pub(crate) fn for_session(session: Session) -> Self {
        Self {
            client: session.client.clone(),
            address: session.inner.address.clone(),
            state: session.inner.auth.clone(),
            session: Some(session),
            temporary: Arc::new(Mutex::new(None)),
            serial: Arc::new(Mutex::new(())),
        }
    }
    /// Subscribes to typed authentication state; intermediate states may coalesce.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<AuthState> {
        self.state.subscribe()
    }
    /// Reads the latest authentication state.
    #[must_use]
    pub fn state(&self) -> AuthState {
        *self.state.borrow()
    }

    pub(crate) async fn run<T, F, Fut>(&self, operation: F) -> Result<T, ClientError>
    where
        T: Send + 'static,
        F: FnOnce(Self, Option<Epoch>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, ClientError>> + Send + 'static,
    {
        let this = self.clone();
        let session = this.session.clone();
        let token = session.as_ref().map(|session| session.inner.tasks.token());
        self.client
            .dispatch(async move {
                let _token = token;
                this.validate(None)?;
                let _serial = this.serial.lock().await;
                if let Some(session) = session {
                    let _account_serial = session.inner.auth_serial.lock().await;
                    let epoch = session.inner.epochs.current();
                    session.inner.epochs.validate(epoch.generation)?;
                    let result = tokio::select! {
                        biased;
                        () = session.inner.root.cancelled() => Err(ClientError::Closed),
                        () = epoch.cancel.cancelled() => Err(ClientError::Superseded),
                        result = operation(this.clone(), Some(epoch.clone())) => result,
                    };
                    if let Err(error) = &result
                        && !matches!(error, ClientError::Closed | ClientError::Superseded)
                    {
                        this.publish(Some(&epoch), Self::failure_state(error, true))?;
                    }
                    result
                } else {
                    operation(this.clone(), None).await
                }
            })
            .await
    }
    pub(crate) fn validate(&self, epoch: Option<&Epoch>) -> Result<(), ClientError> {
        if let (Some(session), Some(epoch)) = (&self.session, epoch) {
            session.inner.epochs.validate(epoch.generation)?;
        }
        if self.state.is_closed() || self.client.is_closed() {
            return Err(ClientError::Closed);
        }
        Ok(())
    }
    pub(crate) fn publish(
        &self,
        epoch: Option<&Epoch>,
        state: AuthState,
    ) -> Result<(), ClientError> {
        if let (Some(session), Some(epoch)) = (&self.session, epoch) {
            session
                .inner
                .epochs
                .apply(epoch.generation, || self.state.send_replace(state))?;
        } else {
            self.validate(epoch)?;
            self.state.send_replace(state);
        }
        Ok(())
    }
    pub(crate) fn failure_state(error: &ClientError, signed_in: bool) -> AuthState {
        if let ClientError::Api(papo_api::Error::Problem { problem, .. }) = error
            && problem.kind() == papo_api::ProblemKind::ServerAccessRequired
        {
            return AuthState::NeedsServerPassword;
        }
        match error {
            ClientError::Api(
                papo_api::Error::Transport
                | papo_api::Error::Decode
                | papo_api::Error::Http {
                    status: 429 | 500..=599,
                }
                | papo_api::Error::Problem {
                    status: 429 | 500..=599,
                    ..
                },
            )
            | ClientError::Gateway(_)
            | ClientError::Timeout
            | ClientError::Secret(_) => AuthState::NeedsServer,
            ClientError::SignedOut => AuthState::SignedOut,
            ClientError::Api(
                papo_api::Error::Http { status: 401 }
                | papo_api::Error::Problem { status: 401, .. }
                | papo_api::Error::Credentials,
            )
            | ClientError::Identity
            | ClientError::InvalidAuth => {
                if signed_in {
                    AuthState::Expired
                } else {
                    AuthState::SignedOut
                }
            }
            _ => {
                if signed_in {
                    AuthState::Ready
                } else {
                    AuthState::SignedOut
                }
            }
        }
    }
    pub(crate) async fn transport(
        &self,
        epoch: Option<&Epoch>,
    ) -> Result<papo_api::RestClient, ClientError> {
        if let Some(session) = &self.session {
            let secret = self
                .client
                .inner
                .secrets
                .read(session.inner.key, &session.inner.root)
                .await?
                .ok_or(ClientError::SignedOut)?;
            self.validate(epoch)?;
            self.client
                .inner
                .transport(&self.address, Some(secret.expose()))
        } else {
            let mut temporary = self.temporary.lock().await;
            if let Some(rest) = temporary.take() {
                Ok(rest)
            } else {
                self.client.inner.transport(&self.address, None)
            }
        }
    }
    pub(crate) fn generated<T>(
        &self,
        epoch: &Epoch,
        value: T,
    ) -> Result<Generated<T>, ClientError> {
        self.validate(Some(epoch))?;
        Ok(Generated {
            generation: epoch.generation,
            value,
        })
    }
}
