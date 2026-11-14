//! Owns acknowledged self-profile edits and administrative reset preparation; authentication owns credentials.

pub use papo_api::wire::GenericResponse;
use papo_api::{Uuid, administration::PasswordResetResult};

pub use papo_api::wire::{
    UpdateAvatarRequest, UpdateBannerRequest, UpdateStatusRequest, UpdateUserRequest,
};

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::AccountId;

/// A reset preparation acknowledgment; links are explicit-copy secrets, never automatically sent.
#[non_exhaustive]
pub enum PasswordResetPreparation {
    /// The requesting account may now use `AuthService::change_password`.
    Prepared,
    /// An owner-authorized, expiring single-use link for the specified target.
    Link(PasswordResetLink),
}

/// An owned backend reset link bound to its originating account and target.
///
/// This type deliberately has no serialization or `Debug` implementation. Hosts must show the
/// originating server identity and request explicit confirmation before navigating or copying.
/// Consuming the link uses the selected account's authentication service, never a URL-selected server.
pub struct PasswordResetLink {
    account: AccountId,
    target: Uuid,
    expires_at: String,
    url: String,
}

impl PasswordResetLink {
    /// The verified account/server that issued this link.
    #[must_use]
    pub fn account(&self) -> AccountId {
        self.account
    }

    /// The intended backend user, never a webhook external author.
    #[must_use]
    pub fn target(&self) -> Uuid {
        self.target
    }

    /// The verbatim backend expiry timestamp.
    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }

    /// Explicitly exposes the secret-bearing link for an approved copy/navigation action.
    #[must_use]
    pub fn expose_url(&self) -> &str {
        &self.url
    }
}

impl Operations {
    /// Saves this account's nickname, status message, bio and optional typing phrase.
    /// Backend length and authorization rules apply; no local optimistic profile is published.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch; never retries.
    pub async fn update_profile(
        &self,
        request: UpdateUserRequest,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let user = self.session.account().user();
        self.mutate(
            move |rest| async move { rest.update_profile(user, &request).await },
            Refresh::Profile(user),
        )
        .await
    }

    /// Sets persisted presence to backend `away`, `busy`, or absent for automatic presence.
    /// Live presence remains ephemeral and scoped to the connection generation.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch; never retries.
    pub async fn update_status(
        &self,
        request: UpdateStatusRequest,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let user = self.session.account().user();
        self.mutate(
            move |rest| async move { rest.update_status(user, &request).await },
            Refresh::Profile(user),
        )
        .await
    }

    /// Uploads an already-encoded avatar. Backend format and two-MiB limits apply.
    /// Cropping, decoding and previewing remain client responsibilities.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain upload; never retries.
    pub async fn update_avatar(
        &self,
        request: UpdateAvatarRequest,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let user = self.session.account().user();
        self.mutate(
            move |rest| async move { rest.update_avatar(user, &request).await },
            Refresh::Profile(user),
        )
        .await
    }

    /// Uploads an encoded banner, or removes it using the backend's empty banner fields.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain upload; never retries.
    pub async fn update_banner(
        &self,
        request: UpdateBannerRequest,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let user = self.session.account().user();
        self.mutate(
            move |rest| async move { rest.update_banner(user, &request).await },
            Refresh::Profile(user),
        )
        .await
    }

    /// Explicitly prepares a self-reset or asks the owner-only backend endpoint for a reset link.
    /// A failed refresh remains a warning on an acknowledged receipt. No link is persisted or sent.
    /// Existing `AuthService` owns password change, reset consumption and connected-session revocation.
    /// # Errors
    /// Returns typed rejection, stale generation, or uncertain dispatch; never retries.
    pub async fn prepare_password_reset(
        &self,
        target: Uuid,
    ) -> Result<OperationReceipt<PasswordResetPreparation>, OperationError> {
        let account = self.session.account();
        self.mutate(
            move |rest| async move {
                rest.reset_user_password(target)
                    .await
                    .map(|result| match result {
                        PasswordResetResult::Prepared(_) => PasswordResetPreparation::Prepared,
                        PasswordResetResult::Link(link) => {
                            PasswordResetPreparation::Link(PasswordResetLink {
                                account,
                                target,
                                expires_at: link.expires_at,
                                url: link.reset_url,
                            })
                        }
                    })
            },
            Refresh::None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies backend request semantics without a network or real clock.

    use super::*;

    #[test]
    fn clearing_persisted_presence_omits_the_optional_status() {
        assert_eq!(
            serde_json::to_value(UpdateStatusRequest::default()).unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn profile_status_message_is_distinct_from_persisted_presence() {
        let request = UpdateUserRequest {
            nickname: String::new(),
            status: "custom message".to_owned(),
            description: String::new(),
            typing: None,
        };
        let encoded = serde_json::to_value(request).unwrap();
        assert_eq!(encoded["status"], "custom message");
        assert!(encoded.get("typing").is_none());
    }
}
