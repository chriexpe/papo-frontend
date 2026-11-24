//! Owns acknowledged self-profile edits and administrative reset preparation; authentication owns credentials.
//!
//! The backend replaces nickname, status message and description together, so a change of
//! one field (`change_profile`) is merged over the profile as the server holds it now.

pub use papo_api::wire::GenericResponse;
use papo_api::{NullableText, Uuid, administration::PasswordResetResult};

pub use papo_api::wire::{
    UpdateAvatarRequest, UpdateBannerRequest, UpdateStatusRequest, UpdateUserRequest,
};

use super::{OperationError, OperationReceipt, Operations, Refresh};
use crate::{
    AccountId, CropRect, FixedImage, OutgoingError, StagedAttachment, images::FixedUpload,
};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

/// Fields of this account's profile to change; a field left `None` keeps its saved value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileChange {
    /// The nickname; empty clears it.
    pub nickname: Option<String>,
    /// The profile's status message; empty clears it.
    pub status_message: Option<String>,
    /// The "about" text; empty clears it.
    pub description: Option<String>,
    /// The typing phrase; empty clears it.
    pub typing: Option<String>,
}

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

    /// Saves only the fields `change` sets. The backend replaces nickname, status message and
    /// description together, so the profile is read back first and the fields left unset are
    /// sent as the server holds them; the typing phrase is sent only when it changes, as the
    /// backend keeps it when absent. A saved nickname or status message the read-back does not
    /// state is never guessed: the change is refused instead. Changes of one account run one
    /// at a time, each merged over what the one before it saved.
    /// Backend length and authorization rules apply; no local optimistic profile is published.
    /// # Errors
    /// Returns the read-back's failure, `InvalidData` when it is incomplete, or the save's
    /// typed rejection, stale generation or uncertain dispatch; never retries.
    pub async fn change_profile(
        &self,
        change: ProfileChange,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let me = self.session.account().user();
        // A second change waits for this one to be saved, then reads it back.
        let _profile = self.session.inner.profile_serial.lock().await;
        let read = self.session.profiles(&[me]).await?;
        if self
            .expected_generation
            .is_some_and(|expected| expected != read.generation)
        {
            return Err(crate::ClientError::Superseded.into());
        }
        let saved = read
            .value
            .into_iter()
            .find(|profile| profile.id.as_deref().and_then(|id| id.parse().ok()) == Some(me))
            .ok_or(OperationError::NotFound)?;
        self.update_profile(merged(change, saved)?).await
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

    /// Uploads an already-encoded avatar. Backend format, 512-pixel and two-MiB limits apply, and
    /// the bytes go as given. Prefer [`Self::update_avatar_from_image`], which crops, scales and
    /// encodes through the SDK's one image path; this stays for a client that encodes itself.
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

    /// Turns, crops, scales (512 × 512, covering) and uploads a staged picture as the avatar.
    ///
    /// The picture is re-encoded from pixels, so nothing of the original file but its colour
    /// profile is sent: JPEG when opaque, PNG with transparency, under the backend's two MiB.
    /// `quarter_turns` are clockwise; `crop` is in the picture after orientation and turns.
    /// There is no removal: the backend has none for avatars.
    /// # Errors
    /// Returns [`OperationError::Image`] when the picture cannot be prepared (nothing is sent),
    /// otherwise as [`Self::update_avatar`].
    pub async fn update_avatar_from_image(
        &self,
        source: &StagedAttachment,
        quarter_turns: u8,
        crop: Option<CropRect>,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let upload = self
            .fixed_image(source, FixedImage::Avatar, quarter_turns, crop)
            .await?;
        self.update_avatar(UpdateAvatarRequest {
            avatar: BASE64.encode(&upload.bytes),
            avatar_format: upload.format.label().to_owned(),
        })
        .await
    }

    /// Turns, crops, scales (1500 × 500, covering) and uploads a staged picture as the banner.
    /// Re-encoding and limits are those of [`Self::update_avatar_from_image`]; the backend caps
    /// either side at 2048 pixels and states no ratio, so 3 : 1 is the client's choice.
    /// # Errors
    /// As [`Self::update_avatar_from_image`].
    pub async fn update_banner_from_image(
        &self,
        source: &StagedAttachment,
        quarter_turns: u8,
        crop: Option<CropRect>,
    ) -> Result<OperationReceipt<GenericResponse>, OperationError> {
        let upload = self
            .fixed_image(source, FixedImage::Banner, quarter_turns, crop)
            .await?;
        self.update_banner(UpdateBannerRequest {
            banner: BASE64.encode(&upload.bytes),
            banner_format: upload.format.label().to_owned(),
        })
        .await
    }

    async fn fixed_image(
        &self,
        source: &StagedAttachment,
        target: FixedImage,
        quarter_turns: u8,
        crop: Option<CropRect>,
    ) -> Result<FixedUpload, OperationError> {
        self.session
            .client
            .inner
            .registry
            .outgoing_fixed_image(&self.session, source, target, quarter_turns, crop)
            .await
            .map_err(|error| match error {
                OutgoingError::Image(error) => OperationError::Image(error),
                OutgoingError::Session(error) => OperationError::Session(error),
                OutgoingError::NotFound => OperationError::NotFound,
                _ => OperationError::InvalidInput,
            })
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

/// `change` over `saved`: the request that changes only what `change` sets.
fn merged(
    change: ProfileChange,
    saved: papo_api::wire::UserProfile,
) -> Result<UpdateUserRequest, OperationError> {
    let kept = |field: NullableText| match field {
        NullableText::Value(text) => Ok(text),
        NullableText::Null => Ok(String::new()),
        // A field the read-back left out is unknown, and sending a guess would erase it.
        NullableText::Missing => Err(OperationError::InvalidData),
    };
    Ok(UpdateUserRequest {
        nickname: match change.nickname {
            Some(nickname) => nickname,
            None => kept(saved.nickname)?,
        },
        status: match change.status_message {
            Some(message) => message,
            None => kept(saved.status_message)?,
        },
        description: change
            .description
            .unwrap_or_else(|| saved.description.unwrap_or_default()),
        typing: change.typing,
    })
}

#[cfg(test)]
mod tests {
    //! Verifies backend request semantics without a network or real clock.

    use super::*;

    fn saved() -> papo_api::wire::UserProfile {
        papo_api::wire::UserProfile {
            nickname: NullableText::Value("Lia".into()),
            status_message: NullableText::Value("on call".into()),
            description: Some("about".into()),
            typing: NullableText::Value("is writing…".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_change_of_one_field_keeps_the_others_as_saved() {
        let request = merged(
            ProfileChange {
                nickname: Some("Lia M.".into()),
                ..ProfileChange::default()
            },
            saved(),
        )
        .unwrap();
        assert_eq!(request.nickname, "Lia M.");
        assert_eq!(request.status, "on call");
        assert_eq!(request.description, "about");
        assert_eq!(
            request.typing, None,
            "the typing phrase is kept by leaving it out"
        );
    }

    #[test]
    fn a_cleared_field_stays_cleared_and_an_unstated_one_refuses_the_change() {
        let mut cleared = saved();
        cleared.status_message = NullableText::Null;
        let request = merged(
            ProfileChange {
                typing: Some(String::new()),
                ..ProfileChange::default()
            },
            cleared,
        )
        .unwrap();
        assert_eq!(request.status, "");
        assert_eq!(request.typing.as_deref(), Some(""));
        let mut unstated = saved();
        unstated.nickname = NullableText::Missing;
        assert!(matches!(
            merged(ProfileChange::default(), unstated),
            Err(OperationError::InvalidData)
        ));
    }

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
