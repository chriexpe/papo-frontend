//! Member, role label, role definition, and profile record types across FFI.
//! List subscriptions and on-demand batch loading live in `people.rs`.

use std::sync::Arc;

use papo_sdk::{RoleFlags, UserProfile};
use papo_sdk_ui::{MemberItem, ProfileItem, RoleItem, RoleLabel};

use crate::define_ffi_vector_diff;

/// Admitted role badge label from resource snapshots.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleLabel {
    /// Backend role UUID string.
    pub id: String,
    /// Backend role name.
    pub name: Option<String>,
    /// Optional hex color string.
    pub color: Option<String>,
}

impl From<&RoleLabel> for FfiRoleLabel {
    fn from(v: &RoleLabel) -> Self {
        Self {
            id: v.id.to_string(),
            name: v.name.clone(),
            color: v.color.clone(),
        }
    }
}

impl From<RoleLabel> for FfiRoleLabel {
    fn from(v: RoleLabel) -> Self {
        (&v).into()
    }
}

/// Admitted backend role capability flag bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleFlags {
    /// Manage server settings.
    pub manage_server: Option<bool>,
    /// Manage channels.
    pub manage_channels: Option<bool>,
    /// Manage roles.
    pub manage_roles: Option<bool>,
    /// Manage webhooks.
    pub manage_webhooks: Option<bool>,
    /// Ban members.
    pub ban_members: Option<bool>,
    /// Pin messages.
    pub pin_message: Option<bool>,
    /// Mention everyone.
    pub everyone_message: Option<bool>,
    /// Send attachments.
    pub send_attachment: Option<bool>,
}

impl From<&RoleFlags> for FfiRoleFlags {
    fn from(v: &RoleFlags) -> Self {
        Self {
            manage_server: v.manage_server,
            manage_channels: v.manage_channels,
            manage_roles: v.manage_roles,
            manage_webhooks: v.manage_webhooks,
            ban_members: v.ban_members,
            pin_message: v.pin_message,
            everyone_message: v.everyone_message,
            send_attachment: v.send_attachment,
        }
    }
}

/// Complete admitted role definition for role administration UIs.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleItem {
    /// Role identity, name, and color label.
    pub label: FfiRoleLabel,
    /// Role permission flags.
    pub permissions: FfiRoleFlags,
    /// Exact backend creation timestamp string.
    pub created_at: Option<String>,
}

impl From<Arc<RoleItem>> for FfiRoleItem {
    fn from(v: Arc<RoleItem>) -> Self {
        Self {
            label: (&v.label).into(),
            permissions: (&v.permissions).into(),
            created_at: v.created_at.clone(),
        }
    }
}

/// Lightweight admitted member or blocked user entry.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMemberItem {
    /// User UUID string.
    pub id: String,
    /// Username.
    pub username: Option<String>,
    /// Optional server nickname.
    pub nickname: Option<String>,
    /// Assigned role labels.
    pub roles: Vec<FfiRoleLabel>,
    /// Backend banned state.
    pub banned: Option<bool>,
}

impl From<&MemberItem> for FfiMemberItem {
    fn from(v: &MemberItem) -> Self {
        Self {
            id: v.id.to_string(),
            username: v.username.clone(),
            nickname: v.nickname.clone(),
            roles: v.roles.iter().map(Into::into).collect(),
            banned: v.banned,
        }
    }
}

impl From<Arc<MemberItem>> for FfiMemberItem {
    fn from(v: Arc<MemberItem>) -> Self {
        v.as_ref().into()
    }
}

/// Loaded user profile summary carrying avatar format and banner token without inline bytes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiProfileItem {
    /// Lightweight member fields.
    pub member: FfiMemberItem,
    /// User bio/description.
    pub description: Option<String>,
    /// Persistent custom status phrase.
    pub typing_phrase: Option<String>,
    /// Revisioned avatar format token (use `MediaSource::UserAvatar` for image bytes).
    pub avatar_format: Option<String>,
    /// Banner media blob token (use `MediaSource::Blob` for banner bytes).
    pub banner_media: Option<String>,
}

impl From<Arc<ProfileItem>> for FfiProfileItem {
    fn from(v: Arc<ProfileItem>) -> Self {
        Self {
            member: (&v.member).into(),
            description: v.description.clone(),
            typing_phrase: v.typing_phrase.clone(),
            avatar_format: v.avatar_format.clone(),
            banner_media: v.banner_media.clone(),
        }
    }
}

/// On-demand loaded full profile result returned directly from `load_profiles`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiLoadedProfile {
    /// User UUID string.
    pub id: Option<String>,
    /// Username.
    pub username: Option<String>,
    /// Nickname.
    pub nickname: Option<String>,
    /// Custom typing status phrase.
    pub custom_status: Option<String>,
    /// Bio/description.
    pub description: Option<String>,
    /// Avatar format token.
    pub avatar_format: Option<String>,
    /// Banner media ID token.
    pub banner_media: Option<String>,
    /// Role UUID strings.
    pub role_ids: Vec<String>,
}

impl From<UserProfile> for FfiLoadedProfile {
    fn from(v: UserProfile) -> Self {
        Self {
            id: v.id,
            username: v.username,
            nickname: crate::operations_channel_records::nullable_to_opt(&v.nickname),
            custom_status: crate::operations_channel_records::nullable_to_opt(&v.typing),
            description: v.description,
            avatar_format: v.avatar_format,
            banner_media: v.banner_media,
            role_ids: v
                .roles
                .unwrap_or_default()
                .into_iter()
                .filter_map(|role| role.id)
                .collect(),
        }
    }
}

/// Result batch returned by `load_profiles` tagged with request generation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiLoadedProfilesResult {
    /// Connection epoch that fetched these profiles.
    pub generation: u64,
    /// Returned user profile payloads.
    pub profiles: Vec<FfiLoadedProfile>,
}

define_ffi_vector_diff!(
    "Ordered diff over member or block items.",
    FfiMemberDiff,
    FfiMemberItem
);
define_ffi_vector_diff!(
    "Ordered diff over role label items.",
    FfiRoleLabelDiff,
    FfiRoleLabel
);
define_ffi_vector_diff!(
    "Ordered diff over complete role definitions.",
    FfiRoleItemDiff,
    FfiRoleItem
);
define_ffi_vector_diff!(
    "Ordered diff over profile items.",
    FfiProfileDiff,
    FfiProfileItem
);

/// Baseline snapshot of `FfiMemberItem`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMemberSnapshot {
    /// Projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Baseline items.
    pub items: Vec<FfiMemberItem>,
}

/// Ordered update batch of `FfiMemberItem`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMemberUpdate {
    /// New projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Sequential diffs.
    pub diffs: Vec<FfiMemberDiff>,
}

/// Listener for member or blocked-user list changes.
#[uniffi::export(with_foreign)]
pub trait FfiMemberListListener: Send + Sync {
    /// Atomic baseline snapshot.
    fn on_snapshot(&self, snapshot: FfiMemberSnapshot);
    /// Incremental ordered diff batch.
    fn on_update(&self, update: FfiMemberUpdate);
    /// Terminal notification on closure.
    fn on_closed(&self);
}

/// Baseline snapshot of role labels.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleLabelSnapshot {
    /// Projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Baseline labels.
    pub items: Vec<FfiRoleLabel>,
}

/// Ordered update batch of role labels.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleLabelUpdate {
    /// New projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Sequential diffs.
    pub diffs: Vec<FfiRoleLabelDiff>,
}

/// Listener for role labels.
#[uniffi::export(with_foreign)]
pub trait FfiRoleLabelListListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, snapshot: FfiRoleLabelSnapshot);
    /// Incremental ordered diff batch.
    fn on_update(&self, update: FfiRoleLabelUpdate);
    /// Terminal notification on closure.
    fn on_closed(&self);
}

/// Baseline snapshot of role definitions including permissions.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleItemSnapshot {
    /// Projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Baseline role definitions.
    pub items: Vec<FfiRoleItem>,
}

/// Ordered update batch of role definitions.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRoleItemUpdate {
    /// New projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Sequential diffs.
    pub diffs: Vec<FfiRoleItemDiff>,
}

/// Listener for role definition lists.
#[uniffi::export(with_foreign)]
pub trait FfiRoleItemListListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, snapshot: FfiRoleItemSnapshot);
    /// Incremental ordered diff batch.
    fn on_update(&self, update: FfiRoleItemUpdate);
    /// Terminal notification on closure.
    fn on_closed(&self);
}

/// Baseline snapshot of requested user profiles.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiProfileSnapshot {
    /// Projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Baseline profiles.
    pub items: Vec<FfiProfileItem>,
}

/// Ordered update batch of requested user profiles.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiProfileUpdate {
    /// New projection version.
    pub version: u64,
    /// Owning session generation.
    pub generation: u64,
    /// Sequential diffs.
    pub diffs: Vec<FfiProfileDiff>,
}

/// Listener for requested user profile lists.
#[uniffi::export(with_foreign)]
pub trait FfiProfileListListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, snapshot: FfiProfileSnapshot);
    /// Incremental ordered diff batch.
    fn on_update(&self, update: FfiProfileUpdate);
    /// Terminal notification on closure.
    fn on_closed(&self);
}
