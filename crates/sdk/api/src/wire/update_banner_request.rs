//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `UpdateBannerRequest`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct UpdateBannerRequest {
    /// Backend `banner` field.
    pub r#banner: String,
    /// Backend `banner_format` field.
    pub r#banner_format: String,
}
