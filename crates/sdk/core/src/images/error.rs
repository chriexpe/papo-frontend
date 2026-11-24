//! Typed image failures; never carries file contents, names or paths.

/// A format the image path recognises but cannot clean or re-encode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnsupportedFormat {
    /// HEIC/HEIF, including iPhone photos.
    Heic,
    /// TIFF, including `BigTIFF`.
    Tiff,
    /// A camera RAW container (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2, CRW).
    Raw,
    /// Another known image format (AVIF, BMP, ICO, JPEG XL, PSD) the path does not write.
    Other,
}

/// Why an image could not be inspected, cleaned or re-encoded.
///
/// Anonymizing fails closed: any file that is not positively a JPEG, PNG, GIF or WebP the path can
/// rewrite is refused, never sent as it is.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ImageError {
    /// A recognised image format the SDK cannot clean.
    #[error("image format cannot be cleaned")]
    Unsupported(UnsupportedFormat),
    /// The bytes are not an image (a video, audio file or document, until their cleaning exists).
    #[error("not an image")]
    NotAnImage,
    /// The image is damaged or uses a structure the cleaner does not understand.
    #[error("image structure not understood")]
    Malformed,
    /// An animation would have to change its pixels (rotate, crop, resize), which the SDK cannot do
    /// without losing its frames.
    #[error("animated image cannot be edited")]
    Animated,
    /// The decoded picture exceeds the pixel limit, or the encoded result the target's size cap.
    #[error("image too large")]
    TooLarge,
    /// An edit asks for an empty or out-of-bounds crop, or a size the target cannot hold.
    #[error("invalid image edit")]
    InvalidEdit,
    /// Reading the source or writing the result failed.
    #[error("image io failed ({0:?})")]
    Source(std::io::ErrorKind),
}
