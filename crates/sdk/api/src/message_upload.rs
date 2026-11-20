//! Multipart message upload wire requests; staging durability and retry policy belong to `papo-sdk`.

use crate::{Error, RestClient, routes, wire::Message};
use bytes::Bytes;
use futures_util::{Stream, stream};
use std::pin::Pin;

/// Byte chunk stream for one streamed multipart attachment.
pub type AttachmentByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static>>;

/// One streamed multipart attachment with its exact known byte length.
pub struct StreamedMessageAttachment {
    /// Filename supplied in the multipart content disposition.
    pub filename: String,
    /// Exact byte length of `stream`; used to compute the multipart content length.
    pub size_bytes: u64,
    /// Incremental byte stream; consumed once during the single upload attempt.
    pub stream: AttachmentByteStream,
}

/// A streamed multipart message upload; attachments are read incrementally without whole-file copies.
pub struct StreamedMessageUpload<'a> {
    /// Target channel identifier.
    pub channel_id: &'a str,
    /// Optional message text.
    pub content: Option<&'a str>,
    /// Optional identifier of the message being replied to.
    pub reply_to: Option<&'a str>,
    /// Optional JSON-encoded embed array.
    pub embeds: Option<&'a str>,
    /// Streamed attachments in their intended order.
    pub attachments: Vec<StreamedMessageAttachment>,
}

/// A borrowed in-memory multipart message upload; each attachment retains its caller-selected filename.
pub struct MessageUpload<'a> {
    /// Target channel identifier.
    pub channel_id: &'a str,
    /// Optional message text.
    pub content: Option<&'a str>,
    /// Optional identifier of the message being replied to.
    pub reply_to: Option<&'a str>,
    /// Optional JSON-encoded embed array.
    pub embeds: Option<&'a str>,
    /// Attachments in their intended order.
    pub attachments: &'a [MessageAttachment<'a>],
}

/// One borrowed multipart attachment, including its backend-visible original filename.
pub struct MessageAttachment<'a> {
    /// Filename supplied in the multipart content disposition.
    pub filename: &'a str,
    /// Unmodified file contents.
    pub bytes: &'a [u8],
}

impl RestClient {
    /// Sends a multipart message once from borrowed slices. The caller owns ambiguous-send policy.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn create_message(&self, request: &MessageUpload<'_>) -> Result<Message, Error> {
        let mut attachments = Vec::with_capacity(request.attachments.len());
        for attachment in request.attachments {
            let size_bytes =
                u64::try_from(attachment.bytes.len()).map_err(|_| Error::NotDispatched)?;
            let chunk = Bytes::copy_from_slice(attachment.bytes);
            let stream: AttachmentByteStream = if chunk.is_empty() {
                Box::pin(stream::empty())
            } else {
                Box::pin(stream::once(std::future::ready(Ok(chunk))))
            };
            attachments.push(StreamedMessageAttachment {
                filename: attachment.filename.to_owned(),
                size_bytes,
                stream,
            });
        }
        self.create_message_streamed(StreamedMessageUpload {
            channel_id: request.channel_id,
            content: request.content,
            reply_to: request.reply_to,
            embeds: request.embeds,
            attachments,
        })
        .await
    }

    /// Sends a streamed multipart message once without materializing attachments in memory.
    /// The caller owns ambiguous-send policy.
    /// # Errors
    /// Returns transport, problem or decoding failures.
    pub async fn create_message_streamed(
        &self,
        request: StreamedMessageUpload<'_>,
    ) -> Result<Message, Error> {
        let mut form =
            reqwest::multipart::Form::new().text("channel_id", request.channel_id.to_owned());
        for (key, value) in [
            ("content", request.content),
            ("reply_to", request.reply_to),
            ("embeds", request.embeds),
        ] {
            if let Some(value) = value {
                form = form.text(key, value.to_owned());
            }
        }
        for attachment in request.attachments {
            form = form.part(
                "attachments",
                reqwest::multipart::Part::stream_with_length(
                    reqwest::Body::wrap_stream(attachment.stream),
                    attachment.size_bytes,
                )
                .file_name(attachment.filename),
            );
        }
        self.json(
            self.route_request(&routes::POST_MESSAGES, &[])?
                .multipart(form),
        )
        .await
    }
}
