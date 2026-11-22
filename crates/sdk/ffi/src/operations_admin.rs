//! Server configuration, channel lifecycle, channel permission, and metadata refresh operations.
//! Authorization and conflict detection remain backend-authoritative; never retries Unknown outcomes.

use papo_sdk::operations::{
    administration::{PatchServerRequest, ReplaceServerRequest},
    channels::{
        ChangeChannelPositionRequest, CreateChannelRequest, UpdateChannelPermissionsRequest,
        UpdateChannelRequest,
    },
};

use crate::{
    client::FfiClient,
    ids::{FfiAccountId, parse_uuid, parse_uuid_list},
    operation_errors::FfiOperationError,
    operations_channel_records::{
        FfiChangeChannelPositionRequest, FfiChannelPermissionEntry, FfiChannelPermissions,
        FfiChannelPermissionsResult, FfiChannelReceipt, FfiChannelWire, FfiCreateChannelRequest,
        FfiUpdateChannelPermissionsReceipt, FfiUpdateChannelRequest,
    },
    operations_records::{
        FfiBatchUnitOutcome, FfiMetadataScope, FfiPatchServerRequest, FfiRefreshOutcome,
        FfiReplaceServerRequest, FfiServerReceipt, FfiUnitReceipt, closed_operation_error,
        operation_session, refresh_outcome,
    },
};

#[uniffi::export]
impl FfiClient {
    /// Retries only the selected metadata read/projection refresh after a prior refresh warning.
    ///
    /// # Errors
    /// Returns account revocation, generation change, or backend failure without repeating mutations.
    pub async fn refresh_metadata(
        &self,
        account: FfiAccountId,
        scope: FfiMetadataScope,
    ) -> Result<FfiRefreshOutcome, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let sdk_scope = scope.into_sdk()?;
        self.run_scoped(closed_operation_error, async move {
            let generated = session.operations().refresh_metadata(sdk_scope).await?;
            Ok(refresh_outcome(generated.generation.get(), generated.value))
        })
        .await
    }

    /// Patches supplied server configuration fields.
    ///
    /// # Errors
    /// Returns definite rejection or `Unknown` dispatch outcome; never auto-retries.
    pub async fn patch_server(
        &self,
        account: FfiAccountId,
        request: FfiPatchServerRequest,
    ) -> Result<FfiServerReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = PatchServerRequest {
            name: request.name,
            icon_blob: request.icon_blob,
            icon_format: request.icon_format,
            password: request.password,
            public: request.public_server,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().patch_server(req).await?;
            Ok(FfiServerReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                server: (&r.value).into(),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Replaces all server settings explicitly.
    ///
    /// # Errors
    /// Returns definite rejection or `Unknown` dispatch outcome; never auto-retries.
    pub async fn replace_server(
        &self,
        account: FfiAccountId,
        request: FfiReplaceServerRequest,
    ) -> Result<FfiServerReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = ReplaceServerRequest {
            name: request.name,
            icon_blob: request.icon_blob,
            icon_format: request.icon_format,
            password: request.password,
            public: request.public_server,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().replace_server(req).await?;
            Ok(FfiServerReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                server: (&r.value).into(),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Creates a backend text, voice, or category channel.
    ///
    /// # Errors
    /// Returns validation, backend rejection, or `Unknown` dispatch failure.
    pub async fn create_channel(
        &self,
        account: FfiAccountId,
        request: FfiCreateChannelRequest,
    ) -> Result<FfiChannelReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let req = CreateChannelRequest {
            name: request.name,
            r#type: request.kind,
            topic: request.topic,
            parent_id: request.parent_id,
            access_roles: request.access_role_ids,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().create_channel(req).await?;
            Ok(FfiChannelReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                channel: FfiChannelWire::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Updates channel name and optional topic.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn update_channel(
        &self,
        account: FfiAccountId,
        channel_id: String,
        request: FfiUpdateChannelRequest,
    ) -> Result<FfiChannelReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let req = UpdateChannelRequest {
            name: request.name,
            topic: request.topic,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_channel(channel, req).await?;
            Ok(FfiChannelReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                channel: FfiChannelWire::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Deletes a single channel cleanly once.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn delete_channel(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().delete_channel(channel).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Deletes up to 100 channels sequentially, retaining every item outcome in order.
    ///
    /// # Errors
    /// Returns [`FfiOperationError::InvalidInput`] for >100 items or malformed UUIDs.
    pub async fn delete_channels_batch(
        &self,
        account: FfiAccountId,
        channel_ids: Vec<String>,
    ) -> Result<Vec<FfiBatchUnitOutcome>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channels = parse_uuid_list(channel_ids).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let results = session.operations().delete_channels_batch(channels).await?;
            Ok(results
                .into_iter()
                .map(FfiBatchUnitOutcome::from_sdk)
                .collect())
        })
        .await
    }

    /// Reorders a channel using backend CAS positions (`old_position -> new_position`).
    ///
    /// # Errors
    /// Returns invalid input, stale-position rejection, or `Unknown` dispatch failure.
    pub async fn move_channel(
        &self,
        account: FfiAccountId,
        channel_id: String,
        request: FfiChangeChannelPositionRequest,
    ) -> Result<FfiChannelReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let req = ChangeChannelPositionRequest {
            old_position: request.old_position,
            new_position: request.new_position,
            parent_id: request.parent_id,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().move_channel(channel, req).await?;
            Ok(FfiChannelReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                channel: FfiChannelWire::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Reads authoritative channel permission overrides directly from the backend.
    ///
    /// # Errors
    /// Returns invalid UUID, backend rejection, or session closure errors.
    pub async fn channel_permissions(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<FfiChannelPermissionsResult, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let g = session.operations().channel_permissions(channel).await?;
            Ok(FfiChannelPermissionsResult {
                generation: g.generation.get(),
                channel_id: g.value.channel_id,
                entries: g
                    .value
                    .permissions
                    .unwrap_or_default()
                    .iter()
                    .map(FfiChannelPermissionEntry::from)
                    .collect(),
            })
        })
        .await
    }

    /// Replaces role permission overrides for a single channel.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn update_channel_permissions(
        &self,
        account: FfiAccountId,
        channel_id: String,
        role_id: String,
        permissions: FfiChannelPermissions,
    ) -> Result<FfiUpdateChannelPermissionsReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let role = parse_uuid(&role_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let req = UpdateChannelPermissionsRequest {
            permissions: permissions.into(),
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session
                .operations()
                .update_channel_permissions(channel, role, req)
                .await?;
            Ok(r.into())
        })
        .await
    }

    /// Removes one role's override from a channel without deleting the server role.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn remove_channel_role(
        &self,
        account: FfiAccountId,
        channel_id: String,
        role_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let role = parse_uuid(&role_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session
                .operations()
                .remove_channel_role(channel, role)
                .await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }
}
