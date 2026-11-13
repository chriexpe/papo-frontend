//! Pinned Go permission evaluation for affordances; requests remain authorized by the backend.
//!
//! Mirrors services/messages.go userHasChannelPermission/userHasRolePermission at the protocol pin.
//! Role membership is explicit, grants combine with OR, and DM access precedes owner privileges.

use crate::{ResourceSnapshot, Uuid};
use papo_api::wire::{Channel, RolePermissions};

/// Missing authoritative inputs cannot be interpreted as an open channel or an empty role set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PermissionDataError {
    /// The snapshot lacks a server, member, role definition or complete channel overwrite list.
    #[error("permission inputs unavailable")]
    Unavailable,
}

/// Effective global grants from explicitly assigned roles, including the owner bypass.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent backend permission bits are not mutually exclusive states"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlobalPermissions {
    /// Server owner, independent of role names.
    pub owner: bool,
    /// Global server configuration; also the backend's current ban gate.
    pub manage_server: bool,
    /// Channel configuration.
    pub manage_channels: bool,
    /// Role configuration and assignment.
    pub manage_roles: bool,
    /// Webhook configuration.
    pub manage_webhooks: bool,
    /// Stored backend flag; currently not the backend ban gate.
    pub ban_members: bool,
    /// Pin ordinary-channel messages.
    pub pin_message: bool,
    /// Expand everyone notifications; this does not gate submitting mention text.
    pub everyone_message: bool,
    /// Upload ordinary-channel attachments; DMs do not require this flag.
    pub send_attachment: bool,
}

impl GlobalPermissions {
    fn grant(&mut self, p: &RolePermissions) {
        self.manage_server |= p.manage_server.unwrap_or(false);
        self.manage_channels |= p.manage_channels.unwrap_or(false);
        self.manage_roles |= p.manage_roles.unwrap_or(false);
        self.manage_webhooks |= p.manage_webhooks.unwrap_or(false);
        self.ban_members |= p.ban_members.unwrap_or(false);
        self.pin_message |= p.pin_message.unwrap_or(false);
        self.everyone_message |= p.everyone_message.unwrap_or(false);
        self.send_attachment |= p.send_attachment.unwrap_or(false);
    }
}

/// Independent channel permission predicates, before operation-specific type/authorship gates.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent backend permission bits are not mutually exclusive states"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelGrants {
    /// Read channel history, list reactions and report typing.
    pub read: bool,
    /// Send messages and add reactions; does not imply read.
    pub send: bool,
    /// Delete someone else's message; never granted by DM membership.
    pub delete_others: bool,
    /// Voice permission predicate; joining also requires channel type voice.
    pub voice: bool,
}

/// Evaluates global roles for a known member without inventing implicit default assignments.
/// # Errors
/// Returns `Unavailable` for missing server/member/role definitions.
pub fn global_permissions(
    snapshot: &ResourceSnapshot,
    user: Uuid,
) -> Result<GlobalPermissions, PermissionDataError> {
    let owner = snapshot
        .server
        .as_ref()
        .ok_or(PermissionDataError::Unavailable)?
        .owner_id
        .as_deref();
    let user = user.to_string();
    if owner == Some(user.as_str()) {
        return Ok(GlobalPermissions {
            owner: true,
            manage_server: true,
            manage_channels: true,
            manage_roles: true,
            manage_webhooks: true,
            ban_members: true,
            pin_message: true,
            everyone_message: true,
            send_attachment: true,
        });
    }
    let assignments = assignments(snapshot, &user)?;
    let mut result = GlobalPermissions::default();
    for id in assignments {
        let role = snapshot
            .roles
            .iter()
            .find(|role| role.id.as_ref() == Some(id))
            .ok_or(PermissionDataError::Unavailable)?;
        result.grant(
            role.permissions
                .as_ref()
                .ok_or(PermissionDataError::Unavailable)?,
        );
    }
    Ok(result)
}

/// Evaluates an ordinary channel's exact OR rules. DM evidence must use `direct_grants`.
/// # Errors
/// Returns `Unavailable` for DM channels or absent required permission inputs.
pub fn channel_grants(
    snapshot: &ResourceSnapshot,
    user: Uuid,
    channel: &Channel,
) -> Result<ChannelGrants, PermissionDataError> {
    if channel.r#type.as_deref() == Some("dm") {
        return Err(PermissionDataError::Unavailable);
    }
    let user = user.to_string();
    let server = snapshot
        .server
        .as_ref()
        .ok_or(PermissionDataError::Unavailable)?;
    if server.owner_id.as_deref() == Some(user.as_str()) {
        return Ok(ChannelGrants {
            read: true,
            send: true,
            delete_others: true,
            voice: true,
        });
    }
    let overwrites = channel
        .permissions
        .as_ref()
        .ok_or(PermissionDataError::Unavailable)?;
    if overwrites.is_empty() {
        return Ok(ChannelGrants {
            read: true,
            send: true,
            delete_others: false,
            voice: true,
        });
    }
    let assignments = assignments(snapshot, &user)?;
    let mut grants = ChannelGrants::default();
    for entry in overwrites {
        let role = entry
            .role_id
            .as_ref()
            .ok_or(PermissionDataError::Unavailable)?;
        if assignments.contains(&role) {
            let p = entry
                .permissions
                .as_ref()
                .ok_or(PermissionDataError::Unavailable)?;
            grants.read |= p.read_channel.unwrap_or(false);
            grants.send |= p.send_messages.unwrap_or(false);
            grants.delete_others |= p.delete_messages.unwrap_or(false);
            grants.voice |= p.connect_voice.unwrap_or(false);
        }
    }
    Ok(grants)
}

/// Uses explicit backend DM membership/block evidence. Owner privileges never enter this branch.
#[must_use]
pub const fn direct_grants(member: bool, blocked_either_direction: bool) -> ChannelGrants {
    let allowed = member && !blocked_either_direction;
    ChannelGrants {
        read: allowed,
        send: allowed,
        delete_others: false,
        voice: allowed,
    }
}

fn assignments<'a>(
    snapshot: &'a ResourceSnapshot,
    user: &str,
) -> Result<Vec<&'a String>, PermissionDataError> {
    snapshot
        .members
        .iter()
        .find(|member| member.id.as_deref() == Some(user))
        .and_then(|member| member.roles.as_ref())
        .ok_or(PermissionDataError::Unavailable)?
        .iter()
        .map(|role| role.id.as_ref().ok_or(PermissionDataError::Unavailable))
        .collect()
}

#[cfg(test)]
mod tests;
