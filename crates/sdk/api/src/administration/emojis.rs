//! Owns server emoji listing and management; image validation belongs to the backend.

use crate::{
    Error, RestClient, Uuid,
    wire::{CreateEmojiRequest, Emoji, EmojiListResponse},
};
use reqwest::Method;

/// Emoji pagination parameters; timestamps retain their exact representation.
#[derive(Clone, Default, serde::Serialize)]
pub struct EmojiQuery<'a> {
    /// Cursor timestamp in RFC 3339.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<&'a str>,
    /// UUID paired with the timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_id: Option<&'a str>,
    /// Backend ordering, `asc` or `desc`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<&'a str>,
}

impl RestClient {
    /// Fetches one page of server emoji.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn emojis(&self, query: &EmojiQuery<'_>) -> Result<EmojiListResponse, Error> {
        self.json(self.request(Method::GET, "emojis")?.query(query))
            .await
    }

    /// Uploads a base64 image as an emoji once.
    /// # Errors
    /// Returns transport, backend or decoding failures.
    pub async fn create_emoji(&self, request: &CreateEmojiRequest) -> Result<Emoji, Error> {
        self.post("emojis", request).await
    }

    /// Deletes an emoji once.
    /// # Errors
    /// Returns transport or backend rejection.
    pub async fn delete_emoji(&self, emoji: Uuid) -> Result<(), Error> {
        self.response(self.request(Method::DELETE, &format!("emojis/{emoji}"))?)
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
