//! Keep-list rewriters: copy only the structure that draws the picture, byte for byte, and drop
//! every other segment, chunk, block or trailer. Nothing is decoded and nothing is recompressed.
//!
//! What stays is the pixels' own data, the colour profile and the animation frames. Anything not
//! understood is [`ImageError::Malformed`], never copied through.

mod gif;
mod jpeg;
mod png;
mod webp;

use super::{error::ImageError, sniff::Container};
use std::io::{BufRead, Read, Seek, Write};

const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// What the rewrite learned about the file on the way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Facts {
    /// More than one frame (GIF, APNG, animated WebP).
    pub(super) animated: bool,
}

/// Streams `src` through its container's keep-list into `out`.
pub(super) fn clean<R: BufRead + Seek>(
    container: Container,
    src: &mut R,
    out: &mut dyn Write,
) -> Result<Facts, ImageError> {
    match container {
        Container::Jpeg => jpeg::clean(src, out),
        Container::Png => png::clean(src, out),
        Container::Gif => gif::clean(src, out),
        Container::Webp => webp::clean(src, out),
    }
}

pub(super) fn source_error(error: &std::io::Error) -> ImageError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        ImageError::Malformed
    } else {
        ImageError::Source(error.kind())
    }
}

fn read_exact(src: &mut impl Read, buffer: &mut [u8]) -> Result<(), ImageError> {
    src.read_exact(buffer).map_err(|error| source_error(&error))
}

fn read_byte(src: &mut impl Read) -> Result<u8, ImageError> {
    let mut byte = [0_u8; 1];
    read_exact(src, &mut byte)?;
    Ok(byte[0])
}

fn write_all(out: &mut dyn Write, bytes: &[u8]) -> Result<(), ImageError> {
    out.write_all(bytes)
        .map_err(|error| ImageError::Source(error.kind()))
}

/// Copies exactly `length` bytes; a source that ends early is malformed.
fn copy_exact(src: &mut impl Read, out: &mut dyn Write, length: u64) -> Result<(), ImageError> {
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let mut remaining = length;
    while remaining > 0 {
        let want =
            usize::try_from(remaining.min(COPY_BUFFER_BYTES as u64)).unwrap_or(COPY_BUFFER_BYTES);
        read_exact(src, &mut buffer[..want])?;
        write_all(out, &buffer[..want])?;
        remaining -= want as u64;
    }
    Ok(())
}

/// Discards exactly `length` bytes; a source that ends early is malformed.
fn skip(src: &mut impl Read, length: u64) -> Result<(), ImageError> {
    copy_exact(src, &mut std::io::sink(), length)
}

#[cfg(test)]
mod tests {
    //! Shared behaviour of the rewriters.

    use super::*;
    use std::io::Cursor;

    #[test]
    fn copy_exact_fails_closed_on_a_short_source() {
        let mut out = Vec::new();
        let result = copy_exact(&mut Cursor::new(vec![1_u8; 3]), &mut out, 4);
        assert_eq!(result, Err(ImageError::Malformed));
    }

    #[test]
    fn skip_consumes_exactly_the_length() {
        let mut src = Cursor::new(vec![1_u8, 2, 3, 4]);
        skip(&mut src, 3).unwrap();
        assert_eq!(read_byte(&mut src), Ok(4));
    }
}
