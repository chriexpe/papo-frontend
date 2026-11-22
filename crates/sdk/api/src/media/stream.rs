//! Bounded incremental response stream for encoded media; caching and file installation belong to `papo-sdk`.

use bytes::Bytes;
use reqwest::header;

use super::{MediaError, MediaResponse, representation_headers};
use crate::Error;

/// Default maximum encoded object size enforced across both streamed and buffered media fetches.
pub(super) const MAX_MEDIA_BYTES: usize = 100 * 1024 * 1024;

/// Selected representation metadata and incremental body reader enforcing `MAX_MEDIA_BYTES`.
pub struct MediaStream {
    status: u16,
    headers: header::HeaderMap,
    response: Option<reqwest::Response>,
    budget: StreamBudget,
}

impl MediaStream {
    /// Creates an incremental reader from an HTTP response, rejecting oversized `Content-Length`.
    pub(super) fn from_response(
        response: reqwest::Response,
        empty_body: bool,
        limit_bytes: usize,
    ) -> Result<Self, MediaError> {
        let budget = StreamBudget::new(limit_bytes);
        if !empty_body {
            budget.check_declared_length(response.content_length())?;
        }
        Ok(Self {
            status: response.status().as_u16(),
            headers: representation_headers(response.headers()),
            response: (!empty_body).then_some(response),
            budget,
        })
    }

    /// HTTP status code returned by the server (`200`, `206`, or `304`).
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// Selected representation, caching, and byte-range headers.
    #[must_use]
    pub const fn headers(&self) -> &header::HeaderMap {
        &self.headers
    }

    /// Reads the next encoded chunk while enforcing the cumulative byte limit.
    ///
    /// # Errors
    /// Returns `TooLarge` if `limit_bytes` would be exceeded, or `Api(Error::Transport)` on I/O failure.
    pub async fn next_chunk(&mut self) -> Result<Option<Bytes>, MediaError> {
        let Some(response) = &mut self.response else {
            return Ok(None);
        };
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if let Err(error) = self.budget.accept(chunk.len()) {
                    self.response = None;
                    return Err(error);
                }
                Ok(Some(chunk))
            }
            Ok(None) => {
                self.response = None;
                Ok(None)
            }
            Err(_) => {
                self.response = None;
                Err(Error::Transport.into())
            }
        }
    }

    /// Materializes all remaining chunks into a single `MediaResponse`.
    pub(super) async fn collect(mut self) -> Result<MediaResponse, MediaError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = self.next_chunk().await? {
            bytes.extend_from_slice(&chunk);
        }
        Ok(MediaResponse {
            status: self.status,
            bytes,
            headers: self.headers,
        })
    }
}

/// Enforces the cumulative per-object byte limit without relying solely on `Content-Length`.
struct StreamBudget {
    received: usize,
    limit_bytes: usize,
}

impl StreamBudget {
    const fn new(limit_bytes: usize) -> Self {
        Self {
            received: 0,
            limit_bytes,
        }
    }

    fn check_declared_length(&self, length: Option<u64>) -> Result<(), MediaError> {
        if length.is_some_and(|length| length > self.limit_bytes as u64) {
            return Err(MediaError::TooLarge);
        }
        Ok(())
    }

    fn accept(&mut self, chunk_len: usize) -> Result<(), MediaError> {
        if chunk_len > self.limit_bytes.saturating_sub(self.received) {
            return Err(MediaError::TooLarge);
        }
        self.received += chunk_len;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Verifies cumulative and declared-length enforcement without network I/O.
    use super::*;

    #[test]
    fn declared_content_length_above_limit_is_rejected_before_streaming() {
        let budget = StreamBudget::new(1024);
        assert!(budget.check_declared_length(Some(1024)).is_ok());
        assert!(matches!(
            budget.check_declared_length(Some(1025)),
            Err(MediaError::TooLarge)
        ));
    }

    #[test]
    fn incremental_chunks_stop_at_cumulative_limit() {
        let mut budget = StreamBudget::new(10);
        assert!(budget.accept(4).is_ok());
        assert!(budget.accept(6).is_ok());
        assert!(matches!(budget.accept(1), Err(MediaError::TooLarge)));
        assert_eq!(budget.received, 10);
    }
}
