//! Owns direct conversations, blocking and user settings; policy belongs to the SDK.

use crate::{
    Error, RestClient, Uuid, routes,
    wire::{
        BlockListResponse, DirectConversation, DirectConversationList, OpenDirectMessageRequest,
        UpdateUserSettingsRequest, UserSettings,
    },
};
impl RestClient {
    /// Lists visible direct conversations.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn direct_conversations(&self) -> Result<DirectConversationList, Error> {
        self.json(self.route_request(&routes::GET_DMS, &[])?).await
    }

    /// Opens or reopens a direct conversation.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn open_direct_conversation(
        &self,
        request: &OpenDirectMessageRequest,
    ) -> Result<DirectConversation, Error> {
        self.post(&routes::POST_DMS, &[], request).await
    }

    /// Retrieves one direct conversation.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn direct_conversation(&self, id: Uuid) -> Result<DirectConversation, Error> {
        self.json(self.route_request(&routes::GET_DMS_DM_ID, &[id.to_string()])?)
            .await
    }

    /// Hides a direct conversation without deleting its history.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn close_direct_conversation(&self, id: Uuid) -> Result<(), Error> {
        self.response(self.route_request(&routes::DELETE_DMS_DM_ID, &[id.to_string()])?)
            .await
            .map(|_| ())
    }

    /// Lists accounts blocked by the authenticated user.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn blocked_users(&self) -> Result<BlockListResponse, Error> {
        self.json(self.route_request(&routes::GET_USERS_BLOCKS, &[])?)
            .await
    }

    /// Blocks direct messages with another user.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn block_user(&self, user: Uuid) -> Result<(), Error> {
        self.response(self.route_request(&routes::POST_USERS_USER_ID_BLOCK, &[user.to_string()])?)
            .await
            .map(|_| ())
    }

    /// Removes a direct-message block without reopening the conversation.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn unblock_user(&self, user: Uuid) -> Result<(), Error> {
        self.response(self.route_request(&routes::DELETE_USERS_USER_ID_BLOCK, &[user.to_string()])?)
            .await
            .map(|_| ())
    }

    /// Replaces the authenticated user's server-side settings.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn update_user_settings(
        &self,
        request: &UpdateUserSettingsRequest,
    ) -> Result<UserSettings, Error> {
        self.json(
            self.route_request(&routes::PUT_USERS_SETTINGS, &[])?
                .json(request),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Owns inline response shape tests.
    use super::*;

    #[test]
    fn empty_block_list_decodes() {
        let list: BlockListResponse = serde_json::from_str(r#"{"users":[]}"#).unwrap();
        assert!(list.users.is_empty());
    }
}
