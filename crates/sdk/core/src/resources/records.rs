//! Canonical resource values. Relationships carry UUIDs, while role labels live on definitions.

use crate::Uuid;
use std::borrow::Cow;

/// Ordinary channel kinds returned by the pinned channel-list endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    /// Text messages.
    Text,
    /// Channel grouping.
    Category,
    /// Voice room.
    Voice,
}

impl ChannelKind {
    /// Backend spelling used for serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Category => "category",
            Self::Voice => "voice",
        }
    }
}

/// Channel notification setting. A missing setting is represented separately by None.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendMode {
    /// Backend suppresses delivery.
    Off,
    /// Backend enables mention triggers.
    OnlyMentions,
    /// Backend enables ordinary messages.
    All,
    /// Unrecognized backend spelling.
    Unknown(String),
}

impl BackendMode {
    /// Preserves unknown values across cache revisions.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Off => "off",
            Self::OnlyMentions => "only_mentions",
            Self::All => "all",
            Self::Unknown(raw) => raw,
        }
    }
}

impl From<String> for BackendMode {
    fn from(raw: String) -> Self {
        match raw.as_str() {
            "off" => Self::Off,
            "only_mentions" => Self::OnlyMentions,
            "all" => Self::All,
            _ => Self::Unknown(raw),
        }
    }
}

/// JSON object key used by the backend for a role overwrite.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OverwriteKey {
    /// Matches the text of an assigned UUID.
    Canonical(Uuid),
    /// Valid UUID whose exact spelling cannot match the canonical assignment.
    Noncanonical {
        /// Parsed identity for deletion-dependency tracking.
        id: Uuid,
        /// Original JSON key.
        spelling: String,
    },
}

impl OverwriteKey {
    /// Corresponding role UUID irrespective of spelling.
    #[must_use]
    pub const fn parsed(&self) -> Uuid {
        match self {
            Self::Canonical(id) | Self::Noncanonical { id, .. } => *id,
        }
    }
    /// Original backend key for cache encoding.
    #[must_use]
    pub fn text(&self) -> Cow<'_, str> {
        match self {
            Self::Canonical(id) => Cow::Owned(id.to_string()),
            Self::Noncanonical { spelling, .. } => Cow::Borrowed(spelling),
        }
    }
}

/// An admitted channel overwrite with a required read decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overwrite {
    /// Exact backend role key.
    pub key: OverwriteKey,
    /// Required read bit.
    pub read_channel: bool,
    /// Optional send bit.
    pub send_messages: Option<bool>,
    /// Optional moderation bit.
    pub delete_messages: Option<bool>,
    /// Optional voice bit.
    pub connect_voice: Option<bool>,
}

/// Last message reported in channel metadata.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct MessageHead {
    /// Backend `id` field.
    pub id: Uuid,
    /// Backend `content` field.
    pub content: Option<String>,
    /// Backend `author_id` field.
    pub author_id: Option<Uuid>,
    /// Backend `author_username` field.
    pub author_username: Option<String>,
    /// Backend `created_at` field.
    pub created_at: String,
    /// Admitted order timestamp, in microseconds since the Unix epoch.
    #[serde(skip)]
    pub created_at_us: i64,
}

/// Admitted backend server metadata.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ServerRecord {
    /// Backend `id` field.
    pub id: Uuid,
    /// Backend `name` field.
    pub name: Option<String>,
    /// Backend `icon_format` field.
    pub icon_format: Option<String>,
    /// Backend `owner_id` field.
    pub owner_id: Option<Uuid>,
    /// Backend `public` field.
    pub public: Option<bool>,
    /// Backend `owner_username` field.
    pub owner_username: Option<String>,
    /// Backend `created_at` field.
    pub created_at: Option<String>,
    /// Backend `channel_count` field.
    pub channel_count: Option<i64>,
    /// Backend `member_count` field.
    pub member_count: Option<i64>,
    /// Backend `role_count` field.
    pub role_count: Option<i64>,
}

/// Optional role grant bits as observed from the backend.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RoleFlags {
    /// Backend `manage_server` field.
    pub manage_server: Option<bool>,
    /// Backend `manage_channels` field.
    pub manage_channels: Option<bool>,
    /// Backend `manage_roles` field.
    pub manage_roles: Option<bool>,
    /// Backend `manage_webhooks` field.
    pub manage_webhooks: Option<bool>,
    /// Backend `ban_members` field.
    pub ban_members: Option<bool>,
    /// Backend `pin_message` field.
    pub pin_message: Option<bool>,
    /// Backend `everyone_message` field.
    pub everyone_message: Option<bool>,
    /// Backend `send_attachment` field.
    pub send_attachment: Option<bool>,
}

/// Sole source of role labels and global grant flags.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RoleRecord {
    /// Backend `id` field.
    pub id: Uuid,
    /// Backend `name` field.
    pub name: Option<String>,
    /// Backend `color` field.
    pub color: Option<String>,
    /// Backend `permissions` field.
    pub permissions: RoleFlags,
    /// Backend `created_at` field.
    pub created_at: Option<String>,
}

/// Shared person summary. Only roster evidence establishes membership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserRecord {
    /// Backend `id` field.
    pub id: Uuid,
    /// Backend `username` field.
    pub username: Option<String>,
    /// Backend `nickname` field.
    pub nickname: Option<String>,
    /// Backend `banned` field.
    pub banned: Option<bool>,
    /// Backend `typing` field.
    pub typing: Option<String>,
    /// Backend `created_at` field.
    pub created_at: Option<String>,
    /// Backend `roles` field.
    pub roles: Option<Vec<Uuid>>,
}

/// Details from a requested full profile. Names and assignments resolve through the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRecord {
    /// Backend `id` field.
    pub id: Uuid,
    /// Backend `avatar_format` field.
    pub avatar_format: Option<String>,
    /// Backend `banner_media` field.
    pub banner_media: Option<String>,
    /// Backend `description` field.
    pub description: Option<String>,
}

/// Admitted ordinary channel and its complete overwrite list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRecord {
    /// Backend `id` field.
    pub id: Uuid,
    /// Backend `name` field.
    pub name: Option<String>,
    /// Backend `type` field.
    pub kind: ChannelKind,
    /// Backend `position` field.
    pub position: Option<i64>,
    /// Backend `topic` field.
    pub topic: Option<String>,
    /// Backend `parent_id` field.
    pub parent_id: Option<Uuid>,
    /// Backend `permissions` field.
    pub permissions: Vec<Overwrite>,
    /// Backend `created_at` field.
    pub created_at: Option<String>,
    /// Backend `last_message` field.
    pub last_message: Option<MessageHead>,
    /// Backend `last_read_message` field.
    pub last_read_message: Option<Uuid>,
    /// Backend `last_read_at` field.
    pub last_read_at: Option<String>,
    /// Backend `notification_settings` field.
    pub notification_settings: Option<BackendMode>,
}

/// A direct conversation referencing a canonical peer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectRecord {
    /// Backend `id` field.
    pub id: Uuid,
    /// Backend `user` field.
    pub peer_id: Uuid,
    /// Backend `created_at` field.
    pub created_at: String,
    /// Backend `last_message` field.
    pub last_message: Option<MessageHead>,
    /// Backend `last_read_message` field.
    pub last_read_message: Option<Uuid>,
    /// Backend `last_read_at` field.
    pub last_read_at: Option<String>,
    /// Backend `unread_count` field.
    pub unread_count: i64,
}
