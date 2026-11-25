//! How many frames a GIF holds, read from its block structure without decoding a pixel, so an
//! animation's decoded size can be counted before it is kept (`media_store`'s budgets). Owns
//! only the count; decoding is Rune's.

/// The frames in GIF `bytes`; none when they are not a GIF or end in the middle of a block.
pub(crate) fn gif_frames(bytes: &[u8]) -> Option<usize> {
    if !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let packed = *bytes.get(10)?;
    let mut at = 13 + colour_table_bytes(packed);
    let mut frames = 0;
    while let Some(&introducer) = bytes.get(at) {
        match introducer {
            // An extension: its label, then its sub-blocks.
            0x21 => at = skip_sub_blocks(bytes, at + 2)?,
            // An image: its descriptor, its own colour table, the LZW code size, the data.
            0x2C => {
                let packed = *bytes.get(at + 9)?;
                at += 10 + colour_table_bytes(packed) + 1;
                at = skip_sub_blocks(bytes, at)?;
                frames += 1;
            }
            // The trailer, or anything else: what came before counts.
            _ => break,
        }
    }
    Some(frames)
}

/// The bytes of the colour table a packed field announces.
fn colour_table_bytes(packed: u8) -> usize {
    if packed & 0x80 == 0 {
        return 0;
    }
    3 << ((packed & 0x07) + 1)
}

/// Where the sub-blocks starting at `at` end (after their zero-length terminator); none when
/// the bytes end first.
fn skip_sub_blocks(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let length = usize::from(*bytes.get(at)?);
        at += 1;
        if length == 0 {
            return Some(at);
        }
        at += length;
    }
}

#[cfg(test)]
mod tests {
    //! Counts on hand-built GIFs.

    use super::gif_frames;

    /// A GIF of `frames` 1 × 1 frames, each with a graphic control extension.
    fn gif(frames: usize) -> Vec<u8> {
        let mut bytes = b"GIF89a".to_vec();
        // 1 × 1, a global table of two colours.
        bytes.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        for _ in 0..frames {
            bytes.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
            bytes.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
            bytes.extend_from_slice(&[2, 2, 0x4C, 0x01, 0]);
        }
        bytes.push(0x3B);
        bytes
    }

    #[test]
    fn frames_are_counted() {
        assert_eq!(gif_frames(&gif(1)), Some(1));
        assert_eq!(gif_frames(&gif(23)), Some(23));
    }

    #[test]
    fn not_a_gif_or_a_cut_one() {
        assert_eq!(gif_frames(b"\x89PNG\r\n\x1a\n"), None);
        let mut cut = gif(3);
        cut.truncate(cut.len() - 10);
        assert_eq!(gif_frames(&cut), None);
        assert_eq!(gif_frames(b"GIF89a"), None);
    }
}
