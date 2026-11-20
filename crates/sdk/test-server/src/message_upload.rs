//! Streaming multipart `POST /messages` body reader with bounded buffers, hashing and deterministic throttling.

mod parser;
#[cfg(test)]
mod tests;

use std::{collections::BTreeMap, time::Duration};

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use crate::{Error, RestHold, TestServer, rest_hold::PendingHold, server::MAX_BODY_BYTES};
use parser::MultipartParser;

const MAX_UPLOAD_BODY_BYTES: u64 = 100 * 1024 * 1024 + 1024 * 1024;

/// Deterministic pacing applied while reading streamed `POST /messages` multipart frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadThrottle {
    /// Pacing interval in body bytes before sleeping `delay`.
    pub chunk_bytes: usize,
    /// Delay inserted each time `chunk_bytes` of request body have been read.
    pub delay: Duration,
}

/// Compact summary of one streamed multipart attachment; never retains raw file bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedAttachment {
    /// Original multipart filename in request order.
    pub filename: String,
    /// Exact number of file payload bytes received.
    pub size_bytes: u64,
    /// Incremental SHA-256 over the received file payload bytes.
    pub sha256: [u8; 32],
}

/// Compact summary of the latest parsed `POST /messages` multipart upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedUpload {
    /// Text form fields (`channel_id`, `content`, `reply_to`, `embeds`).
    pub fields: BTreeMap<String, String>,
    /// Uploaded attachments in multipart order.
    pub files: Vec<ReceivedAttachment>,
}

pub(crate) struct UploadByteHold {
    pub(crate) min_bytes: u64,
    pub(crate) hold: PendingHold,
}

#[derive(Debug)]
pub(crate) enum ReadUploadError {
    TooLarge,
    Malformed,
}

pub(crate) struct ReadUpload {
    pub(crate) upload: ReceivedUpload,
    pub(crate) raw_body: Option<Bytes>,
}

impl TestServer {
    /// Configures optional deterministic throttling on streamed `POST /messages` bodies.
    pub async fn set_message_upload_throttle(&self, throttle: Option<UploadThrottle>) {
        self.state.lock().await.upload_throttle = throttle;
    }

    /// Holds the next `POST /messages` upload mid-body after at least `min_bytes` arrive.
    /// # Errors
    /// Returns [`Error::Configuration`] when `min_bytes` is zero.
    pub async fn hold_message_upload_after_bytes(&self, min_bytes: u64) -> Result<RestHold, Error> {
        if min_bytes == 0 {
            return Err(Error::Configuration);
        }
        let (pending, hold) = PendingHold::pair("POST", "/messages", true);
        self.state
            .lock()
            .await
            .upload_byte_holds
            .push(UploadByteHold {
                min_bytes,
                hold: pending,
            });
        Ok(hold)
    }

    /// Takes the latest parsed multipart upload metadata and SHA-256 digests, if any.
    pub async fn take_received_upload(&self) -> Option<ReceivedUpload> {
        self.state.lock().await.received_upload.take()
    }
}

pub(crate) async fn read_multipart_message(
    mut body: Incoming,
    content_type: &str,
    throttle: Option<UploadThrottle>,
    mut byte_hold: Option<UploadByteHold>,
) -> Result<ReadUpload, ReadUploadError> {
    let mut parser = MultipartParser::new(content_type).ok_or(ReadUploadError::Malformed)?;
    let mut total_read: u64 = 0;
    let mut raw_capture: Option<Vec<u8>> = Some(Vec::new());
    let mut throttled_since: usize = 0;

    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| ReadUploadError::Malformed)?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        total_read = total_read
            .checked_add(u64::try_from(data.len()).map_err(|_| ReadUploadError::TooLarge)?)
            .ok_or(ReadUploadError::TooLarge)?;
        if total_read > MAX_UPLOAD_BODY_BYTES {
            return Err(ReadUploadError::TooLarge);
        }
        if let Some(captured) = raw_capture.as_mut() {
            if total_read <= MAX_BODY_BYTES as u64 {
                captured.extend_from_slice(&data);
            } else {
                raw_capture = None;
            }
        }
        parser.push(&data)?;
        if let Some(pending) = byte_hold.as_ref()
            && total_read >= pending.min_bytes
            && let Some(pending) = byte_hold.take()
        {
            pending.hold.wait().await;
        }
        if let Some(throttle) = throttle
            && throttle.chunk_bytes > 0
        {
            throttled_since = throttled_since.saturating_add(data.len());
            if throttled_since >= throttle.chunk_bytes {
                throttled_since %= throttle.chunk_bytes;
                tokio::time::sleep(throttle.delay).await;
            }
        }
    }
    let upload = parser.finish()?;
    Ok(ReadUpload {
        upload,
        raw_body: raw_capture.map(Bytes::from),
    })
}
