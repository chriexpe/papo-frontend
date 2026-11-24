//! Effective voice-connect decision per channel for one account, derived from one admitted
//! resource snapshot through the shared permission evaluator. Never uses `read_channel`.

use papo_api::Uuid;

use super::{AccountEvidence, PermissionDataError};
use crate::{CacheResource, ChannelKind, ResourceSnapshot};

/// What admitted resource evidence says about joining or watching a voice channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoiceAccess {
    /// Admitted evidence confirms this is a voice channel and grants `connect_voice`.
    Allowed,
    /// Admitted evidence confirms this channel exists but is not a voice channel (or is a DM).
    NotVoiceChannel,
    /// Admitted evidence confirms the channel is missing or denies `connect_voice`.
    Denied,
    /// Required channel, role, owner or member evidence is absent or uncertain.
    Unknown,
}

/// Evaluates admitted voice access for `channel` and `user`, independent of `read_channel`.
#[must_use]
pub fn voice_access(snapshot: &ResourceSnapshot, user: Uuid, channel: Uuid) -> VoiceAccess {
    if snapshot.dms.get(channel).is_some() {
        return VoiceAccess::NotVoiceChannel;
    }
    if snapshot.gaps.rejected(CacheResource::Channels) {
        return VoiceAccess::Unknown;
    }
    let Some(record) = snapshot.channels.get(channel) else {
        return if snapshot.channel_census_complete {
            VoiceAccess::Denied
        } else {
            VoiceAccess::Unknown
        };
    };
    if record.kind != ChannelKind::Voice {
        return VoiceAccess::NotVoiceChannel;
    }
    let evidence = AccountEvidence::new(snapshot, user);
    match evidence.channel_grants(record) {
        Ok(grants) if grants.voice => VoiceAccess::Allowed,
        Ok(_) => VoiceAccess::Denied,
        Err(PermissionDataError::Unavailable) => VoiceAccess::Unknown,
    }
}

#[cfg(test)]
mod tests {
    //! Verifies voice access is independent of read access and fails closed on unknown evidence.

    use super::*;
    use crate::{
        ChannelRecord, Generation, Overwrite, OverwriteKey, RoleFlags, RoleRecord, ServerRecord,
        UserRecord,
    };

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn base_snapshot(user: Uuid, role: Uuid, channel: ChannelRecord) -> ResourceSnapshot {
        let mut snapshot = ResourceSnapshot::empty(Generation::for_test(1));
        snapshot.channel_census_complete = true;
        snapshot.server = Some(std::sync::Arc::new(ServerRecord {
            id: id(1),
            name: None,
            icon_format: None,
            owner_id: Some(id(99)),
            public: Some(true),
            owner_username: None,
            created_at: None,
            channel_count: None,
            member_count: None,
            role_count: None,
        }));
        snapshot.roles = crate::ResourceTable::from_rows([(
            role,
            RoleRecord {
                id: role,
                name: None,
                color: None,
                permissions: RoleFlags {
                    manage_server: None,
                    manage_channels: None,
                    manage_roles: None,
                    manage_webhooks: None,
                    ban_members: None,
                    pin_message: None,
                    everyone_message: None,
                    send_attachment: None,
                },
                created_at: None,
            },
        )]);
        snapshot.users.set(
            user,
            UserRecord {
                id: user,
                username: Some("member".into()),
                nickname: None,
                banned: None,
                typing: None,
                status_message: None,
                created_at: None,
                roles: Some(vec![role]),
            },
        );
        snapshot.members.replace([user].into_iter());
        snapshot.channels = crate::ResourceTable::from_rows([(channel.id, channel)]);
        snapshot
    }

    fn voice_channel(
        channel_id: Uuid,
        kind: ChannelKind,
        overwrites: Vec<Overwrite>,
    ) -> ChannelRecord {
        ChannelRecord {
            id: channel_id,
            name: Some("voice".into()),
            kind,
            position: Some(1),
            topic: None,
            parent_id: None,
            permissions: overwrites,
            created_at: None,
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            notification_settings: None,
        }
    }

    #[test]
    fn voice_granted_without_read_access_is_allowed() {
        let (user, role, chan) = (id(10), id(20), id(30));
        let channel = voice_channel(
            chan,
            ChannelKind::Voice,
            vec![Overwrite {
                key: OverwriteKey::Canonical(role),
                read_channel: false,
                send_messages: Some(false),
                delete_messages: Some(false),
                connect_voice: Some(true),
            }],
        );
        let snapshot = base_snapshot(user, role, channel);
        assert_eq!(voice_access(&snapshot, user, chan), VoiceAccess::Allowed);
    }

    #[test]
    fn read_without_voice_is_denied_and_text_channel_is_not_voice() {
        let (user, role, chan) = (id(10), id(20), id(30));
        let channel = voice_channel(
            chan,
            ChannelKind::Voice,
            vec![Overwrite {
                key: OverwriteKey::Canonical(role),
                read_channel: true,
                send_messages: Some(true),
                delete_messages: Some(false),
                connect_voice: Some(false),
            }],
        );
        let snapshot = base_snapshot(user, role, channel);
        assert_eq!(voice_access(&snapshot, user, chan), VoiceAccess::Denied);

        let text = voice_channel(chan, ChannelKind::Text, Vec::new());
        let snapshot = base_snapshot(user, role, text);
        assert_eq!(
            voice_access(&snapshot, user, chan),
            VoiceAccess::NotVoiceChannel
        );
    }

    #[test]
    fn missing_census_or_missing_role_evidence_is_unknown() {
        let (user, role, chan) = (id(10), id(20), id(30));
        let channel = voice_channel(
            chan,
            ChannelKind::Voice,
            vec![Overwrite {
                key: OverwriteKey::Canonical(role),
                read_channel: false,
                send_messages: None,
                delete_messages: None,
                connect_voice: Some(true),
            }],
        );
        let mut snapshot = base_snapshot(user, role, channel);
        snapshot.roles = crate::ResourceTable::default();
        assert_eq!(voice_access(&snapshot, user, chan), VoiceAccess::Unknown);

        let empty = ResourceSnapshot::empty(Generation::for_test(1));
        assert_eq!(voice_access(&empty, user, chan), VoiceAccess::Unknown);
    }
}
