//! Owns full role replacement and assignments; backend role and owner restrictions remain authoritative.

use super::{Generated, OperationError, OperationReceipt, Operations, Refresh};
pub use papo_api::wire::{
    AssignRoleRequest, CreateRoleRequest, Role, RoleListResponse, RolePermissions, UserRole,
};
use papo_api::{Uuid, wire::UpdateRoleRequest};

/// Explicit full replacement of a role. The backend offers no compare-and-swap or rename-only route.
/// All eight permissions are retained, including `manage_webhooks`.
#[derive(Clone)]
pub struct RoleUpdate {
    /// New role name; the backend checks uniqueness and its 32-character limit.
    pub name: String,
    /// Replacement hexadecimal color, or no color.
    pub color: Option<String>,
    /// Complete replacement permission set.
    pub permissions: RoleGrants,
}

/// All eight permissions in the pinned backend's role definition.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent backend permissions are not mutually exclusive states"
)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RoleGrants {
    /// Manage server settings.
    pub manage_server: bool,
    /// Manage channels and their overrides.
    pub manage_channels: bool,
    /// Manage roles and assignments.
    pub manage_roles: bool,
    /// Manage webhook identities and credentials.
    pub manage_webhooks: bool,
    /// Backend ban-members role bit; routes still enforce their own authorization.
    pub ban_members: bool,
    /// Pin messages.
    pub pin_message: bool,
    /// Mention everyone.
    pub everyone_message: bool,
    /// Send attachments.
    pub send_attachment: bool,
}

impl RoleUpdate {
    /// Builds a full replacement retaining the supplied role's color and eight permissions.
    /// Obtain the current role explicitly before editing; concurrent backend changes cannot be CAS guarded.
    /// # Errors
    /// Returns `InvalidData` when the supplied role omits any permission; never silently resets it.
    pub fn rename(current: &Role, name: String) -> Result<Self, OperationError> {
        let permissions = current
            .permissions
            .as_ref()
            .ok_or(OperationError::InvalidData)?;
        Ok(Self {
            name,
            color: current.color.clone(),
            permissions: RoleGrants::try_from(permissions)?,
        })
    }

    fn into_request(self) -> UpdateRoleRequest {
        UpdateRoleRequest {
            name: self.name,
            color: self.color,
            permissions: Some(self.permissions.into()),
        }
    }
}

impl TryFrom<&RolePermissions> for RoleGrants {
    type Error = OperationError;

    fn try_from(value: &RolePermissions) -> Result<Self, Self::Error> {
        Ok(Self {
            manage_server: value.manage_server.ok_or(OperationError::InvalidData)?,
            manage_channels: value.manage_channels.ok_or(OperationError::InvalidData)?,
            manage_roles: value.manage_roles.ok_or(OperationError::InvalidData)?,
            manage_webhooks: value.manage_webhooks.ok_or(OperationError::InvalidData)?,
            ban_members: value.ban_members.ok_or(OperationError::InvalidData)?,
            pin_message: value.pin_message.ok_or(OperationError::InvalidData)?,
            everyone_message: value.everyone_message.ok_or(OperationError::InvalidData)?,
            send_attachment: value.send_attachment.ok_or(OperationError::InvalidData)?,
        })
    }
}

impl From<RoleGrants> for RolePermissions {
    fn from(value: RoleGrants) -> Self {
        Self {
            manage_server: Some(value.manage_server),
            manage_channels: Some(value.manage_channels),
            manage_roles: Some(value.manage_roles),
            manage_webhooks: Some(value.manage_webhooks),
            ban_members: Some(value.ban_members),
            pin_message: Some(value.pin_message),
            everyone_message: Some(value.everyone_message),
            send_attachment: Some(value.send_attachment),
        }
    }
}

impl Operations {
    /// Assigns at most 100 `(user, role)` pairs, returning one result per input in order.
    /// A rejection or ambiguous outcome is retained without retrying the item.
    /// # Errors
    /// Returns `InvalidInput` when the batch exceeds 100 items.
    pub async fn assign_roles_batch(
        &self,
        assignments: Vec<(Uuid, Uuid)>,
    ) -> Result<Vec<Result<OperationReceipt<UserRole>, OperationError>>, OperationError> {
        if assignments.len() > 100 {
            return Err(OperationError::InvalidInput);
        }
        let operations = self.for_batch();
        let mut results = Vec::with_capacity(assignments.len());
        for (user, role) in assignments {
            results.push(operations.assign_role(user, role).await);
        }
        Ok(results)
    }

    /// Reads current role definitions. No member roster is fetched.
    /// # Errors
    /// Returns a typed backend, decoding or current-session error.
    pub async fn roles(&self) -> Result<Generated<RoleListResponse>, OperationError> {
        self.read(move |rest| async move { rest.roles().await })
            .await
    }

    /// Creates a role and its optional initial member assignments in the backend's transaction.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn create_role(
        &self,
        request: CreateRoleRequest,
    ) -> Result<OperationReceipt<Role>, OperationError> {
        let initial_users = request.users.as_deref().unwrap_or_default();
        if initial_users.len() > 1000 {
            return Err(OperationError::InvalidInput);
        }
        let users = initial_users
            .iter()
            .map(|user| Uuid::parse_str(user).map_err(|_| OperationError::InvalidInput))
            .collect::<Result<Vec<_>, _>>()?;
        self.mutate(
            move |rest| async move { rest.create_role(&request).await },
            Refresh::RolesMembers(users),
        )
        .await
    }

    /// Explicitly replaces name, color and all eight permissions. No rename-only route is fabricated.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn update_role(
        &self,
        role: Uuid,
        update: RoleUpdate,
    ) -> Result<OperationReceipt<Role>, OperationError> {
        let request = update.into_request();
        self.mutate(
            move |rest| async move { rest.update_role(role, &request).await },
            Refresh::Roles,
        )
        .await
    }

    /// Deletes a role and its backend assignments and overrides atomically.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn delete_role(&self, role: Uuid) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.delete_role(role).await },
            Refresh::Roles,
        )
        .await
    }

    /// Assigns one role; current-user or owner restrictions are decided by the backend.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn assign_role(
        &self,
        user: Uuid,
        role: Uuid,
    ) -> Result<OperationReceipt<UserRole>, OperationError> {
        let request = AssignRoleRequest {
            role_id: role.to_string(),
        };
        self.mutate(
            move |rest| async move { rest.assign_role(user, &request).await },
            Refresh::Member(user),
        )
        .await
    }

    /// Removes one assignment without refetching the member roster.
    /// # Errors
    /// Returns a typed rejection, unavailable session or ambiguous dispatch outcome.
    pub async fn remove_role(
        &self,
        user: Uuid,
        role: Uuid,
    ) -> Result<OperationReceipt<()>, OperationError> {
        self.mutate(
            move |rest| async move { rest.remove_role(user, role).await },
            Refresh::Member(user),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies full replacement preserves the backend's eight role bits without network access.

    use super::*;

    #[test]
    fn rename_retains_each_permission_including_webhook_management() {
        for index in 0..8 {
            let grants = RoleGrants {
                manage_server: index == 0,
                manage_channels: index == 1,
                manage_roles: index == 2,
                manage_webhooks: index == 3,
                ban_members: index == 4,
                pin_message: index == 5,
                everyone_message: index == 6,
                send_attachment: index == 7,
            };
            let role = Role {
                permissions: Some(grants.into()),
                color: Some("#112233".to_owned()),
                ..Role::default()
            };
            let update = RoleUpdate::rename(&role, "renamed".to_owned()).unwrap();
            assert_eq!(update.permissions, grants);
            let request = update.into_request();
            assert_eq!(request.name, "renamed");
            assert_eq!(request.color.as_deref(), Some("#112233"));
            assert_eq!(
                RoleGrants::try_from(request.permissions.as_ref().unwrap()).unwrap(),
                grants
            );
        }
    }

    #[test]
    fn rename_rejects_incomplete_permissions_instead_of_clearing_bits() {
        let role = Role {
            permissions: Some(RolePermissions::default()),
            ..Role::default()
        };
        assert!(matches!(
            RoleUpdate::rename(&role, "renamed".to_owned()),
            Err(OperationError::InvalidData)
        ));
    }
}
