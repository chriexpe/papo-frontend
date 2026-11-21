//! Per-post frame identity and center-pixel color verification for Android video proofs.

use papo_call::PackedBgra;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExpectedColor {
    Red,
    Green,
}

impl ExpectedColor {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Green => "green",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelCheck {
    pub rgb: [u8; 3],
    pub matched: bool,
}

pub(crate) fn verify_center_pixel(bgra: &PackedBgra, expected: ExpectedColor) -> PixelCheck {
    let width = bgra.size_px[0] as usize;
    let height = bgra.size_px[1] as usize;
    if width == 0 || height == 0 {
        return PixelCheck {
            rgb: [0, 0, 0],
            matched: false,
        };
    }
    let Some(row_bytes) = width.checked_mul(4) else {
        return PixelCheck {
            rgb: [0, 0, 0],
            matched: false,
        };
    };
    let Some(total_bytes) = row_bytes.checked_mul(height) else {
        return PixelCheck {
            rgb: [0, 0, 0],
            matched: false,
        };
    };
    if bgra.bytes.len() < total_bytes {
        return PixelCheck {
            rgb: [0, 0, 0],
            matched: false,
        };
    }
    let offset = (height / 2) * row_bytes + (width / 2) * 4;
    let Some(px) = bgra.bytes.get(offset..offset + 4) else {
        return PixelCheck {
            rgb: [0, 0, 0],
            matched: false,
        };
    };
    let rgb = [px[2], px[1], px[0]];
    let matched = match expected {
        ExpectedColor::Red => rgb[0] > 180 && rgb[1] < 60 && rgb[2] < 60,
        // FFmpeg color=green produces #008000 (G ≈ 128).
        ExpectedColor::Green => rgb[1] > 100 && rgb[0] < 60 && rgb[2] < 60,
    };
    PixelCheck { rgb, matched }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_bgra(width: u32, height: u32, bgra: [u8; 4]) -> PackedBgra {
        let count = (width as usize) * (height as usize);
        let mut bytes = Vec::with_capacity(count * 4);
        for _ in 0..count {
            bytes.extend_from_slice(&bgra);
        }
        PackedBgra {
            size_px: [width, height],
            bytes,
        }
    }

    #[test]
    fn pixel_check_accepts_expected_chroma_and_rejects_mismatch() {
        let red = solid_bgra(16, 16, [10, 12, 240, 255]);
        assert_eq!(
            verify_center_pixel(&red, ExpectedColor::Red),
            PixelCheck {
                rgb: [240, 12, 10],
                matched: true,
            }
        );
        assert!(!verify_center_pixel(&red, ExpectedColor::Green).matched);

        let green = solid_bgra(16, 16, [8, 128, 6, 255]);
        assert_eq!(
            verify_center_pixel(&green, ExpectedColor::Green),
            PixelCheck {
                rgb: [6, 128, 8],
                matched: true,
            }
        );
        assert!(!verify_center_pixel(&green, ExpectedColor::Red).matched);
    }

    #[test]
    fn malformed_or_short_buffers_fail_without_panicking() {
        let empty = PackedBgra {
            size_px: [0, 10],
            bytes: vec![],
        };
        assert!(!verify_center_pixel(&empty, ExpectedColor::Red).matched);

        let short = PackedBgra {
            size_px: [10, 10],
            bytes: vec![0; 20],
        };
        assert!(!verify_center_pixel(&short, ExpectedColor::Green).matched);
    }
}
