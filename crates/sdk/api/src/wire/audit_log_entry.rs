//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `AuditLogEntry`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct AuditLogEntry {
    /// Backend `id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#id: Option<String>,
    /// Backend `actor_username` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#actor_username: Option<String>,
    /// Backend `action` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#action: Option<String>,
    /// Backend `entity_type` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#entity_type: Option<String>,
    /// Backend `target_user_id` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#target_user_id: Option<String>,
    /// Backend `metadata` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#metadata: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    /// Backend `created_at` field. Timestamp strings are preserved verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#created_at: Option<String>,
}
