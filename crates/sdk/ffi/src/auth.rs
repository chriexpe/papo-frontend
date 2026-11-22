//! Stateful account and server-bootstrap authentication handles across FFI.
//! Preserves `AuthService`'s in-memory temporary cookie jar across `server_login` -> `register` -> `login`.

use std::sync::Arc;

use papo_sdk::{
    AuthService, ChangePasswordRequest, DropConnectionRequest, LoginRequest, LoginServerRequest,
    PasswordResetConsumeRequest, RegisterRequest, UnixMicros,
};

use crate::{
    auth_records::{
        FfiAuthState, FfiAuthStateListener, FfiConnectedDevicesResult, FfiDropConnectionResult,
        FfiRegisterResponse, map_connected_devices, map_drop_connection,
    },
    client::FfiClient,
    errors::{FfiAuthError, FfiClientError},
    ids::{FfiAccountId, parse_server_id, parse_uuid},
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription, watch_stream},
};

/// Handle bound either to server-bootstrap authentication or an active account session.
#[derive(uniffi::Object)]
pub struct FfiAuthHandle {
    /// Parent client runtime and shutdown tracker.
    client: Arc<FfiClient>,
    /// Stateful SDK auth service retaining any temporary server-login cookie.
    service: AuthService,
}

#[uniffi::export]
impl FfiAuthHandle {
    /// Current authentication state without awaiting network I/O.
    #[must_use]
    pub fn state(&self) -> FfiAuthState {
        self.service.state().into()
    }

    /// Subscribes to authentication state transitions until closure or disposal.
    ///
    /// # Errors
    /// Returns worker or shutdown failure.
    pub fn subscribe(
        &self,
        listener: Arc<dyn FfiAuthStateListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let (baseline, stream) = watch_stream(self.service.subscribe());
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.client.runtime,
            &self.client.tasks,
            &self.client.shutdown,
            self.client.callbacks.clone(),
            baseline,
            stream,
            move |state| {
                l_snap.on_snapshot(state.into());
                true
            },
            move |state| {
                l_upd.on_update(state.into());
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Submits private-server shared password; cookie persists on this handle for follow-up auth calls.
    ///
    /// # Errors
    /// Returns typed [`FfiAuthError`].
    pub async fn server_login(&self, password: String) -> Result<(), FfiAuthError> {
        let service = self.service.clone();
        self.client
            .run_scoped(
                || FfiAuthError::Client {
                    error: FfiClientError::Closed,
                },
                async move {
                    service
                        .server_login(LoginServerRequest {
                            server_password: password,
                        })
                        .await
                        .map_err(FfiAuthError::from)
                },
            )
            .await
    }

    /// Registers a new user on this handle's server without automatically signing in.
    ///
    /// # Errors
    /// Returns typed [`FfiAuthError`].
    pub async fn register(
        &self,
        username: String,
        password: String,
    ) -> Result<FfiRegisterResponse, FfiAuthError> {
        let service = self.service.clone();
        self.client
            .run_scoped(
                || FfiAuthError::Client {
                    error: FfiClientError::Closed,
                },
                async move {
                    let resp = service
                        .register(RegisterRequest { username, password })
                        .await
                        .map_err(FfiAuthError::from)?;
                    Ok(FfiRegisterResponse {
                        id: resp.id,
                        username: resp.username,
                        created_at: resp.created_at,
                    })
                },
            )
            .await
    }

    /// Signs in, verifies `whoami`, persists credentials durably, and starts the account `Session`.
    ///
    /// # Errors
    /// Returns typed [`FfiAuthError`].
    pub async fn login(
        &self,
        username: String,
        password: String,
        created_at_us: i64,
    ) -> Result<FfiAccountId, FfiAuthError> {
        let service = self.service.clone();
        self.client
            .run_scoped(
                || FfiAuthError::Client {
                    error: FfiClientError::Closed,
                },
                async move {
                    let session = service
                        .login(
                            LoginRequest { username, password },
                            UnixMicros(created_at_us),
                        )
                        .await
                        .map_err(FfiAuthError::from)?;
                    Ok(FfiAccountId::from_sdk(session.account()))
                },
            )
            .await
    }

    /// Consumes an administrator-issued password reset token + replacement password.
    ///
    /// # Errors
    /// Returns [`FfiAuthError`].
    pub async fn password_reset(
        &self,
        token: String,
        new_password: String,
    ) -> Result<Option<String>, FfiAuthError> {
        let service = self.service.clone();
        self.client
            .run_scoped(
                || FfiAuthError::Client {
                    error: FfiClientError::Closed,
                },
                async move {
                    let resp = service
                        .password_reset(PasswordResetConsumeRequest {
                            token,
                            password: new_password,
                        })
                        .await
                        .map_err(FfiAuthError::from)?;
                    Ok(resp.response)
                },
            )
            .await
    }

    /// Rotates session cookies, persists refreshed credentials, and triggers connection reload.
    ///
    /// # Errors
    /// Returns [`FfiClientError`].
    pub async fn refresh(&self) -> Result<(), FfiClientError> {
        let service = self.service.clone();
        self.client
            .run_scoped(|| FfiClientError::Closed, async move {
                service.refresh().await.map_err(FfiClientError::from)
            })
            .await
    }

    /// Logs out remotely, deletes keystore credentials, revokes handles, and updates state cleanly.
    ///
    /// # Errors
    /// Returns [`FfiClientError`], preserving uncertain keystore deletion without masking it.
    pub async fn logout(&self) -> Result<(), FfiClientError> {
        let service = self.service.clone();
        self.client
            .run_scoped(|| FfiClientError::Closed, async move {
                service.logout().await.map_err(FfiClientError::from)
            })
            .await
    }

    /// Changes this verified account's password.
    ///
    /// # Errors
    /// Returns [`FfiClientError`].
    pub async fn change_password(
        &self,
        new_password: String,
    ) -> Result<Option<String>, FfiClientError> {
        let service = self.service.clone();
        self.client
            .run_scoped(|| FfiClientError::Closed, async move {
                let resp = service
                    .change_password(ChangePasswordRequest {
                        password: new_password,
                    })
                    .await
                    .map_err(FfiClientError::from)?;
                Ok(resp.value.response)
            })
            .await
    }

    /// Fetches connected devices with their request generation.
    ///
    /// # Errors
    /// Returns [`FfiClientError`].
    pub async fn connected_devices(&self) -> Result<FfiConnectedDevicesResult, FfiClientError> {
        let service = self.service.clone();
        self.client
            .run_scoped(|| FfiClientError::Closed, async move {
                let generated = service
                    .connected_devices()
                    .await
                    .map_err(FfiClientError::from)?;
                Ok(map_connected_devices(
                    generated.generation.get(),
                    generated.value,
                ))
            })
            .await
    }

    /// Drops one remote device connection by UUID.
    ///
    /// # Errors
    /// Returns [`FfiClientError::InvalidArgument`] or backend/lifecycle errors.
    pub async fn drop_connection(
        &self,
        connection_id: String,
    ) -> Result<FfiDropConnectionResult, FfiClientError> {
        let connection_id = parse_uuid(&connection_id)?.to_string();
        let service = self.service.clone();
        self.client
            .run_scoped(|| FfiClientError::Closed, async move {
                let generated = service
                    .drop_connection(DropConnectionRequest { connection_id })
                    .await
                    .map_err(FfiClientError::from)?;
                Ok(map_drop_connection(
                    generated.generation.get(),
                    generated.value,
                ))
            })
            .await
    }
}

#[uniffi::export]
impl FfiClient {
    /// Creates a server-scoped bootstrap `FfiAuthHandle` for `server_login`, `register`, and `login`.
    ///
    /// # Errors
    /// Returns [`FfiClientError::InvalidArgument`] or missing server errors.
    pub fn server_auth(
        self: &Arc<Self>,
        server_id: String,
    ) -> Result<Arc<FfiAuthHandle>, FfiClientError> {
        if self.is_closed() {
            return Err(FfiClientError::Closed);
        }
        let server = parse_server_id(&server_id)?;
        let service = self.handle.auth(server)?;
        Ok(Arc::new(FfiAuthHandle {
            client: Arc::clone(self),
            service,
        }))
    }

    /// Binds an account-scoped `FfiAuthHandle` to an existing signed-in session.
    ///
    /// # Errors
    /// Returns invalid argument, missing session, or closure errors.
    pub fn account_auth(
        self: &Arc<Self>,
        account: FfiAccountId,
    ) -> Result<Arc<FfiAuthHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        Ok(Arc::new(FfiAuthHandle {
            client: Arc::clone(self),
            service: session.auth(),
        }))
    }
}
