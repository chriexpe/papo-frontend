//! WebP keep-list: the bitstream chunks, the colour profile and the animation frames.
//!
//! EXIF, XMP and every unknown chunk go, at the top level and inside animation frames. The RIFF
//! sizes are written before their content, so the rewrite first lists the chunks by seeking (no
//! pixel data is read) and then copies the kept ranges.

use super::{Facts, copy_exact, read_exact, write_all};
use crate::images::error::ImageError;
use std::io::{BufRead, Seek, SeekFrom, Write};

const MAX_CHUNKS: usize = 1 << 20;
const FRAME_HEADER_BYTES: u64 = 16;
const EXTENDED_HEADER_BYTES: u64 = 10;
const ANIMATION_HEADER_BYTES: u64 = 6;
const FLAG_ICC: u8 = 0x20;
const FLAG_ANIMATION: u8 = 0x02;
const FLAGS_DROPPED: u8 = 0x08 | 0x04; // EXIF, XMP

const TOP_LEVEL: [&[u8; 4]; 7] = [
    b"VP8X", b"ICCP", b"ANIM", b"ANMF", b"ALPH", b"VP8 ", b"VP8L",
];
const FRAME_PARTS: [&[u8; 4]; 3] = [b"ALPH", b"VP8 ", b"VP8L"];

/// One chunk's place in the source: its payload start and length, not its padding.
#[derive(Clone, Copy)]
struct Chunk {
    kind: [u8; 4],
    at: u64,
    length: u64,
}

impl Chunk {
    fn padded(self) -> u64 {
        self.length + (self.length & 1)
    }
}

/// What will be written for one top-level chunk.
struct Kept {
    chunk: Chunk,
    /// The kept sub-chunks of an animation frame; empty for any other chunk.
    parts: Vec<Chunk>,
}

impl Kept {
    /// Payload bytes after the rewrite.
    fn length(&self) -> u64 {
        if &self.chunk.kind == b"ANMF" {
            FRAME_HEADER_BYTES + self.parts.iter().map(|part| 8 + part.padded()).sum::<u64>()
        } else {
            self.chunk.length
        }
    }

    fn written_bytes(&self) -> u64 {
        8 + self.length() + (self.length() & 1)
    }
}

pub(super) fn clean<R: BufRead + Seek>(
    src: &mut R,
    out: &mut dyn Write,
) -> Result<Facts, ImageError> {
    let total = src
        .seek(SeekFrom::End(0))
        .map_err(|error| super::source_error(&error))?;
    seek(src, 0)?;
    let mut head = [0_u8; 12];
    read_exact(src, &mut head)?;
    if &head[..4] != b"RIFF" || &head[8..12] != b"WEBP" {
        return Err(ImageError::Malformed);
    }
    let end = 8 + u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
    if end > total || end < 12 {
        return Err(ImageError::Malformed);
    }
    let chunks = list(src, 12, end)?;
    let kept = plan(src, chunks)?;
    let animated = kept.iter().any(|entry| &entry.chunk.kind == b"ANMF");
    let flags_animated = kept.first().is_some_and(|entry| {
        &entry.chunk.kind == b"VP8X"
            && peek_flags(src, entry.chunk.at).is_ok_and(|flags| flags & FLAG_ANIMATION != 0)
    });
    let size = 4 + kept.iter().map(Kept::written_bytes).sum::<u64>();
    write_all(out, b"RIFF")?;
    let size = u32::try_from(size).map_err(|_| ImageError::TooLarge)?;
    write_all(out, &size.to_le_bytes())?;
    write_all(out, b"WEBP")?;
    let has_icc = kept.iter().any(|entry| &entry.chunk.kind == b"ICCP");
    for entry in &kept {
        write_chunk(src, out, entry, has_icc)?;
    }
    Ok(Facts {
        animated: animated || flags_animated,
    })
}

fn seek<R: Seek>(src: &mut R, at: u64) -> Result<(), ImageError> {
    src.seek(SeekFrom::Start(at))
        .map(|_| ())
        .map_err(|error| super::source_error(&error))
}

fn peek_flags<R: BufRead + Seek>(src: &mut R, at: u64) -> Result<u8, ImageError> {
    seek(src, at)?;
    super::read_byte(src)
}

/// Lists the chunks between `from` and `end`, validating that each fits inside its parent.
fn list<R: BufRead + Seek>(src: &mut R, from: u64, end: u64) -> Result<Vec<Chunk>, ImageError> {
    let mut chunks = Vec::new();
    let mut at = from;
    while at < end {
        if chunks.len() >= MAX_CHUNKS || end - at < 8 {
            return Err(ImageError::Malformed);
        }
        seek(src, at)?;
        let mut head = [0_u8; 8];
        read_exact(src, &mut head)?;
        let chunk = Chunk {
            kind: [head[0], head[1], head[2], head[3]],
            at: at + 8,
            length: u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]])),
        };
        let next = chunk.at + chunk.padded();
        if next > end {
            return Err(ImageError::Malformed);
        }
        chunks.push(chunk);
        at = next;
    }
    Ok(chunks)
}

/// Chooses the chunks to write and validates the minimum structure of a picture.
fn plan<R: BufRead + Seek>(src: &mut R, chunks: Vec<Chunk>) -> Result<Vec<Kept>, ImageError> {
    let mut kept = Vec::new();
    let mut has_image = false;
    for (index, chunk) in chunks.into_iter().enumerate() {
        if !TOP_LEVEL.contains(&&chunk.kind) {
            continue;
        }
        if &chunk.kind == b"VP8X" && (index != 0 || chunk.length != EXTENDED_HEADER_BYTES) {
            return Err(ImageError::Malformed);
        }
        let once = matches!(&chunk.kind, b"ANIM" | b"ICCP");
        if once
            && kept
                .iter()
                .any(|entry: &Kept| entry.chunk.kind == chunk.kind)
        {
            return Err(ImageError::Malformed);
        }
        if &chunk.kind == b"ANIM" && chunk.length != ANIMATION_HEADER_BYTES {
            return Err(ImageError::Malformed);
        }
        let mut parts = Vec::new();
        if &chunk.kind == b"ANMF" {
            if chunk.length < FRAME_HEADER_BYTES {
                return Err(ImageError::Malformed);
            }
            let inner = list(src, chunk.at + FRAME_HEADER_BYTES, chunk.at + chunk.length)?;
            parts = inner
                .into_iter()
                .filter(|part| FRAME_PARTS.contains(&&part.kind))
                .collect();
            has_image |= parts.iter().any(|part| &part.kind != b"ALPH");
        }
        has_image |= matches!(&chunk.kind, b"VP8 " | b"VP8L");
        kept.push(Kept { chunk, parts });
    }
    if has_image {
        Ok(kept)
    } else {
        Err(ImageError::Malformed)
    }
}

fn write_chunk<R: BufRead + Seek>(
    src: &mut R,
    out: &mut dyn Write,
    entry: &Kept,
    has_icc: bool,
) -> Result<(), ImageError> {
    let length = u32::try_from(entry.length()).map_err(|_| ImageError::TooLarge)?;
    write_all(out, &entry.chunk.kind)?;
    write_all(out, &length.to_le_bytes())?;
    seek(src, entry.chunk.at)?;
    match &entry.chunk.kind {
        b"VP8X" => {
            let mut payload = [0_u8; 10];
            read_exact(src, &mut payload)?;
            payload[0] &= !FLAGS_DROPPED;
            if !has_icc {
                payload[0] &= !FLAG_ICC;
            }
            write_all(out, &payload)?;
        }
        b"ANMF" => {
            copy_exact(src, out, FRAME_HEADER_BYTES)?;
            for part in &entry.parts {
                write_all(out, &part.kind)?;
                let part_length = u32::try_from(part.length).map_err(|_| ImageError::TooLarge)?;
                write_all(out, &part_length.to_le_bytes())?;
                seek(src, part.at)?;
                copy_exact(src, out, part.length)?;
                pad(out, part.length)?;
            }
        }
        _ => copy_exact(src, out, entry.chunk.length)?,
    }
    pad(out, entry.length())
}

fn pad(out: &mut dyn Write, length: u64) -> Result<(), ImageError> {
    if length & 1 == 1 {
        write_all(out, &[0])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Chunk-level checks on synthetic WebP files.

    use super::*;
    use std::io::Cursor;

    fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = kind.to_vec();
        bytes.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(data);
        if data.len() % 2 == 1 {
            bytes.push(0);
        }
        bytes
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.iter().flatten().copied().collect();
        let mut file = b"RIFF".to_vec();
        file.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_le_bytes());
        file.extend_from_slice(b"WEBP");
        file.extend(body);
        file
    }

    fn kinds(file: &[u8]) -> Vec<String> {
        let mut at = 12;
        let mut found = Vec::new();
        while at + 8 <= file.len() {
            let length =
                usize::try_from(u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()))
                    .unwrap();
            found.push(String::from_utf8_lossy(&file[at..at + 4]).into_owned());
            at += 8 + length + (length & 1);
        }
        found
    }

    fn cleaned(file: Vec<u8>) -> Result<(Vec<u8>, Facts), ImageError> {
        let mut out = Vec::new();
        clean(&mut Cursor::new(file), &mut out).map(|facts| (out, facts))
    }

    #[test]
    fn exif_xmp_and_unknown_chunks_go_and_sizes_stay_consistent() {
        let file = riff(&[
            chunk(*b"VP8X", &[0x2C, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            chunk(*b"ICCP", b"profile"),
            chunk(*b"VP8L", &[1, 2, 3]),
            chunk(*b"EXIF", b"II*\0 GPS here"),
            chunk(*b"XMP ", b"<xmp/>"),
            chunk(*b"junk", b"private"),
        ]);
        let (out, facts) = cleaned(file).unwrap();
        assert_eq!(kinds(&out), ["VP8X", "ICCP", "VP8L"]);
        assert!(!facts.animated);
        assert_eq!(out[20] & 0x0C, 0, "EXIF and XMP flags cleared");
        assert_eq!(out[20] & 0x20, 0x20, "ICC flag kept");
        let declared = usize::try_from(u32::from_le_bytes(out[4..8].try_into().unwrap())).unwrap();
        assert_eq!(declared + 8, out.len());
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("GPS") && !text.contains("xmp"));
    }

    #[test]
    fn the_icc_flag_is_cleared_when_the_profile_chunk_is_absent() {
        let file = riff(&[
            chunk(*b"VP8X", &[0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            chunk(*b"VP8 ", &[1]),
        ]);
        assert_eq!(cleaned(file).unwrap().0[20] & 0x20, 0);
    }

    #[test]
    fn animation_frames_are_filtered_inside_and_resized() {
        let mut frame = vec![0_u8; 16];
        frame.extend(chunk(*b"VP8L", &[9, 9, 9]));
        frame.extend(chunk(*b"EXIF", b"frame gps"));
        let file = riff(&[
            chunk(*b"VP8X", &[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            chunk(*b"ANIM", &[0; 6]),
            chunk(*b"ANMF", &frame),
            chunk(*b"ANMF", &frame),
        ]);
        let (out, facts) = cleaned(file).unwrap();
        assert!(facts.animated);
        assert_eq!(kinds(&out), ["VP8X", "ANIM", "ANMF", "ANMF"]);
        assert!(!String::from_utf8_lossy(&out).contains("frame gps"));
        let declared = usize::try_from(u32::from_le_bytes(out[4..8].try_into().unwrap())).unwrap();
        assert_eq!(declared + 8, out.len());
    }

    #[test]
    fn a_simple_file_passes_unchanged() {
        let file = riff(&[chunk(*b"VP8 ", &[1, 2, 3, 4])]);
        assert_eq!(cleaned(file.clone()).unwrap().0, file);
    }

    #[test]
    fn a_repeated_or_oversized_singleton_chunk_is_refused() {
        let twice = riff(&[
            chunk(*b"VP8X", &[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            chunk(*b"ANIM", &[0; 6]),
            chunk(*b"ANIM", &[0; 6]),
            chunk(*b"VP8L", &[1]),
        ]);
        assert_eq!(cleaned(twice).map(|_| ()), Err(ImageError::Malformed));
        let long = riff(&[
            chunk(*b"VP8X", &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            chunk(*b"ANIM", &[0; 40]),
            chunk(*b"VP8L", &[1]),
        ]);
        assert_eq!(cleaned(long).map(|_| ()), Err(ImageError::Malformed));
    }

    #[test]
    fn damaged_files_are_refused() {
        let mut file = riff(&[chunk(*b"VP8 ", &[1, 2, 3, 4])]);
        file[4] = 0xFF;
        assert_eq!(cleaned(file).map(|_| ()), Err(ImageError::Malformed));
        let no_image = riff(&[chunk(*b"EXIF", b"x")]);
        assert_eq!(cleaned(no_image).map(|_| ()), Err(ImageError::Malformed));
        let overrun = {
            let mut bytes = riff(&[chunk(*b"VP8 ", &[1, 2, 3, 4])]);
            bytes[16..20].copy_from_slice(&1000_u32.to_le_bytes());
            bytes
        };
        assert_eq!(cleaned(overrun).map(|_| ()), Err(ImageError::Malformed));
    }
}
