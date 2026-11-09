//! Owns authorized embed retrieval; media storage and presentation belong to callers.

use crate::{Error, RestClient, Uuid, wire::EmbedWithImage};
use reqwest::Method;

fn embed_path(id: Uuid) -> String {
    format!("embeds/{id}")
}

impl RestClient {
    /// Fetches an embed and its optional base64 images using channel read authorization.
    /// Inaccessible and nonexistent embeds may both produce a 404 response.
    /// # Errors
    /// Returns transport, backend rejection or response decoding failures.
    pub async fn embed(&self, id: Uuid) -> Result<EmbedWithImage, Error> {
        self.json(self.request(Method::GET, &embed_path(id))?).await
    }
}

#[cfg(test)]
mod tests {
    //! Verifies the embed route and generated wire shape without network access.

    use super::*;
    use serde_json::json;

    #[test]
    fn embed_path_preserves_uuid() {
        let id = Uuid::nil();
        assert_eq!(embed_path(id), format!("embeds/{id}"));
    }

    #[test]
    fn embed_response_retains_base_and_image_fields() {
        let value = json!({
            "id": "id", "source_type": "custom", "fetch_method": "manual",
            "image_data": "AA==", "author_image_data": "AA=="
        });
        let embed: EmbedWithImage = serde_json::from_value(value.clone()).unwrap();
        let encoded = serde_json::to_value(embed).unwrap();
        for key in [
            "id",
            "source_type",
            "fetch_method",
            "image_data",
            "author_image_data",
        ] {
            assert_eq!(encoded[key], value[key]);
        }
    }
}
