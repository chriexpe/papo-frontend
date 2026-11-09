//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `SearchResponse`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SearchResponse {
    /// Backend `results` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#results: Option<Vec<super::SearchResult>>,
    /// Backend `has_more` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#has_more: Option<bool>,
}
