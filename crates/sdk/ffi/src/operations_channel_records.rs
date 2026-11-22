//! Channel, permission override, and minimal social user summary FFI records.
//! Execution methods live in `operations_admin.rs` and `operations_social.rs`.

use papo_sdk::{
    Channel, ChannelPermissionEntry, ChannelPermissions, NullableText,
    operations::{OperationReceipt, channels::UpdateChannelPermissionsResponse},
};

use crate::{ids::FfiAccountId, operation_errors::FfiRefreshStatus};

/// Channel creation payload.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiCreateChannelRequest {
    /// Channel name.
    pub name: String,
    /// Optional wire channel type (`"text"`, `"voice"`, `"category"`).
    pub kind: Option<String>,
    /// Optional topic description.
    pub topic: Option<String>,
    /// Optional parent category UUID string.
    pub parent_id: Option<String>,
    /// Optional initial access role UUID strings.
    pub access_role_ids: Option<Vec<String>>,
}

/// Channel name/topic update payload.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUpdateChannelRequest {
    /// Replacement channel name.
    pub name: String,
    /// Replacement topic if supplied.
    pub topic: Option<String>,
}

/// Channel one-based position change payload.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChangeChannelPositionRequest {
    /// One-based prior position expected by backend CAS check.
    pub old_position: i64,
    /// Requested one-based position.
    pub new_position: i64,
    /// `None` preserves category, `Some("")` ungroups, or category UUID string moves under parent.
    pub parent_id: Option<String>,
}

/// Authoritative backend channel permission override flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelPermissions {
    /// Read channel permission bit.
    pub read_channel: Option<bool>,
    /// Send messages permission bit.
    pub send_messages: Option<bool>,
    /// Delete messages permission bit.
    pub delete_messages: Option<bool>,
    /// Connect voice permission bit.
    pub connect_voice: Option<bool>,
}

impl From<&ChannelPermissions> for FfiChannelPermissions {
    fn from(p: &ChannelPermissions) -> Self {
        Self {
            read_channel: p.read_channel,
            send_messages: p.send_messages,
            delete_messages: p.delete_messages,
            connect_voice: p.connect_voice,
        }
    }
}

impl From<FfiChannelPermissions> for ChannelPermissions {
    fn from(p: FfiChannelPermissions) -> Self {
        Self {
            read_channel: p.read_channel,
            send_messages: p.send_messages,
            delete_messages: p.delete_messages,
            connect_voice: p.connect_voice,
        }
    }
}

/// Single role override entry inside channel permissions response.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelPermissionEntry {
    /// Role UUID string.
    pub role_id: Option<String>,
    /// Role display name.
    pub role_name: Option<String>,
    /// Override bits.
    pub permissions: Option<FfiChannelPermissions>,
}

impl From<&ChannelPermissionEntry> for FfiChannelPermissionEntry {
    fn from(e: &ChannelPermissionEntry) -> Self {
        Self {
            role_id: e.role_id.clone(),
            role_name: e.role_name.clone(),
            permissions: e.permissions.as_ref().map(Into::into),
        }
    }
}

/// Generation-verified channel permissions readback.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelPermissionsResult {
    /// Connection epoch that executed the read.
    pub generation: u64,
    /// Target channel UUID.
    pub channel_id: Option<String>,
    /// Role override entries.
    pub entries: Vec<FfiChannelPermissionEntry>,
}

/// Receipt acknowledging updated role permissions on a channel.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUpdateChannelPermissionsReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Channel UUID string.
    pub channel_id: Option<String>,
    /// Updated role UUID string.
    pub role_id: Option<String>,
    /// Authoritative override bits.
    pub permissions: Option<FfiChannelPermissions>,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

impl From<OperationReceipt<UpdateChannelPermissionsResponse>>
    for FfiUpdateChannelPermissionsReceipt
{
    fn from(r: OperationReceipt<UpdateChannelPermissionsResponse>) -> Self {
        Self {
            account: FfiAccountId::from_sdk(r.account),
            generation: r.generation.get(),
            channel_id: r.value.channel_id,
            role_id: r.value.role_id,
            permissions: r.value.permissions.as_ref().map(Into::into),
            refresh: r.refresh.into(),
        }
    }
}

/// Authoritative wire channel record returned by channel create/update/move receipts.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelWire {
    /// Channel UUID string.
    pub id: Option<String>,
    /// Channel name.
    pub name: Option<String>,
    /// Wire channel type.
    pub kind: Option<String>,
    /// One-based sort position.
    pub position: Option<i64>,
    /// Channel topic.
    pub topic: Option<String>,
    /// Parent category UUID string.
    pub parent_id: Option<String>,
    /// Role overrides attached to response.
    pub permissions: Vec<FfiChannelPermissionEntry>,
    /// Creation timestamp string.
    pub created_at: Option<String>,
}

impl From<&Channel> for FfiChannelWire {
    fn from(c: &Channel) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            kind: c.r#type.clone(),
            position: c.position,
            topic: c.topic.clone(),
            parent_id: c.parent_id.clone(),
            permissions: c
                .permissions
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(Into::into)
                .collect(),
            created_at: c.created_at.clone(),
        }
    }
}

/// Receipt carrying updated channel record.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Returned channel metadata.
    pub channel: FfiChannelWire,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

/// Minimal user summary returned in DM and block read endpoints.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUserSummaryWire {
    /// User UUID string.
    pub id: Option<String>,
    /// Username.
    pub username: Option<String>,
    /// Optional explicit nickname (absent or cleared -> `None`).
    pub nickname: Option<String>,
    /// Banned state.
    pub banned: Option<bool>,
    /// Presence status mode string.
    pub presence_status: Option<String>,
    /// Status text message.
    pub status_message: Option<String>,
    /// Typing phrase override.
    pub typing: Option<String>,
    /// Creation timestamp string.
    pub created_at: Option<String>,
}

/// Converts optional `NullableText` into `Option<String>`.
pub(crate) fn nullable_to_opt(v: &NullableText) -> Option<String> {
    match v {
        NullableText::Value(s) => Some(s.clone()),
        NullableText::Null | NullableText::Missing => None,
    }
}

impl From<&papo_sdk::UserSummary> for FfiUserSummaryWire {
    fn from(u: &papo_sdk::UserSummary) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            nickname: nullable_to_opt(&u.nickname),
            banned: u.banned,
            presence_status: u.status.clone(),
            status_message: u.status_message.clone(),
            typing: nullable_to_opt(&u.typing),
            created_at: u.created_at.clone(),
        }
    }
}
