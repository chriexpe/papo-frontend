//! Sign-in attempts and verified account replacement; storage alone allocates credential generations.

use crate::{
    AccountId, AuthService, AuthState, ClientError, Generation, Session, UnixMicros,
    secret::{SecretKey, SecretValue},
    session::SessionInner,
};
use papo_api::{
    RestClient,
    wire::{
        GenericResponse, LoginRequest, LoginServerRequest, PasswordResetConsumeRequest,
        RegisterRequest, RegisterResponse, WhoamiResponse,
    },
};
use std::sync::Arc;

impl AuthService {
    /// Registers through the typed backend route; registration does not sign in.
    /// # Errors
    /// Returns closed, transport, backend, timeout or decoding failures.
    pub async fn register(
        &self,
        request: RegisterRequest,
    ) -> Result<RegisterResponse, ClientError> {
        self.run(move |this, epoch| async move {
            let rest = this.transport(epoch.as_ref()).await?;
            let result = this.client.inner.request(rest.register(&request)).await;
            this.validate(epoch.as_ref())?;
            *this.temporary.lock().await = Some(rest);
            if let Err(error) = &result {
                this.publish(
                    epoch.as_ref(),
                    Self::failure_state(error, this.session.is_some()),
                )?;
            }
            result
        })
        .await
    }
    /// Acquires private-server access before login; passwords are never written to SDK files.
    /// # Errors
    /// Returns typed backend, generation or credential persistence failures.
    pub async fn server_login(&self, request: LoginServerRequest) -> Result<(), ClientError> {
        self.run(move |this, epoch| async move {
            let rest = this.transport(epoch.as_ref()).await?;
            let result = this.client.inner.request(rest.login_server(&request)).await;
            this.validate(epoch.as_ref())?;
            if let Err(error) = result {
                this.publish(
                    epoch.as_ref(),
                    Self::failure_state(&error, this.session.is_some()),
                )?;
                return Err(error);
            }
            if let Some(session) = &this.session {
                let snapshot = rest.credential_snapshot()?;
                this.client
                    .inner
                    .secrets
                    .write(
                        session.inner.key,
                        SecretValue::new(snapshot.expose_bytes().to_vec()),
                        &session.inner.root,
                    )
                    .await?;
            } else {
                *this.temporary.lock().await = Some(rest);
            }
            this.publish(epoch.as_ref(), AuthState::SignedOut)
        })
        .await
    }
    /// Signs in, verifies whoami, persists credentials, and replaces only that account's session.
    /// Every earlier handle to a replaced account is revoked before the new one is returned.
    /// # Errors
    /// Returns typed backend failures, identity mismatch, storage or credential uncertainty.
    pub async fn login(
        &self,
        request: LoginRequest,
        created_at: UnixMicros,
    ) -> Result<Session, ClientError> {
        let previous = self.state.send_replace(AuthState::SigningIn);
        let _attempt = SignInState {
            state: self.state.clone(),
            previous,
        };
        let admitted = self.client.inner.pool.admit_login();
        let verified = self
            .run(move |this, epoch| async move {
                let rest = this.transport(epoch.as_ref()).await?;
                let result = async {
                    this.client.inner.request(rest.login(&request)).await?;
                    let identity = this.client.inner.request(rest.whoami()).await?;
                    this.validate(epoch.as_ref())?;
                    let user = identity
                        .id
                        .as_deref()
                        .and_then(|id| id.parse().ok())
                        .ok_or(ClientError::InvalidAuth)?;
                    let account = AccountId::new(this.address.id(), user);
                    if this
                        .session
                        .as_ref()
                        .is_some_and(|session| session.account() != account)
                    {
                        return Err(ClientError::Identity);
                    }
                    Ok((identity, account))
                }
                .await;
                match result {
                    Ok((identity, account)) => Ok(VerifiedLogin {
                        rest,
                        identity,
                        account,
                        generation: epoch.map(|epoch| epoch.generation),
                        admitted,
                    }),
                    Err(error) => {
                        this.publish(
                            epoch.as_ref(),
                            Self::failure_state(&error, this.session.is_some()),
                        )?;
                        *this.temporary.lock().await = Some(rest);
                        Err(error)
                    }
                }
            })
            .await?;
        let this = self.clone();
        let client = self.client.clone();
        self.client
            .dispatch(async move { install(&client, &this, verified, created_at).await })
            .await
    }
    /// Consumes an explicitly supplied reset link through the typed API.
    /// # Errors
    /// Returns backend reset-link rejection, timeout, closed or stale-generation failures.
    pub async fn password_reset(
        &self,
        request: PasswordResetConsumeRequest,
    ) -> Result<GenericResponse, ClientError> {
        self.run(move |this, epoch| async move {
            let rest = this.transport(epoch.as_ref()).await?;
            let result = this
                .client
                .inner
                .request(rest.password_reset(&request))
                .await;
            this.validate(epoch.as_ref())?;
            *this.temporary.lock().await = Some(rest);
            result
        })
        .await
    }
}

struct VerifiedLogin {
    rest: RestClient,
    identity: WhoamiResponse,
    account: AccountId,
    generation: Option<Generation>,
    admitted: u64,
}

async fn install(
    client: &crate::ClientHandle,
    auth: &AuthService,
    verified: VerifiedLogin,
    created_at: UnixMicros,
) -> Result<Session, ClientError> {
    let guard = client.inner.mutation.lock().await;
    if let (Some(session), Some(generation)) = (&auth.session, verified.generation) {
        session.inner.epochs.validate(generation)?;
    }
    if !client
        .inner
        .pool
        .login_is_current(verified.account, verified.admitted)
    {
        return Err(ClientError::Superseded);
    }
    // Recheck visibility under the same serialization as removal; a late login cannot resurrect it.
    if !client
        .inner
        .registry
        .servers()
        .await?
        .iter()
        .any(|server| server.address.id() == verified.account.server())
    {
        return Err(ClientError::NotFound);
    }
    let saved = client
        .inner
        .registry
        .accounts(verified.account.server())
        .await?
        .into_iter()
        .find(|account| account.id == verified.account);
    let generation = if let Some(saved) = saved {
        saved.credential_generation
    } else {
        client
            .inner
            .registry
            .add_account(
                verified.account,
                verified.identity.nickname.or(verified.identity.username),
                None,
                true,
                created_at,
            )
            .await?
    };
    client.inner.pool.stop(verified.account).await;
    client
        .inner
        .secrets
        .activate(verified.account, generation)?;
    let inner = SessionInner::new(
        verified.account,
        auth.address.clone(),
        SecretKey::new(verified.account, generation),
        Arc::clone(&client.inner.pool.next),
        client.inner.pool.root.child_token(),
        AuthState::SigningIn,
    );
    let epoch = inner.epochs.current();
    let task = inner.tasks.token();
    client.inner.pool.insert(Arc::clone(&inner)).await;
    client.inner.publish_registry().await?;
    drop(guard);
    let snapshot = verified.rest.credential_snapshot()?;
    let result = client
        .inner
        .secrets
        .write(
            inner.key,
            SecretValue::new(snapshot.expose_bytes().to_vec()),
            &epoch.cancel,
        )
        .await;
    drop(task);
    if let Err(error) = result {
        inner.auth.send_replace(AuthState::Expired);
        return Err(error.into());
    }
    inner.epochs.apply(epoch.generation, || {
        inner.auth.send_replace(AuthState::Ready)
    })?;
    auth.state.send_replace(AuthState::Ready);
    crate::connection_driver::start(client.clone(), Arc::clone(&inner));
    Ok(Session {
        inner,
        client: client.clone(),
    })
}

/// A canceled/failed attempt cannot strand observers in `SigningIn`.
struct SignInState {
    state: Arc<crate::observable::StateCell<AuthState>>,
    previous: AuthState,
}
impl Drop for SignInState {
    fn drop(&mut self) {
        self.state.send_if_modified(|state| {
            if *state != AuthState::SigningIn {
                return false;
            }
            *state = if self.previous == AuthState::SigningIn {
                AuthState::SignedOut
            } else {
                self.previous
            };
            true
        });
    }
}
