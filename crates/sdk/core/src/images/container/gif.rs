//! GIF keep-list: header, palettes, graphic control, image data, loop count and the ICC
//! application extension.
//!
//! Comments, plain-text extensions, XMP and every other application extension go, as do any bytes
//! after the trailer. A graphic control is written only when an image follows it, so dropping a
//! plain-text block never lets its timing leak onto the next frame.

use super::{Facts, copy_exact, read_byte, read_exact, skip, write_all};
use crate::images::error::ImageError;
use std::io::{Read, Write};

const GRAPHIC_CONTROL_BYTES: u64 = 4;
const LOOP_EXTENSION_BYTES: u64 = 16;
const KEPT_APPLICATIONS: [&[u8; 11]; 3] = [b"NETSCAPE2.0", b"ANIMEXTS1.0", b"ICCRGBG1012"];

pub(super) fn clean(src: &mut impl Read, out: &mut dyn Write) -> Result<Facts, ImageError> {
    let mut header = [0_u8; 13];
    read_exact(src, &mut header)?;
    if &header[..3] != b"GIF" {
        return Err(ImageError::Malformed);
    }
    write_all(out, &header)?;
    copy_palette(src, out, header[10])?;
    let mut graphic_control: Option<Vec<u8>> = None;
    let mut frames = 0_u32;
    loop {
        match read_byte(src)? {
            0x3B => {
                write_all(out, &[0x3B])?;
                return if frames > 0 {
                    Ok(Facts {
                        animated: frames > 1,
                    })
                } else {
                    Err(ImageError::Malformed)
                };
            }
            0x2C => {
                if let Some(control) = graphic_control.take() {
                    write_all(out, &control)?;
                }
                write_all(out, &[0x2C])?;
                let mut descriptor = [0_u8; 9];
                read_exact(src, &mut descriptor)?;
                write_all(out, &descriptor)?;
                copy_palette(src, out, descriptor[8])?;
                let code_size = read_byte(src)?;
                write_all(out, &[code_size])?;
                copy_blocks(src, out, u64::MAX)?;
                frames += 1;
            }
            0x21 => match read_byte(src)? {
                0xF9 => {
                    let mut control = vec![0x21, 0xF9];
                    let mut blocks = Vec::new();
                    copy_blocks(src, &mut blocks, GRAPHIC_CONTROL_BYTES)?;
                    // Exactly one four-byte block: size, four fields, terminator.
                    if blocks.len() != 6 || blocks[0] != 4 {
                        return Err(ImageError::Malformed);
                    }
                    control.extend(blocks);
                    graphic_control = Some(control);
                }
                0xFF => copy_application(src, out)?,
                label => {
                    skip_blocks(src)?;
                    if label == 0x01 {
                        graphic_control = None;
                    }
                }
            },
            _ => return Err(ImageError::Malformed),
        }
    }
}

/// Copies a colour table announced by `flags` (bit 7 present, bits 0–2 its size).
fn copy_palette(src: &mut impl Read, out: &mut dyn Write, flags: u8) -> Result<(), ImageError> {
    if flags & 0x80 == 0 {
        return Ok(());
    }
    let entries = 1_u64 << ((flags & 0x07) + 1);
    copy_exact(src, out, entries * 3)
}

/// Copies data sub-blocks up to and including the zero-length terminator, refusing more than
/// `limit` payload bytes so a kept structure cannot carry a hidden message.
fn copy_blocks(src: &mut impl Read, out: &mut dyn Write, limit: u64) -> Result<(), ImageError> {
    let mut copied = 0_u64;
    loop {
        let size = read_byte(src)?;
        copied += u64::from(size);
        if copied > limit {
            return Err(ImageError::Malformed);
        }
        write_all(out, &[size])?;
        if size == 0 {
            return Ok(());
        }
        copy_exact(src, out, u64::from(size))?;
    }
}

fn skip_blocks(src: &mut impl Read) -> Result<(), ImageError> {
    loop {
        let size = read_byte(src)?;
        if size == 0 {
            return Ok(());
        }
        skip(src, u64::from(size))?;
    }
}

/// Keeps an application extension only when its identifier is on the list.
fn copy_application(src: &mut impl Read, out: &mut dyn Write) -> Result<(), ImageError> {
    let size = read_byte(src)?;
    let mut identifier = vec![0_u8; usize::from(size)];
    read_exact(src, &mut identifier)?;
    let kept = KEPT_APPLICATIONS
        .iter()
        .any(|name| identifier == name.as_slice());
    if kept {
        write_all(out, &[0x21, 0xFF, size])?;
        write_all(out, &identifier)?;
        // Loop and buffering counters are a handful of bytes; only a profile is large.
        let limit = if identifier == b"ICCRGBG1012" {
            u64::MAX
        } else {
            LOOP_EXTENSION_BYTES
        };
        copy_blocks(src, out, limit)
    } else {
        skip_blocks(src)
    }
}

#[cfg(test)]
mod tests {
    //! Block-level checks on a synthetic two-frame GIF.

    use super::*;
    use std::io::Cursor;

    fn blocks(data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![u8::try_from(data.len()).unwrap()];
        bytes.extend_from_slice(data);
        bytes.push(0);
        bytes
    }

    fn extension(label: u8, data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x21, label];
        bytes.extend(blocks(data));
        bytes
    }

    fn application(name: &[u8; 11], data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x21, 0xFF, 11];
        bytes.extend_from_slice(name);
        bytes.extend(blocks(data));
        bytes
    }

    fn image() -> Vec<u8> {
        let mut bytes = vec![0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2];
        bytes.extend(blocks(&[0x44, 0x01]));
        bytes
    }

    fn fixture(frames: usize) -> Vec<u8> {
        let mut file = b"GIF89a".to_vec();
        file.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]);
        file.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        file.extend(application(b"NETSCAPE2.0", &[1, 0, 0]));
        file.extend(application(b"XMP DataXMP", b"<x:xmpmeta GPS/>"));
        file.extend(application(b"ICCRGBG1012", b"profile"));
        file.extend(extension(0xFE, b"made by Jane"));
        for _ in 0..frames {
            file.extend(extension(0xF9, &[0x04, 10, 0, 0]));
            file.extend(extension(0x01, b"plain text overlay"));
            file.extend(image());
        }
        file.push(0x3B);
        file.extend_from_slice(b"TRAILER");
        file
    }

    fn cleaned(file: Vec<u8>) -> Result<(Vec<u8>, Facts), ImageError> {
        let mut out = Vec::new();
        clean(&mut Cursor::new(file), &mut out).map(|facts| (out, facts))
    }

    #[test]
    fn only_the_keep_list_survives_and_frames_stay() {
        let (out, facts) = cleaned(fixture(2)).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("NETSCAPE2.0") && text.contains("ICCRGBG1012"));
        for gone in ["XMP Data", "xmpmeta", "Jane", "plain text", "TRAILER"] {
            assert!(!text.contains(gone), "{gone} survived");
        }
        assert!(facts.animated);
        assert_eq!(
            out.iter()
                .map(|byte| usize::from(*byte == 0x2C))
                .sum::<usize>(),
            2
        );
        assert_eq!(out.last(), Some(&0x3B));
    }

    #[test]
    fn a_graphic_control_before_dropped_text_does_not_reach_the_next_image() {
        let (out, _) = cleaned(fixture(1)).unwrap();
        assert!(!out.windows(2).any(|pair| pair == [0x21, 0xF9]));
    }

    #[test]
    fn a_graphic_control_or_loop_block_that_carries_more_is_refused() {
        let mut file = b"GIF89a".to_vec();
        file.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0]);
        file.extend(extension(0xF9, &[0x04, 10, 0, 0, 9, 9]));
        file.extend(image());
        file.push(0x3B);
        assert_eq!(cleaned(file).map(|_| ()), Err(ImageError::Malformed));
        let mut file = b"GIF89a".to_vec();
        file.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0]);
        file.extend(application(b"NETSCAPE2.0", &[7; 40]));
        file.extend(image());
        file.push(0x3B);
        assert_eq!(cleaned(file).map(|_| ()), Err(ImageError::Malformed));
    }

    #[test]
    fn a_single_frame_is_not_animated() {
        assert!(!cleaned(fixture(1)).unwrap().1.animated);
    }

    #[test]
    fn a_file_without_frames_or_trailer_is_refused() {
        assert_eq!(cleaned(fixture(0)).map(|_| ()), Err(ImageError::Malformed));
        let mut truncated = fixture(1);
        truncated.truncate(30);
        assert_eq!(cleaned(truncated).map(|_| ()), Err(ImageError::Malformed));
    }
}
