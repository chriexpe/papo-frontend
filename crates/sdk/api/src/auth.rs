//! Owns cookie authentication calls; session refresh and credential policy belong to the SDK.

use crate::{
    Error, RestClient, Uuid, routes,
    wire::{
        ChangePasswordRequest, ConnectedDevicesResponse, DropConnectionRequest,
        DropConnectionResponse, GenericResponse, LoginRequest, LoginResponse, LoginServerRequest,
        PasswordResetConsumeRequest, RefreshResponse, RegisterRequest, RegisterResponse,
        WhoamiResponse,
    },
};

impl RestClient {
    /// Registers an account; this does not create a user session.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn register(&self, request: &RegisterRequest) -> Result<RegisterResponse, Error> {
        self.post(&routes::POST_AUTH_REGISTER, &[], request).await
    }
    /// Signs in and accepts the session cookie without exposing it.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn login(&self, request: &LoginRequest) -> Result<LoginResponse, Error> {
        self.post(&routes::POST_AUTH_LOGIN, &[], request).await
    }
    /// Acquires temporary access to a private server.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn login_server(&self, request: &LoginServerRequest) -> Result<(), Error> {
        self.response(
            self.route_request(&routes::POST_AUTH_LOGIN_SERVER, &[])?
                .json(request),
        )
        .await
        .map(|_| ())
    }
    /// Reads the authenticated account identity.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn whoami(&self) -> Result<WhoamiResponse, Error> {
        self.json(self.route_request(&routes::GET_AUTH_WHOAMI, &[])?)
            .await
    }
    /// Revokes the current session and accepts cookie deletion.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn logout(&self) -> Result<(), Error> {
        self.response(self.route_request(&routes::POST_AUTH_LOGOUT, &[])?)
            .await
            .map(|_| ())
    }
    /// Explicitly rotates the current cookie; there is no automatic refresh.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn refresh(&self) -> Result<RefreshResponse, Error> {
        self.json(self.route_request(&routes::POST_AUTH_REFRESH, &[])?)
            .await
    }
    /// Lists active connections.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn connected_devices(&self) -> Result<ConnectedDevicesResponse, Error> {
        self.json(self.route_request(&routes::GET_AUTH_CONNECTED_DEVICES, &[])?)
            .await
    }
    /// Revokes one connection or all connections as explicitly requested.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn drop_connection(
        &self,
        request: &DropConnectionRequest,
    ) -> Result<DropConnectionResponse, Error> {
        self.post(&routes::POST_AUTH_DROP_CONNECTION, &[], request)
            .await
    }
    /// Consumes a password reset on this server only.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn password_reset(
        &self,
        request: &PasswordResetConsumeRequest,
    ) -> Result<GenericResponse, Error> {
        self.post(&routes::POST_AUTH_PASSWORD_RESET, &[], request)
            .await
    }

    /// Changes the authenticated user's password when the backend requires a reset.
    ///
    /// The backend rejects changes for other users and may return conflict if the
    /// account is not marked for password reset. No automatic retry is performed.
    /// # Errors
    /// Returns transport, backend rejection or response decoding failures.
    pub async fn change_password(
        &self,
        id: Uuid,
        request: &ChangePasswordRequest,
    ) -> Result<GenericResponse, Error> {
        self.json(
            self.route_request(&routes::PUT_USERS_USER_ID_PASSWORD, &[id.to_string()])?
                .json(request),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies password wire field names without network or secrets.

    use crate::wire::{ChangePasswordRequest, GenericResponse};
    use serde_json::json;

    #[test]
    fn password_request_and_response_use_backend_field_names() {
        let request = ChangePasswordRequest {
            password: String::new(),
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({"password": ""})
        );
        let response: GenericResponse = serde_json::from_value(json!({"response": "ok"})).unwrap();
        assert_eq!(response.response.as_deref(), Some("ok"));
    }
}
