//! JPEG keep-list: frame and scan structure, tables, the Adobe colour marker and the ICC profile.
//!
//! Everything else goes: the JFIF header (it can embed a thumbnail), EXIF, XMP, IPTC, MPF, maker
//! notes, comments, JFXX thumbnails, and any bytes after the first end-of-image (motion-photo
//! video, appended gain maps, hidden payloads).

use super::{Facts, copy_exact, read_byte, read_exact, skip, write_all};
use crate::images::error::ImageError;
use std::io::{BufRead, Write};

const ICC_TAG: &[u8; 12] = b"ICC_PROFILE\0";
const ADOBE_TAG: &[u8; 5] = b"Adobe";
const ADOBE_BYTES: u64 = 12;

pub(super) fn clean(src: &mut impl BufRead, out: &mut dyn Write) -> Result<Facts, ImageError> {
    let mut start = [0_u8; 2];
    read_exact(src, &mut start)?;
    if start != [0xFF, 0xD8] {
        return Err(ImageError::Malformed);
    }
    write_all(out, &start)?;
    let mut saw_frame = false;
    let mut saw_scan = false;
    let mut pending = None;
    loop {
        let marker = match pending.take() {
            Some(marker) => marker,
            None => next_marker(src)?,
        };
        match marker {
            0xD9 => {
                if !(saw_frame && saw_scan) {
                    return Err(ImageError::Malformed);
                }
                write_all(out, &[0xFF, 0xD9])?;
                return Ok(Facts { animated: false });
            }
            0x00 | 0x01 | 0xD0..=0xD8 => return Err(ImageError::Malformed),
            _ => {}
        }
        let mut length = [0_u8; 2];
        read_exact(src, &mut length)?;
        let payload = u64::from(u16::from_be_bytes(length))
            .checked_sub(2)
            .ok_or(ImageError::Malformed)?;
        match marker {
            0xE2 => keep_if_tagged(src, out, marker, length, payload, ICC_TAG)?,
            0xEE => {
                // The Adobe marker is exactly 12 bytes; a longer one could carry anything.
                if payload > ADOBE_BYTES {
                    return Err(ImageError::Malformed);
                }
                keep_if_tagged(src, out, marker, length, payload, ADOBE_TAG)?;
            }
            0xDA => {
                write_segment_head(out, marker, length)?;
                copy_exact(src, out, payload)?;
                saw_scan = true;
                pending = Some(copy_scan(src, out)?);
            }
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                write_segment_head(out, marker, length)?;
                copy_exact(src, out, payload)?;
                saw_frame = true;
            }
            0xC4 | 0xCC | 0xDB | 0xDC | 0xDD | 0xDE | 0xDF => {
                write_segment_head(out, marker, length)?;
                copy_exact(src, out, payload)?;
            }
            _ => skip(src, payload)?,
        }
    }
}

fn write_segment_head(out: &mut dyn Write, marker: u8, length: [u8; 2]) -> Result<(), ImageError> {
    write_all(out, &[0xFF, marker])?;
    write_all(out, &length)
}

/// Keeps an application segment only when it starts with `tag`; any other `APPn` is dropped.
fn keep_if_tagged<const N: usize>(
    src: &mut impl BufRead,
    out: &mut dyn Write,
    marker: u8,
    length: [u8; 2],
    payload: u64,
    tag: &[u8; N],
) -> Result<(), ImageError> {
    let tag_len = N as u64;
    if payload < tag_len {
        return skip(src, payload);
    }
    let mut head = [0_u8; 16];
    let head = &mut head[..N];
    read_exact(src, head)?;
    if head[..] != tag[..] {
        return skip(src, payload - tag_len);
    }
    write_segment_head(out, marker, length)?;
    write_all(out, head)?;
    copy_exact(src, out, payload - tag_len)
}

/// The next marker code: skips fill bytes, rejects anything between segments that is not `0xFF`.
fn next_marker(src: &mut impl BufRead) -> Result<u8, ImageError> {
    if read_byte(src)? != 0xFF {
        return Err(ImageError::Malformed);
    }
    loop {
        let code = read_byte(src)?;
        if code != 0xFF {
            return Ok(code);
        }
    }
}

/// Copies entropy-coded data (with its stuffed zeros and restart markers) up to the next real
/// marker, which is consumed and returned.
fn copy_scan(src: &mut impl BufRead, out: &mut dyn Write) -> Result<u8, ImageError> {
    loop {
        let chunk = src
            .fill_buf()
            .map_err(|error| super::source_error(&error))?;
        if chunk.is_empty() {
            return Err(ImageError::Malformed);
        }
        let Some(at) = chunk.iter().position(|byte| *byte == 0xFF) else {
            let length = chunk.len();
            write_all(out, chunk)?;
            src.consume(length);
            continue;
        };
        write_all(out, &chunk[..at])?;
        src.consume(at + 1);
        let mut code = read_byte(src)?;
        while code == 0xFF {
            code = read_byte(src)?;
        }
        match code {
            0x00 | 0xD0..=0xD7 => write_all(out, &[0xFF, code])?,
            marker => return Ok(marker),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The rewrite is checked on the output's structure, never by trusting the input.

    use super::*;
    use std::io::Cursor;

    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len() + 2).unwrap().to_be_bytes();
        let mut bytes = vec![0xFF, marker];
        bytes.extend_from_slice(&length);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn segments(file: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut found = Vec::new();
        let mut at = 2;
        while at + 4 <= file.len() {
            assert_eq!(file[at], 0xFF);
            let marker = file[at + 1];
            if marker == 0xD9 {
                break;
            }
            let length = usize::from(u16::from_be_bytes([file[at + 2], file[at + 3]]));
            found.push((marker, file[at + 4..at + 2 + length].to_vec()));
            at += 2 + length;
            if marker == 0xDA {
                while !(file[at] == 0xFF
                    && file[at + 1] != 0
                    && !(0xD0..=0xD7).contains(&file[at + 1]))
                {
                    at += 1;
                }
            }
        }
        found
    }

    fn fixture() -> Vec<u8> {
        let mut file = vec![0xFF, 0xD8];
        let mut jfif = b"JFIF\0\x01\x01\0\0\x01\0\x01\x02\x02".to_vec();
        jfif.extend_from_slice(&[9, 9, 9, 9, 9, 9]);
        file.extend(segment(0xE0, &jfif));
        file.extend(segment(0xE0, b"JFXX\0\x10thumbnail"));
        file.extend(segment(0xE1, b"Exif\0\0II*\0 GPS camera MakerNote"));
        file.extend(segment(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta/>"));
        file.extend(segment(0xED, b"Photoshop 3.0\0IPTC"));
        file.extend(segment(0xE2, b"MPF\0second image"));
        file.extend(segment(0xE2, b"ICC_PROFILE\0\x01\x01profile-bytes"));
        file.extend(segment(0xEE, b"Adobe\0\x64\0\0\0\0\x01"));
        file.extend(segment(0xFE, b"comment with a name"));
        file.extend(segment(0xDB, &[0; 65]));
        file.extend(segment(0xC0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]));
        file.extend(segment(0xC4, &[0; 17]));
        file.extend(segment(0xDA, &[1, 1, 0, 0, 0x3F, 0]));
        file.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56]);
        file.extend_from_slice(&[0xFF, 0xD9]);
        file.extend_from_slice(b"TRAILER: motion photo video and a hidden message");
        file
    }

    fn cleaned(file: Vec<u8>) -> Result<Vec<u8>, ImageError> {
        let mut out = Vec::new();
        clean(&mut Cursor::new(file), &mut out).map(|_| out)
    }

    #[test]
    fn only_the_keep_list_survives() {
        let out = cleaned(fixture()).unwrap();
        let markers: Vec<u8> = segments(&out).iter().map(|(marker, _)| *marker).collect();
        assert_eq!(markers, vec![0xE2, 0xEE, 0xDB, 0xC0, 0xC4, 0xDA]);
        let text = String::from_utf8_lossy(&out);
        for gone in [
            "JFIF",
            "JFXX",
            "Exif",
            "xmpmeta",
            "Photoshop",
            "MPF",
            "comment",
            "TRAILER",
        ] {
            assert!(!text.contains(gone), "{gone} survived");
        }
        assert!(text.contains("profile-bytes"));
    }

    #[test]
    fn scan_data_with_stuffing_and_restart_markers_is_copied_exactly() {
        let out = cleaned(fixture()).unwrap();
        let scan = [0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD0, 0x56, 0xFF, 0xD9];
        assert!(out.ends_with(&scan));
    }

    #[test]
    fn cleaning_is_idempotent() {
        let once = cleaned(fixture()).unwrap();
        assert_eq!(cleaned(once.clone()).unwrap(), once);
    }

    #[test]
    fn a_file_without_a_scan_or_end_is_refused() {
        let mut file = vec![0xFF, 0xD8];
        file.extend(segment(0xC0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]));
        file.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(cleaned(file), Err(ImageError::Malformed));
        let truncated = fixture()[..40].to_vec();
        assert_eq!(cleaned(truncated), Err(ImageError::Malformed));
    }

    #[test]
    fn an_oversized_adobe_marker_is_refused() {
        let mut file = fixture();
        let at = file.windows(5).position(|w| w == b"Adobe").unwrap() - 4;
        let mut big = segment(0xEE, &[b"Adobe".as_slice(), &[7; 60]].concat());
        file.splice(at..at + 4 + 12, std::iter::empty());
        file.splice(at..at, big.drain(..));
        assert_eq!(cleaned(file), Err(ImageError::Malformed));
    }

    #[test]
    fn garbage_between_segments_is_refused() {
        let mut file = vec![0xFF, 0xD8, 0x00];
        file.extend(segment(0xDB, &[0; 65]));
        assert_eq!(cleaned(file), Err(ImageError::Malformed));
    }
}
