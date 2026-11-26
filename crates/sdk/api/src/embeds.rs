//! Owns authorized embed retrieval; media storage and presentation belong to callers.

use crate::{Error, RestClient, Uuid, media::bounded_body, routes, wire::EmbedWithImage};

/// The largest answer read, in bytes. It carries the embed's picture in base64 (the backend
/// keeps a relayed picture under 5 MiB, so about 6.7 MiB encoded) beside a few kilobytes of
/// text.
const EMBED_RESPONSE_LIMIT_BYTES: usize = 16 * 1024 * 1024;

impl RestClient {
    /// Fetches an embed and its optional base64 images using channel read authorization.
    /// Inaccessible and nonexistent embeds may both produce a 404 response. The body is read
    /// up to 16 MiB, whatever length the server announces.
    /// # Errors
    /// Returns transport or backend rejection, or [`Error::Decode`] for an answer that is not
    /// an embed or is larger than the limit.
    pub async fn embed(&self, id: Uuid) -> Result<EmbedWithImage, Error> {
        let request = self.route_request(&routes::GET_EMBEDS_EMBED_ID, &[id.to_string()])?;
        let response = self.response(request).await?;
        let body = bounded_body(response, EMBED_RESPONSE_LIMIT_BYTES)
            .await
            .map_err(|error| match error {
                crate::media::MediaError::Api(error) => error,
                _ => Error::Decode,
            })?;
        serde_json::from_slice(&body).map_err(|_| Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    //! Verifies the generated embed wire shape without network access.

    use super::EmbedWithImage;
    use serde_json::json;

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

    #[test]
    fn attachment_backed_media_omits_url_and_preserves_exact_filename() {
        let media: crate::wire::EmbedMediaInput = serde_json::from_value(json!({
            "attachment_name": "SPOILER_diagram.png", "mime_type": "image/png"
        }))
        .unwrap();
        assert!(media.url.is_none());
        assert_eq!(
            serde_json::to_value(media).unwrap(),
            json!({"attachment_name": "SPOILER_diagram.png", "mime_type": "image/png"})
        );
    }

    #[test]
    fn url_media_remains_compatible_without_an_attachment_reference() {
        let value = json!({"url": "https://example.invalid/synthetic.png"});
        let media: crate::wire::EmbedMediaInput = serde_json::from_value(value.clone()).unwrap();
        assert!(media.attachment_name.is_none());
        assert_eq!(serde_json::to_value(media).unwrap(), value);
    }

    #[test]
    fn attachment_dimensions_round_trip_without_filling_unknown_sizes() {
        for value in [
            json!({"width": 1920, "height": 1080, "thumbnail_width": 320, "thumbnail_height": 180}),
            json!({}),
        ] {
            let attachment: crate::wire::Attachment =
                serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(attachment).unwrap(), value);
        }
    }
}
