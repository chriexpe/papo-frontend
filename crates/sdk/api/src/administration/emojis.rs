//! Owns server emoji listing and management; image validation belongs to the backend.

use crate::{
    Cursor, Error, RestClient, Uuid, routes,
    wire::{CreateEmojiRequest, Emoji, EmojiListResponse},
};

const EMOJI_PAGE_LIMIT_BYTES: usize = 12 * 1024 * 1024;

impl RestClient {
    /// Fetches one page of server emoji.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn emojis(&self, cursor: Option<&Cursor<'_>>) -> Result<EmojiListResponse, Error> {
        let parameters = cursor.map_or_else(Vec::new, Cursor::parameters);
        let mut response = self
            .response(
                self.route_request(&routes::GET_EMOJIS, &[])?
                    .query(&parameters),
            )
            .await?;
        if response
            .content_length()
            .is_some_and(|length| length > EMOJI_PAGE_LIMIT_BYTES as u64)
        {
            return Err(Error::Decode);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
            if chunk.len() > EMOJI_PAGE_LIMIT_BYTES.saturating_sub(bytes.len()) {
                return Err(Error::Decode);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::Decode)
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
