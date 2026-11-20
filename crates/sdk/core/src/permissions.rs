//! Pinned Go permission evaluation for affordances; requests remain authorized by the backend.
//!
//! Mirrors services/messages.go userHasChannelPermission/userHasRolePermission at the protocol pin.
//! Role membership is explicit, grants combine with OR, and DM access precedes owner privileges.

use crate::{CacheResource, ChannelRecord, OverwriteKey, ResourceSnapshot, RoleFlags, Uuid};

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
    fn grant(&mut self, p: &RoleFlags) {
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
    if snapshot.gaps.owner_uncertain() {
        return Err(PermissionDataError::Unavailable);
    }
    let evidence = AccountEvidence::new(snapshot, user);
    if evidence.owner? {
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
    if snapshot.gaps.global_roles_uncertain() {
        return Err(PermissionDataError::Unavailable);
    }
    let mut result = GlobalPermissions::default();
    for id in evidence.assignments? {
        if snapshot.gaps.role_flags_uncertain(*id) {
            return Err(PermissionDataError::Unavailable);
        }
        let role = snapshot
            .roles
            .get(*id)
            .ok_or(PermissionDataError::Unavailable)?;
        result.grant(&role.permissions);
    }
    Ok(result)
}

/// Evaluates an ordinary channel's exact OR rules. DM evidence must use `direct_grants`.
/// # Errors
/// Returns `Unavailable` for DM channels or absent required permission inputs.
pub fn channel_grants(
    snapshot: &ResourceSnapshot,
    user: Uuid,
    channel: &ChannelRecord,
) -> Result<ChannelGrants, PermissionDataError> {
    AccountEvidence::new(snapshot, user).channel_grants(channel)
}

/// One account's server-wide permission inputs, read once so many channels can be
/// evaluated against the same snapshot. Errors surface only for channels that need them.
pub(crate) struct AccountEvidence<'a> {
    owner: Result<bool, PermissionDataError>,
    assignments: Result<&'a [Uuid], PermissionDataError>,
    snapshot: &'a ResourceSnapshot,
}

impl<'a> AccountEvidence<'a> {
    pub(crate) fn new(snapshot: &'a ResourceSnapshot, user: Uuid) -> Self {
        let owner = snapshot
            .server
            .as_ref()
            .filter(|_| !snapshot.gaps.rejected(CacheResource::Server))
            .map(|server| server.owner_id == Some(user))
            .ok_or(PermissionDataError::Unavailable);
        let assignments = snapshot
            .users
            .get(user)
            .filter(|_| {
                snapshot.members.contains(user)
                    && !snapshot.gaps.rejected(CacheResource::Members)
                    && !snapshot.gaps.rejected(CacheResource::Roles)
            })
            .and_then(|member| member.roles.as_deref())
            .filter(|ids| ids.iter().all(|id| snapshot.roles.get(*id).is_some()))
            .ok_or(PermissionDataError::Unavailable);
        Self {
            owner,
            assignments,
            snapshot,
        }
    }

    /// The single ordinary-channel algorithm behind `channel_grants`.
    pub(crate) fn channel_grants(
        &self,
        channel: &ChannelRecord,
    ) -> Result<ChannelGrants, PermissionDataError> {
        if self.snapshot.gaps.unknown(channel.id)
            || self.snapshot.gaps.rejected(CacheResource::Channels)
            || self.snapshot.dms.get(channel.id).is_some()
        {
            return Err(PermissionDataError::Unavailable);
        }
        let owner_uncertain = self.snapshot.gaps.owner_uncertain();
        if !owner_uncertain && self.owner? {
            return Ok(ChannelGrants {
                read: true,
                send: true,
                delete_others: true,
                voice: true,
            });
        }
        if self.snapshot.gaps.restriction_uncertain(channel.id) {
            return Err(PermissionDataError::Unavailable);
        }
        if channel.permissions.is_empty() {
            return Ok(ChannelGrants {
                read: true,
                send: true,
                delete_others: false,
                voice: true,
            });
        }
        if owner_uncertain || channel
            .permissions
            .iter()
            .any(|entry| self.snapshot.gaps.unconfirmed_role(entry.key.parsed()) || matches!(&entry.key, OverwriteKey::Canonical(role) if self.snapshot.gaps.assignment_uncertain(*role)))
        {
            return Err(PermissionDataError::Unavailable);
        }
        let assignments = self.assignments?;
        let mut grants = ChannelGrants::default();
        for entry in &channel.permissions {
            if let OverwriteKey::Canonical(id) = &entry.key
                && assignments.contains(id)
            {
                grants.read |= entry.read_channel;
                grants.send |= entry.send_messages.unwrap_or(false);
                grants.delete_others |= entry.delete_messages.unwrap_or(false);
                grants.voice |= entry.connect_voice.unwrap_or(false);
            }
        }
        Ok(grants)
    }
}

/// Evaluates a listed DM using current social evidence for both directions of the relationship.
/// # Errors
/// Returns unavailable for ambiguous channel identity or incomplete membership/block evidence.
pub fn direct_channel_grants(
    snapshot: &ResourceSnapshot,
    channel: Uuid,
) -> Result<ChannelGrants, PermissionDataError> {
    let dm = snapshot
        .dms
        .get(channel)
        .ok_or(PermissionDataError::Unavailable)?;
    if snapshot.gaps.dm_unknown(dm.peer_id)
        || snapshot.gaps.unknown(channel)
        || snapshot.channels.get(channel).is_some()
        || snapshot.gaps.rejected(CacheResource::Dms)
        || snapshot.gaps.rejected(CacheResource::Blocks)
        || snapshot.users.get(dm.peer_id).is_none()
    {
        return Err(PermissionDataError::Unavailable);
    }
    Ok(direct_grants(true, snapshot.blocks.contains(dm.peer_id)))
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

pub(crate) mod read_access;

#[cfg(test)]
mod tests;
