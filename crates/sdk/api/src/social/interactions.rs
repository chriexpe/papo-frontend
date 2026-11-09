//! Owns pin, reaction and search REST calls; caching and user-facing search belong to the SDK.

use super::{Cursor, Page};
use crate::{
    Error, RestClient, Uuid,
    wire::{
        MessageReaction, MessageReactionList, PinnedMessage, PinnedMessageList, ReactionRequest,
        SearchRequest, SearchResponse,
    },
};
use reqwest::Method;

impl RestClient {
    /// Lists pinned messages in pin order.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn pinned_messages(&self, channel: Uuid) -> Result<PinnedMessageList, Error> {
        self.json(self.request(Method::GET, &format!("channels/{channel}/pinned"))?)
            .await
    }

    /// Pins a message; an already-pinned message is returned as well.
    /// # Errors
    /// Returns transport, backend or decoding errors.
    pub async fn pin_message(&self, channel: Uuid, message: Uuid) -> Result<PinnedMessage, Error> {
        self.json(self.request(
            Method::POST,
            &format!("channels/{channel}/messages/{message}/pin"),
        )?)
        .await
    }

    /// Removes a pin; the backend returns an error if no pin exists.
    /// # Errors
    /// Returns transport or backend errors.
    pub async fn unpin_message(&self, channel: Uuid, message: Uuid) -> Result<(), Error> {
        self.response(self.request(
            Method::DELETE,
            &format!("channels/{channel}/messages/{message}/pin"),
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
        page: &Page<'_>,
    ) -> Result<MessageReactionList, Error> {
        self.json(
            self.request(
                Method::GET,
                &format!("channels/{channel}/messages/{message}/reactions"),
            )?
            .query(&page.parameters()),
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
            &format!("channels/{channel}/messages/{message}/reactions"),
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
            self.request(
                Method::DELETE,
                &format!("channels/{channel}/messages/{message}/reactions"),
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
        let mut builder = self.request(Method::POST, "search")?.json(request);
        if let Some(cursor) = cursor {
            builder = builder.query(cursor);
        }
        self.json(builder).await
    }
}
