//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Message`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Message {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `channel_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channel_id: Option<String>,
    /// Backend `author_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#author_id: Option<String>,
    /// Backend `content` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#content: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `edited_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#edited_at: Option<String>,
    /// Backend `reply_to` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#reply_to: Option<String>,
    /// Backend `webhook_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#webhook_id: Option<String>,
    /// Backend `external_author` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#external_author: Option<super::ExternalAuthor>,
    /// Backend `attachments` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#attachments: Option<Vec<super::Attachment>>,
    /// Backend `embeds` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#embeds: Option<Vec<super::Embed>>,
    /// Backend `reactions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#reactions: Option<Vec<super::MessageReactionSummary>>,
    /// Backend `user_reactions` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#user_reactions: Option<Vec<super::MessageUserReaction>>,
}
