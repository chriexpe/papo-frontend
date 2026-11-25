//! Video frames drawn by Rune (`docs/papo-desktop.md` §7, D7c "Frames"): each `playback`
//! frame, NV12 as decoded, handed to a `rune::NativeSurface`, which keeps only the newest and
//! converts YUV to RGB in its shader. Owns the colour mapping (matrix and range, so SD clips
//! don't wash out); the frames are `playback`'s, the element that draws the surface the view's.

use std::sync::atomic::{AtomicU64, Ordering};

use papo_call::{ColourMatrix, ColourRange, ColourTransfer, VideoColour};
use rune::NativeSurface;
use rune::native_surface::{ByteFrame, ColorInfo, Format, Matrix, Transfer};

use crate::playback::{FrameSink, PackedNv12};

/// The height from which a stream that states no matrix is taken as HD (BT.709), as GStreamer
/// and players assume; below it, SD (BT.601).
const HD_HEIGHT_PX: u32 = 720;

/// A surface fed with frames.
pub(crate) struct SurfaceSink {
    surface: NativeSurface,
    next_frame: AtomicU64,
}

impl SurfaceSink {
    /// Frames into `surface` (a clone shares its slot).
    pub(crate) fn new(surface: NativeSurface) -> Self {
        Self {
            surface,
            next_frame: AtomicU64::new(1),
        }
    }
}

impl FrameSink for SurfaceSink {
    fn frame(&self, frame: PackedNv12) {
        let id = self.next_frame.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = self.surface.push_bytes(byte_frame(frame), id) {
            log::debug!("A video frame wasn't drawn: {error}");
        }
    }
}

/// `frame` as Rune's NV12 frame.
pub(crate) fn byte_frame(frame: PackedNv12) -> ByteFrame {
    let [width, height] = frame.size_px;
    let (format, color) = rune_colour(frame.colour, height);
    ByteFrame {
        width,
        height,
        format,
        color,
        planes: frame.planes,
    }
}

/// Rune's format and colour for a stream of `colour`, `height_px` tall. Rune knows BT.601 and
/// BT.709 matrices and the BT.709 and sRGB curves: BT.2020 is drawn with BT.709's matrix (a
/// slight shift, no tone mapping), and what a stream doesn't state is taken as players take it.
pub(crate) fn rune_colour(colour: VideoColour, height_px: u32) -> (Format, ColorInfo) {
    let sd = match colour.matrix {
        ColourMatrix::Bt601 => true,
        ColourMatrix::Bt709 | ColourMatrix::Bt2020 => false,
        ColourMatrix::Unknown => height_px < HD_HEIGHT_PX,
    };
    let matrix = if sd { Matrix::Bt601 } else { Matrix::Bt709 };
    let format = match colour.range {
        ColourRange::Full => Format::Nv12Full,
        ColourRange::Limited | ColourRange::Unknown => Format::Nv12Video,
    };
    let transfer = match colour.transfer {
        ColourTransfer::Srgb => Transfer::Srgb,
        ColourTransfer::Bt709 | ColourTransfer::Other => Transfer::Bt709,
    };
    (format, ColorInfo { matrix, transfer })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour(matrix: ColourMatrix, range: ColourRange) -> VideoColour {
        VideoColour {
            matrix,
            range,
            transfer: ColourTransfer::Bt709,
        }
    }

    #[test]
    fn range_and_matrix_pass_through() {
        let (format, color) = rune_colour(colour(ColourMatrix::Bt601, ColourRange::Limited), 360);
        assert_eq!(format, Format::Nv12Video);
        assert_eq!(color.matrix, Matrix::Bt601);
        let (format, color) = rune_colour(colour(ColourMatrix::Bt709, ColourRange::Full), 360);
        assert_eq!(format, Format::Nv12Full);
        assert_eq!(color.matrix, Matrix::Bt709);
    }

    #[test]
    fn an_unstated_matrix_follows_the_height() {
        let unknown = colour(ColourMatrix::Unknown, ColourRange::Unknown);
        assert_eq!(rune_colour(unknown, 360).1.matrix, Matrix::Bt601);
        assert_eq!(rune_colour(unknown, 1080).1.matrix, Matrix::Bt709);
        assert_eq!(rune_colour(unknown, 1080).0, Format::Nv12Video);
    }

    #[test]
    fn a_frame_keeps_its_planes_and_size() {
        let frame = PackedNv12 {
            size_px: [4, 2],
            colour: colour(ColourMatrix::Bt709, ColourRange::Limited),
            planes: [vec![16; 8], vec![128; 4]],
        };
        let bytes = byte_frame(frame);
        assert_eq!([bytes.width, bytes.height], [4, 2]);
        assert_eq!(bytes.planes[0].len(), 8);
        let surface = NativeSurface::new();
        assert!(surface.push_bytes(bytes, 1).is_ok());
    }
}
