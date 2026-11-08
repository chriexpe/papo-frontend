//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `Problem`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct Problem {
    /// Backend `type` field.
    pub r#type: String,
    /// Backend `title` field.
    pub r#title: String,
    /// Backend `status` field.
    pub r#status: i64,
    /// Backend `detail` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#detail: Option<String>,
    /// Backend `instance` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#instance: Option<String>,
}
