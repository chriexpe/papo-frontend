//! The SDK's one image path: anonymizing, the editor's turn and crop, and fixed-size uploads.
//!
//! Owns inspecting an image, rewriting it to its keep-list, re-encoding pixels and the random
//! name. Leaves staging, drafts and dispatch to `outgoing`, and the profile requests to
//! `operations`.
//!
//! Two routes, one contract: *the sent file shows the picture and nothing else*.
//! - The container route copies only the bytes that draw the picture, the colour profile and the
//!   animation frames, byte for byte (no recompression, animations intact).
//! - The pixel route decodes under hard limits, applies orientation, turns and crop, and encodes
//!   again, keeping the colour profile. It runs only when pixels must change, and for fixed-size
//!   uploads.
//!
//! Anonymizing fails closed: anything that is not positively a JPEG, PNG, GIF or WebP the path can
//! rewrite is a typed refusal.

mod container;
mod edit;
mod error;
mod name;
mod pixels;
mod sniff;

pub use edit::{AttachmentEdit, CropRect, FixedImage, ImageFormat, ImageInfo};
pub use error::{ImageError, UnsupportedFormat};
pub use name::{NAME_LENGTH, random_attachment_name};

pub(crate) use name::with_extension;

use image::metadata::Orientation;
use sniff::{Container, SNIFF_BYTES, sniff};
use std::io::{BufRead, Read, Seek, SeekFrom, Write};

impl Container {
    const fn format(self) -> ImageFormat {
        match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
            Self::Gif => ImageFormat::Gif,
            Self::Webp => ImageFormat::Webp,
        }
    }
}

/// A fixed-size upload, encoded and ready for its request.
pub(crate) struct FixedUpload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) format: ImageFormat,
}

fn container_of<R: Read + Seek>(src: &mut R) -> Result<Container, ImageError> {
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    src.take(SNIFF_BYTES as u64)
        .read_to_end(&mut head)
        .map_err(|error| ImageError::Source(error.kind()))?;
    rewind(src)?;
    sniff(&head)
}

fn rewind<S: Seek>(src: &mut S) -> Result<(), ImageError> {
    src.seek(SeekFrom::Start(0))
        .map(|_| ())
        .map_err(|error| ImageError::Source(error.kind()))
}

/// True when the stored orientation swaps the picture's sides.
const fn swaps_sides(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// Reads a staged file's format, displayed size and whether it is animated, validating its
/// structure on the way. A file that fails here cannot be anonymized.
pub(crate) fn inspect<R: BufRead + Seek>(src: &mut R) -> Result<ImageInfo, ImageError> {
    let container = container_of(src)?;
    let facts = container::clean(container, src, &mut std::io::sink())?;
    rewind(src)?;
    let (width, height, orientation) = pixels::headers(&mut *src, container.format())?;
    let (width, height) = if swaps_sides(orientation) {
        (height, width)
    } else {
        (width, height)
    };
    Ok(ImageInfo {
        format: container.format(),
        width,
        height,
        animated: facts.animated,
    })
}

/// Applies `edit` to a staged file, writing the result to `out` and returning its format.
///
/// With nothing to anonymize or change the caller never gets here; the bytes go as staged.
pub(crate) fn apply<R: BufRead + Seek>(
    src: &mut R,
    out: &mut dyn Write,
    edit: &AttachmentEdit,
) -> Result<ImageFormat, ImageError> {
    let container = container_of(src)?;
    let format = container.format();
    let upright = if edit.changes_pixels() {
        false
    } else {
        let (_, _, orientation) = pixels::headers(&mut *src, format)?;
        rewind(src)?;
        orientation == Orientation::NoTransforms
    };
    if upright {
        container::clean(container, src, out)?;
        return Ok(format);
    }
    if container::clean(container, src, &mut std::io::sink())?.animated {
        return Err(ImageError::Animated);
    }
    rewind(src)?;
    let mut picture = pixels::open(&mut *src, format)?;
    picture.pixels = pixels::edit(picture.pixels, edit.quarter_turns, edit.crop)?;
    pixels::encode_like(&picture, format, out)
}

/// Re-encodes a source picture for a fixed-size upload: turned and cropped as asked, covering the
/// target, within its byte cap. Re-encoding from pixels leaves no metadata but the colour profile.
pub(crate) fn fixed<R: BufRead + Seek>(
    src: &mut R,
    target: FixedImage,
    quarter_turns: u8,
    crop: Option<CropRect>,
) -> Result<FixedUpload, ImageError> {
    let container = container_of(src)?;
    if container::clean(container, src, &mut std::io::sink())?.animated {
        return Err(ImageError::Animated);
    }
    rewind(src)?;
    let mut picture = pixels::open(&mut *src, container.format())?;
    picture.pixels = pixels::edit(picture.pixels, quarter_turns, crop)?;
    let (width, height) = target.size();
    let (bytes, format) = pixels::encode_fixed(&picture, width, height, target.max_bytes())?;
    Ok(FixedUpload { bytes, format })
}

#[cfg(test)]
mod tests;
