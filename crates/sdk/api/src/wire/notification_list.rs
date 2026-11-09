//! Generated wire record; transport and policy belong to other modules.

/// Wire record for `NotificationList`. Optional fields follow the pinned `OpenAPI` contract.
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct NotificationList {
    /// Backend `notifications` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#notifications: Option<Vec<super::Notification>>,
    /// Backend `has_more` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#has_more: Option<bool>,
}
