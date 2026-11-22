//! Role permission grant records, requests, receipts, and assignment batch outcomes across FFI.
//! Role administration methods live in `operations_roles.rs`.

use papo_sdk::{
    RolePermissions,
    operations::{
        OperationError, OperationReceipt,
        roles::{Role, RoleGrants, UserRole},
    },
};

use crate::{
    ids::FfiAccountId,
    operation_errors::{FfiOperationError, FfiRefreshStatus},
};

/// All eight backend role permission booleans preserved atomically on updates.
#[expect(
    clippy::struct_excessive_bools,
    reason = "backend role schema defines eight independent capability booleans"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleGrants {
    /// Manage server settings.
    pub manage_server: bool,
    /// Manage channels.
    pub manage_channels: bool,
    /// Manage roles.
    pub manage_roles: bool,
    /// Manage webhooks.
    pub manage_webhooks: bool,
    /// Ban members.
    pub ban_members: bool,
    /// Pin messages.
    pub pin_message: bool,
    /// Mention everyone.
    pub everyone_message: bool,
    /// Send attachments.
    pub send_attachment: bool,
}

impl From<RoleGrants> for FfiRoleGrants {
    fn from(g: RoleGrants) -> Self {
        Self {
            manage_server: g.manage_server,
            manage_channels: g.manage_channels,
            manage_roles: g.manage_roles,
            manage_webhooks: g.manage_webhooks,
            ban_members: g.ban_members,
            pin_message: g.pin_message,
            everyone_message: g.everyone_message,
            send_attachment: g.send_attachment,
        }
    }
}

impl From<FfiRoleGrants> for RoleGrants {
    fn from(g: FfiRoleGrants) -> Self {
        Self {
            manage_server: g.manage_server,
            manage_channels: g.manage_channels,
            manage_roles: g.manage_roles,
            manage_webhooks: g.manage_webhooks,
            ban_members: g.ban_members,
            pin_message: g.pin_message,
            everyone_message: g.everyone_message,
            send_attachment: g.send_attachment,
        }
    }
}

impl TryFrom<&RolePermissions> for FfiRoleGrants {
    type Error = FfiOperationError;

    fn try_from(p: &RolePermissions) -> Result<Self, Self::Error> {
        RoleGrants::try_from(p).map(Into::into).map_err(Into::into)
    }
}

impl From<FfiRoleGrants> for RolePermissions {
    fn from(g: FfiRoleGrants) -> Self {
        RoleGrants::from(g).into()
    }
}

/// Authoritative backend role record returned by role queries and mutations.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleWire {
    /// Role UUID string.
    pub id: Option<String>,
    /// Role name.
    pub name: Option<String>,
    /// Optional hex color string.
    pub color: Option<String>,
    /// Complete eight-bit permission grants (omitted if server omitted any required bit).
    pub grants: Option<FfiRoleGrants>,
    /// Exact creation timestamp string.
    pub created_at: Option<String>,
}

impl From<&Role> for FfiRoleWire {
    fn from(r: &Role) -> Self {
        let grants = r
            .permissions
            .as_ref()
            .and_then(|p| FfiRoleGrants::try_from(p).ok());
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            color: r.color.clone(),
            grants,
            created_at: r.created_at.clone(),
        }
    }
}

/// Generation-tagged role definition list read directly from backend.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleListResult {
    /// Connection epoch authorizing the read.
    pub generation: u64,
    /// Returned server roles.
    pub roles: Vec<FfiRoleWire>,
}

/// Role creation request with optional initial member user UUIDs (max 1000).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCreateRoleRequest {
    /// New role name.
    pub name: String,
    /// Optional hex color string.
    pub color: Option<String>,
    /// Complete eight-bit permissions when overriding server defaults.
    pub grants: Option<FfiRoleGrants>,
    /// Optional initial member user UUID strings (max 1000).
    pub initial_user_ids: Vec<String>,
}

/// Full role replacement payload preserving all eight permissions.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleUpdate {
    /// Replacement role name.
    pub name: String,
    /// Replacement hex color or `None`.
    pub color: Option<String>,
    /// Complete replacement permissions (all eight bits).
    pub grants: FfiRoleGrants,
}

/// Receipt carrying updated or created role definition.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Returned role definition.
    pub role: FfiRoleWire,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

/// User-role assignment record.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUserRole {
    /// Target user UUID string.
    pub user_id: Option<String>,
    /// Assigned role UUID string.
    pub role_id: Option<String>,
    /// Assignment timestamp string.
    pub assigned_at: Option<String>,
}

/// Receipt acknowledging one user-role assignment.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUserRoleReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Assignment metadata.
    pub assignment: FfiUserRole,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

impl From<OperationReceipt<UserRole>> for FfiUserRoleReceipt {
    fn from(r: OperationReceipt<UserRole>) -> Self {
        Self {
            account: FfiAccountId::from_sdk(r.account),
            generation: r.generation.get(),
            assignment: FfiUserRole {
                user_id: r.value.user_id,
                role_id: r.value.role_id,
                assigned_at: r.value.assigned_at,
            },
            refresh: r.refresh.into(),
        }
    }
}

/// Single `(user_id, role_id)` pair for `assign_roles_batch`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleAssignmentInput {
    /// Target user UUID string.
    pub user_id: String,
    /// Role UUID string.
    pub role_id: String,
}

/// Single `(user_id, banned)` pair for `set_banned_batch`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiBanChangeInput {
    /// Target user UUID string.
    pub user_id: String,
    /// Desired ban state.
    pub banned: bool,
}

/// Per-item outcome inside `assign_roles_batch`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiBatchUserRoleOutcome {
    /// Assignment succeeded.
    Ok {
        /// Acknowledgment receipt.
        receipt: FfiUserRoleReceipt,
    },
    /// Assignment rejected or outcome unknown.
    Err {
        /// Typed error.
        error: FfiOperationError,
    },
}

impl FfiBatchUserRoleOutcome {
    /// Converts one SDK role-assignment batch item result.
    pub(crate) fn from_sdk(res: Result<OperationReceipt<UserRole>, OperationError>) -> Self {
        match res {
            Ok(receipt) => Self::Ok {
                receipt: receipt.into(),
            },
            Err(err) => Self::Err { error: err.into() },
        }
    }
}
