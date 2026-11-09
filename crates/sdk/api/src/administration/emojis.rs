//! Owns server emoji listing and management; image validation belongs to the backend.

use crate::{
    Cursor, Error, RestClient, Uuid, routes,
    wire::{CreateEmojiRequest, Emoji, EmojiListResponse},
};

impl RestClient {
    /// Fetches one page of server emoji.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn emojis(&self, cursor: Option<&Cursor<'_>>) -> Result<EmojiListResponse, Error> {
        let parameters = cursor.map_or_else(Vec::new, Cursor::parameters);
        self.json(
            self.route_request(&routes::GET_EMOJIS, &[])?
                .query(&parameters),
        )
        .await
    }

    /// Uploads a base64 image as an emoji once.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn create_emoji(&self, request: &CreateEmojiRequest) -> Result<Emoji, Error> {
        self.post(&routes::POST_EMOJIS, &[], request).await
    }

    /// Deletes an emoji once.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn delete_emoji(&self, emoji: Uuid) -> Result<(), Error> {
        self.response(self.route_request(&routes::DELETE_EMOJIS_EMOJI_ID, &[emoji.to_string()])?)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    //! Tests emoji page decoding without exercising transport.

    use super::EmojiListResponse;

    #[test]
    fn emoji_page_decodes_empty_list_and_more_flag() {
        let page: EmojiListResponse =
            serde_json::from_str(r#"{"emojis":[],"has_more":false}"#).unwrap();
        assert_eq!(page.emojis.unwrap().len(), 0);
        assert_eq!(page.has_more, Some(false));
    }
}
