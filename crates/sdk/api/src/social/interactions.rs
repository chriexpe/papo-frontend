//! Owns pin, reaction and search REST calls; caching and user-facing search belong to the SDK.

use crate::{
    Cursor, Error, RestClient, Uuid, routes,
    wire::{
        MessageReaction, MessageReactionList, PinnedMessage, PinnedMessageList, ReactionRequest,
        SearchRequest, SearchResponse,
    },
};
impl RestClient {
    /// Lists pinned messages in pin order.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn pinned_messages(&self, channel: Uuid) -> Result<PinnedMessageList, Error> {
        self.json(self.route_request(
            &routes::GET_CHANNELS_CHANNEL_ID_PINNED,
            &[channel.to_string()],
        )?)
        .await
    }

    /// Pins a message; an already-pinned message is returned as well.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn pin_message(&self, channel: Uuid, message: Uuid) -> Result<PinnedMessage, Error> {
        self.json(self.route_request(
            &routes::POST_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_PIN,
            &[channel.to_string(), message.to_string()],
        )?)
        .await
    }

    /// Removes a pin; the backend returns an error if no pin exists.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn unpin_message(&self, channel: Uuid, message: Uuid) -> Result<(), Error> {
        self.response(self.route_request(
            &routes::DELETE_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_PIN,
            &[channel.to_string(), message.to_string()],
        )?)
        .await
        .map(|_| ())
    }

    /// Lists reaction users, grouped by emoji within the returned page.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn message_reactions(
        &self,
        channel: Uuid,
        message: Uuid,
        cursor: Option<&Cursor<'_>>,
    ) -> Result<MessageReactionList, Error> {
        let parameters = cursor.map_or_else(Vec::new, Cursor::parameters);
        self.json(
            self.route_request(
                &routes::GET_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_REACTIONS,
                &[channel.to_string(), message.to_string()],
            )?
            .query(&parameters),
        )
        .await
    }

    /// Adds the authenticated user's reaction.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn add_reaction(
        &self,
        channel: Uuid,
        message: Uuid,
        request: &ReactionRequest,
    ) -> Result<MessageReaction, Error> {
        self.post(
            &routes::POST_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_REACTIONS,
            &[channel.to_string(), message.to_string()],
            request,
        )
        .await
    }

    /// Removes the authenticated user's reaction using a JSON request body.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn remove_reaction(
        &self,
        channel: Uuid,
        message: Uuid,
        request: &ReactionRequest,
    ) -> Result<(), Error> {
        self.response(
            self.route_request(
                &routes::DELETE_CHANNELS_CHANNEL_ID_MESSAGES_MESSAGE_ID_REACTIONS,
                &[channel.to_string(), message.to_string()],
            )?
            .json(request),
        )
        .await
        .map(|_| ())
    }

    /// Searches readable channels with an optional exact tuple cursor.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn search(
        &self,
        request: &SearchRequest,
        cursor: Option<&Cursor<'_>>,
    ) -> Result<SearchResponse, Error> {
        let mut builder = self.route_request(&routes::POST_SEARCH, &[])?.json(request);
        if let Some(cursor) = cursor {
            builder = builder.query(&cursor.parameters());
        }
        self.json(builder).await
    }
}
