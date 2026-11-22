//! Role administration, assignment batches, member bans, and moderator deletion across FFI.
//! Role edits always pass all eight permission bits to preserve server-side invariants.

use papo_sdk::operations::roles::{CreateRoleRequest, Role, RoleUpdate};

use crate::{
    client::FfiClient,
    ids::{FfiAccountId, parse_uuid},
    operation_errors::FfiOperationError,
    operations_records::{
        FfiBatchGenericOutcome, FfiGenericReceipt, FfiUnitReceipt, closed_operation_error,
        operation_session,
    },
    operations_role_records::{
        FfiBanChangeInput, FfiBatchUserRoleOutcome, FfiCreateRoleRequest, FfiRoleAssignmentInput,
        FfiRoleListResult, FfiRoleReceipt, FfiRoleUpdate, FfiRoleWire, FfiUserRoleReceipt,
    },
};

/// Builds a rename replacement preserving all eight existing permission bits via `RoleUpdate::rename`.
///
/// # Errors
/// Returns [`FfiOperationError::InvalidData`] if the current role omits any permission bit.
#[uniffi::export]
pub fn build_role_rename_update(
    current: FfiRoleWire,
    new_name: String,
) -> Result<FfiRoleUpdate, FfiOperationError> {
    let sdk_role = Role {
        id: current.id,
        name: current.name,
        color: current.color,
        permissions: current.grants.map(Into::into),
        created_at: current.created_at,
    };
    let update = RoleUpdate::rename(&sdk_role, new_name)?;
    Ok(FfiRoleUpdate {
        name: update.name,
        color: update.color,
        grants: update.permissions.into(),
    })
}

#[uniffi::export]
impl FfiClient {
    /// Reads current server role definitions directly from backend.
    ///
    /// # Errors
    /// Returns backend rejection, invalid data, or session closure errors.
    pub async fn fetch_roles(
        &self,
        account: FfiAccountId,
    ) -> Result<FfiRoleListResult, FfiOperationError> {
        let session = operation_session(self, &account)?;
        self.run_scoped(closed_operation_error, async move {
            let g = session.operations().roles().await?;
            Ok(FfiRoleListResult {
                generation: g.generation.get(),
                roles: g
                    .value
                    .roles
                    .unwrap_or_default()
                    .iter()
                    .map(FfiRoleWire::from)
                    .collect(),
            })
        })
        .await
    }

    /// Creates a new role and optional initial member assignments.
    ///
    /// # Errors
    /// Returns input validation, backend rejection, or `Unknown` dispatch failure.
    pub async fn create_role(
        &self,
        account: FfiAccountId,
        request: FfiCreateRoleRequest,
    ) -> Result<FfiRoleReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let users = (!request.initial_user_ids.is_empty()).then_some(request.initial_user_ids);
        let req = CreateRoleRequest {
            name: request.name,
            color: request.color,
            permissions: request.grants.map(Into::into),
            users,
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().create_role(req).await?;
            Ok(FfiRoleReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                role: FfiRoleWire::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Replaces role name, optional color, and all eight permission bits atomically.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch failure.
    pub async fn update_role(
        &self,
        account: FfiAccountId,
        role_id: String,
        update: FfiRoleUpdate,
    ) -> Result<FfiRoleReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let role = parse_uuid(&role_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let sdk_update = RoleUpdate {
            name: update.name,
            color: update.color,
            permissions: update.grants.into(),
        };
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().update_role(role, sdk_update).await?;
            Ok(FfiRoleReceipt {
                account: FfiAccountId::from_sdk(r.account),
                generation: r.generation.get(),
                role: FfiRoleWire::from(&r.value),
                refresh: r.refresh.into(),
            })
        })
        .await
    }

    /// Deletes a role and its assignments atomically on backend.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn delete_role(
        &self,
        account: FfiAccountId,
        role_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let role = parse_uuid(&role_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().delete_role(role).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Assigns one role to one user.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn assign_role(
        &self,
        account: FfiAccountId,
        user_id: String,
        role_id: String,
    ) -> Result<FfiUserRoleReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let user = parse_uuid(&user_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let role = parse_uuid(&role_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().assign_role(user, role).await?;
            Ok(r.into())
        })
        .await
    }

    /// Assigns up to 100 `(user_id, role_id)` pairs sequentially without dropping partial acks.
    ///
    /// # Errors
    /// Returns [`FfiOperationError::InvalidInput`] for >100 items or malformed UUIDs.
    pub async fn assign_roles_batch(
        &self,
        account: FfiAccountId,
        assignments: Vec<FfiRoleAssignmentInput>,
    ) -> Result<Vec<FfiBatchUserRoleOutcome>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let pairs = assignments
            .into_iter()
            .map(|a| {
                let u = parse_uuid(&a.user_id).map_err(|_| FfiOperationError::InvalidInput)?;
                let r = parse_uuid(&a.role_id).map_err(|_| FfiOperationError::InvalidInput)?;
                Ok((u, r))
            })
            .collect::<Result<Vec<_>, FfiOperationError>>()?;
        self.run_scoped(closed_operation_error, async move {
            let results = session.operations().assign_roles_batch(pairs).await?;
            Ok(results
                .into_iter()
                .map(FfiBatchUserRoleOutcome::from_sdk)
                .collect())
        })
        .await
    }

    /// Removes one role assignment from a user.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn remove_role(
        &self,
        account: FfiAccountId,
        user_id: String,
        role_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let user = parse_uuid(&user_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let role = parse_uuid(&role_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().remove_role(user, role).await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }

    /// Sets ban state for one backend member.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn set_banned(
        &self,
        account: FfiAccountId,
        user_id: String,
        banned: bool,
    ) -> Result<FfiGenericReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let user = parse_uuid(&user_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session.operations().set_banned(user, banned).await?;
            Ok(r.into())
        })
        .await
    }

    /// Sets ban state for up to 100 members sequentially without dropping partial results.
    ///
    /// # Errors
    /// Returns [`FfiOperationError::InvalidInput`] for >100 items or malformed UUIDs.
    pub async fn set_banned_batch(
        &self,
        account: FfiAccountId,
        changes: Vec<FfiBanChangeInput>,
    ) -> Result<Vec<FfiBatchGenericOutcome>, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let pairs = changes
            .into_iter()
            .map(|c| {
                let u = parse_uuid(&c.user_id).map_err(|_| FfiOperationError::InvalidInput)?;
                Ok((u, c.banned))
            })
            .collect::<Result<Vec<_>, FfiOperationError>>()?;
        self.run_scoped(closed_operation_error, async move {
            let results = session.operations().set_banned_batch(pairs).await?;
            Ok(results
                .into_iter()
                .map(FfiBatchGenericOutcome::from_sdk)
                .collect())
        })
        .await
    }

    /// Moderator/admin deletion of one authoritative message in `channel_id`.
    ///
    /// # Errors
    /// Returns invalid UUID, rejection, or `Unknown` dispatch outcome.
    pub async fn moderate_delete_message(
        &self,
        account: FfiAccountId,
        channel_id: String,
        message_id: String,
    ) -> Result<FfiUnitReceipt, FfiOperationError> {
        let session = operation_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiOperationError::InvalidInput)?;
        let message = parse_uuid(&message_id).map_err(|_| FfiOperationError::InvalidInput)?;
        self.run_scoped(closed_operation_error, async move {
            let r = session
                .operations()
                .moderate_delete_message(channel, message)
                .await?;
            Ok(FfiUnitReceipt::from_sdk(r))
        })
        .await
    }
}
