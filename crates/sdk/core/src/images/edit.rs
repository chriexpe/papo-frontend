//! The edit a client records on one attachment, and the fixed-size targets of uploads that
//! always re-encode.

use serde::{Deserialize, Serialize};

/// A rectangle of pixels, `x` and `y` from the top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropRect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width; never zero.
    pub width: u32,
    /// Height; never zero.
    pub height: u32,
}

/// What the sender chose for one attachment, applied when the message is enqueued; until then the
/// original bytes stay staged unchanged, so an edit can be changed or undone.
///
/// The default changes nothing: the bytes go as staged, byte for byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentEdit {
    /// Send only what shows the picture (see the image path's keep-list) under a random name.
    /// A file the SDK cannot clean is refused, never sent as it is.
    pub anonymize: bool,
    /// Send under a random 15-character name (extension kept), whatever the file is. The bytes
    /// are not touched; anonymizing implies it.
    #[serde(default)]
    pub random_name: bool,
    /// Clockwise quarter turns (0 to 3) applied after the image's own orientation.
    pub quarter_turns: u8,
    /// The part kept, in the picture as it looks after orientation and `quarter_turns`.
    pub crop: Option<CropRect>,
}

impl AttachmentEdit {
    /// True when the edit changes nothing.
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        !self.anonymize
            && !self.random_name
            && self.quarter_turns.is_multiple_of(4)
            && self.crop.is_none()
    }

    /// True when the staged bytes must be rewritten (anonymizing, a turn or a crop).
    #[must_use]
    pub const fn rewrites_bytes(&self) -> bool {
        self.anonymize || self.changes_pixels()
    }

    /// True when pixels must change (turns or a crop), whatever anonymizing needs.
    #[must_use]
    pub const fn changes_pixels(&self) -> bool {
        !self.quarter_turns.is_multiple_of(4) || self.crop.is_some()
    }
}

/// An upload that always re-encodes to a fixed size, so it carries no metadata at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FixedImage {
    /// Profile photo: 512 × 512 (the backend's dimension limit), at most two MiB.
    Avatar,
    /// Profile banner: 1500 × 500 (3 : 1 until the backend states its ratio), at most two MiB.
    Banner,
}

impl FixedImage {
    /// Output size in pixels, width then height.
    #[must_use]
    pub const fn size(self) -> (u32, u32) {
        match self {
            Self::Avatar => (512, 512),
            Self::Banner => (1500, 500),
        }
    }

    /// The backend's cap on the encoded bytes.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        2 * 1024 * 1024
    }
}

/// Raster formats the image path reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImageFormat {
    /// JPEG.
    Jpeg,
    /// PNG, including APNG.
    Png,
    /// GIF, including animations.
    Gif,
    /// WebP, including animations.
    Webp,
}

impl ImageFormat {
    /// The label the backend's `*_format` fields and clients use, lowercase.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }

    /// The conventional file extension, without the dot.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            other => other.label(),
        }
    }

    /// True when `extension` (any case, no dot) is a common name for this format.
    #[must_use]
    pub fn accepts_extension(self, extension: &str) -> bool {
        let lower = extension.to_ascii_lowercase();
        match self {
            Self::Jpeg => matches!(lower.as_str(), "jpg" | "jpeg" | "jpe" | "jfif"),
            other => lower == other.label(),
        }
    }
}

/// What the image path can tell a client about a staged file before it is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Container format.
    pub format: ImageFormat,
    /// Width as displayed (orientation applied), in pixels.
    pub width: u32,
    /// Height as displayed (orientation applied), in pixels.
    pub height: u32,
    /// More than one frame; such a file can be anonymized but not cropped or turned.
    pub animated: bool,
}

#[cfg(test)]
mod tests {
    //! The identity edit must stay the one case that never touches bytes.

    use super::*;

    #[test]
    fn the_default_edit_is_the_identity() {
        assert!(AttachmentEdit::default().is_identity());
    }

    #[test]
    fn a_random_name_alone_is_not_the_identity_but_rewrites_nothing() {
        let edit = AttachmentEdit {
            random_name: true,
            ..AttachmentEdit::default()
        };
        assert!(!edit.is_identity() && !edit.rewrites_bytes());
    }

    #[test]
    fn four_turns_change_no_pixels() {
        let edit = AttachmentEdit {
            quarter_turns: 4,
            ..AttachmentEdit::default()
        };
        assert!(edit.is_identity() && !edit.changes_pixels());
    }

    #[test]
    fn anonymizing_alone_leaves_pixels_to_the_cleaner() {
        let edit = AttachmentEdit {
            anonymize: true,
            ..AttachmentEdit::default()
        };
        assert!(!edit.is_identity() && !edit.changes_pixels());
    }

    #[test]
    fn jpeg_names_are_matched_case_insensitively() {
        assert!(ImageFormat::Jpeg.accepts_extension("JPEG"));
        assert!(!ImageFormat::Png.accepts_extension("jpg"));
    }
}
