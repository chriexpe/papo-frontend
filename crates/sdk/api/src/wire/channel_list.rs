//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `ChannelList`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ChannelList {
    /// Backend `channels` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#channels: Option<Vec<super::Channel>>,
}
