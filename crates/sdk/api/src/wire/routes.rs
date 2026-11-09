//! Generated REST operation table; transport and policy belong to other modules.

#![allow(
    dead_code,
    reason = "generated operation metadata is checked by drift tests"
)]

/// Backend query parameter `action`.
pub(crate) const ACTION: &str = "action";

/// Backend query parameter `actor_id`.
pub(crate) const ACTOR_ID: &str = "actor_id";

/// Backend query parameter `entity_type`.
pub(crate) const ENTITY_TYPE: &str = "entity_type";

/// Backend query parameter `last_id`.
pub(crate) const LAST_ID: &str = "last_id";

/// Backend query parameter `limit`.
pub(crate) const LIMIT: &str = "limit";

/// Backend query parameter `order`.
pub(crate) const ORDER: &str = "order";

/// Backend query parameter `since`.
pub(crate) const SINCE: &str = "since";

/// Backend query parameter `until`.
pub(crate) const UNTIL: &str = "until";

/// One pinned backend operation and its transport shape.
pub(crate) struct Route {
    /// HTTP method.
    pub(crate) method: reqwest::Method,
    /// Path template; `{name}` placeholders are filled by [`Route::url`].
    pub(crate) path: &'static str,
    /// Accepted query parameter names.
    pub(crate) query: &'static [&'static str],
    /// Request body content type, when the operation accepts a body.
    pub(crate) request_body: Option<&'static str>,
    /// Successful HTTP statuses.
    pub(crate) success: &'static [u16],
}

impl Route {
    /// Expands path placeholders in their template order. Missing values are left empty.
    pub(crate) fn url(&self, values: &[String]) -> String {
        let mut url = String::new();
        let mut rest = self.path;
        let mut values = values.iter();
        while let Some(start) = rest.find('{') {
            url.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}') else {
                break;
            };
            let end = start + end;
            if let Some(value) = values.next() {
                url.push_str(value);
            }
            rest = &rest[end + 1..];
        }
        url.push_str(rest);
        url
    }
}

/// `GET /health`.
pub(crate) const GET_HEALTH: Route = Route {
    method: reqwest::Method::GET,
    path: "/health",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /auth/register`.
pub(crate) const POST_AUTH_REGISTER: Route = Route {
    method: reqwest::Method::POST,
    path: "/auth/register",
    query: &[],
    request_body: Some("application/json"),
    success: &[201],
};

/// `POST /auth/login`.
pub(crate) const POST_AUTH_LOGIN: Route = Route {
    method: reqwest::Method::POST,
    path: "/auth/login",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `POST /auth/login_server`.
pub(crate) const POST_AUTH_LOGIN_SERVER: Route = Route {
    method: reqwest::Method::POST,
    path: "/auth/login_server",
    query: &[],
    request_body: Some("application/json"),
    success: &[204],
};

/// `GET /auth/whoami`.
pub(crate) const GET_AUTH_WHOAMI: Route = Route {
    method: reqwest::Method::GET,
    path: "/auth/whoami",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /auth/logout`.
pub(crate) const POST_AUTH_LOGOUT: Route = Route {
    method: reqwest::Method::POST,
    path: "/auth/logout",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `POST /auth/refresh`.
pub(crate) const POST_AUTH_REFRESH: Route = Route {
    method: reqwest::Method::POST,
    path: "/auth/refresh",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `GET /auth/connected_devices`.
pub(crate) const GET_AUTH_CONNECTED_DEVICES: Route = Route {
    method: reqwest::Method::GET,
    path: "/auth/connected_devices",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /auth/drop_connection`.
pub(crate) const POST_AUTH_DROP_CONNECTION: Route = Route {
    method: reqwest::Method::POST,
    path: "/auth/drop_connection",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `POST /auth/password_reset`.
pub(crate) const POST_AUTH_PASSWORD_RESET: Route = Route {
    method: reqwest::Method::POST,
    path: "/auth/password_reset",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `GET /users`.
pub(crate) const GET_USERS: Route = Route {
    method: reqwest::Method::GET,
    path: "/users",
    query: &[SINCE, LAST_ID, ORDER],
    request_body: None,
    success: &[200],
};

/// `GET /users/{user_id}/profile`.
pub(crate) const GET_USERS_USER_ID_PROFILE: Route = Route {
    method: reqwest::Method::GET,
    path: "/users/{user_id}/profile",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /users/profile_batch`.
pub(crate) const POST_USERS_PROFILE_BATCH: Route = Route {
    method: reqwest::Method::POST,
    path: "/users/profile_batch",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `POST /users/user_summary_batch`.
pub(crate) const POST_USERS_USER_SUMMARY_BATCH: Route = Route {
    method: reqwest::Method::POST,
    path: "/users/user_summary_batch",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `GET /users/blocks`.
pub(crate) const GET_USERS_BLOCKS: Route = Route {
    method: reqwest::Method::GET,
    path: "/users/blocks",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /users/{user_id}/block`.
pub(crate) const POST_USERS_USER_ID_BLOCK: Route = Route {
    method: reqwest::Method::POST,
    path: "/users/{user_id}/block",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `DELETE /users/{user_id}/block`.
pub(crate) const DELETE_USERS_USER_ID_BLOCK: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/users/{user_id}/block",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `PUT /users/settings`.
pub(crate) const PUT_USERS_SETTINGS: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/settings",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PUT /users/{user_id}`.
pub(crate) const PUT_USERS_USER_ID: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/{user_id}",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `GET /users/{user_id}/avatar`.
pub(crate) const GET_USERS_USER_ID_AVATAR: Route = Route {
    method: reqwest::Method::GET,
    path: "/users/{user_id}/avatar",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `PUT /users/{user_id}/avatar`.
pub(crate) const PUT_USERS_USER_ID_AVATAR: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/{user_id}/avatar",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PUT /users/{user_id}/banner`.
pub(crate) const PUT_USERS_USER_ID_BANNER: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/{user_id}/banner",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PUT /users/{user_id}/status`.
pub(crate) const PUT_USERS_USER_ID_STATUS: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/{user_id}/status",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PUT /users/{user_id}/password`.
pub(crate) const PUT_USERS_USER_ID_PASSWORD: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/{user_id}/password",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PUT /users/{user_id}/ban`.
pub(crate) const PUT_USERS_USER_ID_BAN: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/{user_id}/ban",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `POST /users/{user_id}/reset`.
pub(crate) const POST_USERS_USER_ID_RESET: Route = Route {
    method: reqwest::Method::POST,
    path: "/users/{user_id}/reset",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `PUT /users/me/push-device`.
pub(crate) const PUT_USERS_ME_PUSH_DEVICE: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/me/push-device",
    query: &[],
    request_body: Some("application/json"),
    success: &[204],
};

/// `DELETE /users/me/push-device`.
pub(crate) const DELETE_USERS_ME_PUSH_DEVICE: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/users/me/push-device",
    query: &[],
    request_body: Some("application/json"),
    success: &[204],
};

/// `POST /users/{user_id}/roles`.
pub(crate) const POST_USERS_USER_ID_ROLES: Route = Route {
    method: reqwest::Method::POST,
    path: "/users/{user_id}/roles",
    query: &[],
    request_body: Some("application/json"),
    success: &[201],
};

/// `DELETE /users/{user_id}/roles/{role_id}`.
pub(crate) const DELETE_USERS_USER_ID_ROLES_ROLE_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/users/{user_id}/roles/{role_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `GET /users/{user_id}/notifications`.
pub(crate) const GET_USERS_USER_ID_NOTIFICATIONS: Route = Route {
    method: reqwest::Method::GET,
    path: "/users/{user_id}/notifications",
    query: &[SINCE, LAST_ID, ORDER],
    request_body: None,
    success: &[200],
};

/// `PUT /users/{user_id}/read_notification`.
pub(crate) const PUT_USERS_USER_ID_READ_NOTIFICATION: Route = Route {
    method: reqwest::Method::PUT,
    path: "/users/{user_id}/read_notification",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `GET /dms`.
pub(crate) const GET_DMS: Route = Route {
    method: reqwest::Method::GET,
    path: "/dms",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /dms`.
pub(crate) const POST_DMS: Route = Route {
    method: reqwest::Method::POST,
    path: "/dms",
    query: &[],
    request_body: Some("application/json"),
    success: &[200, 201],
};

/// `GET /dms/{dm_id}`.
pub(crate) const GET_DMS_DM_ID: Route = Route {
    method: reqwest::Method::GET,
    path: "/dms/{dm_id}",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `DELETE /dms/{dm_id}`.
pub(crate) const DELETE_DMS_DM_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/dms/{dm_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `GET /server`.
pub(crate) const GET_SERVER: Route = Route {
    method: reqwest::Method::GET,
    path: "/server",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /server`.
pub(crate) const POST_SERVER: Route = Route {
    method: reqwest::Method::POST,
    path: "/server",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PUT /server`.
pub(crate) const PUT_SERVER: Route = Route {
    method: reqwest::Method::PUT,
    path: "/server",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PATCH /server`.
pub(crate) const PATCH_SERVER: Route = Route {
    method: reqwest::Method::PATCH,
    path: "/server",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `GET /roles`.
pub(crate) const GET_ROLES: Route = Route {
    method: reqwest::Method::GET,
    path: "/roles",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /roles`.
pub(crate) const POST_ROLES: Route = Route {
    method: reqwest::Method::POST,
    path: "/roles",
    query: &[],
    request_body: Some("application/json"),
    success: &[201],
};

/// `PUT /roles/{role_id}`.
pub(crate) const PUT_ROLES_ROLE_ID: Route = Route {
    method: reqwest::Method::PUT,
    path: "/roles/{role_id}",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `DELETE /roles/{role_id}`.
pub(crate) const DELETE_ROLES_ROLE_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/roles/{role_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `GET /channels`.
pub(crate) const GET_CHANNELS: Route = Route {
    method: reqwest::Method::GET,
    path: "/channels",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /channels`.
pub(crate) const POST_CHANNELS: Route = Route {
    method: reqwest::Method::POST,
    path: "/channels",
    query: &[],
    request_body: Some("application/json"),
    success: &[201],
};

/// `PUT /channels/{channel_id}`.
pub(crate) const PUT_CHANNELS_CHANNEL_ID: Route = Route {
    method: reqwest::Method::PUT,
    path: "/channels/{channel_id}",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `DELETE /channels/{channel_id}`.
pub(crate) const DELETE_CHANNELS_CHANNEL_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/channels/{channel_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `PUT /channels/{channel_id}/change_position`.
pub(crate) const PUT_CHANNELS_CHANNEL_ID_CHANGE_POSITION: Route = Route {
    method: reqwest::Method::PUT,
    path: "/channels/{channel_id}/change_position",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `GET /channels/{channel_id}/permissions`.
pub(crate) const GET_CHANNELS_CHANNEL_ID_PERMISSIONS: Route = Route {
    method: reqwest::Method::GET,
    path: "/channels/{channel_id}/permissions",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `PUT /channels/{channel_id}/permissions/{role_id}`.
pub(crate) const PUT_CHANNELS_CHANNEL_ID_PERMISSIONS_ROLE_ID: Route = Route {
    method: reqwest::Method::PUT,
    path: "/channels/{channel_id}/permissions/{role_id}",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `DELETE /channels/{channel_id}/role/{role_id}`.
pub(crate) const DELETE_CHANNELS_CHANNEL_ID_ROLE_ROLE_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/channels/{channel_id}/role/{role_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `POST /channels/{channel_id}/user/{user_id}/settings`.
pub(crate) const POST_CHANNELS_CHANNEL_ID_USER_USER_ID_SETTINGS: Route = Route {
    method: reqwest::Method::POST,
    path: "/channels/{channel_id}/user/{user_id}/settings",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `PUT /channels/{channel_id}/read`.
pub(crate) const PUT_CHANNELS_CHANNEL_ID_READ: Route = Route {
    method: reqwest::Method::PUT,
    path: "/channels/{channel_id}/read",
    query: &[],
    request_body: Some("application/json"),
    success: &[204],
};

/// `GET /voice/ice-servers`.
pub(crate) const GET_VOICE_ICE_SERVERS: Route = Route {
    method: reqwest::Method::GET,
    path: "/voice/ice-servers",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `GET /channels/{channel_id}/messages`.
pub(crate) const GET_CHANNELS_CHANNEL_ID_MESSAGES: Route = Route {
    method: reqwest::Method::GET,
    path: "/channels/{channel_id}/messages",
    query: &[SINCE, LAST_ID, ORDER, LIMIT],
    request_body: None,
    success: &[200],
};

/// `POST /messages`.
pub(crate) const POST_MESSAGES: Route = Route {
    method: reqwest::Method::POST,
    path: "/messages",
    query: &[],
    request_body: Some("multipart/form-data"),
    success: &[201],
};

/// `PUT /messages/{message_id}`.
pub(crate) const PUT_MESSAGES_MESSAGE_ID: Route = Route {
    method: reqwest::Method::PUT,
    path: "/messages/{message_id}",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `DELETE /messages/{message_id}`.
pub(crate) const DELETE_MESSAGES_MESSAGE_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/messages/{message_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `POST /channels/{channel_id}/messages/{message_id}/pin`.
pub(crate) const POST_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_PIN: Route = Route {
    method: reqwest::Method::POST,
    path: "/channels/{channel_id}/messages/{message_id}/pin",
    query: &[],
    request_body: None,
    success: &[200, 201],
};

/// `DELETE /channels/{channel_id}/messages/{message_id}/pin`.
pub(crate) const DELETE_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_PIN: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/channels/{channel_id}/messages/{message_id}/pin",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `GET /channels/{channel_id}/pinned`.
pub(crate) const GET_CHANNELS_CHANNEL_ID_PINNED: Route = Route {
    method: reqwest::Method::GET,
    path: "/channels/{channel_id}/pinned",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `GET /channels/{channel_id}/messages/{message_id}/reactions`.
pub(crate) const GET_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_REACTIONS: Route = Route {
    method: reqwest::Method::GET,
    path: "/channels/{channel_id}/messages/{message_id}/reactions",
    query: &[SINCE, LAST_ID, ORDER],
    request_body: None,
    success: &[200],
};

/// `POST /channels/{channel_id}/messages/{message_id}/reactions`.
pub(crate) const POST_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_REACTIONS: Route = Route {
    method: reqwest::Method::POST,
    path: "/channels/{channel_id}/messages/{message_id}/reactions",
    query: &[],
    request_body: Some("application/json"),
    success: &[200, 201],
};

/// `DELETE /channels/{channel_id}/messages/{message_id}/reactions`.
pub(crate) const DELETE_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_REACTIONS: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/channels/{channel_id}/messages/{message_id}/reactions",
    query: &[],
    request_body: Some("application/json"),
    success: &[204],
};

/// `GET /emojis`.
pub(crate) const GET_EMOJIS: Route = Route {
    method: reqwest::Method::GET,
    path: "/emojis",
    query: &[SINCE, LAST_ID, ORDER],
    request_body: None,
    success: &[200],
};

/// `POST /emojis`.
pub(crate) const POST_EMOJIS: Route = Route {
    method: reqwest::Method::POST,
    path: "/emojis",
    query: &[],
    request_body: Some("application/json"),
    success: &[201],
};

/// `DELETE /emojis/{emoji_id}`.
pub(crate) const DELETE_EMOJIS_EMOJI_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/emojis/{emoji_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `GET /attachments/{file_id}`.
pub(crate) const GET_ATTACHMENTS_FILE_ID: Route = Route {
    method: reqwest::Method::GET,
    path: "/attachments/{file_id}",
    query: &[],
    request_body: None,
    success: &[200, 206],
};

/// `GET /attachments/{file_id}/thumbnail`.
pub(crate) const GET_ATTACHMENTS_FILE_ID_THUMBNAIL: Route = Route {
    method: reqwest::Method::GET,
    path: "/attachments/{file_id}/thumbnail",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `GET /media/{sha_hash}`.
pub(crate) const GET_MEDIA_SHA_HASH: Route = Route {
    method: reqwest::Method::GET,
    path: "/media/{sha_hash}",
    query: &[],
    request_body: None,
    success: &[200, 206],
};

/// `GET /embeds/{embed_id}`.
pub(crate) const GET_EMBEDS_EMBED_ID: Route = Route {
    method: reqwest::Method::GET,
    path: "/embeds/{embed_id}",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `GET /embeds/{embed_id}/video`.
pub(crate) const GET_EMBEDS_EMBED_ID_VIDEO: Route = Route {
    method: reqwest::Method::GET,
    path: "/embeds/{embed_id}/video",
    query: &[],
    request_body: None,
    success: &[200, 206],
};

/// `POST /search`.
pub(crate) const POST_SEARCH: Route = Route {
    method: reqwest::Method::POST,
    path: "/search",
    query: &[SINCE, LAST_ID],
    request_body: Some("application/json"),
    success: &[200],
};

/// `GET /admin/audit-logs`.
pub(crate) const GET_ADMIN_AUDIT_LOGS: Route = Route {
    method: reqwest::Method::GET,
    path: "/admin/audit-logs",
    query: &[ACTION, ACTOR_ID, ENTITY_TYPE, SINCE, UNTIL, LAST_ID, ORDER],
    request_body: None,
    success: &[200],
};

/// `GET /webhooks`.
pub(crate) const GET_WEBHOOKS: Route = Route {
    method: reqwest::Method::GET,
    path: "/webhooks",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /webhooks`.
pub(crate) const POST_WEBHOOKS: Route = Route {
    method: reqwest::Method::POST,
    path: "/webhooks",
    query: &[],
    request_body: Some("application/json"),
    success: &[201],
};

/// `PATCH /webhooks/{webhook_id}`.
pub(crate) const PATCH_WEBHOOKS_WEBHOOK_ID: Route = Route {
    method: reqwest::Method::PATCH,
    path: "/webhooks/{webhook_id}",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `DELETE /webhooks/{webhook_id}`.
pub(crate) const DELETE_WEBHOOKS_WEBHOOK_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/webhooks/{webhook_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `GET /webhooks/{webhook_id}/avatar`.
pub(crate) const GET_WEBHOOKS_WEBHOOK_ID_AVATAR: Route = Route {
    method: reqwest::Method::GET,
    path: "/webhooks/{webhook_id}/avatar",
    query: &[],
    request_body: None,
    success: &[200],
};

/// `POST /webhooks/{webhook_id}/messages`.
pub(crate) const POST_WEBHOOKS_WEBHOOK_ID_MESSAGES: Route = Route {
    method: reqwest::Method::POST,
    path: "/webhooks/{webhook_id}/messages",
    query: &[],
    request_body: Some("multipart/form-data"),
    success: &[200, 201],
};

/// `PUT /webhooks/{webhook_id}/messages/{message_id}`.
pub(crate) const PUT_WEBHOOKS_WEBHOOK_ID_MESSAGES_MESSAGE_ID: Route = Route {
    method: reqwest::Method::PUT,
    path: "/webhooks/{webhook_id}/messages/{message_id}",
    query: &[],
    request_body: Some("application/json"),
    success: &[200],
};

/// `DELETE /webhooks/{webhook_id}/messages/{message_id}`.
pub(crate) const DELETE_WEBHOOKS_WEBHOOK_ID_MESSAGES_MESSAGE_ID: Route = Route {
    method: reqwest::Method::DELETE,
    path: "/webhooks/{webhook_id}/messages/{message_id}",
    query: &[],
    request_body: None,
    success: &[204],
};

/// `GET /ws`.
pub(crate) const GET_WS: Route = Route {
    method: reqwest::Method::GET,
    path: "/ws",
    query: &[],
    request_body: None,
    success: &[],
};

#[cfg(test)]
mod tests {
    //! Verifies path-template expansion without network access.

    use super::*;

    #[test]
    fn path_templates_expand_in_order() {
        assert_eq!(GET_USERS.url(&[]), "/users");
        assert_eq!(
            PUT_CHANNELS_CHANNEL_ID.url(&["channel".to_owned()]),
            "/channels/channel"
        );
        assert_eq!(
            DELETE_USERS_USER_ID_ROLES_ROLE_ID.url(&["user".to_owned(), "role".to_owned()]),
            "/users/user/roles/role"
        );
    }
}
