//! Checked NV12 rows and packed CPU rotations. Rendering and GPU lifetime belong to the host.

use crate::CpuVideoFrame;
use gstreamer_video::{VideoColorMatrix, VideoColorRange, VideoColorimetry};
use std::{error::Error, fmt};

/// Clockwise rotation from source orientation to presentation orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoRotation {
    /// Preserve source orientation.
    #[default]
    Upright,
    /// Rotate clockwise by one quarter turn.
    Right,
    /// Rotate by two quarter turns.
    Down,
    /// Rotate clockwise by three quarter turns.
    Left,
}

/// A tightly packed Y plane and interleaved CbCr plane in source orientation.
pub struct PackedNv12 {
    /// Width and height in pixels. The chroma plane uses ceil(width/2) by ceil(height/2) pairs.
    pub size_px: [u32; 2],
    /// Source color matrix, range, transfer and primaries.
    pub colorimetry: VideoColorimetry,
    /// Packed planes: Y, then interleaved CbCr.
    pub planes: [Vec<u8>; 2],
}

/// Tightly packed opaque BGRA pixels in the requested presentation orientation.
pub struct PackedBgra {
    /// Width and height in pixels after rotation.
    pub size_px: [u32; 2],
    /// Row-major BGRA bytes, width times four bytes per row.
    pub bytes: Vec<u8>,
}

/// Why a decoded frame cannot be packed or converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum VideoPixelsError {
    /// Empty dimensions, a short/negative stride, overlapping planes or an out-of-bounds row.
    InvalidLayout,
    /// An RGB conversion was requested for an unknown or non-YCbCr range/matrix.
    UnsupportedColorimetry,
    /// The packed buffer size was unrepresentable or allocation was refused.
    OutputTooLarge,
}

impl fmt::Display for VideoPixelsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLayout => "invalid NV12 plane layout",
            Self::UnsupportedColorimetry => "unsupported NV12 range or matrix",
            Self::OutputTooLarge => "packed video output is too large",
        })
    }
}

impl Error for VideoPixelsError {}

struct Plane<'a> {
    bytes: &'a [u8],
    stride: usize,
    row_bytes: usize,
    rows: usize,
}

impl<'a> Plane<'a> {
    fn new(
        bytes: &'a [u8],
        stride: i32,
        row_bytes: usize,
        rows: usize,
    ) -> Result<Self, VideoPixelsError> {
        let stride = usize::try_from(stride).map_err(|_| VideoPixelsError::InvalidLayout)?;
        let extent = (rows - 1)
            .checked_mul(stride)
            .and_then(|last| last.checked_add(row_bytes));
        let Some(bytes) = extent
            .and_then(|extent| bytes.get(..extent))
            .filter(|_| stride >= row_bytes)
        else {
            return Err(VideoPixelsError::InvalidLayout);
        };
        Ok(Self {
            bytes,
            stride,
            row_bytes,
            rows,
        })
    }

    fn row(&self, row: usize) -> &[u8] {
        let start = row * self.stride;
        &self.bytes[start..start + self.row_bytes]
    }

    fn packed(&self) -> Result<Vec<u8>, VideoPixelsError> {
        let mut output = allocate(self.rows, self.row_bytes, 1)?;
        for row in 0..self.rows {
            output.extend_from_slice(self.row(row));
        }
        Ok(output)
    }
}

impl CpuVideoFrame {
    fn checked_planes(&self) -> Result<[Plane<'_>; 2], VideoPixelsError> {
        let width = self.width_px as usize;
        let height = self.height_px as usize;
        if width == 0 || height == 0 {
            return Err(VideoPixelsError::InvalidLayout);
        }
        let [y_start, uv_start] = self.offsets;
        let y = self
            .bytes
            .get(y_start..uv_start)
            .ok_or(VideoPixelsError::InvalidLayout)?;
        let uv = self
            .bytes
            .get(uv_start..)
            .ok_or(VideoPixelsError::InvalidLayout)?;
        let uv_width = width
            .div_ceil(2)
            .checked_mul(2)
            .ok_or(VideoPixelsError::InvalidLayout)?;
        Ok([
            Plane::new(y, self.strides[0], width, height)?,
            Plane::new(uv, self.strides[1], uv_width, height.div_ceil(2))?,
        ])
    }

    /// Packs the two planes without converting color or assuming even dimensions.
    pub fn pack_nv12(&self) -> Result<PackedNv12, VideoPixelsError> {
        let [y, uv] = self.checked_planes()?;
        Ok(PackedNv12 {
            size_px: [self.width_px, self.height_px],
            colorimetry: self.colorimetry,
            planes: [y.packed()?, uv.packed()?],
        })
    }

    /// Converts the admitted YCbCr matrix and range into opaque BGRA, and rotates clockwise.
    /// Chroma is sampled once per 2x2 luma block. Transfer remains in source encoding.
    pub fn pack_bgra(&self, rotation: VideoRotation) -> Result<PackedBgra, VideoPixelsError> {
        let [y, uv] = self.checked_planes()?;
        let matrix = Conversion::new(self.colorimetry)?;
        let [width, height] = [y.row_bytes, y.rows];
        let [output_width, output_height] = match rotation {
            VideoRotation::Upright | VideoRotation::Down => [width, height],
            VideoRotation::Right | VideoRotation::Left => [height, width],
        };
        let mut bytes = allocate(output_width, output_height, 4)?;
        bytes.resize(output_width * output_height * 4, 0);
        for row in 0..height {
            let chroma = uv.row(row / 2);
            for (col, luma) in y.row(row).iter().copied().enumerate() {
                let pair = col / 2 * 2;
                let [x, y] = match rotation {
                    VideoRotation::Upright => [col, row],
                    VideoRotation::Right => [height - row - 1, col],
                    VideoRotation::Down => [width - col - 1, height - row - 1],
                    VideoRotation::Left => [row, width - col - 1],
                };
                let start = (y * output_width + x) * 4;
                bytes[start..start + 4].copy_from_slice(&matrix.bgra(
                    luma,
                    chroma[pair],
                    chroma[pair + 1],
                ));
            }
        }
        Ok(PackedBgra {
            size_px: [output_width as u32, output_height as u32],
            bytes,
        })
    }
}

fn allocate(width: usize, height: usize, channels: usize) -> Result<Vec<u8>, VideoPixelsError> {
    let count = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(channels))
        .ok_or(VideoPixelsError::OutputTooLarge)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| VideoPixelsError::OutputTooLarge)?;
    Ok(bytes)
}

struct Conversion {
    y_offset: f64,
    y_scale: f64,
    blue_cb: f64,
    green_cb: f64,
    green_cr: f64,
    red_cr: f64,
}

impl Conversion {
    fn new(colorimetry: VideoColorimetry) -> Result<Self, VideoPixelsError> {
        let (kr, kb) = match colorimetry.matrix() {
            VideoColorMatrix::Bt601 => (0.299, 0.114),
            VideoColorMatrix::Bt709 => (0.2126, 0.0722),
            VideoColorMatrix::Bt2020 => (0.2627, 0.0593),
            _ => return Err(VideoPixelsError::UnsupportedColorimetry),
        };
        let (y_offset, y_scale, chroma_scale) = match colorimetry.range() {
            VideoColorRange::Range0_255 => (0.0, 1.0, 1.0),
            VideoColorRange::Range16_235 => (16.0, 255.0 / 219.0, 255.0 / 224.0),
            _ => return Err(VideoPixelsError::UnsupportedColorimetry),
        };
        let kg = 1.0 - kr - kb;
        Ok(Self {
            y_offset,
            y_scale,
            blue_cb: 2.0 * (1.0 - kb) * chroma_scale,
            green_cb: -2.0 * kb * (1.0 - kb) / kg * chroma_scale,
            green_cr: -2.0 * kr * (1.0 - kr) / kg * chroma_scale,
            red_cr: 2.0 * (1.0 - kr) * chroma_scale,
        })
    }

    fn bgra(&self, y: u8, cb: u8, cr: u8) -> [u8; 4] {
        let y = (f64::from(y) - self.y_offset) * self.y_scale;
        let cb = f64::from(cb) - 128.0;
        let cr = f64::from(cr) - 128.0;
        let component = |value: f64| value.round().clamp(0.0, 255.0) as u8;
        [
            component(y + self.blue_cb * cb),
            component(y + self.green_cb * cb + self.green_cr * cr),
            component(y + self.red_cr * cr),
            255,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer_video::{VideoColorPrimaries, VideoTransferFunction};

    fn test_frame(range: VideoColorRange) -> CpuVideoFrame {
        CpuVideoFrame {
            width_px: 3,
            height_px: 3,
            offsets: [1, 18],
            strides: [5, 6],
            colorimetry: VideoColorimetry::new(
                range,
                VideoColorMatrix::Bt601,
                VideoTransferFunction::Bt709,
                VideoColorPrimaries::Bt709,
            ),
            bytes: vec![
                255, 16, 21, 26, 255, 255, 31, 36, 41, 255, 255, 46, 51, 56, 255, 255, 255, 255,
                128, 128, 128, 128, 255, 255, 128, 128, 128, 128,
            ],
        }
    }

    #[test]
    fn odd_planes_strip_padding_and_rotate_every_corner() {
        let frame = test_frame(VideoColorRange::Range0_255);
        assert_eq!(
            frame.pack_nv12().unwrap().planes,
            [vec![16, 21, 26, 31, 36, 41, 46, 51, 56], vec![128; 8]]
        );
        for (rotation, expected) in [
            (VideoRotation::Upright, [16, 21, 26, 31, 36, 41, 46, 51, 56]),
            (VideoRotation::Right, [46, 31, 16, 51, 36, 21, 56, 41, 26]),
            (VideoRotation::Down, [56, 51, 46, 41, 36, 31, 26, 21, 16]),
            (VideoRotation::Left, [26, 41, 56, 21, 36, 51, 16, 31, 46]),
        ] {
            let pixels = frame.pack_bgra(rotation).unwrap();
            assert_eq!(pixels.size_px, [3, 3]);
            for (pixel, y) in pixels.bytes.as_chunks::<4>().0.iter().zip(expected) {
                assert_eq!(*pixel, [y, y, y, 255]);
            }
        }
    }

    #[test]
    fn quarter_turn_swaps_rectangular_dimensions_without_sampling_padding() {
        let mut frame = test_frame(VideoColorRange::Range0_255);
        frame.height_px = 2;
        let packed = frame.pack_bgra(VideoRotation::Right).unwrap();
        assert_eq!(packed.size_px, [2, 3]);
        let luma: Vec<_> = packed
            .bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[0])
            .collect();
        assert_eq!(luma, [31, 16, 36, 21, 41, 26]);
    }

    #[test]
    fn limited_color_and_range_use_the_declared_matrix() {
        let mut frame = test_frame(VideoColorRange::Range16_235);
        frame.bytes[1..4].copy_from_slice(&[16, 235, 81]);
        frame.bytes[20..22].copy_from_slice(&[90, 240]);
        let bytes = frame.pack_bgra(VideoRotation::Upright).unwrap().bytes;
        assert_eq!(&bytes[..8], &[0, 0, 0, 255, 255, 255, 255, 255]);
        assert!(bytes[8] <= 2 && bytes[9] <= 2 && bytes[10] >= 252);
    }

    #[test]
    fn malformed_stride_and_short_last_chroma_row_are_rejected() {
        let mut frame = test_frame(VideoColorRange::Range0_255);
        frame.strides[1] = 3;
        assert!(matches!(
            frame.pack_nv12(),
            Err(VideoPixelsError::InvalidLayout)
        ));
        frame.strides[1] = 6;
        frame.bytes.pop();
        assert!(matches!(
            frame.pack_bgra(VideoRotation::Upright),
            Err(VideoPixelsError::InvalidLayout)
        ));
    }
}
