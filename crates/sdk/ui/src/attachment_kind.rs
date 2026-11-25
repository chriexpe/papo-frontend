//! What kind of media an attachment is, the one rule every view of a message follows: the
//! media box, a file card, a reply line's word. Owns only the classification, from the type the
//! server detected, the file's name and its size; drawing, decoding and the words are the
//! client's.
//!
//! The server labels a file by its container, not its tracks: an MP4 holding only AAC (Android's
//! voice notes, most `.m4a` files) is `video/mp4`, a `WebM` voice recording `video/webm`, and any
//! Ogg `application/ogg`. So, as the web client does, a recording's name (`audio_….webm`, as the
//! web client names them) or an audio-only extension makes the file audio whatever its label,
//! and an Ogg is audio unless named `.ogv`.
//!
//! A picture is one of the four formats the server makes thumbnails of and every client decodes
//! (JPEG, PNG, WebP and GIF), so a client classifying a local file by its first bytes reaches
//! the same kind, and a message keeps its shape from unsent to sent.

use papo_sdk::Attachment;

/// The largest picture drawn as a picture, in bytes: a larger one is a file, so no view holds a
/// huge decode for a thumbnail. A client classifying a local file applies the same limit.
pub const MAX_PICTURE_BYTES: u64 = 64 * 1024 * 1024;

/// Extensions only audio is saved with (lowercase).
const AUDIO_EXTENSIONS: &[&str] = &[
    "aac", "aif", "aiff", "flac", "m4a", "m4b", "m4p", "mp3", "oga", "ogg", "opus", "wav", "weba",
    "wma",
];

/// Extensions a web client's recording, named `audio_…`, is saved with: a `WebM` among them,
/// which alone could be video.
const RECORDING_EXTENSIONS: &[&str] = &["aac", "m4a", "mp3", "ogg", "opus", "wav", "webm"];

/// Extensions only video is saved with, for a file the server didn't recognise (lowercase).
/// MPEG transport streams (`.ts`, `.mts`) are left out: `.ts` is also TypeScript.
const VIDEO_EXTENSIONS: &[&str] = &[
    "3g2", "3gp", "asf", "avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ogv", "qt",
    "webm", "wmv",
];

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

    /// The kind of a file of media type `mime` named `file_name` (when known) and `size_bytes`
    /// long: [`AttachmentKind::from_mime`], except that a container the server labels as video,
    /// Ogg or unknown is audio when its name says so (see the module header), and an unlabelled
    /// file with a video extension is video.
    #[must_use]
    pub fn from_file(
        mime: &str,
        file_name: Option<&str>,
        size_bytes: Option<u64>,
    ) -> AttachmentKind {
        let by_mime = AttachmentKind::from_mime(mime, size_bytes);
        let Some(name) = file_name.map(str::to_ascii_lowercase) else {
            return by_mime;
        };
        let essence = mime
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension);
        let recording = name.starts_with("audio_") && RECORDING_EXTENSIONS.contains(&extension);
        let audio_name = recording || AUDIO_EXTENSIONS.contains(&extension);
        let unlabelled = matches!(essence.as_str(), "" | "application/octet-stream");
        match by_mime {
            AttachmentKind::Video if audio_name => AttachmentKind::Audio,
            AttachmentKind::File if essence == "application/ogg" => {
                if extension == "ogv" {
                    AttachmentKind::Video
                } else {
                    AttachmentKind::Audio
                }
            }
            AttachmentKind::File if unlabelled && audio_name => AttachmentKind::Audio,
            AttachmentKind::File if unlabelled && VIDEO_EXTENSIONS.contains(&extension) => {
                AttachmentKind::Video
            }
            kind => kind,
        }
    }

    /// The kind of a received attachment, from the type the server detected at upload and the
    /// name it was sent with.
    #[must_use]
    pub fn of(attachment: &Attachment) -> AttachmentKind {
        let size_bytes = attachment
            .size_bytes
            .and_then(|size| u64::try_from(size).ok());
        AttachmentKind::from_file(
            attachment.mime_type.as_deref().unwrap_or_default(),
            attachment.original_file_name.as_deref(),
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
    fn a_container_labelled_video_is_audio_by_its_name() {
        let kind = |mime: &str, name: &str| AttachmentKind::from_file(mime, Some(name), None);
        // Android's voice note, an MP4 the server labels video.
        assert_eq!(
            kind("video/mp4", "Voice note 2026-11-25 10.00.00.m4a"),
            AttachmentKind::Audio
        );
        assert_eq!(kind("video/mp4", "Song.MP3"), AttachmentKind::Audio);
        // The web client's recordings.
        assert_eq!(
            kind("video/webm", "audio_20261125_100000.webm"),
            AttachmentKind::Audio
        );
        assert_eq!(
            kind("application/ogg", "audio_20261125_100000.ogg"),
            AttachmentKind::Audio
        );
        // A WebM that isn't a recording stays video, as does an MP4.
        assert_eq!(kind("video/webm", "clip.webm"), AttachmentKind::Video);
        assert_eq!(kind("video/mp4", "clip.mp4"), AttachmentKind::Video);
        assert_eq!(kind("video/mp4", "audio_notes.mp4"), AttachmentKind::Video);
    }

    #[test]
    fn an_ogg_is_audio_unless_named_as_video() {
        let kind = |name: Option<&str>| AttachmentKind::from_file("application/ogg", name, None);
        assert_eq!(kind(Some("track.ogg")), AttachmentKind::Audio);
        assert_eq!(kind(Some("track")), AttachmentKind::Audio);
        assert_eq!(kind(Some("film.ogv")), AttachmentKind::Video);
        // Without a name, the label alone: a file.
        assert_eq!(kind(None), AttachmentKind::File);
    }

    #[test]
    fn an_unlabelled_file_goes_by_its_extension() {
        let kind = |mime: &str, name: &str| AttachmentKind::from_file(mime, Some(name), None);
        assert_eq!(
            kind("application/octet-stream", "a.flac"),
            AttachmentKind::Audio
        );
        assert_eq!(kind("", "a.mkv"), AttachmentKind::Video);
        assert_eq!(
            kind("application/octet-stream", "a.bin"),
            AttachmentKind::File
        );
        // A name never turns another label into media, nor a picture into anything else.
        assert_eq!(kind("application/pdf", "a.mp3"), AttachmentKind::File);
        assert_eq!(kind("image/png", "a.m4a"), AttachmentKind::Image);
        assert_eq!(kind("audio/mpeg", "a.mp4"), AttachmentKind::Audio);
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
        let voice_note = Attachment {
            mime_type: Some("video/mp4".to_owned()),
            original_file_name: Some("Voice note.m4a".to_owned()),
            ..Attachment::default()
        };
        assert_eq!(AttachmentKind::of(&voice_note), AttachmentKind::Audio);
    }
}
