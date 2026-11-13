//! Owns byte-only media downloads and range metadata; decoding and storage belong to callers.

use crate::{Error, RestClient, Uuid, routes};
use reqwest::header;

/// A single inclusive HTTP byte range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ByteRange {
    /// Bytes from `start_bytes` through `end_bytes`, inclusive.
    Bounded {
        /// First byte offset, inclusive.
        start_bytes: u64,
        /// Last byte offset, inclusive.
        end_bytes: u64,
    },
    /// All bytes starting at `start_bytes`.
    From {
        /// First byte offset, inclusive.
        start_bytes: u64,
    },
    /// The final `length_bytes` bytes.
    Suffix {
        /// Number of trailing bytes, nonzero.
        length_bytes: u64,
    },
}

impl ByteRange {
    fn header_value(self) -> Result<String, MediaError> {
        match self {
            Self::Bounded {
                start_bytes,
                end_bytes,
            } if start_bytes <= end_bytes => Ok(format!("bytes={start_bytes}-{end_bytes}")),
            Self::From { start_bytes } => Ok(format!("bytes={start_bytes}-")),
            Self::Suffix { length_bytes } if length_bytes > 0 => {
                Ok(format!("bytes=-{length_bytes}"))
            }
            _ => Err(MediaError::InvalidRange),
        }
    }
}

/// Download failures, excluding server-provided text from their formatted representation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MediaError {
    /// The media identifier is not the backend's 64-character hexadecimal identifier.
    #[error("invalid media identifier")]
    InvalidMediaId,
    /// A byte range is empty or reversed.
    #[error("invalid byte range")]
    InvalidRange,
    /// Encoded representation exceeded the caller's byte budget.
    #[error("media response exceeds byte limit")]
    TooLarge,
    /// The transport failed or the backend rejected the request.
    #[error(transparent)]
    Api(#[from] Error),
}

/// Selected representation metadata, as sent by the server, including 200 versus 206.
/// Header values are not interpreted; a server may ignore a range and return 200.
pub struct MediaResponse {
    /// Actual successful HTTP status code.
    pub status: u16,
    /// Response body bytes, possibly a partial representation.
    pub bytes: Vec<u8>,
    /// Selected content, range, caching, and representation headers.
    pub headers: header::HeaderMap,
}

impl RestClient {
    /// Fetches a content-addressed avatar, banner, icon or other backend media blob.
    ///
    /// # Errors
    /// Returns invalid input, transport failure or backend rejection.
    pub async fn media_bytes(
        &self,
        media_id: &str,
        range: Option<ByteRange>,
    ) -> Result<MediaResponse, MediaError> {
        validate_media_id(media_id)?;
        self.download_bytes(
            &routes::GET_MEDIA_SHA_HASH,
            &[media_id.to_owned()],
            range,
            false,
        )
        .await
    }

    /// Fetches an attachment after backend channel authorization.
    ///
    /// # Errors
    /// Returns invalid input, transport failure or backend rejection.
    pub async fn attachment_bytes(
        &self,
        id: Uuid,
        range: Option<ByteRange>,
    ) -> Result<MediaResponse, MediaError> {
        self.download_bytes(
            &routes::GET_ATTACHMENTS_FILE_ID,
            &[id.to_string()],
            range,
            false,
        )
        .await
    }

    /// Fetches an attachment thumbnail. The pinned backend ignores range requests here.
    ///
    /// # Errors
    /// Returns transport failure or backend rejection.
    pub async fn attachment_thumbnail_bytes(&self, id: Uuid) -> Result<MediaResponse, MediaError> {
        self.download_bytes(
            &routes::GET_ATTACHMENTS_FILE_ID_THUMBNAIL,
            &[id.to_string()],
            None,
            false,
        )
        .await
    }

    /// Fetches video relayed by the backend for an accessible embed.
    ///
    /// # Errors
    /// Returns invalid input, transport failure or backend rejection.
    pub async fn embed_video_bytes(
        &self,
        id: Uuid,
        range: Option<ByteRange>,
    ) -> Result<MediaResponse, MediaError> {
        self.download_bytes(
            &routes::GET_EMBEDS_EMBED_ID_VIDEO,
            &[id.to_string()],
            range,
            false,
        )
        .await
    }

    /// Fetches a public user avatar without sending session cookies.
    ///
    /// # Errors
    /// Returns transport failure or backend rejection.
    pub async fn public_user_avatar_bytes(&self, id: Uuid) -> Result<MediaResponse, MediaError> {
        self.download_bytes(
            &routes::GET_USERS_USER_ID_AVATAR,
            &[id.to_string()],
            None,
            true,
        )
        .await
    }

    /// Fetches a public webhook avatar without sending session cookies.
    ///
    /// # Errors
    /// Returns transport failure or backend rejection.
    pub async fn public_webhook_avatar_bytes(&self, id: Uuid) -> Result<MediaResponse, MediaError> {
        self.download_bytes(
            &routes::GET_WEBHOOKS_WEBHOOK_ID_AVATAR,
            &[id.to_string()],
            None,
            true,
        )
        .await
    }

    async fn download_bytes(
        &self,
        route: &routes::Route,
        values: &[String],
        range: Option<ByteRange>,
        anonymous: bool,
    ) -> Result<MediaResponse, MediaError> {
        let range = range.map(ByteRange::header_value).transpose()?;
        let mut request = if anonymous {
            self.anonymous_route_request(route, values)?
        } else {
            self.route_request(route, values)?
        };
        if let Some(range) = range {
            request = request.header(header::RANGE, range);
        }
        let response = if anonymous {
            self.anonymous_response(request).await?
        } else {
            self.response(request).await?
        };
        let status = response.status().as_u16();
        let headers = representation_headers(response.headers());
        let bytes = bounded_body(response, 100 * 1024 * 1024).await?;
        Ok(MediaResponse {
            status,
            bytes,
            headers,
        })
    }
}

/// Read encoded media incrementally; a dishonest or absent Content-Length cannot bypass the limit.
async fn bounded_body(
    mut response: reqwest::Response,
    limit_bytes: usize,
) -> Result<Vec<u8>, MediaError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit_bytes as u64)
    {
        return Err(MediaError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
        if chunk.len() > limit_bytes.saturating_sub(bytes.len()) {
            return Err(MediaError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

impl RestClient {
    /// Conditionally fetch an anonymous mutable avatar. A 304 has no body; callers retain their scoped representation.
    /// # Errors
    /// Returns transport/backend rejection or a bounded-body failure. No session cookies are attached.
    pub async fn conditional_avatar_bytes(
        &self,
        id: Uuid,
        webhook: bool,
        etag: Option<&str>,
        modified: Option<&str>,
    ) -> Result<MediaResponse, MediaError> {
        let route = if webhook {
            &routes::GET_WEBHOOKS_WEBHOOK_ID_AVATAR
        } else {
            &routes::GET_USERS_USER_ID_AVATAR
        };
        let mut request = self.anonymous_route_request(route, &[id.to_string()])?;
        if let Some(value) = etag {
            request = request.header(header::IF_NONE_MATCH, value);
        }
        if let Some(value) = modified {
            request = request.header(header::IF_MODIFIED_SINCE, value);
        }
        let response = request.send().await.map_err(|_| Error::Transport)?;
        let status = response.status();
        if !status.is_success() && status != reqwest::StatusCode::NOT_MODIFIED {
            return Err(Error::Http {
                status: status.as_u16(),
            }
            .into());
        }
        let headers = representation_headers(response.headers());
        let bytes = if status == reqwest::StatusCode::NOT_MODIFIED {
            Vec::new()
        } else {
            bounded_body(response, 100 * 1024 * 1024).await?
        };
        Ok(MediaResponse {
            status: status.as_u16(),
            bytes,
            headers,
        })
    }
}

fn validate_media_id(media_id: &str) -> Result<(), MediaError> {
    if media_id.len() != 64 || !media_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MediaError::InvalidMediaId);
    }
    Ok(())
}

fn representation_headers(source: &header::HeaderMap) -> header::HeaderMap {
    let mut selected = header::HeaderMap::new();
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::CONTENT_DISPOSITION,
        header::ETAG,
        header::LAST_MODIFIED,
        header::CACHE_CONTROL,
    ] {
        for value in source.get_all(&name) {
            selected.append(&name, value.clone());
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    //! Covers input validation and metadata selection without network access.
    use super::*;

    #[test]
    fn ranges_encode_single_inclusive_and_suffix_ranges() {
        assert_eq!(
            ByteRange::Bounded {
                start_bytes: 10,
                end_bytes: 19
            }
            .header_value()
            .unwrap(),
            "bytes=10-19"
        );
        assert_eq!(
            ByteRange::From { start_bytes: 3 }.header_value().unwrap(),
            "bytes=3-"
        );
        assert_eq!(
            ByteRange::Suffix { length_bytes: 5 }
                .header_value()
                .unwrap(),
            "bytes=-5"
        );
    }

    #[test]
    fn invalid_ranges_are_rejected_before_dispatch() {
        assert!(matches!(
            ByteRange::Bounded {
                start_bytes: 2,
                end_bytes: 1
            }
            .header_value(),
            Err(MediaError::InvalidRange)
        ));
        assert!(matches!(
            ByteRange::Suffix { length_bytes: 0 }.header_value(),
            Err(MediaError::InvalidRange)
        ));
    }

    #[test]
    fn media_ids_cannot_escape_the_fixed_route() {
        assert!(validate_media_id(&"a".repeat(64)).is_ok());
        for id in [
            "../attachments/x",
            "https://example.invalid",
            "%2e%2e",
            "",
            &"/".repeat(64),
        ] {
            assert!(matches!(
                validate_media_id(id),
                Err(MediaError::InvalidMediaId)
            ));
        }
    }

    #[test]
    fn metadata_retains_range_headers_but_not_cookies() {
        let mut input = header::HeaderMap::new();
        input.insert(
            header::CONTENT_RANGE,
            header::HeaderValue::from_static("bytes 0-9/100"),
        );
        input.insert(
            header::SET_COOKIE,
            header::HeaderValue::from_static("redacted"),
        );
        let output = representation_headers(&input);
        assert_eq!(output[header::CONTENT_RANGE], "bytes 0-9/100");
        assert!(!output.contains_key(header::SET_COOKIE));
    }
}
