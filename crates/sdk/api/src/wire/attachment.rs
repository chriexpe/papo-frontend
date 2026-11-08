//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Attachment`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Attachment {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `mime_type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#mime_type: Option<String>,
    /// Backend `original_file_name` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#original_file_name: Option<String>,
    /// Backend `size_bytes` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#size_bytes: Option<i64>,
    /// Backend `thumbnail_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#thumbnail_id: Option<String>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
    /// Backend `moderation_status` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#moderation_status: Option<String>,
}
