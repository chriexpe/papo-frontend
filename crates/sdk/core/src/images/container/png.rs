//! PNG keep-list: the image's own chunks, its colour description and APNG animation chunks.
//!
//! Text, time, EXIF, physical size, significant bits, background, histograms and every private
//! chunk go, as do any bytes after IEND.

use super::{Facts, copy_exact, read_exact, skip, write_all};
use crate::images::error::ImageError;
use std::io::{Read, Write};

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const MAX_CHUNK_BYTES: u32 = 0x7FFF_FFFF;
const KEEP: [&[u8; 4]; 12] = [
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"iCCP", b"cICP",
    b"acTL", b"fcTL",
];
const FRAME_DATA: &[u8; 4] = b"fdAT";
const ANIMATION_CONTROL: &[u8; 4] = b"acTL";
const MAX_PROFILE_BYTES: u32 = 1 << 24;
/// Kept chunks with one fixed length, and whether they may appear only once.
const FIXED: [(&[u8; 4], u32, bool); 7] = [
    (b"IHDR", 13, true),
    (b"gAMA", 4, true),
    (b"cHRM", 32, true),
    (b"sRGB", 1, true),
    (b"cICP", 4, true),
    (b"acTL", 8, true),
    (b"fcTL", 26, false),
];

/// Checks a kept chunk's length and count, so a kept type cannot carry a payload.
fn well_formed(kind: [u8; 4], length: u32, seen: &[[u8; 4]]) -> bool {
    let repeated = seen.contains(&kind);
    if let Some((_, fixed, once)) = FIXED.iter().find(|(name, ..)| **name == kind)
        && (length != *fixed || (*once && repeated))
    {
        return false;
    }
    match &kind {
        b"PLTE" => length <= 768 && length.is_multiple_of(3) && !repeated,
        b"tRNS" => length <= 768 && !repeated,
        b"iCCP" => length <= MAX_PROFILE_BYTES && !repeated,
        _ => true,
    }
}

pub(super) fn clean(src: &mut impl Read, out: &mut dyn Write) -> Result<Facts, ImageError> {
    let mut signature = [0_u8; 8];
    read_exact(src, &mut signature)?;
    if signature != SIGNATURE {
        return Err(ImageError::Malformed);
    }
    write_all(out, &signature)?;
    let mut first = true;
    let mut animated = false;
    let mut saw_data = false;
    let mut seen: Vec<[u8; 4]> = Vec::new();
    loop {
        let mut head = [0_u8; 8];
        read_exact(src, &mut head)?;
        let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        let kind: [u8; 4] = [head[4], head[5], head[6], head[7]];
        if length > MAX_CHUNK_BYTES || (first && &kind != b"IHDR") {
            return Err(ImageError::Malformed);
        }
        first = false;
        // An orientation after the pixels cannot be trusted to have been read first.
        if &kind == b"eXIf" && saw_data {
            return Err(ImageError::Malformed);
        }
        // Data plus the four-byte CRC.
        let body = u64::from(length) + 4;
        let keep = KEEP.contains(&&kind) || &kind == FRAME_DATA;
        if keep && !well_formed(kind, length, &seen) {
            return Err(ImageError::Malformed);
        }
        if keep {
            if !matches!(&kind, b"IDAT" | b"fdAT" | b"fcTL" | b"IEND") {
                seen.push(kind);
            }
            write_all(out, &head)?;
            copy_exact(src, out, body)?;
        } else {
            skip(src, body)?;
        }
        animated |= &kind == ANIMATION_CONTROL;
        saw_data |= &kind == b"IDAT";
        if &kind == b"IEND" {
            return if saw_data {
                Ok(Facts { animated })
            } else {
                Err(ImageError::Malformed)
            };
        }
    }
}

#[cfg(test)]
mod tests {
    //! Chunk-level checks on a synthetic PNG; the CRC is copied, not recomputed.

    use super::*;
    use std::io::Cursor;

    fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(&kind);
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(&[0xC0, 0xFF, 0xEE, 0x01]);
        bytes
    }

    fn kinds(file: &[u8]) -> Vec<String> {
        let mut at = 8;
        let mut found = Vec::new();
        while at + 12 <= file.len() {
            let length =
                usize::try_from(u32::from_be_bytes(file[at..at + 4].try_into().unwrap())).unwrap();
            found.push(String::from_utf8_lossy(&file[at + 4..at + 8]).into_owned());
            at += 12 + length;
        }
        found
    }

    fn fixture(extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut file = SIGNATURE.to_vec();
        file.extend(chunk(*b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]));
        file.extend(chunk(*b"eXIf", b"MM\0*GPS"));
        file.extend(chunk(*b"iCCP", b"profile\0\0zlib"));
        file.extend(chunk(*b"tEXt", b"Author\0Jane Doe"));
        file.extend(chunk(*b"zTXt", b"Comment\0\0zz"));
        file.extend(chunk(*b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<xmp/>"));
        file.extend(chunk(*b"tIME", &[7, 232, 11, 24, 1, 2, 3]));
        file.extend(chunk(*b"pHYs", &[0; 9]));
        file.extend(chunk(*b"prVt", b"private"));
        for (kind, data) in extra {
            file.extend(chunk(**kind, data));
        }
        file.extend(chunk(*b"IDAT", &[1, 2, 3]));
        file.extend(chunk(*b"IEND", &[]));
        file.extend_from_slice(b"TRAILER hidden");
        file
    }

    fn cleaned(file: Vec<u8>) -> Result<(Vec<u8>, Facts), ImageError> {
        let mut out = Vec::new();
        clean(&mut Cursor::new(file), &mut out).map(|facts| (out, facts))
    }

    #[test]
    fn only_the_keep_list_survives() {
        let (out, facts) = cleaned(fixture(&[])).unwrap();
        assert_eq!(kinds(&out), ["IHDR", "iCCP", "IDAT", "IEND"]);
        assert!(!facts.animated);
        assert!(!String::from_utf8_lossy(&out).contains("TRAILER"));
    }

    #[test]
    fn animation_chunks_survive_and_mark_the_file_animated() {
        let extra: [(&[u8; 4], &[u8]); 3] =
            [(b"acTL", &[0; 8]), (b"fcTL", &[0; 26]), (b"fdAT", &[0; 8])];
        let (out, facts) = cleaned(fixture(&extra)).unwrap();
        assert_eq!(
            kinds(&out),
            ["IHDR", "iCCP", "acTL", "fcTL", "fdAT", "IDAT", "IEND"]
        );
        assert!(facts.animated);
    }

    #[test]
    fn a_file_without_pixels_or_end_is_refused() {
        let mut file = SIGNATURE.to_vec();
        file.extend(chunk(*b"IHDR", &[0; 13]));
        file.extend(chunk(*b"IEND", &[]));
        assert_eq!(cleaned(file).map(|_| ()), Err(ImageError::Malformed));
        let mut truncated = fixture(&[]);
        truncated.truncate(60);
        assert_eq!(cleaned(truncated).map(|_| ()), Err(ImageError::Malformed));
    }

    #[test]
    fn a_kept_chunk_with_the_wrong_length_or_a_repeat_is_refused() {
        let oversized = fixture(&[(b"gAMA", &[0; 10])]);
        assert_eq!(cleaned(oversized).map(|_| ()), Err(ImageError::Malformed));
        let repeated = fixture(&[(b"gAMA", &[0; 4]), (b"gAMA", &[0; 4])]);
        assert_eq!(cleaned(repeated).map(|_| ()), Err(ImageError::Malformed));
    }

    #[test]
    fn an_orientation_after_the_pixels_is_refused_not_ignored() {
        let mut file = SIGNATURE.to_vec();
        file.extend(chunk(*b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]));
        file.extend(chunk(*b"IDAT", &[1]));
        file.extend(chunk(*b"eXIf", b"MM\0*"));
        file.extend(chunk(*b"IEND", &[]));
        assert_eq!(cleaned(file).map(|_| ()), Err(ImageError::Malformed));
    }

    #[test]
    fn the_first_chunk_must_be_the_header() {
        let mut file = SIGNATURE.to_vec();
        file.extend(chunk(*b"IDAT", &[1]));
        assert_eq!(cleaned(file).map(|_| ()), Err(ImageError::Malformed));
    }
}
