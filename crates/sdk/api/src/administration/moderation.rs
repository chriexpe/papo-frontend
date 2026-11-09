//! Owns moderation and administrative password-reset calls; the backend decides authorization.

use crate::{
    Error, RestClient, Uuid,
    wire::{BanUserRequest, GenericResponse, PasswordResetLinkResponse},
};
use reqwest::Method;

/// The self-reset acknowledgment or the owner-generated reset link.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum PasswordResetResult {
    /// The owner received a time-limited link for another account.
    Link(PasswordResetLinkResponse),
    /// Reset was prepared for the requesting account.
    Prepared(GenericResponse),
}

impl RestClient {
    /// Changes a user's ban state; the path ID is authoritative.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn ban_user(
        &self,
        user: Uuid,
        request: &BanUserRequest,
    ) -> Result<GenericResponse, Error> {
        self.json(
            self.request(Method::PUT, &format!("users/{user}/ban"))?
                .json(request),
        )
        .await
    }

    /// Prepares a self-reset or generates an owner-authorized reset link.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn reset_user_password(&self, user: Uuid) -> Result<PasswordResetResult, Error> {
        self.json(self.request(Method::POST, &format!("users/{user}/reset"))?)
            .await
    }
}

#[cfg(test)]
mod tests {
    //! Tests reset response decoding without exercising transport.

    use super::PasswordResetResult;

    #[test]
    fn reset_link_is_not_confused_with_self_reset() {
        let result: PasswordResetResult = serde_json::from_str(
            r#"{"reset_url":"https://example.invalid/reset","expires_at":"2030-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert!(matches!(result, PasswordResetResult::Link(_)));
    }
}
