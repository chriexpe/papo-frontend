//! What kind of media an attachment is, the one rule every view of a message follows: the
//! media box, a file card, a reply line's word. Owns only the classification, from the type the
//! server detected and the size; drawing, decoding and the words are the client's.
//!
//! A picture is one of the four formats the server makes thumbnails of and every client decodes
//! (JPEG, PNG, WebP and GIF), so a client classifying a local file by its first bytes reaches
//! the same kind, and a message keeps its shape from unsent to sent.

use papo_sdk::Attachment;

/// The largest picture drawn as a picture, in bytes: a larger one is a file, so no view holds a
/// huge decode for a thumbnail. A client classifying a local file applies the same limit.
pub const MAX_PICTURE_BYTES: u64 = 64 * 1024 * 1024;

/// What an attachment is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AttachmentKind {
    /// A still JPEG, PNG or WebP picture (an animated WebP shows as a picture too).
    Image,
    /// A GIF.
    Gif,
    /// A video file.
    Video,
    /// An audio file.
    Audio,
    /// Anything else, including pictures in other formats or over [`MAX_PICTURE_BYTES`].
    File,
}

impl AttachmentKind {
    /// The kind of a file of media type `mime` (parameters and case ignored) and
    /// `size_bytes` long, when known.
    #[must_use]
    pub fn from_mime(mime: &str, size_bytes: Option<u64>) -> AttachmentKind {
        let essence = mime
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let picture = match essence.as_str() {
            "image/gif" => Some(AttachmentKind::Gif),
            "image/jpeg" | "image/png" | "image/webp" => Some(AttachmentKind::Image),
            _ => None,
        };
        if let Some(picture) = picture {
            let too_large = size_bytes.is_some_and(|size| size > MAX_PICTURE_BYTES);
            return if too_large {
                AttachmentKind::File
            } else {
                picture
            };
        }
        if essence.starts_with("video/") {
            AttachmentKind::Video
        } else if essence.starts_with("audio/") {
            AttachmentKind::Audio
        } else {
            AttachmentKind::File
        }
    }

    /// The kind of a received attachment, from the type the server detected at upload.
    #[must_use]
    pub fn of(attachment: &Attachment) -> AttachmentKind {
        let size_bytes = attachment
            .size_bytes
            .and_then(|size| u64::try_from(size).ok());
        AttachmentKind::from_mime(
            attachment.mime_type.as_deref().unwrap_or_default(),
            size_bytes,
        )
    }

    /// Whether it is a picture (a still image or a GIF).
    #[must_use]
    pub fn is_picture(self) -> bool {
        matches!(self, AttachmentKind::Image | AttachmentKind::Gif)
    }
}

#[cfg(test)]
mod tests {
    //! The kinds of synthetic media types and sizes.

    use super::*;

    #[test]
    fn the_four_picture_formats_are_pictures() {
        for mime in ["image/jpeg", "image/png", "image/webp", "IMAGE/PNG"] {
            assert_eq!(AttachmentKind::from_mime(mime, None), AttachmentKind::Image);
        }
        assert_eq!(
            AttachmentKind::from_mime("image/gif", Some(1)),
            AttachmentKind::Gif
        );
    }

    #[test]
    fn other_picture_formats_are_files() {
        for mime in [
            "image/bmp",
            "image/heic",
            "image/tiff",
            "image/svg+xml",
            "image/avif",
        ] {
            assert_eq!(AttachmentKind::from_mime(mime, None), AttachmentKind::File);
        }
    }

    #[test]
    fn a_picture_over_the_limit_is_a_file() {
        assert_eq!(
            AttachmentKind::from_mime("image/jpeg", Some(MAX_PICTURE_BYTES)),
            AttachmentKind::Image
        );
        assert_eq!(
            AttachmentKind::from_mime("image/jpeg", Some(MAX_PICTURE_BYTES + 1)),
            AttachmentKind::File
        );
        assert_eq!(
            AttachmentKind::from_mime("image/gif", Some(MAX_PICTURE_BYTES + 1)),
            AttachmentKind::File
        );
    }

    #[test]
    fn video_audio_and_the_rest() {
        assert_eq!(
            AttachmentKind::from_mime("video/mp4", Some(u64::MAX)),
            AttachmentKind::Video
        );
        assert_eq!(
            AttachmentKind::from_mime("audio/ogg; codecs=vorbis", None),
            AttachmentKind::Audio
        );
        assert_eq!(
            AttachmentKind::from_mime("text/plain; charset=utf-8", None),
            AttachmentKind::File
        );
        assert_eq!(AttachmentKind::from_mime("", None), AttachmentKind::File);
    }

    #[test]
    fn a_received_attachment_reads_its_type_and_size() {
        let attachment = |mime: &str, size: i64| Attachment {
            mime_type: Some(mime.to_owned()),
            size_bytes: Some(size),
            ..Attachment::default()
        };
        assert_eq!(
            AttachmentKind::of(&attachment("image/png", 10)),
            AttachmentKind::Image
        );
        // A negative size is no size: the type decides.
        assert_eq!(
            AttachmentKind::of(&attachment("image/png", -1)),
            AttachmentKind::Image
        );
        assert_eq!(
            AttachmentKind::of(&Attachment::default()),
            AttachmentKind::File
        );
    }
}
