//! Converts server, role and channel responses. Overwrites keep their exact backend key.

use super::{Admission, AdmissionFault, MAX_CHANNELS, ResourceRejection, parse_id};
use crate::{
    CacheResource, ChannelKind, ChannelRecord, MessageHead, Overwrite, OverwriteKey, RoleFlags,
    RoleRecord, ServerRecord, Uuid,
};
use papo_api::wire;
use std::collections::HashSet;

impl Admission<'_> {
    pub fn server(self, raw: wire::Server) -> Result<ServerRecord, ResourceRejection> {
        let group = CacheResource::Server;
        Ok(ServerRecord {
            id: self.required_id(group, raw.id.as_deref())?,
            owner_id: self.optional_id(group, raw.owner_id.as_deref())?,
            name: raw.name,
            icon_format: raw.icon_format,
            public: raw.public,
            owner_username: raw.owner_username,
            created_at: raw.created_at,
            channel_count: raw.channel_count,
            member_count: raw.member_count,
            role_count: raw.role_count,
        })
    }

    pub fn channels(
        self,
        list: wire::ChannelList,
    ) -> Result<Vec<ChannelRecord>, ResourceRejection> {
        let group = CacheResource::Channels;
        let channels = list
            .channels
            .ok_or_else(|| self.reject(group, AdmissionFault::MissingList))?;
        if channels.len() > MAX_CHANNELS {
            return Err(self.reject(group, AdmissionFault::ExceedsLimit));
        }
        let mut seen = HashSet::with_capacity(channels.len());
        channels
            .into_iter()
            .map(|raw| {
                let id = self.required_id(group, raw.id.as_deref())?;
                if !seen.insert(id) {
                    return Err(self.reject(group, AdmissionFault::DuplicateId));
                }
                let kind = match raw.r#type.as_deref() {
                    Some("text") => ChannelKind::Text,
                    Some("category") => ChannelKind::Category,
                    Some("voice") => ChannelKind::Voice,
                    None | Some("") => return Err(self.reject(group, AdmissionFault::MissingField)),
                    _ => return Err(self.reject(group, AdmissionFault::UnsupportedValue)),
                };
                let entries = raw
                    .permissions
                    .ok_or_else(|| self.reject(group, AdmissionFault::MissingField))?;
                let mut keys = HashSet::with_capacity(entries.len());
                let mut permissions = Vec::with_capacity(entries.len());
                for entry in entries {
                    let key = entry
                        .role_id
                        .ok_or_else(|| self.reject(group, AdmissionFault::InvalidId))?;
                    let role = Uuid::try_parse(&key)
                        .map_err(|_| self.reject(group, AdmissionFault::InvalidId))?;
                    let key = if role.hyphenated().encode_lower(&mut Uuid::encode_buffer())
                        == key.as_str()
                    {
                        OverwriteKey::Canonical(role)
                    } else {
                        OverwriteKey::Noncanonical {
                            id: role,
                            spelling: key,
                        }
                    };
                    if !keys.insert(key.clone()) {
                        return Err(self.reject(group, AdmissionFault::DuplicateId));
                    }
                    let flags = entry
                        .permissions
                        .ok_or_else(|| self.reject(group, AdmissionFault::MissingField))?;
                    permissions.push(Overwrite {
                        key,
                        read_channel: flags
                            .read_channel
                            .ok_or_else(|| self.reject(group, AdmissionFault::MissingField))?,
                        send_messages: flags.send_messages,
                        delete_messages: flags.delete_messages,
                        connect_voice: flags.connect_voice,
                    });
                }
                Ok(ChannelRecord {
                    id,
                    kind,
                    permissions,
                    name: raw.name,
                    position: raw.position,
                    topic: raw.topic,
                    created_at: raw.created_at,
                    parent_id: self.optional_id(group, raw.parent_id.as_deref())?,
                    last_message: raw
                        .last_message
                        .map(|head| self.head(group, head))
                        .transpose()?,
                    last_read_message: self.optional_id(group, raw.last_read_message.as_deref())?,
                    last_read_at: raw.last_read_at,
                    notification_settings: raw.notification_settings.map(Into::into),
                })
            })
            .collect()
    }

    pub(super) fn head(
        self,
        group: CacheResource,
        raw: wire::ChannelLastMessage,
    ) -> Result<MessageHead, ResourceRejection> {
        let created_at = raw
            .created_at
            .ok_or_else(|| self.reject(group, AdmissionFault::MissingField))?;
        let created_at_us = crate::parse_message_timestamp_us(&created_at)
            .map_err(|_| self.reject(group, AdmissionFault::UnsupportedValue))?;
        Ok(MessageHead {
            id: self.required_id(group, raw.id.as_deref())?,
            author_id: self.optional_id(group, raw.author_id.as_deref())?,
            author_username: raw.author_username,
            content: raw.content,
            created_at,
            created_at_us,
        })
    }

    pub(super) fn optional_id(
        self,
        group: CacheResource,
        raw: Option<&str>,
    ) -> Result<Option<Uuid>, ResourceRejection> {
        raw.map(|value| self.required_id(group, Some(value)))
            .transpose()
    }

    pub(super) fn required_id(
        self,
        group: CacheResource,
        raw: Option<&str>,
    ) -> Result<Uuid, ResourceRejection> {
        parse_id(raw).ok_or_else(|| self.reject(group, AdmissionFault::InvalidId))
    }

    pub fn roles(self, list: wire::RoleListResponse) -> Result<Vec<RoleRecord>, ResourceRejection> {
        let group = CacheResource::Roles;
        let roles = list
            .roles
            .ok_or_else(|| self.reject(group, AdmissionFault::MissingList))?;
        let mut seen = HashSet::with_capacity(roles.len());
        roles
            .into_iter()
            .map(|role| {
                let id = self.required_id(group, role.id.as_deref())?;
                if !seen.insert(id) {
                    return Err(self.reject(group, AdmissionFault::DuplicateId));
                }
                let p = role
                    .permissions
                    .ok_or_else(|| self.reject(group, AdmissionFault::MissingField))?;
                Ok(RoleRecord {
                    id,
                    name: role.name,
                    color: role.color,
                    created_at: role.created_at,
                    permissions: RoleFlags {
                        manage_server: p.manage_server,
                        manage_channels: p.manage_channels,
                        manage_roles: p.manage_roles,
                        manage_webhooks: p.manage_webhooks,
                        ban_members: p.ban_members,
                        pin_message: p.pin_message,
                        everyone_message: p.everyone_message,
                        send_attachment: p.send_attachment,
                    },
                })
            })
            .collect()
    }
}
