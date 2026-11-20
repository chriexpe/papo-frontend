//! Borrowed JSON rows readable by the v2 wire codec. Live fields and role labels have no disk slots.

use crate::{ChannelRecord, DirectRecord, MessageHead, ProfileRecord, UserRecord, Uuid};
use serde::{Serialize, Serializer};
use std::borrow::Cow;

#[derive(Serialize)]
struct RoleReference {
    id: Uuid,
}

struct Assignments<'a>(&'a [Uuid]);

impl Serialize for Assignments<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|&id| RoleReference { id }))
    }
}

#[derive(Serialize)]
pub(super) struct Summary<'a> {
    id: Uuid,
    username: Option<&'a str>,
    nickname: Option<&'a str>,
    created_at: Option<&'a str>,
    banned: Option<bool>,
    roles: Option<Assignments<'a>>,
}

pub(super) fn summary(id: Uuid, user: Option<&UserRecord>) -> Summary<'_> {
    Summary {
        id,
        username: user.and_then(|user| user.username.as_deref()),
        nickname: user.and_then(|user| user.nickname.as_deref()),
        created_at: user.and_then(|user| user.created_at.as_deref()),
        banned: user.and_then(|user| user.banned),
        roles: user.and_then(|user| user.roles.as_deref()).map(Assignments),
    }
}

#[derive(Serialize)]
struct Permissions {
    read_channel: bool,
    send_messages: Option<bool>,
    delete_messages: Option<bool>,
    connect_voice: Option<bool>,
}

#[derive(Serialize)]
struct Overwrite<'a> {
    role_id: Cow<'a, str>,
    permissions: Permissions,
}

#[derive(Serialize)]
struct Channel<'a> {
    id: Uuid,
    name: Option<&'a str>,
    r#type: &'a str,
    position: Option<i64>,
    topic: Option<&'a str>,
    parent_id: Option<Uuid>,
    permissions: Vec<Overwrite<'a>>,
    created_at: Option<&'a str>,
    last_message: Option<&'a MessageHead>,
    last_read_message: Option<Uuid>,
    last_read_at: Option<&'a str>,
    notification_settings: Option<&'a str>,
}

pub(super) fn channel(row: &ChannelRecord) -> impl Serialize + '_ {
    Channel {
        id: row.id,
        name: row.name.as_deref(),
        r#type: row.kind.as_str(),
        position: row.position,
        topic: row.topic.as_deref(),
        parent_id: row.parent_id,
        permissions: row
            .permissions
            .iter()
            .map(|entry| Overwrite {
                role_id: entry.key.text(),
                permissions: Permissions {
                    read_channel: entry.read_channel,
                    send_messages: entry.send_messages,
                    delete_messages: entry.delete_messages,
                    connect_voice: entry.connect_voice,
                },
            })
            .collect(),
        created_at: row.created_at.as_deref(),
        last_message: row.last_message.as_ref(),
        last_read_message: row.last_read_message,
        last_read_at: row.last_read_at.as_deref(),
        notification_settings: row
            .notification_settings
            .as_ref()
            .map(crate::BackendMode::as_str),
    }
}

#[derive(Serialize)]
struct Profile<'a> {
    #[serde(flatten)]
    summary: Summary<'a>,
    avatar_format: Option<&'a str>,
    banner_media: Option<&'a str>,
    description: Option<&'a str>,
}

pub(super) fn profile<'a>(
    detail: &'a ProfileRecord,
    user: Option<&'a UserRecord>,
) -> impl Serialize + 'a {
    Profile {
        summary: summary(detail.id, user),
        avatar_format: detail.avatar_format.as_deref(),
        banner_media: detail.banner_media.as_deref(),
        description: detail.description.as_deref(),
    }
}

#[derive(Serialize)]
struct Direct<'a> {
    id: Uuid,
    user: Summary<'a>,
    created_at: &'a str,
    last_message: Option<&'a MessageHead>,
    last_read_message: Option<Uuid>,
    last_read_at: Option<&'a str>,
    unread_count: i64,
    summary_unavailable: bool,
}

pub(super) fn direct<'a>(
    row: &'a DirectRecord,
    user: Option<&'a UserRecord>,
) -> impl Serialize + 'a {
    Direct {
        id: row.id,
        user: summary(row.peer_id, user),
        created_at: &row.created_at,
        last_message: row.last_message.as_ref(),
        last_read_message: row.last_read_message,
        last_read_at: row.last_read_at.as_deref(),
        unread_count: row.unread_count,
        summary_unavailable: user.is_none(),
    }
}

#[derive(Serialize)]
struct Block<'a> {
    #[serde(flatten)]
    user: Summary<'a>,
    summary_unavailable: bool,
}

pub(super) fn blocked(id: Uuid, user: Option<&UserRecord>) -> impl Serialize + '_ {
    Block {
        user: summary(id, user),
        summary_unavailable: user.is_none(),
    }
}
