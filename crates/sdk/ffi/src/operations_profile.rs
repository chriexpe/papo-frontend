//! Self-profile updates, password reset preparation, user/channel settings, and server emoji operations.
//! Webhooks live in `operations_webhooks.rs`.

use std::sync::Arc;

use papo_sdk::{
    BackendMode,
    operations::{
        emojis::CreateEmojiRequest,
        profile::{
            PasswordResetPreparation, UpdateAvatarRequest, UpdateBannerRequest,
            UpdateStatusRequest, UpdateUserRequest,
        },
        settings::{UpdateChannelUserSettingRequest, UpdateUserSettingsRequest},
    },
};

use crate::{
    client::FfiClient,
    ids::{FfiAccountId, FfiBackendMode, parse_uuid, parse_uuid_list},
    operation_errors::FfiOperationError,
    operations_profile_records::{
        FfiChannelSettingReceipt, FfiEmojiReceipt, FfiEmojiWire, FfiPasswordResetPreparation,
        FfiPasswordResetReceipt, FfiUserConfig, FfiUserSettingsReceipt,
    },
    operations_records::{
        FfiBatchUnitOutcome, FfiGenericReceipt, FfiUnitReceipt, closed_operation_error,
        operation_session,
    },
};

#[uniffi::export]
impl FfiClient {
    /// Updates self nickname, custom status text, bio, and optional typing phrase.
    ///
    /// # Errors
    /// Returns backend rejection or `Unknown` dispatch failure.
    pub async fn update_profile(
        &self,
        account: FfiAccountId,
        nickname: String,
        status_text: String,
        description: String,
        typing_phrase: Option<String>,
    ) -> Result<FfiGenericReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = UpdateUserRequest {
            nickname,
            status: status_text,
            description,
            typing: typing_phrase,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_profile(req).await?;
            Ok(r.into())
        })
        .await
    }

    /// Sets persisted presence (`"away"`, `"busy"`, or `None` to reset to automatic).
    ///
    /// # Errors
    /// Returns rejection or `Unknown` dispatch failure.
    pub async fn update_status(
        &self,
        account: FfiAccountId,
        status_mode: Option<String>,
    ) -> Result<FfiGenericReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = UpdateStatusRequest {
            status: status_mode,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_status(req).await?;
            Ok(r.into())
        })
        .await
    }

    /// Uploads pre-encoded avatar data (`<= 2 MiB`).
    ///
    /// # Errors
    /// Returns rejection or `Unknown` upload failure.
    pub async fn update_avatar(
        &self,
        account: FfiAccountId,
        avatar_base64: String,
        avatar_format: String,
    ) -> Result<FfiGenericReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = UpdateAvatarRequest {
            avatar: avatar_base64,
            avatar_format,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_avatar(req).await?;
            Ok(r.into())
        })
        .await
    }

    /// Uploads or clears pre-encoded profile banner.
    ///
    /// # Errors
    /// Returns rejection or `Unknown` upload failure.
    pub async fn update_banner(
        &self,
        account: FfiAccountId,
        banner_base64: String,
        banner_format: String,
    ) -> Result<FfiGenericReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = UpdateBannerRequest {
            banner: banner_base64,
            banner_format,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_banner(req).await?;
            Ok(r.into())
        })
        .await
    }

    /// Prepares a password reset or requests an owner-authorized single-use reset link.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch failure.
    pub async fn prepare_password_reset(
        &self,
        account: FfiAccountId,
        target_user_id: String,
    ) -> Result<FfiPasswordResetReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let target = parse_uuid(&target_user_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().prepare_password_reset(target).await?;
            let preparation = match r.value {
                PasswordResetPreparation::Link(link) => FfiPasswordResetPreparation::Link {
                    link: Arc::new(link.into()),
                },
                // Prepared and future preparation variants map to Prepared.
                _ => FfiPasswordResetPreparation::Prepared,
            };
            Ok(FfiPasswordResetReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                preparation,
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Replaces server-side user configuration document.
    ///
    /// # Errors
    /// Returns rejection or `Unknown` dispatch failure.
    pub async fn update_user_settings(
        &self,
        account: FfiAccountId,
        config: FfiUserConfig,
    ) -> Result<FfiUserSettingsReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = UpdateUserSettingsRequest {
            config: config.into(),
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_user_settings(req).await?;
            Ok(FfiUserSettingsReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                version: r.value.version,
                config: r.value.config.as_ref().map(Into::into),
                updated_at: r.value.updated_at,
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Updates server-side notification mode override for one channel or DM.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch failure.
    pub async fn update_channel_settings(
        &self,
        account: FfiAccountId,
        channel_id: String,
        mode: FfiBackendMode,
    ) -> Result<FfiChannelSettingReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let sdk_mode: BackendMode = mode.into();
        let req = UpdateChannelUserSettingRequest {
            notification_settings: sdk_mode.as_str().to_owned(),
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session
                .operations()
                .update_channel_settings(channel, req)
                .await?;
            Ok(FfiChannelSettingReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                channel_id: r.value.channel_id,
                mode: r
                    .value
                    .notification_settings
                    .map(BackendMode::from)
                    .map(|m| FfiBackendMode::from(&m)),
                updated_at: r.value.updated_at,
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Uploads a custom server emoji/sticker image once.
    ///
    /// # Errors
    /// Returns rejection, size/format failure, or `Unknown` dispatch failure.
    pub async fn create_emoji(
        &self,
        account: FfiAccountId,
        name: String,
        image_blob_base64: String,
        format: String,
    ) -> Result<FfiEmojiReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = CreateEmojiRequest {
            name,
            image_blob: image_blob_base64,
            format,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().create_emoji(req).await?;
            Ok(FfiEmojiReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                emoji: FfiEmojiWire::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Deletes one server emoji.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch failure.
    pub async fn delete_emoji(
        &self,
        account: FfiAccountId,
        emoji_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let emoji = parse_uuid(&emoji_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().delete_emoji(emoji).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Deletes up to 100 server emojis sequentially without auto-retrying failures.
    ///
    /// # Errors
    /// Returns [`FfiOperationError::InvalidInput`] for >100 items or invalid UUIDs.
    pub async fn delete_emojis_batch(
        &self,
        account: FfiAccountId,
        emoji_ids: Vec<String>,
    ) -> Result<Vec<FfiBatchUnitOutcome>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let ids = parse_uuid_list(emoji_ids).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let results = session.operations().delete_emojis_batch(ids).await?;
            Ok(results
                .into_iter()
                .map(FfiBatchUnitOutcome::from_sdk)
                .collect())
        })
        .await
    }
}
