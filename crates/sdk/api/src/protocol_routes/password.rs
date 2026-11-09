//! Owns the password-change route; credential storage and retry policy belong to callers.

use crate::{
    Error, RestClient, Uuid,
    wire::{ChangePasswordRequest, GenericResponse},
};
use reqwest::Method;

fn password_path(id: Uuid) -> String {
    format!("users/{id}/password")
}

impl RestClient {
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
        self.json(self.request(Method::PUT, &password_path(id))?.json(request))
            .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies the password route and generated wire shapes without network or secrets.

    use super::*;
    use serde_json::json;

    #[test]
    fn password_path_preserves_uuid() {
        let id = Uuid::nil();
        assert_eq!(password_path(id), format!("users/{id}/password"));
    }

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
