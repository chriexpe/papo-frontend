//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `DirectConversationList`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct DirectConversationList {
    /// Backend `dms` field.
    pub r#dms: Vec<super::DirectConversation>,
}
