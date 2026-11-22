//! Media source descriptors, byte range headers, inline buffers, and emoji page records.
//! Account-authorized fetching and `.part` atomic file export live in `media.rs`.

use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use papo_sdk::{Generation, MediaMetadata, MediaRange, MediaSource};

use crate::{
    ids::{FfiHistoryCursor, parse_uuid},
    media_disk_errors::FfiMediaError,
};

/// Maximum encoded bytes allowed across direct inline FFI return buffers (4 MiB).
pub const MAX_FFI_INLINE_BYTES: usize = 4 * 1024 * 1024;

/// Bounded `std::io::Write` sink rejecting inline reads above [`MAX_FFI_INLINE_BYTES`].
pub(crate) struct BoundedVecWriter {
    /// Captured bytes written so far.
    pub(crate) buf: Arc<Mutex<Vec<u8>>>,
    /// Set once a write chunk would exceed [`MAX_FFI_INLINE_BYTES`].
    pub(crate) overflowed: Arc<AtomicBool>,
}

impl Write for BoundedVecWriter {
    fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
        let mut buf = self
            .buf
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if buf.len().saturating_add(chunk.len()) > MAX_FFI_INLINE_BYTES {
            self.overflowed.store(true, Ordering::Release);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "exceeds inline FFI limit",
            ));
        }
        buf.extend_from_slice(chunk);
        Ok(chunk.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Subset of immutable remote sources valid inside [`FfiMediaSource::Range`].
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRangeMediaBase {
    /// Opaque backend media blob ID.
    Blob {
        /// Backend media blob token.
        blob_id: String,
    },
    /// Original attachment UUID.
    Attachment {
        /// Attachment UUID string.
        attachment_id: String,
    },
    /// Backend-relayed embed video UUID.
    EmbedVideo {
        /// Embed video UUID string.
        embed_id: String,
    },
}

/// Account-scoped encoded media capability descriptor; never accepts a local SHA-256 digest.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiMediaSource {
    /// Opaque backend blob ID.
    Blob {
        /// Backend media blob token.
        blob_id: String,
    },
    /// Authenticated server icon bytes.
    ServerIcon,
    /// Original channel attachment.
    Attachment {
        /// Attachment UUID string.
        attachment_id: String,
    },
    /// Backend attachment thumbnail.
    Thumbnail {
        /// Attachment UUID string.
        attachment_id: String,
    },
    /// Backend-relayed embed video.
    EmbedVideo {
        /// Embed video UUID string.
        embed_id: String,
    },
    /// Bounded inclusive byte range over a blob, attachment, or embed video.
    Range {
        /// Base source supporting bounded range requests.
        base: FfiRangeMediaBase,
        /// Inclusive start byte offset.
        start_bytes: u64,
        /// Inclusive end byte offset.
        end_bytes: u64,
    },
    /// User avatar keyed by explicit resource revision token.
    UserAvatar {
        /// Backend user UUID string.
        user_id: String,
        /// Resource revision tag forcing refresh when changed.
        revision: String,
    },
    /// Webhook avatar keyed by explicit resource revision token.
    WebhookAvatar {
        /// Webhook UUID string.
        webhook_id: String,
        /// Explicit revision tag.
        revision: String,
    },
    /// External anonymous HTTPS URL fetched without Papo cookies.
    External {
        /// HTTPS URL string.
        url: String,
    },
    /// Custom server emoji image keyed by backend ID + creation timestamp revision.
    Emoji {
        /// Emoji UUID string.
        emoji_id: String,
        /// Exact creation timestamp revision string.
        revision: String,
    },
}

impl FfiMediaSource {
    /// Converts FFI source specification into validated `papo_sdk::MediaSource`.
    pub(crate) fn into_sdk(self) -> Result<MediaSource, FfiMediaError> {
        let map_uuid = |raw: &str| parse_uuid(raw).map_err(|_| FfiMediaError::InvalidData);
        Ok(match self {
            Self::Blob { blob_id } => MediaSource::Blob(blob_id),
            Self::ServerIcon => MediaSource::ServerIcon,
            Self::Attachment { attachment_id } => {
                MediaSource::Attachment(map_uuid(&attachment_id)?)
            }
            Self::Thumbnail { attachment_id } => MediaSource::Thumbnail(map_uuid(&attachment_id)?),
            Self::EmbedVideo { embed_id } => MediaSource::EmbedVideo(map_uuid(&embed_id)?),
            Self::Range {
                base,
                start_bytes,
                end_bytes,
            } => {
                let inner = match base {
                    FfiRangeMediaBase::Blob { blob_id } => MediaSource::Blob(blob_id),
                    FfiRangeMediaBase::Attachment { attachment_id } => {
                        MediaSource::Attachment(map_uuid(&attachment_id)?)
                    }
                    FfiRangeMediaBase::EmbedVideo { embed_id } => {
                        MediaSource::EmbedVideo(map_uuid(&embed_id)?)
                    }
                };
                MediaSource::Range {
                    source: Box::new(inner),
                    start_bytes,
                    end_bytes,
                }
            }
            Self::UserAvatar { user_id, revision } => MediaSource::UserAvatar {
                user: map_uuid(&user_id)?,
                revision,
            },
            Self::WebhookAvatar {
                webhook_id,
                revision,
            } => MediaSource::WebhookAvatar {
                webhook: map_uuid(&webhook_id)?,
                revision,
            },
            Self::External { url } => MediaSource::External(url),
            Self::Emoji { emoji_id, revision } => MediaSource::Emoji {
                id: map_uuid(&emoji_id)?,
                revision,
            },
        })
    }

    /// Converts SDK `MediaSource` back into FFI representation.
    pub(crate) fn from_sdk(source: &MediaSource) -> Self {
        match source {
            MediaSource::Blob(blob_id) => Self::Blob {
                blob_id: blob_id.clone(),
            },
            MediaSource::Attachment(id) => Self::Attachment {
                attachment_id: id.to_string(),
            },
            MediaSource::Thumbnail(id) => Self::Thumbnail {
                attachment_id: id.to_string(),
            },
            MediaSource::EmbedVideo(id) => Self::EmbedVideo {
                embed_id: id.to_string(),
            },
            MediaSource::Range {
                source,
                start_bytes,
                end_bytes,
            } => {
                let base = match source.as_ref() {
                    MediaSource::Blob(blob_id) => FfiRangeMediaBase::Blob {
                        blob_id: blob_id.clone(),
                    },
                    MediaSource::Attachment(id) => FfiRangeMediaBase::Attachment {
                        attachment_id: id.to_string(),
                    },
                    MediaSource::EmbedVideo(id) => FfiRangeMediaBase::EmbedVideo {
                        embed_id: id.to_string(),
                    },
                    _ => FfiRangeMediaBase::Blob {
                        blob_id: String::new(),
                    },
                };
                Self::Range {
                    base,
                    start_bytes: *start_bytes,
                    end_bytes: *end_bytes,
                }
            }
            MediaSource::UserAvatar { user, revision } => Self::UserAvatar {
                user_id: user.to_string(),
                revision: revision.clone(),
            },
            MediaSource::WebhookAvatar { webhook, revision } => Self::WebhookAvatar {
                webhook_id: webhook.to_string(),
                revision: revision.clone(),
            },
            MediaSource::External(url) => Self::External { url: url.clone() },
            MediaSource::Emoji { id, revision } => Self::Emoji {
                emoji_id: id.to_string(),
                revision: revision.clone(),
            },
            // ServerIcon and future variants map to ServerIcon.
            _ => Self::ServerIcon,
        }
    }
}

/// Validated HTTP byte-range bounds for partial responses.
// `AGENTS.md` requires explicit `_bytes` units on raw integer byte counts.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMediaRange {
    /// Inclusive first byte index.
    pub start_bytes: u64,
    /// Inclusive last byte index.
    pub end_bytes: u64,
    /// Known total representation length when reported by `Content-Range`.
    pub total_bytes: Option<u64>,
}

impl From<MediaRange> for FfiMediaRange {
    fn from(r: MediaRange) -> Self {
        Self {
            start_bytes: r.start_bytes,
            end_bytes: r.end_bytes,
            total_bytes: r.total_bytes,
        }
    }
}

/// Metadata returned when streaming an object cleanly to a file path.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMediaExportResult {
    /// Session connection epoch authorizing this export.
    pub generation: u64,
    /// Validated backend/provider MIME content type if supplied.
    pub content_type: Option<String>,
    /// Partial range bounds if partial representation.
    pub range: Option<FfiMediaRange>,
    /// Exact committed and published file length in bytes.
    pub size_bytes: u64,
}

impl FfiMediaExportResult {
    /// Converts `Generated<MediaMetadata>` into FFI result.
    pub(crate) fn from_generated(generation: Generation, meta: MediaMetadata) -> Self {
        Self {
            generation: generation.get(),
            content_type: meta.content_type,
            range: meta.range.map(Into::into),
            size_bytes: meta.size_bytes,
        }
    }
}

/// Small inline encoded media payload (`<= 4 MiB`) returned directly over FFI.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMediaData {
    /// Connection epoch authorizing this read.
    pub generation: u64,
    /// Encoded image/audio/snippet bytes.
    pub bytes: Vec<u8>,
    /// Content-Type header if present.
    pub content_type: Option<String>,
    /// Partial range bounds if partial representation.
    pub range: Option<FfiMediaRange>,
}

/// One server custom emoji metadata entry without inline bytes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmojiMetadata {
    /// Emoji UUID string.
    pub id: String,
    /// Shortcode name.
    pub name: String,
    /// Encoded image format string.
    pub format: String,
    /// Exact backend creation timestamp.
    pub created_at: String,
    /// Media source descriptor for loading image bytes.
    pub source: FfiMediaSource,
}

/// Bounded page of server custom emojis (up to 25 items).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmojiPage {
    /// Connection epoch authorizing this page.
    pub generation: u64,
    /// Custom emoji metadata entries.
    pub emojis: Vec<FfiEmojiMetadata>,
    /// Exact continuation cursor if another page exists.
    pub next_cursor: Option<FfiHistoryCursor>,
}
