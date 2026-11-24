//! Recognises an image by its magic bytes, never by its name or declared type.

use super::error::{ImageError, UnsupportedFormat};

/// A container the keep-list rewriters can clean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Container {
    Jpeg,
    Png,
    Gif,
    Webp,
}

/// Bytes `sniff` needs to decide.
pub(crate) const SNIFF_BYTES: usize = 32;

/// Classifies the first bytes of a file.
///
/// A recognised format the SDK cannot clean is [`ImageError::Unsupported`]; anything else is
/// [`ImageError::NotAnImage`].
pub(crate) fn sniff(head: &[u8]) -> Result<Container, ImageError> {
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(Container::Jpeg);
    }
    if head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok(Container::Png);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Ok(Container::Gif);
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Ok(Container::Webp);
    }
    Err(refusal(head))
}

fn refusal(head: &[u8]) -> ImageError {
    let unsupported = |kind| ImageError::Unsupported(kind);
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") || head.starts_with(b"II+\0") {
        return unsupported(UnsupportedFormat::Tiff);
    }
    let raw_signatures: [&[u8]; 5] = [b"FUJIFILMCCD-RAW", b"IIRO", b"IIRS", b"MMOR", b"IIU\0"];
    if raw_signatures
        .iter()
        .any(|signature| head.starts_with(signature))
        || head.starts_with(b"II\x1a\0\0\0HEAPCCDR")
    {
        return unsupported(UnsupportedFormat::Raw);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return match &head[8..12] {
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs"
            | b"mif1" | b"msf1" => unsupported(UnsupportedFormat::Heic),
            b"crx " => unsupported(UnsupportedFormat::Raw),
            b"avif" | b"avis" | b"jxl " => unsupported(UnsupportedFormat::Other),
            _ => ImageError::NotAnImage,
        };
    }
    let other_images: [&[u8]; 6] = [
        b"BM",
        b"\0\0\x01\0",
        b"8BPS",
        b"\xFF\x0A",
        b"\0\0\0\x0CJXL ",
        b"qoif",
    ];
    if other_images
        .iter()
        .any(|signature| head.starts_with(signature))
    {
        return unsupported(UnsupportedFormat::Other);
    }
    ImageError::NotAnImage
}

#[cfg(test)]
mod tests {
    //! Format recognition decides what is cleaned and what is refused.

    use super::*;

    #[test]
    fn the_four_cleanable_containers_are_recognised_by_magic() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE1]), Ok(Container::Jpeg));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Ok(Container::Png));
        assert_eq!(sniff(b"GIF89a...."), Ok(Container::Gif));
        assert_eq!(sniff(b"RIFF\x10\0\0\0WEBPVP8 "), Ok(Container::Webp));
    }

    #[test]
    fn heic_tiff_and_raw_are_typed_refusals() {
        let heic = sniff(b"\0\0\0\x18ftypheic\0\0\0\0");
        assert_eq!(heic, Err(ImageError::Unsupported(UnsupportedFormat::Heic)));
        let tiff = sniff(b"II*\0\x08\0\0\0");
        assert_eq!(tiff, Err(ImageError::Unsupported(UnsupportedFormat::Tiff)));
        let cr3 = sniff(b"\0\0\0\x18ftypcrx \0\0\0\0");
        assert_eq!(cr3, Err(ImageError::Unsupported(UnsupportedFormat::Raw)));
        let raf = sniff(b"FUJIFILMCCD-RAW 0201");
        assert_eq!(raf, Err(ImageError::Unsupported(UnsupportedFormat::Raw)));
    }

    #[test]
    fn video_text_and_empty_input_are_not_images() {
        let mp4 = sniff(b"\0\0\0\x18ftypisom\0\0\0\0");
        assert_eq!(mp4, Err(ImageError::NotAnImage));
        assert_eq!(sniff(b"%PDF-1.7"), Err(ImageError::NotAnImage));
        assert_eq!(sniff(b""), Err(ImageError::NotAnImage));
    }

    #[test]
    fn the_extension_never_matters() {
        // The sniffer sees bytes only; a PNG named .jpg is still a PNG.
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Ok(Container::Png));
    }
}
