//! Admission of server metadata, channels with their overwrites, and role definitions;
//! people, DMs and blocks live in the sibling `people` module.

use super::{Admission, AdmissionFault, CHANNEL_LIST_CAP, ResourceRejection, parse_id};
use crate::{CacheResource, Uuid};
use papo_api::wire::{
    Channel, ChannelList, ChannelPermissionEntry, Role, RoleListResponse, Server,
};
use std::collections::HashSet;

impl Admission<'_> {
    /// Admits `GET /server`. A null `owner_id` is the backend's "no owner" and grants nothing.
    /// # Errors
    /// Rejects `Server` for an invalid server ID or a present but invalid owner ID.
    pub fn server(self, server: Server) -> Result<Server, ResourceRejection> {
        if parse_id(server.id.as_deref()).is_none() {
            return Err(self.reject(CacheResource::Server, AdmissionFault::InvalidId));
        }
        if server.owner_id.is_some() && parse_id(server.owner_id.as_deref()).is_none() {
            return Err(self.reject(CacheResource::Server, AdmissionFault::InvalidId));
        }
        Ok(server)
    }

    /// Admits the complete `GET /channels` replacement, including every nested overwrite.
    /// # Errors
    /// Rejects `Channels` for an absent list, a list at the backend cap, or any
    /// malformed channel or overwrite; no channel is ever dropped individually.
    pub fn channels(self, list: ChannelList) -> Result<Vec<Channel>, ResourceRejection> {
        let channels = list
            .channels
            .ok_or_else(|| self.reject(CacheResource::Channels, AdmissionFault::MissingList))?;
        if channels.len() >= CHANNEL_LIST_CAP {
            return Err(self.reject(CacheResource::Channels, AdmissionFault::Incomplete));
        }
        let mut seen = HashSet::with_capacity(channels.len());
        for channel in &channels {
            let fault = channel_fault(channel, &mut seen);
            if let Some(fault) = fault {
                return Err(self.reject(CacheResource::Channels, fault));
            }
        }
        Ok(channels)
    }

    /// Admits the complete `GET /roles` replacement.
    /// # Errors
    /// Rejects `Roles` for an absent list or a role without a unique ID or permissions.
    pub fn roles(self, list: RoleListResponse) -> Result<Vec<Role>, ResourceRejection> {
        let roles = list
            .roles
            .ok_or_else(|| self.reject(CacheResource::Roles, AdmissionFault::MissingList))?;
        let mut seen = HashSet::with_capacity(roles.len());
        for role in &roles {
            let Some(id) = parse_id(role.id.as_deref()) else {
                return Err(self.reject(CacheResource::Roles, AdmissionFault::InvalidId));
            };
            if !seen.insert(id) {
                return Err(self.reject(CacheResource::Roles, AdmissionFault::DuplicateId));
            }
            if role.permissions.is_none() {
                return Err(self.reject(CacheResource::Roles, AdmissionFault::MissingField));
            }
        }
        Ok(roles)
    }
}

fn channel_fault(channel: &Channel, seen: &mut HashSet<Uuid>) -> Option<AdmissionFault> {
    let Some(id) = parse_id(channel.id.as_deref()) else {
        return Some(AdmissionFault::InvalidId);
    };
    if !seen.insert(id) {
        return Some(AdmissionFault::DuplicateId);
    }
    if channel.r#type.as_deref().is_none_or(str::is_empty) {
        return Some(AdmissionFault::MissingField);
    }
    // An absent overwrite list is unknown access; only an explicit `[]` means open.
    let Some(overwrites) = &channel.permissions else {
        return Some(AdmissionFault::MissingField);
    };
    let mut roles: HashSet<&str> = HashSet::with_capacity(overwrites.len());
    overwrites
        .iter()
        .find_map(|overwrite| overwrite_fault(overwrite, &mut roles))
}

/// The backend stores an overwrite under the role ID text its route received, after
/// Postgres accepted any UUID spelling, so a valid overwrite can be non-canonical. Such
/// an entry matches no role on the backend or here (both compare text), so it is
/// admitted verbatim: dropping or canonicalizing it would change who may read.
fn overwrite_fault<'a>(
    overwrite: &'a ChannelPermissionEntry,
    roles: &mut HashSet<&'a str>,
) -> Option<AdmissionFault> {
    let Some(role) = overwrite.role_id.as_deref() else {
        return Some(AdmissionFault::InvalidId);
    };
    if Uuid::try_parse(role).is_err() {
        return Some(AdmissionFault::InvalidId);
    }
    // JSONB object keys are unique text, so only a textual repeat is malformed.
    if !roles.insert(role) {
        return Some(AdmissionFault::DuplicateId);
    }
    let read = overwrite
        .permissions
        .as_ref()
        .and_then(|permissions| permissions.read_channel);
    if read.is_none() {
        return Some(AdmissionFault::MissingField);
    }
    None
}
